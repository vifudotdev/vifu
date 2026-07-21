use chrono::Utc;
use serde_json::{json, Value};
use sqlx::types::Json;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;
use vifu_game_runtime::{
    BackendResourceSnapshot, CompileOutput, EffectResult, GameCommand, GamePlanV1, GameRuntime,
    GameSnapshotV1, GameSourceV1, HostDescriptor, RuntimeAdvance, SessionStatus,
};

use crate::error::{map_database_error, ApiError};
use crate::models::ProjectWithBindings;

use super::models::{
    GameAnalyticsCount, GameAsset, GameAssetVersion, GameAssetWithVersions, GameBuildJob,
    GameDraft, GameDraftRow, GameEffectTrace, GameEffectWork, GameEffectWorkRow, GameOverview,
    GamePresentationRelease, GameRelease, GameReleaseRow, GameReleaseSummary, GameResource,
    GameSession, GameSessionRow, GameSessionStatusCount, SessionExecutionRow, StoredCommandRow,
    StoredEventRow,
};

pub async fn ensure_game_draft(
    pool: &PgPool,
    project: &ProjectWithBindings,
    source: &GameSourceV1,
    content_hash: &str,
) -> Result<GameDraft, ApiError> {
    sqlx::query(
        "INSERT INTO project_game_drafts (project_id, source, content_hash)
         VALUES ($1, $2, $3)
         ON CONFLICT (project_id) DO NOTHING",
    )
    .bind(project.project.id)
    .bind(Json(source))
    .bind(content_hash)
    .execute(pool)
    .await?;
    get_game_draft(pool, project.project.id).await
}

pub async fn get_game_draft(pool: &PgPool, project_id: Uuid) -> Result<GameDraft, ApiError> {
    let row = sqlx::query_as::<_, GameDraftRow>(
        "SELECT project_id, source, revision, content_hash, created_at, updated_at
         FROM project_game_drafts
         WHERE project_id = $1",
    )
    .bind(project_id)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)?;
    row.try_into()
}

pub async fn update_game_draft(
    pool: &PgPool,
    project_id: Uuid,
    source: &GameSourceV1,
    content_hash: &str,
    expected_revision: Option<u64>,
    expected_hash: Option<&str>,
) -> Result<GameDraft, ApiError> {
    let expected_revision = expected_revision
        .map(i64::try_from)
        .transpose()
        .map_err(|_| ApiError::Invalid("expectedRevision is too large".to_string()))?;
    let updated = sqlx::query_scalar::<_, Uuid>(
        "UPDATE project_game_drafts
         SET source = $2,
             content_hash = $3,
             revision = revision + 1,
             updated_at = NOW()
         WHERE project_id = $1
           AND ($4::BIGINT IS NULL OR revision = $4)
           AND ($5::TEXT IS NULL OR content_hash = $5)
         RETURNING project_id",
    )
    .bind(project_id)
    .bind(Json(source))
    .bind(content_hash)
    .bind(expected_revision)
    .bind(expected_hash)
    .fetch_optional(pool)
    .await?;
    if updated.is_none() {
        return Err(ApiError::Conflict(
            "the game draft changed; reload it before saving".to_string(),
        ));
    }
    get_game_draft(pool, project_id).await
}

pub async fn game_overview(
    pool: &PgPool,
    project: &ProjectWithBindings,
    draft: &GameDraft,
) -> Result<GameOverview, ApiError> {
    let active_release = active_game_release(pool, project.project.id)
        .await?
        .as_ref()
        .map(GameReleaseSummary::from);
    let unpublished_changes = active_release
        .as_ref()
        .is_none_or(|release| release.source_revision != draft.revision);
    Ok(GameOverview {
        project_id: project.project.id,
        project_slug: project.project.slug.clone(),
        draft_revision: draft.revision,
        draft_hash: draft.content_hash.clone(),
        active_release,
        unpublished_changes,
    })
}

pub async fn publish_game(
    pool: &PgPool,
    project_id: Uuid,
    expected_revision: u64,
    compiled: &CompileOutput,
    backend_resources: &[BackendResourceSnapshot],
    change_summary: Option<&str>,
) -> Result<GameRelease, ApiError> {
    let expected_revision = i64::try_from(expected_revision)
        .map_err(|_| ApiError::Invalid("expectedRevision is too large".to_string()))?;
    let mut transaction = pool.begin().await?;
    let current_revision = sqlx::query_scalar::<_, i64>(
        "SELECT revision
         FROM project_game_drafts
         WHERE project_id = $1
         FOR UPDATE",
    )
    .bind(project_id)
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or(ApiError::NotFound)?;
    if current_revision != expected_revision {
        return Err(ApiError::Conflict(
            "the game draft changed before it could be published".to_string(),
        ));
    }

    if let Some(existing_id) = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM game_releases
         WHERE project_id = $1 AND content_hash = $2",
    )
    .bind(project_id)
    .bind(&compiled.content_hash)
    .fetch_optional(&mut *transaction)
    .await?
    {
        activate_release_in_transaction(&mut transaction, project_id, existing_id).await?;
        transaction.commit().await?;
        return get_game_release(pool, project_id, existing_id).await;
    }

    let release_number = sqlx::query_scalar::<_, i32>(
        "SELECT COALESCE(MAX(release_number), 0) + 1
         FROM game_releases
         WHERE project_id = $1",
    )
    .bind(project_id)
    .fetch_one(&mut *transaction)
    .await?;
    let release_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO game_releases (
            id, project_id, release_number, source_revision, content_hash,
            plan, manifest, backend_resources, change_summary
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(release_id)
    .bind(project_id)
    .bind(release_number)
    .bind(current_revision)
    .bind(&compiled.content_hash)
    .bind(Json(&compiled.plan))
    .bind(Json(&compiled.manifest))
    .bind(Json(backend_resources))
    .bind(change_summary)
    .execute(&mut *transaction)
    .await
    .map_err(map_database_error)?;
    activate_release_in_transaction(&mut transaction, project_id, release_id).await?;
    transaction.commit().await?;
    get_game_release(pool, project_id, release_id).await
}

async fn activate_release_in_transaction(
    transaction: &mut Transaction<'_, Postgres>,
    project_id: Uuid,
    release_id: Uuid,
) -> Result<(), ApiError> {
    let updated = sqlx::query(
        "UPDATE projects
         SET active_game_release_id = $2,
             active_game_presentation_release_id = (
                 SELECT id
                 FROM game_presentation_releases
                 WHERE project_id = $1 AND game_release_id = $2
                 ORDER BY release_number DESC
                 LIMIT 1
             ),
             updated_at = NOW()
         WHERE id = $1
           AND EXISTS (
               SELECT 1 FROM game_releases
               WHERE id = $2 AND project_id = $1
           )",
    )
    .bind(project_id)
    .bind(release_id)
    .execute(&mut **transaction)
    .await?;
    if updated.rows_affected() != 1 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

pub async fn activate_game_release(
    pool: &PgPool,
    project_id: Uuid,
    release_id: Uuid,
) -> Result<GameRelease, ApiError> {
    let mut transaction = pool.begin().await?;
    activate_release_in_transaction(&mut transaction, project_id, release_id).await?;
    transaction.commit().await?;
    get_game_release(pool, project_id, release_id).await
}

pub async fn list_game_releases(
    pool: &PgPool,
    project_id: Uuid,
) -> Result<Vec<GameRelease>, ApiError> {
    let rows = sqlx::query_as::<_, GameReleaseRow>(
        "SELECT id, project_id, release_number, source_revision, content_hash,
                plan, manifest, backend_resources, change_summary, created_at
         FROM game_releases
         WHERE project_id = $1
         ORDER BY release_number DESC",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(TryInto::try_into).collect()
}

pub async fn get_game_release(
    pool: &PgPool,
    project_id: Uuid,
    release_id: Uuid,
) -> Result<GameRelease, ApiError> {
    let row = sqlx::query_as::<_, GameReleaseRow>(
        "SELECT id, project_id, release_number, source_revision, content_hash,
                plan, manifest, backend_resources, change_summary, created_at
         FROM game_releases
         WHERE project_id = $1 AND id = $2",
    )
    .bind(project_id)
    .bind(release_id)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)?;
    row.try_into()
}

pub async fn active_game_release(
    pool: &PgPool,
    project_id: Uuid,
) -> Result<Option<GameRelease>, ApiError> {
    let row = sqlx::query_as::<_, GameReleaseRow>(
        "SELECT release.id, release.project_id, release.release_number,
                release.source_revision, release.content_hash, release.plan,
                release.manifest, release.backend_resources,
                release.change_summary, release.created_at
         FROM projects AS project
         JOIN game_releases AS release ON release.id = project.active_game_release_id
         WHERE project.id = $1",
    )
    .bind(project_id)
    .fetch_optional(pool)
    .await?;
    row.map(TryInto::try_into).transpose()
}

pub async fn create_game_session(
    pool: &PgPool,
    project_id: Uuid,
    release: &GameRelease,
    api_key_id: Option<Uuid>,
    host: &HostDescriptor,
    random_seed: u64,
) -> Result<GameSession, ApiError> {
    let snapshot = GameSnapshotV1::initial(
        release.plan.entry_node,
        &release.plan.variables,
        random_seed,
    );
    let session_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO game_sessions (
            id, project_id, game_release_id, api_key_id, status,
            revision, snapshot, host
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(session_id)
    .bind(project_id)
    .bind(release.id)
    .bind(api_key_id)
    .bind(snapshot.status.to_string())
    .bind(i64::try_from(snapshot.revision).map_err(|_| ApiError::Internal)?)
    .bind(Json(&snapshot))
    .bind(Json(host))
    .execute(pool)
    .await?;
    get_game_session(pool, project_id, session_id).await
}

pub async fn create_preview_game_session(
    pool: &PgPool,
    project_id: Uuid,
    source_revision: u64,
    plan: &GamePlanV1,
    host: &HostDescriptor,
    random_seed: u64,
) -> Result<GameSession, ApiError> {
    let snapshot = GameSnapshotV1::initial(plan.entry_node, &plan.variables, random_seed);
    let session_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO game_sessions (
            id, project_id, source_revision, execution_plan, is_preview,
            status, revision, snapshot, host
         ) VALUES ($1, $2, $3, $4, TRUE, $5, $6, $7, $8)",
    )
    .bind(session_id)
    .bind(project_id)
    .bind(i64::try_from(source_revision).map_err(|_| ApiError::Internal)?)
    .bind(Json(plan))
    .bind(snapshot.status.to_string())
    .bind(i64::try_from(snapshot.revision).map_err(|_| ApiError::Internal)?)
    .bind(Json(&snapshot))
    .bind(Json(host))
    .execute(pool)
    .await?;
    get_game_session(pool, project_id, session_id).await
}

pub async fn get_game_session(
    pool: &PgPool,
    project_id: Uuid,
    session_id: Uuid,
) -> Result<GameSession, ApiError> {
    let row = sqlx::query_as::<_, GameSessionRow>(
        "SELECT id, project_id, game_release_id, source_revision, is_preview,
                api_key_id, status, revision,
                snapshot, host, public_output, failure, created_at, updated_at,
                completed_at
         FROM game_sessions
         WHERE project_id = $1 AND id = $2",
    )
    .bind(project_id)
    .bind(session_id)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)?;
    row.try_into()
}

pub async fn get_published_game_session(
    pool: &PgPool,
    project_id: Uuid,
    session_id: Uuid,
) -> Result<GameSession, ApiError> {
    let session = get_game_session(pool, project_id, session_id).await?;
    if session.is_preview {
        return Err(ApiError::NotFound);
    }
    Ok(session)
}

pub async fn submit_game_command(
    pool: &PgPool,
    project_id: Uuid,
    session_id: Uuid,
    command: &GameCommand,
    effect_trace: Option<GameEffectTrace>,
) -> Result<RuntimeAdvance, ApiError> {
    if command.idempotency_key.trim().is_empty() || command.idempotency_key.len() > 200 {
        return Err(ApiError::Invalid(
            "idempotencyKey must contain between 1 and 200 characters".to_string(),
        ));
    }
    let mut transaction = pool.begin().await?;
    let execution = sqlx::query_as::<_, SessionExecutionRow>(
        "SELECT session.id, session.project_id, session.game_release_id,
                session.revision, session.snapshot,
                COALESCE(session.execution_plan, release.plan) AS plan
         FROM game_sessions AS session
         LEFT JOIN game_releases AS release ON release.id = session.game_release_id
         WHERE session.id = $1 AND session.project_id = $2
           AND (session.execution_plan IS NOT NULL OR release.id IS NOT NULL)
         FOR UPDATE OF session",
    )
    .bind(session_id)
    .bind(project_id)
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or(ApiError::NotFound)?;

    if let Some(stored) = sqlx::query_as::<_, StoredCommandRow>(
        "SELECT result
         FROM game_commands
         WHERE session_id = $1 AND idempotency_key = $2",
    )
    .bind(session_id)
    .bind(&command.idempotency_key)
    .fetch_optional(&mut *transaction)
    .await?
    {
        let result = stored.result.ok_or(ApiError::Internal)?.0;
        transaction.commit().await?;
        return Ok(result);
    }

    if let Some(expected_revision) = command.expected_revision {
        let actual_revision = u64::try_from(execution.revision).map_err(|_| ApiError::Internal)?;
        if expected_revision != actual_revision {
            return Err(ApiError::Conflict(format!(
                "session revision is {actual_revision}, not {expected_revision}"
            )));
        }
    }

    let advance = {
        let mut runtime =
            GameRuntime::restore(execution.plan.0.clone(), execution.snapshot.0.clone())
                .map_err(|error| ApiError::Invalid(error.to_string()))?;
        runtime
            .dispatch(command.clone())
            .map_err(|error| ApiError::Invalid(error.to_string()))?
    };
    persist_advance(
        &mut transaction,
        &execution,
        command,
        &advance,
        effect_trace,
    )
    .await?;
    transaction.commit().await?;
    Ok(advance)
}

async fn persist_advance(
    transaction: &mut Transaction<'_, Postgres>,
    execution: &SessionExecutionRow,
    command: &GameCommand,
    advance: &RuntimeAdvance,
    effect_trace: Option<GameEffectTrace>,
) -> Result<(), ApiError> {
    let revision = i64::try_from(advance.snapshot.revision).map_err(|_| ApiError::Internal)?;
    let terminal = matches!(
        advance.snapshot.status,
        SessionStatus::Completed | SessionStatus::Failed | SessionStatus::Cancelled
    );
    let updated = sqlx::query(
        "UPDATE game_sessions
         SET status = $3,
             revision = $4,
             snapshot = $5,
             public_output = $6,
             failure = $7,
             updated_at = NOW(),
             completed_at = CASE WHEN $8 THEN COALESCE(completed_at, NOW()) ELSE NULL END
         WHERE id = $1 AND revision = $2",
    )
    .bind(execution.id)
    .bind(execution.revision)
    .bind(advance.snapshot.status.to_string())
    .bind(revision)
    .bind(Json(&advance.snapshot))
    .bind(advance.snapshot.public_output.as_ref().map(Json))
    .bind(
        advance
            .snapshot
            .failure
            .as_ref()
            .map(|failure| Json(json!(failure))),
    )
    .bind(terminal)
    .execute(&mut **transaction)
    .await?;
    if updated.rows_affected() != 1 {
        return Err(ApiError::Conflict(
            "the game session changed while processing the command".to_string(),
        ));
    }

    for event in &advance.events {
        let sequence = i64::try_from(event.sequence).map_err(|_| ApiError::Internal)?;
        sqlx::query(
            "INSERT INTO game_events (session_id, sequence, event)
             VALUES ($1, $2, $3)",
        )
        .bind(execution.id)
        .bind(sequence)
        .bind(Json(event))
        .execute(&mut **transaction)
        .await?;
        if let (Some(game_release_id), Some(dimensions)) =
            (execution.game_release_id, analytics_dimensions(event))
        {
            sqlx::query(
                "INSERT INTO game_analytics_events (
                    project_id, game_release_id, session_id, event_type, dimensions
                 ) VALUES ($1, $2, $3, $4, $5)",
            )
            .bind(execution.project_id)
            .bind(game_release_id)
            .bind(execution.id)
            .bind(&event.event_type)
            .bind(Json(dimensions))
            .execute(&mut **transaction)
            .await?;
        }
    }
    for effect in &advance.effects {
        sqlx::query(
            "INSERT INTO game_effects (
                session_id, effect_id, node_id, kind, request, trace_id, parent_span_id
             ) VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (session_id, effect_id) DO NOTHING",
        )
        .bind(execution.id)
        .bind(&effect.effect_id)
        .bind(&effect.node_id)
        .bind(match effect.kind {
            vifu_game_runtime::EffectKind::Agent => "agent",
            vifu_game_runtime::EffectKind::Tool => "tool",
        })
        .bind(Json(effect))
        .bind(effect_trace.map(|context| context.trace_id))
        .bind(effect_trace.map(|context| context.parent_span_id))
        .execute(&mut **transaction)
        .await?;
    }
    sqlx::query(
        "INSERT INTO game_commands (
            id, session_id, idempotency_key, command, status, result, completed_at
         ) VALUES ($1, $2, $3, $4, 'completed', $5, NOW())",
    )
    .bind(Uuid::new_v4())
    .bind(execution.id)
    .bind(&command.idempotency_key)
    .bind(Json(command))
    .bind(Json(advance))
    .execute(&mut **transaction)
    .await
    .map_err(map_database_error)?;
    Ok(())
}

pub async fn list_game_events_after(
    pool: &PgPool,
    session_id: Uuid,
    after_sequence: u64,
    limit: i64,
) -> Result<Vec<vifu_game_runtime::GameEvent>, ApiError> {
    let after_sequence = i64::try_from(after_sequence)
        .map_err(|_| ApiError::Invalid("Last-Event-ID is too large".to_string()))?;
    let rows = sqlx::query_as::<_, StoredEventRow>(
        "SELECT sequence, event
         FROM game_events
         WHERE session_id = $1 AND sequence > $2 AND public = TRUE
         ORDER BY sequence ASC
         LIMIT $3",
    )
    .bind(session_id)
    .bind(after_sequence)
    .bind(limit.clamp(1, 1000))
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|row| {
            let event = row.event.0;
            if i64::try_from(event.sequence).ok() != Some(row.sequence) {
                return Err(ApiError::Internal);
            }
            Ok(event)
        })
        .collect()
}

pub async fn claim_game_effect(
    pool: &PgPool,
    worker_id: &str,
) -> Result<Option<GameEffectWork>, ApiError> {
    let mut transaction = pool.begin().await?;
    let row = sqlx::query_as::<_, GameEffectWorkRow>(
        "SELECT effect.session_id, effect.effect_id, effect.status,
                effect.request, effect.result, session.project_id,
                project.slug AS project_slug, effect.trace_id, effect.parent_span_id
         FROM game_effects AS effect
         JOIN game_sessions AS session ON session.id = effect.session_id
         JOIN projects AS project ON project.id = session.project_id
         WHERE (
             effect.status = 'queued'
             OR (effect.status = 'running' AND effect.lease_expires_at < NOW())
             OR (
                 effect.status IN ('completed', 'failed')
                 AND NOT EXISTS (
                     SELECT 1 FROM game_commands AS command
                     WHERE command.session_id = effect.session_id
                       AND command.idempotency_key = 'effect:' || effect.effect_id
                 )
             )
         )
         ORDER BY effect.created_at ASC
         LIMIT 1
         FOR UPDATE OF effect SKIP LOCKED",
    )
    .fetch_optional(&mut *transaction)
    .await?;
    let Some(row) = row else {
        transaction.commit().await?;
        return Ok(None);
    };
    if matches!(row.status.as_str(), "queued" | "running") {
        sqlx::query(
            "UPDATE game_effects
             SET status = 'running', lease_owner = $3,
                 lease_expires_at = NOW() + INTERVAL '60 seconds',
                 attempts = attempts + 1, updated_at = NOW()
             WHERE session_id = $1 AND effect_id = $2",
        )
        .bind(row.session_id)
        .bind(&row.effect_id)
        .bind(worker_id)
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;
    Ok(Some(row.into()))
}

pub async fn store_game_effect_result(
    pool: &PgPool,
    session_id: Uuid,
    effect_id: &str,
    result: &EffectResult,
) -> Result<(), ApiError> {
    let status = if result.error.is_some() {
        "failed"
    } else {
        "completed"
    };
    let updated = sqlx::query(
        "UPDATE game_effects
         SET status = $3, result = $4, lease_owner = NULL,
             lease_expires_at = NULL, updated_at = NOW(), completed_at = NOW()
         WHERE session_id = $1 AND effect_id = $2",
    )
    .bind(session_id)
    .bind(effect_id)
    .bind(status)
    .bind(Json(result))
    .execute(pool)
    .await?;
    if updated.rows_affected() != 1 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

pub async fn resume_stored_effect(
    pool: &PgPool,
    work: &GameEffectWork,
    result: &EffectResult,
) -> Result<RuntimeAdvance, ApiError> {
    submit_game_command(
        pool,
        work.project_id,
        work.session_id,
        &GameCommand {
            idempotency_key: format!("effect:{}", work.effect_id),
            expected_revision: None,
            command_type: "effect.completed".to_string(),
            data: serde_json::to_value(result).map_err(|_| ApiError::Internal)?,
        },
        work.trace_context(),
    )
    .await
}

pub async fn list_game_resources(
    pool: &PgPool,
    project_id: Uuid,
) -> Result<Vec<GameResource>, ApiError> {
    Ok(sqlx::query_as::<_, GameResource>(
        "SELECT DISTINCT ON (resource_key)
                id, project_id, resource_key, name, kind, content, version,
                content_hash, approved, created_at, updated_at
         FROM project_game_resources
         WHERE project_id = $1
         ORDER BY resource_key, version DESC",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await?)
}

pub async fn get_game_resource_version(
    pool: &PgPool,
    project_id: Uuid,
    resource_id: Uuid,
) -> Result<GameResource, ApiError> {
    sqlx::query_as::<_, GameResource>(
        "SELECT id, project_id, resource_key, name, kind, content, version,
                content_hash, approved, created_at, updated_at
         FROM project_game_resources
         WHERE project_id = $1 AND id = $2",
    )
    .bind(project_id)
    .bind(resource_id)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)
}

pub struct NewGameResource<'a> {
    pub project_id: Uuid,
    pub resource_key: &'a str,
    pub name: &'a str,
    pub kind: &'a str,
    pub content: &'a Value,
    pub content_hash: &'a str,
    pub approved: bool,
}

pub async fn create_game_resource(
    pool: &PgPool,
    input: NewGameResource<'_>,
) -> Result<GameResource, ApiError> {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO project_game_resources (
            id, project_id, resource_key, name, kind, content, version,
            content_hash, approved
         ) VALUES ($1, $2, $3, $4, $5, $6, 1, $7, $8)",
    )
    .bind(id)
    .bind(input.project_id)
    .bind(input.resource_key)
    .bind(input.name)
    .bind(input.kind)
    .bind(input.content)
    .bind(input.content_hash)
    .bind(input.approved)
    .execute(pool)
    .await
    .map_err(map_database_error)?;
    get_game_resource_version(pool, input.project_id, id).await
}

pub async fn create_game_resource_version(
    pool: &PgPool,
    current: &GameResource,
    name: &str,
    kind: &str,
    content: &Value,
    content_hash: &str,
    approved: bool,
) -> Result<GameResource, ApiError> {
    let id = Uuid::new_v4();
    let mut transaction = pool.begin().await?;
    sqlx::query_scalar::<_, Uuid>("SELECT id FROM projects WHERE id = $1 FOR UPDATE")
        .bind(current.project_id)
        .fetch_one(&mut *transaction)
        .await?;
    let next_version = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(MAX(version), 0) + 1
         FROM project_game_resources
         WHERE project_id = $1 AND resource_key = $2",
    )
    .bind(current.project_id)
    .bind(&current.resource_key)
    .fetch_one(&mut *transaction)
    .await?;
    sqlx::query(
        "INSERT INTO project_game_resources (
            id, project_id, resource_key, name, kind, content, version,
            content_hash, approved
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(id)
    .bind(current.project_id)
    .bind(&current.resource_key)
    .bind(name)
    .bind(kind)
    .bind(content)
    .bind(next_version)
    .bind(content_hash)
    .bind(approved)
    .execute(&mut *transaction)
    .await
    .map_err(map_database_error)?;
    transaction.commit().await?;
    get_game_resource_version(pool, current.project_id, id).await
}

pub async fn delete_game_resource(
    pool: &PgPool,
    project_id: Uuid,
    resource_id: Uuid,
) -> Result<(), ApiError> {
    let deleted = sqlx::query(
        "DELETE FROM project_game_resources
         WHERE project_id = $1
           AND resource_key = (
               SELECT resource_key FROM project_game_resources
               WHERE project_id = $1 AND id = $2
           )",
    )
    .bind(project_id)
    .bind(resource_id)
    .execute(pool)
    .await?;
    if deleted.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

pub async fn backend_resource_snapshot(
    pool: &PgPool,
    project_id: Uuid,
    resources: &[vifu_game_runtime::PinnedResource],
) -> Result<Vec<BackendResourceSnapshot>, ApiError> {
    let mut snapshots = Vec::with_capacity(resources.len());
    for reference in resources {
        let version_id = Uuid::parse_str(&reference.version_id)
            .map_err(|_| ApiError::Invalid("resource versionId must be a UUID".to_string()))?;
        let resource = get_game_resource_version(pool, project_id, version_id).await?;
        if resource.resource_key != reference.id {
            return Err(ApiError::Invalid(format!(
                "resource `{}` does not match version {}",
                reference.id, reference.version_id
            )));
        }
        if resource.kind != reference.kind {
            return Err(ApiError::Invalid(format!(
                "resource `{}` kind does not match its pinned version",
                reference.id
            )));
        }
        if resource.content_hash != reference.content_hash {
            return Err(ApiError::Invalid(format!(
                "resource `{}` content hash does not match its pinned version",
                reference.id
            )));
        }
        if !resource.approved {
            return Err(ApiError::Invalid(format!(
                "resource `{}` is not approved",
                reference.id
            )));
        }
        snapshots.push(BackendResourceSnapshot {
            id: resource.resource_key,
            version_id: resource.id.to_string(),
            version: u64::try_from(resource.version).map_err(|_| ApiError::Internal)?,
            kind: resource.kind,
            content_hash: resource.content_hash,
            content: resource.content,
        });
    }
    Ok(snapshots)
}

pub async fn list_game_assets(
    pool: &PgPool,
    project_id: Uuid,
) -> Result<Vec<GameAssetWithVersions>, ApiError> {
    let assets = sqlx::query_as::<_, GameAsset>(
        "SELECT id, project_id, asset_key, name, kind, created_at, updated_at
         FROM project_game_assets
         WHERE project_id = $1
         ORDER BY created_at ASC",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await?;
    let versions = sqlx::query_as::<_, GameAssetVersion>(
        "SELECT id, project_id, asset_id, content_hash, mime_type, size_bytes,
                storage_key, metadata, provenance, rights_status,
                approval_status, created_at
         FROM game_asset_versions
         WHERE project_id = $1
         ORDER BY created_at DESC",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await?;
    Ok(assets
        .into_iter()
        .map(|asset| GameAssetWithVersions {
            versions: versions
                .iter()
                .filter(|version| version.asset_id == asset.id)
                .cloned()
                .collect(),
            asset,
        })
        .collect())
}

pub async fn create_game_asset(
    pool: &PgPool,
    project_id: Uuid,
    asset_key: &str,
    name: &str,
    kind: &str,
) -> Result<GameAsset, ApiError> {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO project_game_assets (id, project_id, asset_key, name, kind)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(id)
    .bind(project_id)
    .bind(asset_key)
    .bind(name)
    .bind(kind)
    .execute(pool)
    .await
    .map_err(map_database_error)?;
    get_game_asset(pool, project_id, id).await
}

pub async fn get_game_asset(
    pool: &PgPool,
    project_id: Uuid,
    asset_id: Uuid,
) -> Result<GameAsset, ApiError> {
    sqlx::query_as::<_, GameAsset>(
        "SELECT id, project_id, asset_key, name, kind, created_at, updated_at
         FROM project_game_assets WHERE project_id = $1 AND id = $2",
    )
    .bind(project_id)
    .bind(asset_id)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)
}

pub struct NewGameAssetVersion<'a> {
    pub project_id: Uuid,
    pub asset_id: Uuid,
    pub content_hash: &'a str,
    pub mime_type: &'a str,
    pub size_bytes: i64,
    pub storage_key: &'a str,
    pub metadata: &'a Value,
    pub provenance: &'a Value,
    pub rights_status: &'a str,
}

pub async fn create_game_asset_version(
    pool: &PgPool,
    input: NewGameAssetVersion<'_>,
) -> Result<GameAssetVersion, ApiError> {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO game_asset_versions (
            id, project_id, asset_id, content_hash, mime_type, size_bytes,
            storage_key, metadata, provenance, rights_status
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
    )
    .bind(id)
    .bind(input.project_id)
    .bind(input.asset_id)
    .bind(input.content_hash)
    .bind(input.mime_type)
    .bind(input.size_bytes)
    .bind(input.storage_key)
    .bind(input.metadata)
    .bind(input.provenance)
    .bind(input.rights_status)
    .execute(pool)
    .await
    .map_err(map_database_error)?;
    get_game_asset_version(pool, input.project_id, id).await
}

pub async fn get_game_asset_version(
    pool: &PgPool,
    project_id: Uuid,
    version_id: Uuid,
) -> Result<GameAssetVersion, ApiError> {
    sqlx::query_as::<_, GameAssetVersion>(
        "SELECT id, project_id, asset_id, content_hash, mime_type, size_bytes,
                storage_key, metadata, provenance, rights_status,
                approval_status, created_at
         FROM game_asset_versions
         WHERE project_id = $1 AND id = $2",
    )
    .bind(project_id)
    .bind(version_id)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)
}

pub async fn list_game_asset_versions(
    pool: &PgPool,
    project_id: Uuid,
    asset_id: Uuid,
) -> Result<Vec<GameAssetVersion>, ApiError> {
    get_game_asset(pool, project_id, asset_id).await?;
    Ok(sqlx::query_as::<_, GameAssetVersion>(
        "SELECT id, project_id, asset_id, content_hash, mime_type, size_bytes,
                storage_key, metadata, provenance, rights_status,
                approval_status, created_at
         FROM game_asset_versions
         WHERE project_id = $1 AND asset_id = $2
         ORDER BY created_at DESC",
    )
    .bind(project_id)
    .bind(asset_id)
    .fetch_all(pool)
    .await?)
}

pub async fn find_game_asset_version_by_hash(
    pool: &PgPool,
    project_id: Uuid,
    asset_id: Uuid,
    content_hash: &str,
) -> Result<Option<GameAssetVersion>, ApiError> {
    Ok(sqlx::query_as::<_, GameAssetVersion>(
        "SELECT id, project_id, asset_id, content_hash, mime_type, size_bytes,
                storage_key, metadata, provenance, rights_status,
                approval_status, created_at
         FROM game_asset_versions
         WHERE project_id = $1 AND asset_id = $2 AND content_hash = $3",
    )
    .bind(project_id)
    .bind(asset_id)
    .bind(content_hash)
    .fetch_optional(pool)
    .await?)
}

pub async fn approve_game_asset_version(
    pool: &PgPool,
    project_id: Uuid,
    asset_id: Uuid,
    version_id: Uuid,
    status: &str,
) -> Result<GameAssetVersion, ApiError> {
    let updated = sqlx::query(
        "UPDATE game_asset_versions
         SET approval_status = $4
         WHERE project_id = $1 AND asset_id = $2 AND id = $3",
    )
    .bind(project_id)
    .bind(asset_id)
    .bind(version_id)
    .bind(status)
    .execute(pool)
    .await?;
    if updated.rows_affected() != 1 {
        return Err(ApiError::NotFound);
    }
    get_game_asset_version(pool, project_id, version_id).await
}

pub async fn delete_game_asset(
    pool: &PgPool,
    project_id: Uuid,
    asset_id: Uuid,
) -> Result<(), ApiError> {
    let deleted = sqlx::query("DELETE FROM project_game_assets WHERE project_id = $1 AND id = $2")
        .bind(project_id)
        .bind(asset_id)
        .execute(pool)
        .await?;
    if deleted.rows_affected() != 1 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

pub async fn create_completed_game_build(
    pool: &PgPool,
    project_id: Uuid,
    source_revision: u64,
    kind: &str,
    input_hash: &str,
    output: &Value,
) -> Result<GameBuildJob, ApiError> {
    let id = Uuid::new_v4();
    let source_revision = i64::try_from(source_revision)
        .map_err(|_| ApiError::Invalid("source revision is too large".to_string()))?;
    sqlx::query(
        "INSERT INTO game_build_jobs (
            id, project_id, source_revision, kind, status, input_hash,
            input, output, attempts, started_at, completed_at
         ) VALUES ($1, $2, $3, $4, 'completed', $5, '{}'::jsonb, $6, 1, NOW(), NOW())",
    )
    .bind(id)
    .bind(project_id)
    .bind(source_revision)
    .bind(kind)
    .bind(input_hash)
    .bind(output)
    .execute(pool)
    .await?;
    get_game_build(pool, project_id, id).await
}

pub async fn get_game_build(
    pool: &PgPool,
    project_id: Uuid,
    build_id: Uuid,
) -> Result<GameBuildJob, ApiError> {
    sqlx::query_as::<_, GameBuildJob>(
        "SELECT id, project_id, source_revision, kind, status, input_hash,
                input, output, error, attempts, created_at, started_at,
                completed_at
         FROM game_build_jobs WHERE project_id = $1 AND id = $2",
    )
    .bind(project_id)
    .bind(build_id)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)
}

pub async fn cancel_game_build(
    pool: &PgPool,
    project_id: Uuid,
    build_id: Uuid,
) -> Result<GameBuildJob, ApiError> {
    let updated = sqlx::query(
        "UPDATE game_build_jobs
         SET status = 'cancelled', completed_at = NOW()
         WHERE project_id = $1 AND id = $2 AND status IN ('queued', 'running')",
    )
    .bind(project_id)
    .bind(build_id)
    .execute(pool)
    .await?;
    if updated.rows_affected() == 0 {
        let existing = get_game_build(pool, project_id, build_id).await?;
        if existing.status != "cancelled" {
            return Err(ApiError::Conflict(
                "only an active build can be cancelled".to_string(),
            ));
        }
    }
    get_game_build(pool, project_id, build_id).await
}

pub async fn game_analytics(
    pool: &PgPool,
    project_id: Uuid,
) -> Result<
    (
        Vec<GameAnalyticsCount>,
        Vec<GameSessionStatusCount>,
        i64,
        i64,
    ),
    ApiError,
> {
    let events = sqlx::query_as::<_, GameAnalyticsCount>(
        "SELECT event_type, COUNT(*)::BIGINT AS count
         FROM game_analytics_events
         WHERE project_id = $1
         GROUP BY event_type ORDER BY count DESC, event_type ASC",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await?;
    let sessions = sqlx::query_as::<_, GameSessionStatusCount>(
        "SELECT status, COUNT(*)::BIGINT AS count
         FROM game_sessions
         WHERE project_id = $1 AND is_preview = FALSE
         GROUP BY status ORDER BY count DESC, status ASC",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await?;
    let total_sessions = sessions.iter().map(|item| item.count).sum();
    let average_duration_ms = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT AVG(EXTRACT(EPOCH FROM (completed_at - created_at)) * 1000)::BIGINT
         FROM game_sessions
         WHERE project_id = $1 AND is_preview = FALSE AND completed_at IS NOT NULL",
    )
    .bind(project_id)
    .fetch_one(pool)
    .await?
    .unwrap_or_default();
    Ok((events, sessions, total_sessions, average_duration_ms))
}

fn analytics_dimensions(event: &vifu_game_runtime::GameEvent) -> Option<Value> {
    let subject = event.subject.clone();
    match event.event_type.as_str() {
        "game.session.started" | "game.session.completed" | "game.session.cancelled" => {
            Some(json!({"nodeId": subject}))
        }
        "game.session.failed" => Some(json!({
            "nodeId": subject,
            "code": event.data.get("code").and_then(Value::as_str),
        })),
        "choice.selected" => Some(json!({
            "nodeId": subject,
            "optionId": event.data.get("optionId").and_then(Value::as_str),
        })),
        "ending.reached" => Some(json!({
            "nodeId": subject,
            "endingId": event.data.get("endingId").and_then(Value::as_str),
        })),
        _ => None,
    }
}

pub async fn list_game_sessions(
    pool: &PgPool,
    project_id: Uuid,
    limit: i64,
) -> Result<Vec<GameSession>, ApiError> {
    let rows = sqlx::query_as::<_, GameSessionRow>(
        "SELECT id, project_id, game_release_id, source_revision, is_preview,
                api_key_id, status, revision,
                snapshot, host, public_output, failure, created_at, updated_at,
                completed_at
         FROM game_sessions
         WHERE project_id = $1
         ORDER BY created_at DESC LIMIT $2",
    )
    .bind(project_id)
    .bind(limit.clamp(1, 100))
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(TryInto::try_into).collect()
}

pub async fn list_published_game_sessions(
    pool: &PgPool,
    project_id: Uuid,
    limit: i64,
) -> Result<Vec<GameSession>, ApiError> {
    let rows = sqlx::query_as::<_, GameSessionRow>(
        "SELECT id, project_id, game_release_id, source_revision, is_preview,
                api_key_id, status, revision,
                snapshot, host, public_output, failure, created_at, updated_at,
                completed_at
         FROM game_sessions
         WHERE project_id = $1 AND is_preview = FALSE
         ORDER BY created_at DESC LIMIT $2",
    )
    .bind(project_id)
    .bind(limit.clamp(1, 100))
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(TryInto::try_into).collect()
}

pub async fn publish_game_presentation(
    pool: &PgPool,
    project_id: Uuid,
    game_release_id: Uuid,
    content_hash: &str,
    binding_manifest: &vifu_game_runtime::HostBindingManifestV1,
    asset_version_ids: &[Uuid],
) -> Result<GamePresentationRelease, ApiError> {
    let mut transaction = pool.begin().await?;
    sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM game_releases WHERE project_id = $1 AND id = $2 FOR SHARE",
    )
    .bind(project_id)
    .bind(game_release_id)
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or(ApiError::NotFound)?;
    if let Some(existing_id) = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM game_presentation_releases
         WHERE project_id = $1 AND content_hash = $2",
    )
    .bind(project_id)
    .bind(content_hash)
    .fetch_optional(&mut *transaction)
    .await?
    {
        activate_presentation_in_transaction(&mut transaction, project_id, existing_id).await?;
        transaction.commit().await?;
        return get_game_presentation_release(pool, project_id, existing_id).await;
    }
    let release_number = sqlx::query_scalar::<_, i32>(
        "SELECT COALESCE(MAX(release_number), 0) + 1
         FROM game_presentation_releases WHERE project_id = $1",
    )
    .bind(project_id)
    .fetch_one(&mut *transaction)
    .await?;
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO game_presentation_releases (
            id, project_id, game_release_id, release_number, content_hash,
            binding_manifest, asset_version_ids
         ) VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(id)
    .bind(project_id)
    .bind(game_release_id)
    .bind(release_number)
    .bind(content_hash)
    .bind(Json(binding_manifest))
    .bind(asset_version_ids)
    .execute(&mut *transaction)
    .await
    .map_err(map_database_error)?;
    activate_presentation_in_transaction(&mut transaction, project_id, id).await?;
    transaction.commit().await?;
    get_game_presentation_release(pool, project_id, id).await
}

async fn activate_presentation_in_transaction(
    transaction: &mut Transaction<'_, Postgres>,
    project_id: Uuid,
    presentation_id: Uuid,
) -> Result<(), ApiError> {
    let updated = sqlx::query(
        "UPDATE projects AS project
         SET active_game_presentation_release_id = $2
         WHERE project.id = $1
           AND EXISTS (
               SELECT 1
               FROM game_presentation_releases AS presentation
               WHERE presentation.id = $2
                 AND presentation.project_id = project.id
                 AND presentation.game_release_id = project.active_game_release_id
           )",
    )
    .bind(project_id)
    .bind(presentation_id)
    .execute(&mut **transaction)
    .await?;
    if updated.rows_affected() != 1 {
        return Err(ApiError::Conflict(
            "the Presentation release must target the active Game release".to_string(),
        ));
    }
    Ok(())
}

pub async fn get_game_presentation_release(
    pool: &PgPool,
    project_id: Uuid,
    presentation_id: Uuid,
) -> Result<GamePresentationRelease, ApiError> {
    sqlx::query_as::<_, GamePresentationRelease>(
        "SELECT id, project_id, game_release_id, release_number, content_hash,
                binding_manifest, asset_version_ids, created_at
         FROM game_presentation_releases
         WHERE project_id = $1 AND id = $2",
    )
    .bind(project_id)
    .bind(presentation_id)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)
}

pub async fn list_game_presentations(
    pool: &PgPool,
    project_id: Uuid,
) -> Result<Vec<GamePresentationRelease>, ApiError> {
    Ok(sqlx::query_as::<_, GamePresentationRelease>(
        "SELECT id, project_id, game_release_id, release_number, content_hash,
                binding_manifest, asset_version_ids, created_at
         FROM game_presentation_releases
         WHERE project_id = $1 ORDER BY release_number DESC",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await?)
}

pub async fn activate_game_presentation(
    pool: &PgPool,
    project_id: Uuid,
    presentation_id: Uuid,
) -> Result<GamePresentationRelease, ApiError> {
    get_game_presentation_release(pool, project_id, presentation_id).await?;
    let mut transaction = pool.begin().await?;
    activate_presentation_in_transaction(&mut transaction, project_id, presentation_id).await?;
    transaction.commit().await?;
    get_game_presentation_release(pool, project_id, presentation_id).await
}

pub async fn active_presentation_asset(
    pool: &PgPool,
    project_id: Uuid,
    version_id: Uuid,
) -> Result<GameAssetVersion, ApiError> {
    sqlx::query_as::<_, GameAssetVersion>(
        "SELECT version.id, version.project_id, version.asset_id,
                version.content_hash, version.mime_type, version.size_bytes,
                version.storage_key, version.metadata, version.provenance,
                version.rights_status, version.approval_status, version.created_at
         FROM projects AS project
         JOIN game_presentation_releases AS presentation
           ON presentation.id = project.active_game_presentation_release_id
          AND presentation.project_id = project.id
         JOIN game_asset_versions AS version
           ON version.id = ANY(presentation.asset_version_ids)
          AND version.project_id = project.id
         WHERE project.id = $1 AND version.id = $2
           AND version.approval_status = 'approved'",
    )
    .bind(project_id)
    .bind(version_id)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)
}

pub fn random_seed_from_session(session_id: Uuid) -> u64 {
    let bytes = session_id.as_bytes();
    u64::from_be_bytes(bytes[..8].try_into().expect("UUID prefix is eight bytes"))
}

pub fn session_is_terminal(session: &GameSession) -> bool {
    matches!(
        session.snapshot.status,
        SessionStatus::Completed | SessionStatus::Failed | SessionStatus::Cancelled
    )
}

pub fn now_millis() -> i64 {
    Utc::now().timestamp_millis()
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use vifu_game_runtime::GameEvent;

    use super::analytics_dimensions;

    fn event(event_type: &str, data: serde_json::Value) -> GameEvent {
        GameEvent {
            specversion: "1.0".to_string(),
            id: "event-1".to_string(),
            source: "vifu://game-runtime".to_string(),
            event_type: event_type.to_string(),
            subject: Some("choice-one".to_string()),
            sequence: 1,
            data,
        }
    }

    #[test]
    fn analytics_records_choice_identifiers_without_player_content() {
        let dimensions = analytics_dimensions(&event(
            "choice.selected",
            json!({"optionId": "follow", "playerText": "private response"}),
        ))
        .expect("choice analytics");

        assert_eq!(dimensions["optionId"], "follow");
        assert!(dimensions.get("playerText").is_none());
    }

    #[test]
    fn analytics_ignores_dialogue_and_player_input_events() {
        assert!(analytics_dimensions(&event(
            "dialogue.completed",
            json!({"text": "private dialogue"}),
        ))
        .is_none());
    }
}
