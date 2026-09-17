//! Code deployment metadata for hosted Apps. Runtime configuration releases
//! remain independent in `project_runtime_releases`.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use crate::auth::{Identity, Operation};
use crate::db::Storage;
use crate::error::ApiError;
use crate::AppState;

#[derive(Debug, Clone, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeRelease {
    pub release_id: String,
    pub project_id: Uuid,
    pub deployment_id: Uuid,
    pub status: String,
    pub python_version: String,
    pub dependency_strategy: String,
    pub integration: String,
    pub http_target: Option<String>,
    pub http_interface: Option<String>,
    pub source_key: String,
    pub source_sha256: String,
    pub source_crc32c: String,
    pub source_size_bytes: i64,
    pub source_bytes: i64,
    pub file_count: i64,
    pub build_ref: Option<String>,
    pub artifact_digest: Option<String>,
    pub service_origin: Option<String>,
    pub revision_origin: Option<String>,
    pub failure_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateCodeRelease {
    pub release_id: String,
    pub deployment_id: Uuid,
    pub python_version: String,
    pub dependency_strategy: String,
    pub integration: String,
    pub http_target: Option<String>,
    pub http_interface: Option<String>,
    pub source_key: String,
    pub source_sha256: String,
    pub source_crc32c: String,
    pub source_size_bytes: i64,
    pub source_bytes: i64,
    pub file_count: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateCodeRelease {
    pub status: String,
    pub build_ref: Option<String>,
    pub artifact_digest: Option<String>,
    pub service_origin: Option<String>,
    pub revision_origin: Option<String>,
    pub failure_message: Option<String>,
}

fn hosted_pool(storage: &Storage) -> Result<&PgPool, ApiError> {
    match storage {
        Storage::Postgres(pool) => Ok(pool),
        Storage::Sqlite(_) => Err(ApiError::Invalid(
            "Hosted code releases require a PostgreSQL Vifu Server".to_owned(),
        )),
    }
}

fn validate_release_id(value: &str) -> Result<(), ApiError> {
    if value.len() != 36
        || !value.starts_with("rel_")
        || !value[4..].bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(ApiError::Invalid("releaseId is invalid".to_owned()));
    }
    Ok(())
}

fn validate_create(input: &CreateCodeRelease) -> Result<(), ApiError> {
    validate_release_id(&input.release_id)?;
    if !matches!(
        input.python_version.as_str(),
        "3.10" | "3.11" | "3.12" | "3.13" | "3.14"
    ) {
        return Err(ApiError::Invalid("pythonVersion is invalid".to_owned()));
    }
    if !matches!(
        input.dependency_strategy.as_str(),
        "none" | "uv.lock" | "pyproject.toml" | "requirements.txt"
    ) {
        return Err(ApiError::Invalid(
            "dependencyStrategy is invalid".to_owned(),
        ));
    }
    if !matches!(input.integration.as_str(), "vifu" | "web") {
        return Err(ApiError::Invalid("integration is invalid".to_owned()));
    }
    if input.integration == "web"
        && (input.http_target.is_none()
            || !matches!(input.http_interface.as_deref(), Some("asgi" | "wsgi")))
        || input.integration == "vifu"
            && (input.http_target.is_some() || input.http_interface.is_some())
    {
        return Err(ApiError::Invalid(
            "HTTP entrypoint does not match App integration".to_owned(),
        ));
    }
    if input.source_size_bytes <= 0 || input.source_bytes <= 0 || input.file_count <= 0 {
        return Err(ApiError::Invalid("source metadata is invalid".to_owned()));
    }
    Ok(())
}

async fn owned_project(
    state: &AppState,
    headers: &HeaderMap,
    slug: &str,
    operation: Operation,
) -> Result<crate::models::ProjectWithBindings, ApiError> {
    let project = crate::db::get_project_by_slug(&state.pool, slug).await?;
    state
        .auth
        .authorize_project(headers, operation, project.project.owner_user_id.as_deref())
        .await?;
    Ok(project)
}

pub async fn list(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let project = owned_project(&state, &headers, &slug, Operation::ProjectRead).await?;
    let pool = hosted_pool(&state.pool)?;
    let releases = sqlx::query_as::<_, CodeRelease>(
        "WITH recent AS (
            SELECT * FROM project_code_releases WHERE project_id = $1 ORDER BY created_at DESC LIMIT 100
         )
         SELECT * FROM recent UNION
         SELECT * FROM project_code_releases WHERE project_id = $1 AND release_id = (
            SELECT active_release_id FROM project_code_deployment_state WHERE deployment_id =
                (SELECT id FROM runtime_deployments WHERE project_id = $1 AND is_primary)
         ) ORDER BY created_at DESC",
    ).bind(project.project.id).fetch_all(pool).await?;
    let state = sqlx::query_as::<_, (Option<String>, Option<String>)>(
        "SELECT active_release_id, pending_release_id FROM project_code_deployment_state WHERE deployment_id = \
         (SELECT id FROM runtime_deployments WHERE project_id = $1 AND is_primary)",
    )
    .bind(project.project.id)
    .fetch_optional(pool)
    .await?;
    let (active, pending) = state.unwrap_or((None, None));
    Ok(Json(
        json!({ "releases": releases, "activeReleaseId": active, "pendingReleaseId": pending }),
    ))
}

pub async fn create(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Json(input): Json<CreateCodeRelease>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let project = owned_project(&state, &headers, &slug, Operation::ProjectWrite).await?;
    validate_create(&input)?;
    let pool = hosted_pool(&state.pool)?;
    let deployment_exists = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)::BIGINT FROM runtime_deployments WHERE id = $1 AND project_id = $2",
    )
    .bind(input.deployment_id)
    .bind(project.project.id)
    .fetch_one(pool)
    .await?
        > 0;
    if !deployment_exists {
        return Err(ApiError::NotFound);
    }
    let mut tx = pool.begin().await?;
    let release = sqlx::query_as::<_, CodeRelease>(
        "INSERT INTO project_code_releases (
            release_id, project_id, deployment_id, status, python_version, dependency_strategy,
            integration, http_target, http_interface, source_key, source_sha256, source_crc32c,
            source_size_bytes, source_bytes, file_count
         ) VALUES ($1, $2, $3, 'uploading', $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
         RETURNING *",
    )
    .bind(&input.release_id)
    .bind(project.project.id)
    .bind(input.deployment_id)
    .bind(&input.python_version)
    .bind(&input.dependency_strategy)
    .bind(&input.integration)
    .bind(&input.http_target)
    .bind(&input.http_interface)
    .bind(&input.source_key)
    .bind(&input.source_sha256)
    .bind(&input.source_crc32c)
    .bind(input.source_size_bytes)
    .bind(input.source_bytes)
    .bind(input.file_count)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO project_code_deployment_state(deployment_id, pending_release_id)
         VALUES ($1, $2) ON CONFLICT (deployment_id) DO UPDATE
         SET pending_release_id = EXCLUDED.pending_release_id, updated_at = NOW()",
    )
    .bind(input.deployment_id)
    .bind(&input.release_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(json!({ "release": release }))))
}

pub async fn get(
    State(state): State<AppState>,
    Path((slug, release_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let project = owned_project(&state, &headers, &slug, Operation::ProjectRead).await?;
    validate_release_id(&release_id)?;
    let release = get_owned(hosted_pool(&state.pool)?, project.project.id, &release_id).await?;
    Ok(Json(json!({ "release": release })))
}

pub async fn update(
    State(state): State<AppState>,
    Path((slug, release_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(input): Json<UpdateCodeRelease>,
) -> Result<Json<Value>, ApiError> {
    let project = owned_project(&state, &headers, &slug, Operation::ProjectWrite).await?;
    let identity = state
        .auth
        .authorize_project(
            &headers,
            Operation::ProjectWrite,
            project.project.owner_user_id.as_deref(),
        )
        .await?;
    if identity != Identity::DeploymentAdmin {
        return Err(ApiError::Forbidden);
    }
    validate_release_id(&release_id)?;
    let pool = hosted_pool(&state.pool)?;
    let current = get_owned(pool, project.project.id, &release_id).await?;
    let release = match input.status.as_str() {
        "building" if current.status == "uploading" => {
            let build_ref = input.build_ref.ok_or_else(|| ApiError::Invalid("buildRef is required".to_owned()))?;
            sqlx::query_as::<_, CodeRelease>(
                "UPDATE project_code_releases SET status = 'building', build_ref = $3, updated_at = NOW()
                 WHERE project_id = $1 AND release_id = $2 AND status = 'uploading' RETURNING *",
            ).bind(project.project.id).bind(&release_id).bind(build_ref).fetch_optional(pool).await?
        }
        "ready" if current.status == "building" => {
            let digest = input.artifact_digest.ok_or_else(|| ApiError::Invalid("artifactDigest is required".to_owned()))?;
            let service = input.service_origin.ok_or_else(|| ApiError::Invalid("serviceOrigin is required".to_owned()))?;
            let revision = input.revision_origin.ok_or_else(|| ApiError::Invalid("revisionOrigin is required".to_owned()))?;
            if !digest.starts_with("sha256:") || digest.len() != 71 || !service.starts_with("https://") || !revision.starts_with("https://") {
                return Err(ApiError::Invalid("ready artifact metadata is invalid".to_owned()));
            }
            sqlx::query_as::<_, CodeRelease>(
                "UPDATE project_code_releases SET status = 'ready', artifact_digest = $3,
                    service_origin = $4, revision_origin = $5, completed_at = NOW(), updated_at = NOW()
                 WHERE project_id = $1 AND release_id = $2 AND status = 'building' RETURNING *",
            ).bind(project.project.id).bind(&release_id).bind(digest).bind(service).bind(revision)
                .fetch_optional(pool).await?
        }
        "failed" if current.status == "uploading" || current.status == "building" => {
            let failure = input.failure_message.unwrap_or_else(|| "Build failed".to_owned());
            let updated = sqlx::query_as::<_, CodeRelease>(
                "UPDATE project_code_releases SET status = 'failed', failure_message = $3,
                    completed_at = NOW(), updated_at = NOW()
                 WHERE project_id = $1 AND release_id = $2 AND status IN ('uploading', 'building') RETURNING *",
            ).bind(project.project.id).bind(&release_id).bind(failure).fetch_optional(pool).await?;
            sqlx::query(
                "UPDATE project_code_deployment_state SET pending_release_id = NULL, updated_at = NOW()
                 WHERE deployment_id = $1 AND pending_release_id = $2",
            ).bind(current.deployment_id).bind(&release_id).execute(pool).await?;
            updated
        }
        _ => return Err(ApiError::Conflict("Invalid CodeRelease transition".to_owned())),
    }.ok_or_else(|| ApiError::Conflict("CodeRelease changed concurrently".to_owned()))?;
    Ok(Json(json!({ "release": release })))
}

pub async fn activate(
    State(state): State<AppState>,
    Path((slug, release_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let project = owned_project(&state, &headers, &slug, Operation::ProjectWrite).await?;
    validate_release_id(&release_id)?;
    let pool = hosted_pool(&state.pool)?;
    let release = get_owned(pool, project.project.id, &release_id).await?;
    if release.status != "ready"
        || release.service_origin.is_none()
        || release.revision_origin.is_none()
    {
        return Err(ApiError::Conflict(
            "Only a ready CodeRelease can be activated".to_owned(),
        ));
    }
    sqlx::query(
        "INSERT INTO project_code_deployment_state(deployment_id, active_release_id)
         VALUES ($1, $2) ON CONFLICT (deployment_id) DO UPDATE
         SET active_release_id = EXCLUDED.active_release_id, pending_release_id = NULL, updated_at = NOW()",
    ).bind(release.deployment_id).bind(&release_id).execute(pool).await?;
    Ok(Json(json!({ "release": release, "active": true })))
}

pub async fn activate_if_pending(
    State(state): State<AppState>,
    Path((slug, release_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let project = owned_project(&state, &headers, &slug, Operation::ProjectWrite).await?;
    let identity = state
        .auth
        .authorize_project(
            &headers,
            Operation::ProjectWrite,
            project.project.owner_user_id.as_deref(),
        )
        .await?;
    if identity != Identity::DeploymentAdmin {
        return Err(ApiError::Forbidden);
    }
    validate_release_id(&release_id)?;
    let pool = hosted_pool(&state.pool)?;
    let release = get_owned(pool, project.project.id, &release_id).await?;
    if release.status != "ready" {
        return Err(ApiError::Conflict("CodeRelease is not ready".to_owned()));
    }
    let updated = sqlx::query(
        "UPDATE project_code_deployment_state SET active_release_id = $2,
            pending_release_id = NULL, updated_at = NOW()
         WHERE deployment_id = $1 AND pending_release_id = $2",
    )
    .bind(release.deployment_id)
    .bind(&release_id)
    .execute(pool)
    .await?;
    Ok(Json(json!({ "active": updated.rows_affected() == 1 })))
}

async fn get_owned(
    pool: &PgPool,
    project_id: Uuid,
    release_id: &str,
) -> Result<CodeRelease, ApiError> {
    sqlx::query_as::<_, CodeRelease>(
        "SELECT * FROM project_code_releases WHERE project_id = $1 AND release_id = $2",
    )
    .bind(project_id)
    .bind(release_id)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_id_rejects_non_generated_paths() {
        assert!(validate_release_id("../../active").is_err());
    }

    #[test]
    fn ordinary_web_release_requires_http_target() {
        let input = CreateCodeRelease {
            release_id: format!("rel_{}", "a".repeat(32)),
            deployment_id: Uuid::new_v4(),
            python_version: "3.12".to_owned(),
            dependency_strategy: "pyproject.toml".to_owned(),
            integration: "web".to_owned(),
            http_target: None,
            http_interface: Some("asgi".to_owned()),
            source_key: "source.tar.gz".to_owned(),
            source_sha256: "a".repeat(64),
            source_crc32c: "4waSgw==".to_owned(),
            source_size_bytes: 1,
            source_bytes: 1,
            file_count: 1,
        };
        assert!(validate_create(&input).is_err());
    }
}
