use std::collections::BTreeMap;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::types::Json as SqlJson;
use sqlx::{FromRow, PgPool, SqlitePool};
use uuid::Uuid;
use vifu_gateway::optimization::{
    MetricRange, RuntimeComparisonDevice, RuntimeComparisonRunUpload, RuntimeComparisonUpload,
};

use crate::auth::{bearer_token, hash_agent_gateway_credential, Operation};
use crate::db::{self, Storage};
use crate::error::ApiError;
use crate::trace_redaction::contains_sensitive_trace_text;
use crate::AppState;

const DEFAULT_COMPARISON_LIMIT: i64 = 20;
const MAX_COMPARISON_LIMIT: i64 = 100;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeComparisonQuery {
    limit: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeComparisonUploadResponse {
    comparison_id: Uuid,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeComparisonsResponse {
    comparisons: Vec<RuntimeComparisonView>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeComparisonView {
    id: Uuid,
    project_id: Uuid,
    deployment_id: Uuid,
    gateway_id: String,
    status: String,
    recommendation: Option<String>,
    not_exhaustive: bool,
    sequential_replay: bool,
    corpus_agents: u32,
    configured_models: u32,
    tested_models: u32,
    passed_models: u32,
    device: RuntimeComparisonDevice,
    monotonic_duration_ms: u64,
    started_at: DateTime<Utc>,
    completed_at: Option<DateTime<Utc>>,
    runs: Vec<RuntimeComparisonRunView>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeComparisonRunView {
    id: Uuid,
    comparison_id: Uuid,
    combination_id: String,
    label: String,
    rule: String,
    routes: BTreeMap<String, String>,
    route_labels: BTreeMap<String, String>,
    outcome: String,
    first_total_ms: Option<u64>,
    first_run_cold: Option<bool>,
    repeat_runs_resident: Option<bool>,
    repeat_total: Option<MetricRange>,
    repeat_ttft: Option<MetricRange>,
    tokens_per_second: Option<f64>,
    first_process_cpu_percent: Option<f64>,
    process_cpu_percent: Option<f64>,
    peak_rss_bytes: Option<u64>,
    error: Option<String>,
}

pub(crate) async fn upload_runtime_comparison(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<RuntimeComparisonUpload>,
) -> Result<(StatusCode, Json<RuntimeComparisonUploadResponse>), ApiError> {
    input.validate().map_err(ApiError::Invalid)?;
    validate_comparison_errors(&input)?;
    let gateway_id = authenticated_agent_gateway(&state, &headers).await?;
    let deployment = db::list_runtime_deployments_for_gateway(&state.pool, &gateway_id)
        .await?
        .into_iter()
        .find(|deployment| deployment.id == input.deployment_id)
        .ok_or(ApiError::Forbidden)?;
    let project = db::get_project(&state.pool, deployment.project_id).await?;
    validate_project_routes(&input, &project.binding_ids)?;
    let inserted =
        insert_runtime_comparison(&state.pool, project.project.id, &gateway_id, &input).await?;
    let status = if inserted {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((
        status,
        Json(RuntimeComparisonUploadResponse {
            comparison_id: input.id,
        }),
    ))
}

fn validate_comparison_errors(input: &RuntimeComparisonUpload) -> Result<(), ApiError> {
    if input
        .runs
        .iter()
        .filter_map(|run| run.error.as_deref())
        .any(contains_sensitive_trace_text)
    {
        Err(ApiError::Invalid(
            "comparison error contains sensitive data".to_string(),
        ))
    } else {
        Ok(())
    }
}

pub(crate) async fn list_project_runtime_comparisons(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Query(query): Query<RuntimeComparisonQuery>,
) -> Result<Json<RuntimeComparisonsResponse>, ApiError> {
    let project = db::get_project_by_slug(&state.pool, &slug).await?;
    state
        .auth
        .authorize_project(
            &headers,
            Operation::ProjectRead,
            project.project.owner_user_id.as_deref(),
        )
        .await?;
    let limit = query
        .limit
        .unwrap_or(DEFAULT_COMPARISON_LIMIT)
        .clamp(1, MAX_COMPARISON_LIMIT);
    let comparisons = list_runtime_comparisons(&state.pool, project.project.id, limit).await?;
    Ok(Json(RuntimeComparisonsResponse { comparisons }))
}

async fn authenticated_agent_gateway(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<String, ApiError> {
    let credential = bearer_token(headers).ok_or(ApiError::Unauthorized)?;
    validate_agent_gateway_credential(credential)?;
    let credential_hash = hash_agent_gateway_credential(credential, &state.config.api_key_pepper);
    db::authenticate_agent_gateway_device_token(&state.pool, &credential_hash).await
}

fn validate_agent_gateway_credential(value: &str) -> Result<(), ApiError> {
    let value = value.trim();
    let secret = value
        .strip_prefix("vifu_gw_")
        .ok_or(ApiError::Unauthorized)?;
    if !(48..=256).contains(&value.len())
        || !secret
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(ApiError::Unauthorized);
    }
    Ok(())
}

fn validate_project_routes(
    comparison: &RuntimeComparisonUpload,
    project_binding_ids: &[Uuid],
) -> Result<(), ApiError> {
    for route in comparison.runs.iter().flat_map(|run| run.routes.keys()) {
        let binding_id = Uuid::parse_str(route).map_err(|_| {
            ApiError::Invalid("comparison routes must use project binding IDs".to_string())
        })?;
        if !project_binding_ids.contains(&binding_id) {
            return Err(ApiError::Forbidden);
        }
    }
    Ok(())
}

async fn insert_runtime_comparison(
    storage: &Storage,
    project_id: Uuid,
    gateway_id: &str,
    input: &RuntimeComparisonUpload,
) -> Result<bool, ApiError> {
    let prepared = PreparedComparison::new(project_id, gateway_id, input)?;
    match storage {
        Storage::Postgres(pool) => {
            insert_runtime_comparison_postgres(pool, project_id, gateway_id, input, &prepared).await
        }
        Storage::Sqlite(pool) => {
            insert_runtime_comparison_sqlite(pool, project_id, gateway_id, input, &prepared).await
        }
    }
}

async fn list_runtime_comparisons(
    storage: &Storage,
    project_id: Uuid,
    limit: i64,
) -> Result<Vec<RuntimeComparisonView>, ApiError> {
    match storage {
        Storage::Postgres(pool) => list_runtime_comparisons_postgres(pool, project_id, limit).await,
        Storage::Sqlite(pool) => list_runtime_comparisons_sqlite(pool, project_id, limit).await,
    }
}

struct PreparedComparison {
    content_hash: String,
    corpus_agents: i32,
    configured_models: i32,
    tested_models: i32,
    passed_models: i32,
    monotonic_duration_ms: i64,
    started_at: DateTime<Utc>,
    completed_at: Option<DateTime<Utc>>,
    runs: Vec<PreparedRun>,
}

impl PreparedComparison {
    fn new(
        project_id: Uuid,
        gateway_id: &str,
        input: &RuntimeComparisonUpload,
    ) -> Result<Self, ApiError> {
        let started_at = timestamp(input.started_at_ms)?;
        let completed_at = input.completed_at_ms.map(timestamp).transpose()?;
        let runs = input
            .runs
            .iter()
            .map(PreparedRun::new)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            content_hash: comparison_content_hash(project_id, gateway_id, input)?,
            corpus_agents: database_i32(input.corpus_agents)?,
            configured_models: database_i32(input.configured_models)?,
            tested_models: database_i32(input.tested_models)?,
            passed_models: database_i32(input.passed_models)?,
            monotonic_duration_ms: database_i64(input.monotonic_duration_ms)?,
            started_at,
            completed_at,
            runs,
        })
    }
}

struct PreparedRun {
    first_total_ms: Option<i64>,
    repeat_total: PreparedMetricRange,
    repeat_ttft: PreparedMetricRange,
    peak_rss_bytes: Option<i64>,
}

impl PreparedRun {
    fn new(input: &RuntimeComparisonRunUpload) -> Result<Self, ApiError> {
        Ok(Self {
            first_total_ms: input.first_total_ms.map(database_i64).transpose()?,
            repeat_total: PreparedMetricRange::new(input.repeat_total.as_ref())?,
            repeat_ttft: PreparedMetricRange::new(input.repeat_ttft.as_ref())?,
            peak_rss_bytes: input.peak_rss_bytes.map(database_i64).transpose()?,
        })
    }
}

#[derive(Default)]
struct PreparedMetricRange {
    median: Option<i64>,
    min: Option<i64>,
    max: Option<i64>,
    samples: Option<i32>,
}

impl PreparedMetricRange {
    fn new(range: Option<&MetricRange>) -> Result<Self, ApiError> {
        let Some(range) = range else {
            return Ok(Self::default());
        };
        Ok(Self {
            median: Some(database_i64(range.median)?),
            min: Some(database_i64(range.min)?),
            max: Some(database_i64(range.max)?),
            samples: Some(i32::try_from(range.samples).map_err(|_| ApiError::Internal)?),
        })
    }
}

async fn insert_runtime_comparison_postgres(
    pool: &PgPool,
    project_id: Uuid,
    gateway_id: &str,
    input: &RuntimeComparisonUpload,
    prepared: &PreparedComparison,
) -> Result<bool, ApiError> {
    let mut transaction = pool.begin().await?;
    let result = sqlx::query(
        "INSERT INTO comparisons(
            id, project_id, deployment_id, gateway_id, content_hash, status, recommendation,
            not_exhaustive, sequential_replay, corpus_agents, configured_models,
            tested_models, passed_models, device_architecture, device_backend, device_os,
            monotonic_duration_ms, started_at, completed_at
         ) VALUES (
            $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14,
            $15, $16, $17, $18, $19
         ) ON CONFLICT (id) DO NOTHING",
    )
    .bind(input.id)
    .bind(project_id)
    .bind(input.deployment_id)
    .bind(gateway_id)
    .bind(&prepared.content_hash)
    .bind(input.status.as_str())
    .bind(input.recommendation.as_deref())
    .bind(input.not_exhaustive)
    .bind(input.sequential_replay)
    .bind(prepared.corpus_agents)
    .bind(prepared.configured_models)
    .bind(prepared.tested_models)
    .bind(prepared.passed_models)
    .bind(&input.device.architecture)
    .bind(input.device.backend.as_deref())
    .bind(input.device.os.as_deref())
    .bind(prepared.monotonic_duration_ms)
    .bind(prepared.started_at)
    .bind(prepared.completed_at)
    .execute(&mut *transaction)
    .await?;
    if result.rows_affected() == 0 {
        verify_existing_comparison(
            sqlx::query_as::<_, ExistingComparison>(
                "SELECT project_id, deployment_id, gateway_id, content_hash
                 FROM comparisons WHERE id = $1",
            )
            .bind(input.id)
            .fetch_one(&mut *transaction)
            .await?,
            project_id,
            input.deployment_id,
            gateway_id,
            &prepared.content_hash,
        )?;
        transaction.commit().await?;
        return Ok(false);
    }
    for (position, (run, prepared_run)) in input.runs.iter().zip(&prepared.runs).enumerate() {
        insert_runtime_comparison_run_postgres(
            &mut transaction,
            input.id,
            position,
            run,
            prepared_run,
        )
        .await?;
    }
    transaction.commit().await?;
    Ok(true)
}

async fn insert_runtime_comparison_run_postgres(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    comparison_id: Uuid,
    position: usize,
    input: &RuntimeComparisonRunUpload,
    prepared: &PreparedRun,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO comparison_runs(
            id, comparison_id, position, combination_id, label, rule, routes, route_labels, outcome,
            first_total_ms, first_run_cold, repeat_runs_resident,
            repeat_total_median_ms, repeat_total_min_ms, repeat_total_max_ms,
            repeat_total_samples, repeat_ttft_median_ms, repeat_ttft_min_ms,
            repeat_ttft_max_ms, repeat_ttft_samples, tokens_per_second,
            first_process_cpu_percent, process_cpu_percent, peak_rss_bytes, error
         ) VALUES (
            $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14,
            $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25
         )",
    )
    .bind(input.id)
    .bind(comparison_id)
    .bind(i32::try_from(position).map_err(|_| ApiError::Internal)?)
    .bind(&input.combination_id)
    .bind(&input.label)
    .bind(&input.rule)
    .bind(SqlJson(&input.routes))
    .bind(SqlJson(&input.route_labels))
    .bind(input.outcome.as_str())
    .bind(prepared.first_total_ms)
    .bind(input.first_run_cold)
    .bind(input.repeat_runs_resident)
    .bind(prepared.repeat_total.median)
    .bind(prepared.repeat_total.min)
    .bind(prepared.repeat_total.max)
    .bind(prepared.repeat_total.samples)
    .bind(prepared.repeat_ttft.median)
    .bind(prepared.repeat_ttft.min)
    .bind(prepared.repeat_ttft.max)
    .bind(prepared.repeat_ttft.samples)
    .bind(input.tokens_per_second)
    .bind(input.first_process_cpu_percent)
    .bind(input.process_cpu_percent)
    .bind(prepared.peak_rss_bytes)
    .bind(input.error.as_deref())
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn insert_runtime_comparison_sqlite(
    pool: &SqlitePool,
    project_id: Uuid,
    gateway_id: &str,
    input: &RuntimeComparisonUpload,
    prepared: &PreparedComparison,
) -> Result<bool, ApiError> {
    let mut transaction = pool.begin().await?;
    let result = sqlx::query(
        "INSERT OR IGNORE INTO comparisons(
            id, project_id, deployment_id, gateway_id, content_hash, status, recommendation,
            not_exhaustive, sequential_replay, corpus_agents, configured_models,
            tested_models, passed_models, device_architecture, device_backend, device_os,
            monotonic_duration_ms, started_at, completed_at
         ) VALUES (
            $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14,
            $15, $16, $17, $18, $19
         )",
    )
    .bind(input.id)
    .bind(project_id)
    .bind(input.deployment_id)
    .bind(gateway_id)
    .bind(&prepared.content_hash)
    .bind(input.status.as_str())
    .bind(input.recommendation.as_deref())
    .bind(input.not_exhaustive)
    .bind(input.sequential_replay)
    .bind(prepared.corpus_agents)
    .bind(prepared.configured_models)
    .bind(prepared.tested_models)
    .bind(prepared.passed_models)
    .bind(&input.device.architecture)
    .bind(input.device.backend.as_deref())
    .bind(input.device.os.as_deref())
    .bind(prepared.monotonic_duration_ms)
    .bind(prepared.started_at)
    .bind(prepared.completed_at)
    .execute(&mut *transaction)
    .await?;
    if result.rows_affected() == 0 {
        verify_existing_comparison(
            sqlx::query_as::<_, ExistingComparison>(
                "SELECT project_id, deployment_id, gateway_id, content_hash
                 FROM comparisons WHERE id = $1",
            )
            .bind(input.id)
            .fetch_one(&mut *transaction)
            .await?,
            project_id,
            input.deployment_id,
            gateway_id,
            &prepared.content_hash,
        )?;
        transaction.commit().await?;
        return Ok(false);
    }
    for (position, (run, prepared_run)) in input.runs.iter().zip(&prepared.runs).enumerate() {
        insert_runtime_comparison_run_sqlite(
            &mut transaction,
            input.id,
            position,
            run,
            prepared_run,
        )
        .await?;
    }
    transaction.commit().await?;
    Ok(true)
}

async fn insert_runtime_comparison_run_sqlite(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    comparison_id: Uuid,
    position: usize,
    input: &RuntimeComparisonRunUpload,
    prepared: &PreparedRun,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO comparison_runs(
            id, comparison_id, position, combination_id, label, rule, routes, route_labels, outcome,
            first_total_ms, first_run_cold, repeat_runs_resident,
            repeat_total_median_ms, repeat_total_min_ms, repeat_total_max_ms,
            repeat_total_samples, repeat_ttft_median_ms, repeat_ttft_min_ms,
            repeat_ttft_max_ms, repeat_ttft_samples, tokens_per_second,
            first_process_cpu_percent, process_cpu_percent, peak_rss_bytes, error
         ) VALUES (
            $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14,
            $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25
         )",
    )
    .bind(input.id)
    .bind(comparison_id)
    .bind(i32::try_from(position).map_err(|_| ApiError::Internal)?)
    .bind(&input.combination_id)
    .bind(&input.label)
    .bind(&input.rule)
    .bind(SqlJson(&input.routes))
    .bind(SqlJson(&input.route_labels))
    .bind(input.outcome.as_str())
    .bind(prepared.first_total_ms)
    .bind(input.first_run_cold)
    .bind(input.repeat_runs_resident)
    .bind(prepared.repeat_total.median)
    .bind(prepared.repeat_total.min)
    .bind(prepared.repeat_total.max)
    .bind(prepared.repeat_total.samples)
    .bind(prepared.repeat_ttft.median)
    .bind(prepared.repeat_ttft.min)
    .bind(prepared.repeat_ttft.max)
    .bind(prepared.repeat_ttft.samples)
    .bind(input.tokens_per_second)
    .bind(input.first_process_cpu_percent)
    .bind(input.process_cpu_percent)
    .bind(prepared.peak_rss_bytes)
    .bind(input.error.as_deref())
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

#[derive(Debug, FromRow)]
struct ExistingComparison {
    project_id: Uuid,
    deployment_id: Uuid,
    gateway_id: String,
    content_hash: String,
}

fn verify_existing_comparison(
    existing: ExistingComparison,
    project_id: Uuid,
    deployment_id: Uuid,
    gateway_id: &str,
    content_hash: &str,
) -> Result<(), ApiError> {
    if existing.project_id == project_id
        && existing.deployment_id == deployment_id
        && existing.gateway_id == gateway_id
        && existing.content_hash == content_hash
    {
        Ok(())
    } else if existing.project_id == project_id
        && existing.deployment_id == deployment_id
        && existing.gateway_id == gateway_id
    {
        Err(ApiError::Conflict(
            "comparison ID already contains different evidence".to_string(),
        ))
    } else {
        Err(ApiError::Forbidden)
    }
}

#[derive(Debug, FromRow)]
struct ComparisonRow {
    id: Uuid,
    project_id: Uuid,
    deployment_id: Uuid,
    gateway_id: String,
    status: String,
    recommendation: Option<String>,
    not_exhaustive: bool,
    sequential_replay: bool,
    corpus_agents: i32,
    configured_models: i32,
    tested_models: i32,
    passed_models: i32,
    device_architecture: String,
    device_backend: Option<String>,
    device_os: Option<String>,
    monotonic_duration_ms: i64,
    started_at: DateTime<Utc>,
    completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, FromRow)]
struct ComparisonRunRow {
    id: Uuid,
    comparison_id: Uuid,
    combination_id: String,
    label: String,
    rule: String,
    routes: SqlJson<BTreeMap<String, String>>,
    route_labels: SqlJson<BTreeMap<String, String>>,
    outcome: String,
    first_total_ms: Option<i64>,
    first_run_cold: Option<bool>,
    repeat_runs_resident: Option<bool>,
    repeat_total_median_ms: Option<i64>,
    repeat_total_min_ms: Option<i64>,
    repeat_total_max_ms: Option<i64>,
    repeat_total_samples: Option<i32>,
    repeat_ttft_median_ms: Option<i64>,
    repeat_ttft_min_ms: Option<i64>,
    repeat_ttft_max_ms: Option<i64>,
    repeat_ttft_samples: Option<i32>,
    tokens_per_second: Option<f64>,
    first_process_cpu_percent: Option<f64>,
    process_cpu_percent: Option<f64>,
    peak_rss_bytes: Option<i64>,
    error: Option<String>,
}

const SELECT_COMPARISONS: &str =
    "SELECT id, project_id, deployment_id, gateway_id, status, recommendation,
            not_exhaustive, sequential_replay, corpus_agents, configured_models,
            tested_models, passed_models, device_architecture, device_backend, device_os,
            monotonic_duration_ms, started_at, completed_at
     FROM comparisons
     WHERE project_id = $1
     ORDER BY started_at DESC, id DESC
     LIMIT $2";

const SELECT_COMPARISON_RUNS: &str =
    "SELECT id, comparison_id, combination_id, label, rule, routes, route_labels, outcome,
            first_total_ms, first_run_cold, repeat_runs_resident,
            repeat_total_median_ms, repeat_total_min_ms, repeat_total_max_ms,
            repeat_total_samples, repeat_ttft_median_ms, repeat_ttft_min_ms,
            repeat_ttft_max_ms, repeat_ttft_samples, tokens_per_second,
            first_process_cpu_percent, process_cpu_percent, peak_rss_bytes, error
     FROM comparison_runs
     WHERE comparison_id = $1
     ORDER BY position ASC";

async fn list_runtime_comparisons_postgres(
    pool: &PgPool,
    project_id: Uuid,
    limit: i64,
) -> Result<Vec<RuntimeComparisonView>, ApiError> {
    let rows = sqlx::query_as::<_, ComparisonRow>(SELECT_COMPARISONS)
        .bind(project_id)
        .bind(limit)
        .fetch_all(pool)
        .await?;
    let mut comparisons = Vec::with_capacity(rows.len());
    for row in rows {
        let runs = sqlx::query_as::<_, ComparisonRunRow>(SELECT_COMPARISON_RUNS)
            .bind(row.id)
            .fetch_all(pool)
            .await?;
        comparisons.push(comparison_view(row, runs)?);
    }
    Ok(comparisons)
}

async fn list_runtime_comparisons_sqlite(
    pool: &SqlitePool,
    project_id: Uuid,
    limit: i64,
) -> Result<Vec<RuntimeComparisonView>, ApiError> {
    let rows = sqlx::query_as::<_, ComparisonRow>(SELECT_COMPARISONS)
        .bind(project_id)
        .bind(limit)
        .fetch_all(pool)
        .await?;
    let mut comparisons = Vec::with_capacity(rows.len());
    for row in rows {
        let runs = sqlx::query_as::<_, ComparisonRunRow>(SELECT_COMPARISON_RUNS)
            .bind(row.id)
            .fetch_all(pool)
            .await?;
        comparisons.push(comparison_view(row, runs)?);
    }
    Ok(comparisons)
}

fn comparison_view(
    comparison: ComparisonRow,
    runs: Vec<ComparisonRunRow>,
) -> Result<RuntimeComparisonView, ApiError> {
    Ok(RuntimeComparisonView {
        id: comparison.id,
        project_id: comparison.project_id,
        deployment_id: comparison.deployment_id,
        gateway_id: comparison.gateway_id,
        status: comparison.status,
        recommendation: comparison.recommendation,
        not_exhaustive: comparison.not_exhaustive,
        sequential_replay: comparison.sequential_replay,
        corpus_agents: database_u32(comparison.corpus_agents)?,
        configured_models: database_u32(comparison.configured_models)?,
        tested_models: database_u32(comparison.tested_models)?,
        passed_models: database_u32(comparison.passed_models)?,
        device: RuntimeComparisonDevice {
            architecture: comparison.device_architecture,
            backend: comparison.device_backend,
            os: comparison.device_os,
        },
        monotonic_duration_ms: database_u64(comparison.monotonic_duration_ms)?,
        started_at: comparison.started_at,
        completed_at: comparison.completed_at,
        runs: runs
            .into_iter()
            .map(comparison_run_view)
            .collect::<Result<Vec<_>, _>>()?,
    })
}

fn comparison_run_view(run: ComparisonRunRow) -> Result<RuntimeComparisonRunView, ApiError> {
    Ok(RuntimeComparisonRunView {
        id: run.id,
        comparison_id: run.comparison_id,
        combination_id: run.combination_id,
        label: run.label,
        rule: run.rule,
        routes: run.routes.0,
        route_labels: run.route_labels.0,
        outcome: run.outcome,
        first_total_ms: run.first_total_ms.map(database_u64).transpose()?,
        first_run_cold: run.first_run_cold,
        repeat_runs_resident: run.repeat_runs_resident,
        repeat_total: metric_range(
            run.repeat_total_median_ms,
            run.repeat_total_min_ms,
            run.repeat_total_max_ms,
            run.repeat_total_samples,
        )?,
        repeat_ttft: metric_range(
            run.repeat_ttft_median_ms,
            run.repeat_ttft_min_ms,
            run.repeat_ttft_max_ms,
            run.repeat_ttft_samples,
        )?,
        tokens_per_second: run.tokens_per_second,
        first_process_cpu_percent: run.first_process_cpu_percent,
        process_cpu_percent: run.process_cpu_percent,
        peak_rss_bytes: run.peak_rss_bytes.map(database_u64).transpose()?,
        error: run.error,
    })
}

fn metric_range(
    median: Option<i64>,
    min: Option<i64>,
    max: Option<i64>,
    samples: Option<i32>,
) -> Result<Option<MetricRange>, ApiError> {
    match (median, min, max, samples) {
        (None, None, None, None) => Ok(None),
        (Some(median), Some(min), Some(max), Some(samples)) => Ok(Some(MetricRange {
            median: database_u64(median)?,
            min: database_u64(min)?,
            max: database_u64(max)?,
            samples: u32::try_from(samples).map_err(|_| ApiError::Internal)?,
        })),
        _ => Err(ApiError::Internal),
    }
}

fn timestamp(milliseconds: u64) -> Result<DateTime<Utc>, ApiError> {
    let milliseconds = database_i64(milliseconds)?;
    DateTime::from_timestamp_millis(milliseconds).ok_or(ApiError::Internal)
}

fn comparison_content_hash(
    project_id: Uuid,
    gateway_id: &str,
    input: &RuntimeComparisonUpload,
) -> Result<String, ApiError> {
    let mut hasher = Sha256::new();
    hasher.update(b"vifu-runtime-comparison-v1\0");
    hasher.update(project_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(gateway_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(serde_json::to_vec(input).map_err(|_| ApiError::Internal)?);
    let digest = hasher.finalize();
    let mut encoded = String::with_capacity(digest.len() * 2);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in digest {
        encoded.push(char::from(HEX[(byte >> 4) as usize]));
        encoded.push(char::from(HEX[(byte & 0x0f) as usize]));
    }
    Ok(encoded)
}

fn database_i64(value: u64) -> Result<i64, ApiError> {
    i64::try_from(value).map_err(|_| ApiError::Internal)
}

fn database_i32(value: u32) -> Result<i32, ApiError> {
    i32::try_from(value).map_err(|_| ApiError::Internal)
}

fn database_u64(value: i64) -> Result<u64, ApiError> {
    u64::try_from(value).map_err(|_| ApiError::Internal)
}

fn database_u32(value: i32) -> Result<u32, ApiError> {
    u32::try_from(value).map_err(|_| ApiError::Internal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::NewProject;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use serde_json::{json, Value};
    use tower::ServiceExt;
    use vifu_gateway::optimization::{RuntimeComparisonOutcome, RuntimeComparisonStatus};

    async fn sqlite_storage() -> Storage {
        let storage = db::connect("sqlite::memory:", 1).await.unwrap();
        db::migrate(&storage).await.unwrap();
        storage
    }

    fn upload(deployment_id: Uuid) -> RuntimeComparisonUpload {
        let route_id = Uuid::new_v4().to_string();
        RuntimeComparisonUpload {
            id: Uuid::new_v4(),
            deployment_id,
            status: RuntimeComparisonStatus::Completed,
            recommendation: Some("fastest-local".to_string()),
            not_exhaustive: true,
            sequential_replay: true,
            corpus_agents: 1,
            configured_models: 2,
            tested_models: 2,
            passed_models: 1,
            device: RuntimeComparisonDevice {
                architecture: "aarch64".to_string(),
                backend: Some("llama.cpp".to_string()),
                os: Some("linux".to_string()),
            },
            started_at_ms: 1_700_000_000_000,
            completed_at_ms: Some(1_700_000_001_000),
            monotonic_duration_ms: 900,
            runs: vec![RuntimeComparisonRunUpload {
                id: Uuid::new_v4(),
                combination_id: "fastest-local".to_string(),
                label: "fastest-local".to_string(),
                rule: "Fastest passing configured local candidate".to_string(),
                routes: BTreeMap::from([(route_id.clone(), "qwen-2b".to_string())]),
                route_labels: BTreeMap::from([(route_id, "NPC planner · chat".to_string())]),
                outcome: RuntimeComparisonOutcome::Passed,
                first_total_ms: Some(420),
                first_run_cold: Some(true),
                repeat_runs_resident: Some(true),
                repeat_total: Some(MetricRange {
                    median: 200,
                    min: 190,
                    max: 220,
                    samples: 3,
                }),
                repeat_ttft: Some(MetricRange {
                    median: 50,
                    min: 45,
                    max: 55,
                    samples: 3,
                }),
                tokens_per_second: Some(32.5),
                first_process_cpu_percent: Some(190.0),
                process_cpu_percent: Some(178.0),
                peak_rss_bytes: Some(512 * 1024 * 1024),
                error: None,
            }],
        }
    }

    #[test]
    fn comparison_error_boundary_reuses_trace_secret_detection() {
        let mut input = upload(Uuid::new_v4());
        input.status = RuntimeComparisonStatus::Failed;
        input.recommendation = None;
        input.runs[0].outcome = RuntimeComparisonOutcome::Failed;
        input.runs[0].error = Some("password: private-value".to_string());

        assert!(matches!(
            validate_comparison_errors(&input),
            Err(ApiError::Invalid(message)) if message == "comparison error contains sensitive data"
        ));
    }

    #[tokio::test]
    async fn sqlite_comparison_storage_round_trips_dashboard_contract() {
        let storage = sqlite_storage().await;
        let project_id = Uuid::new_v4();
        db::create_project(
            &storage,
            NewProject {
                id: project_id,
                owner_user_id: None,
                slug: "arm-comparison",
                name: "ARM comparison",
                description: None,
                gateway_id: "gateway-arm",
                binding_ids: &[],
            },
        )
        .await
        .unwrap();
        let deployment_id = db::list_runtime_deployments(&storage, project_id)
            .await
            .unwrap()[0]
            .id;
        let input = upload(deployment_id);

        assert!(
            insert_runtime_comparison(&storage, project_id, "gateway-arm", &input)
                .await
                .unwrap()
        );
        assert!(
            !insert_runtime_comparison(&storage, project_id, "gateway-arm", &input)
                .await
                .unwrap()
        );
        let mut conflicting = input.clone();
        conflicting.runs[0].tokens_per_second = Some(99.0);
        assert!(matches!(
            insert_runtime_comparison(&storage, project_id, "gateway-arm", &conflicting).await,
            Err(ApiError::Conflict(_))
        ));
        let records = list_runtime_comparisons(&storage, project_id, 20)
            .await
            .unwrap();

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].id, input.id);
        assert_eq!(records[0].monotonic_duration_ms, 900);
        assert_eq!(
            records[0].started_at.timestamp_millis(),
            input.started_at_ms as i64
        );
        assert_eq!(
            records[0].completed_at.unwrap().timestamp_millis(),
            input.completed_at_ms.unwrap() as i64
        );
        assert_eq!(records[0].runs.len(), 1);
        assert_eq!(records[0].runs[0].process_cpu_percent, Some(178.0));
        assert_eq!(records[0].runs[0].repeat_total.as_ref().unwrap().samples, 3);

        let Storage::Sqlite(pool) = &storage else {
            panic!("test storage must use SQLite");
        };
        let incomplete =
            sqlx::query("UPDATE comparison_runs SET repeat_total_samples = 2 WHERE id = $1")
                .bind(input.runs[0].id)
                .execute(pool)
                .await;
        assert!(
            incomplete.is_err(),
            "passing SQL evidence must retain all three warm samples"
        );
    }

    async fn project_with_binding(
        storage: &Storage,
        slug: &str,
        gateway_id: &str,
    ) -> (Uuid, Uuid, Uuid) {
        let project_id = Uuid::new_v4();
        db::create_project(
            storage,
            NewProject {
                id: project_id,
                owner_user_id: None,
                slug,
                name: "ARM comparison",
                description: None,
                gateway_id,
                binding_ids: &[],
            },
        )
        .await
        .unwrap();
        let profile_id = Uuid::new_v4();
        db::create_profile(storage, profile_id, project_id, "planner", "Planner", None)
            .await
            .unwrap();
        let binding_id = Uuid::new_v4();
        db::create_binding(
            storage,
            binding_id,
            profile_id,
            "vifu-runtime",
            gateway_id,
            "planner",
            &json!({"providerKey": "qwen-2b"}),
        )
        .await
        .unwrap();
        db::attach_project_binding(storage, project_id, binding_id)
            .await
            .unwrap();
        let deployment_id = db::list_runtime_deployments(storage, project_id)
            .await
            .unwrap()[0]
            .id;
        (project_id, deployment_id, binding_id)
    }

    #[tokio::test]
    async fn comparison_routes_require_device_deployment_and_project_binding() {
        let storage = sqlite_storage().await;
        let gateway_id = "gateway-arm-api";
        let (project_id, deployment_id, binding_id) =
            project_with_binding(&storage, "arm-api", gateway_id).await;
        let (_other_project_id, other_deployment_id, _other_binding_id) =
            project_with_binding(&storage, "other-arm-api", gateway_id).await;

        let mut config = crate::config::Config::from_env().unwrap();
        let admin_key = config.admin_key.clone();
        let device_token = format!("vifu_gw_{}", "a".repeat(64));
        let token_hash = hash_agent_gateway_credential(&device_token, &config.api_key_pepper);
        db::upsert_agent_gateway_machine(&storage, "machine-arm-api", "public-key-arm-api")
            .await
            .unwrap();
        db::create_agent_gateway_authorization(
            &storage,
            db::NewAgentGatewayAuthorization {
                gateway_id,
                machine_id: "machine-arm-api",
                owner_user_id: None,
                token_prefix: "vifu_gw_aaaaaaaaaa",
                token_hash: &token_hash,
                token_expires_at: Utc::now() + chrono::Duration::hours(1),
            },
        )
        .await
        .unwrap();
        config.database_url = "sqlite::memory:".to_string();
        let service = crate::app(crate::state_with_storage(config, storage.clone()));

        let mut input = upload(deployment_id);
        input.runs[0].routes = BTreeMap::from([(binding_id.to_string(), "qwen-2b".to_string())]);
        input.runs[0].route_labels =
            BTreeMap::from([(binding_id.to_string(), "NPC planner · chat".to_string())]);
        let response = service
            .clone()
            .oneshot(
                Request::post("/v1/agent-gateway/runtime-comparisons")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {device_token}"))
                    .body(Body::from(serde_json::to_vec(&input).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);

        let response = service
            .clone()
            .oneshot(
                Request::get("/v1/project/arm-api/comparisons?limit=20")
                    .header("authorization", format!("Bearer {admin_key}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 256 * 1024).await.unwrap();
        let payload: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            payload["comparisons"][0]["projectId"],
            project_id.to_string()
        );
        assert_eq!(payload["comparisons"][0]["monotonicDurationMs"], 900);
        assert_eq!(
            payload["comparisons"][0]["startedAt"],
            "2023-11-14T22:13:20Z"
        );
        assert_eq!(
            payload["comparisons"][0]["completedAt"],
            "2023-11-14T22:13:21Z"
        );
        assert_eq!(
            payload["comparisons"][0]["runs"][0]["processCpuPercent"],
            178.0
        );
        assert_eq!(
            payload["comparisons"][0]["runs"][0]["firstProcessCpuPercent"],
            190.0
        );
        assert_eq!(
            payload["comparisons"][0]["runs"][0]["routeLabels"][binding_id.to_string()],
            "NPC planner · chat"
        );

        input.id = Uuid::new_v4();
        input.deployment_id = other_deployment_id;
        input.runs[0].id = Uuid::new_v4();
        let response = service
            .oneshot(
                Request::post("/v1/agent-gateway/runtime-comparisons")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {device_token}"))
                    .body(Body::from(serde_json::to_vec(&input).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
}
