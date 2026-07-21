use std::convert::Infallible;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures_util::stream;
use serde_json::{json, Value};
use uuid::Uuid;
use vifu_game_runtime::{GameCommand, ValidationSeverity};

use crate::auth::{bearer_token, hash_api_key, is_secret_match};
use crate::db as runtime_db;
use crate::error::ApiError;
use crate::AppState;

use super::db;
use super::models::{
    CreateGameSession, PublicGameSession, PublicRuntimeAdvance, PublishGame, RunGame,
    UpdateGameSource, ValidateGame,
};
use super::service;

#[derive(Clone, Copy)]
struct GameAuthority {
    api_key_id: Option<Uuid>,
    admin: bool,
}

pub async fn get_game_overview(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &headers)?;
    let project = runtime_db::get_project_by_slug(&state.pool, &slug).await?;
    let draft = service::ensure_draft(&state.pool, &project).await?;
    let overview = db::game_overview(&state.pool, &project, &draft).await?;
    Ok(Json(json!({"game": overview})))
}

pub async fn get_game_source(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &headers)?;
    let project = runtime_db::get_project_by_slug(&state.pool, &slug).await?;
    let draft = service::ensure_draft(&state.pool, &project).await?;
    Ok(Json(json!({"draft": draft})))
}

pub async fn put_game_source(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(input): Json<UpdateGameSource>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &headers)?;
    let project = runtime_db::get_project_by_slug(&state.pool, &slug).await?;
    let draft = service::update_draft(
        &state.pool,
        &project,
        &input.source,
        input.expected_revision,
        input.expected_hash.as_deref(),
    )
    .await?;
    Ok(Json(json!({"draft": draft})))
}

pub async fn import_game_source(
    state: State<AppState>,
    headers: HeaderMap,
    path: Path<String>,
    input: Json<UpdateGameSource>,
) -> Result<Json<Value>, ApiError> {
    put_game_source(state, headers, path, input).await
}

pub async fn export_game_source(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Result<Response, ApiError> {
    require_admin(&state, &headers)?;
    let project = runtime_db::get_project_by_slug(&state.pool, &slug).await?;
    let draft = service::ensure_draft(&state.pool, &project).await?;
    let body = serde_json::to_vec_pretty(&draft.source).map_err(|_| ApiError::Internal)?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/json; charset=utf-8"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=game-source.vifu.json",
            ),
        ],
        body,
    )
        .into_response())
}

pub async fn list_node_definitions(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &headers)?;
    let definitions: Vec<_> = vifu_game_runtime::GameCompiler::default()
        .registry()
        .definitions()
        .cloned()
        .collect();
    Ok(Json(json!({"nodeDefinitions": definitions})))
}

pub async fn validate_game(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(input): Json<ValidateGame>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &headers)?;
    let project = runtime_db::get_project_by_slug(&state.pool, &slug).await?;
    let source = match input.source {
        Some(source) => source,
        None => service::ensure_draft(&state.pool, &project).await?.source,
    };
    service::validate_source_limits(&source)?;
    let (_, issues) = service::validate_for_project(&state.pool, &project, &source).await?;
    let valid = issues
        .iter()
        .all(|issue| issue.severity != ValidationSeverity::Error);
    Ok(Json(json!({"valid": valid, "issues": issues})))
}

pub async fn publish_game(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(input): Json<PublishGame>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    require_admin(&state, &headers)?;
    let project = runtime_db::get_project_by_slug(&state.pool, &slug).await?;
    let change_summary = normalized_summary(input.change_summary.as_deref())?;
    let selection_key = input.expected_revision.to_string();
    let request_summary = json!({
        "expectedRevision": input.expected_revision,
        "hasChangeSummary": change_summary.is_some(),
    });
    let trace = super::tracing::start(
        &state,
        project.project.id,
        "game.publish",
        Some(&selection_key),
        &request_summary,
    )
    .await;
    let release = match service::publish(
        &state.pool,
        &project,
        input.expected_revision,
        change_summary.as_deref(),
    )
    .await
    {
        Ok(release) => release,
        Err(error) => {
            super::tracing::fail(&state, trace, &error.to_string()).await;
            return Err(error);
        }
    };
    super::tracing::complete(
        &state,
        trace,
        None,
        &json!({
            "releaseId": release.id,
            "releaseNumber": release.release_number,
            "contentHash": release.content_hash,
        }),
    )
    .await;
    Ok((StatusCode::CREATED, Json(json!({"release": release}))))
}

pub async fn list_game_releases(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &headers)?;
    let project = runtime_db::get_project_by_slug(&state.pool, &slug).await?;
    let releases = db::list_game_releases(&state.pool, project.project.id).await?;
    Ok(Json(json!({"releases": releases})))
}

pub async fn activate_game_release(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((slug, release_id)): Path<(String, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &headers)?;
    let project = runtime_db::get_project_by_slug(&state.pool, &slug).await?;
    let release = db::activate_game_release(&state.pool, project.project.id, release_id).await?;
    Ok(Json(json!({"release": release})))
}

pub async fn get_runtime_game(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_slug): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let (project, _) = authorize_game(&state, &headers, &project_slug).await?;
    let release = db::active_game_release(&state.pool, project.project.id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(json!({
        "game": {
            "projectSlug": project.project.slug,
            "releaseId": release.id,
            "releaseNumber": release.release_number,
            "contentHash": release.content_hash,
            "createdAt": release.created_at,
            "compatibility": release.manifest.compatibility,
            "inputs": release.manifest.inputs,
            "outputs": release.manifest.outputs,
            "requiredHostCapabilities": release.manifest.required_host_capabilities,
            "optionalHostCapabilities": release.manifest.optional_host_capabilities
        }
    })))
}

pub async fn get_runtime_manifest(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_slug): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let (project, _) = authorize_game(&state, &headers, &project_slug).await?;
    let release = db::active_game_release(&state.pool, project.project.id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(json!({
        "releaseId": release.id,
        "contentHash": release.content_hash,
        "manifest": release.manifest
    })))
}

pub async fn get_runtime_presentation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_slug): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let (project, _) = authorize_game(&state, &headers, &project_slug).await?;
    let presentation = sqlx::query_scalar::<_, Value>(
        "SELECT presentation.binding_manifest
         FROM projects AS project
         JOIN game_presentation_releases AS presentation
           ON presentation.id = project.active_game_presentation_release_id
          AND presentation.game_release_id = project.active_game_release_id
         WHERE project.id = $1",
    )
    .bind(project.project.id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(ApiError::NotFound)?;
    Ok(Json(json!({"presentation": presentation})))
}

pub async fn create_runtime_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_slug): Path<String>,
    Json(input): Json<CreateGameSession>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let (project, authority) = authorize_game(&state, &headers, &project_slug).await?;
    let release = db::active_game_release(&state.pool, project.project.id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let missing_optional = service::validate_host(&release.manifest, &input.host)?;
    let random_seed = input.random_seed.unwrap_or_else(random_seed);
    let session = db::create_game_session(
        &state.pool,
        project.project.id,
        &release,
        authority.api_key_id,
        &input.host,
        random_seed,
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "session": PublicGameSession::try_from(&session)?,
            "optionalCapabilityFallbacks": missing_optional
        })),
    ))
}

pub async fn get_runtime_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((project_slug, session_id)): Path<(String, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    let (project, _) = authorize_game(&state, &headers, &project_slug).await?;
    let session =
        db::get_published_game_session(&state.pool, project.project.id, session_id).await?;
    Ok(Json(
        json!({"session": PublicGameSession::try_from(&session)?}),
    ))
}

pub async fn submit_runtime_command(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((project_slug, session_id)): Path<(String, Uuid)>,
    Json(command): Json<GameCommand>,
) -> Result<Json<Value>, ApiError> {
    let (project, authority) = authorize_game(&state, &headers, &project_slug).await?;
    authorize_session_command(&state, project.project.id, session_id, authority).await?;
    let selection_key = session_id.to_string();
    let trace = super::tracing::start(
        &state,
        project.project.id,
        "game.command",
        Some(&selection_key),
        &json!({
            "sessionId": session_id,
            "commandType": command.command_type,
            "expectedRevision": command.expected_revision,
        }),
    )
    .await;
    let advance = match db::submit_game_command(
        &state.pool,
        project.project.id,
        session_id,
        &command,
        trace
            .as_ref()
            .map(super::tracing::GameTrace::effect_context),
    )
    .await
    {
        Ok(advance) => advance,
        Err(error) => {
            super::tracing::fail(&state, trace, &error.to_string()).await;
            return Err(error);
        }
    };
    super::tracing::complete(
        &state,
        trace,
        Some(&advance),
        &json!({
            "sessionId": session_id,
            "status": advance.snapshot.status,
            "revision": advance.snapshot.revision,
            "eventCount": advance.events.len(),
            "effectCount": advance.effects.len(),
        }),
    )
    .await;
    Ok(Json(
        json!({"advance": PublicRuntimeAdvance::from(&advance)}),
    ))
}

pub async fn stream_runtime_events(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((project_slug, session_id)): Path<(String, Uuid)>,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let (project, authority) = authorize_game(&state, &headers, &project_slug).await?;
    authorize_session_command(&state, project.project.id, session_id, authority).await?;
    let after_sequence = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .map(str::parse::<u64>)
        .transpose()
        .map_err(|_| ApiError::Invalid("Last-Event-ID must be an integer".to_string()))?
        .unwrap_or_default();
    let stream_state = EventStreamState {
        pool: state.pool,
        project_id: project.project.id,
        session_id,
        next_sequence: after_sequence,
        closed: false,
    };
    let events = stream::unfold(stream_state, |mut stream_state| async move {
        if stream_state.closed {
            return None;
        }
        loop {
            match db::list_game_events_after(
                &stream_state.pool,
                stream_state.session_id,
                stream_state.next_sequence,
                100,
            )
            .await
            {
                Ok(events) if !events.is_empty() => {
                    let event = events[0].clone();
                    stream_state.next_sequence = event.sequence;
                    let data = serde_json::to_string(&event)
                        .unwrap_or_else(|_| r#"{"type":"game.stream.failed"}"#.to_string());
                    let sse = Event::default()
                        .id(event.sequence.to_string())
                        .event(event.event_type)
                        .data(data);
                    return Some((Ok(sse), stream_state));
                }
                Ok(_) => match db::get_game_session(
                    &stream_state.pool,
                    stream_state.project_id,
                    stream_state.session_id,
                )
                .await
                {
                    Ok(session) if db::session_is_terminal(&session) => return None,
                    Ok(_) => tokio::time::sleep(Duration::from_millis(350)).await,
                    Err(_) => {
                        stream_state.closed = true;
                        let event = Event::default()
                            .event("game.stream.failed")
                            .data(r#"{"message":"event stream is unavailable"}"#);
                        return Some((Ok(event), stream_state));
                    }
                },
                Err(_) => {
                    stream_state.closed = true;
                    let event = Event::default()
                        .event("game.stream.failed")
                        .data(r#"{"message":"event stream is unavailable"}"#);
                    return Some((Ok(event), stream_state));
                }
            }
        }
    });
    Ok(Sse::new(events).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    ))
}

pub async fn run_game(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_slug): Path<String>,
    Json(input): Json<RunGame>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let (project, authority) = authorize_game(&state, &headers, &project_slug).await?;
    let release = db::active_game_release(&state.pool, project.project.id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let missing_optional = service::validate_host(&release.manifest, &input.host)?;
    let session = db::create_game_session(
        &state.pool,
        project.project.id,
        &release,
        authority.api_key_id,
        &input.host,
        random_seed(),
    )
    .await?;
    let selection_key = session.id.to_string();
    let trace = super::tracing::start(
        &state,
        project.project.id,
        "game.run",
        Some(&selection_key),
        &json!({
            "sessionId": session.id,
            "releaseId": release.id,
            "hostEngine": input.host.engine,
            "hostCapabilityCount": input.host.capabilities.len(),
        }),
    )
    .await;
    let advance = match db::submit_game_command(
        &state.pool,
        project.project.id,
        session.id,
        &GameCommand {
            idempotency_key: input
                .idempotency_key
                .unwrap_or_else(|| format!("run:{}", Uuid::new_v4())),
            expected_revision: Some(session.revision),
            command_type: "game.start".to_string(),
            data: input.input,
        },
        trace
            .as_ref()
            .map(super::tracing::GameTrace::effect_context),
    )
    .await
    {
        Ok(advance) => advance,
        Err(error) => {
            super::tracing::fail(&state, trace, &error.to_string()).await;
            return Err(error);
        }
    };
    super::tracing::complete(
        &state,
        trace,
        Some(&advance),
        &json!({
            "sessionId": session.id,
            "releaseId": release.id,
            "status": advance.snapshot.status,
            "revision": advance.snapshot.revision,
            "eventCount": advance.events.len(),
            "effectCount": advance.effects.len(),
        }),
    )
    .await;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "sessionId": session.id,
            "releaseId": release.id,
            "advance": PublicRuntimeAdvance::from(&advance),
            "optionalCapabilityFallbacks": missing_optional
        })),
    ))
}

struct EventStreamState {
    pool: sqlx::PgPool,
    project_id: Uuid,
    session_id: Uuid,
    next_sequence: u64,
    closed: bool,
}

fn require_admin(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    let token = bearer_token(headers).ok_or(ApiError::Unauthorized)?;
    if is_secret_match(token, &state.config.admin_key) {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

async fn authorize_game(
    state: &AppState,
    headers: &HeaderMap,
    project_slug: &str,
) -> Result<(crate::models::ProjectWithBindings, GameAuthority), ApiError> {
    let project = runtime_db::get_project_by_slug(&state.pool, project_slug).await?;
    let token = bearer_token(headers).ok_or(ApiError::Unauthorized)?;
    if is_secret_match(token, &state.config.admin_key) {
        return Ok((
            project,
            GameAuthority {
                api_key_id: None,
                admin: true,
            },
        ));
    }
    let key_hash = hash_api_key(token, &state.config.api_key_pepper);
    let key = runtime_db::active_api_key_by_hash(&state.pool, &key_hash).await?;
    if key.project_id != project.project.id || !key.permissions.game_allowed() {
        return Err(ApiError::EndpointAccessDenied);
    }
    Ok((
        project,
        GameAuthority {
            api_key_id: Some(key.id),
            admin: false,
        },
    ))
}

async fn authorize_session_command(
    state: &AppState,
    project_id: Uuid,
    session_id: Uuid,
    authority: GameAuthority,
) -> Result<(), ApiError> {
    let session = db::get_game_session(&state.pool, project_id, session_id).await?;
    if session.is_preview && !authority.admin {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

pub(super) async fn authorize_game_project(
    state: &AppState,
    headers: &HeaderMap,
    project_slug: &str,
) -> Result<crate::models::ProjectWithBindings, ApiError> {
    authorize_game(state, headers, project_slug)
        .await
        .map(|(project, _)| project)
}

fn normalized_summary(summary: Option<&str>) -> Result<Option<String>, ApiError> {
    let summary = summary.map(str::trim).filter(|summary| !summary.is_empty());
    if summary.is_some_and(|summary| summary.len() > 2_000) {
        return Err(ApiError::Invalid(
            "changeSummary must not exceed 2000 characters".to_string(),
        ));
    }
    Ok(summary.map(str::to_string))
}

fn random_seed() -> u64 {
    let id = Uuid::new_v4();
    let mut bytes = [0_u8; 8];
    bytes.copy_from_slice(&id.as_bytes()[..8]);
    u64::from_be_bytes(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_empty_change_summaries() {
        assert_eq!(normalized_summary(Some("  ")).unwrap(), None);
        assert_eq!(
            normalized_summary(Some("  First release ")).unwrap(),
            Some("First release".to_string())
        );
    }

    #[test]
    fn generated_random_seed_is_not_constant() {
        assert_ne!(random_seed(), random_seed());
    }

    #[test]
    fn game_source_remains_the_import_contract() {
        let source = vifu_game_runtime::GameSourceV1::new("Import");
        let body = serde_json::to_value(source).unwrap();
        assert_eq!(body["schemaVersion"], 1);
    }

    #[test]
    fn host_descriptor_is_engine_neutral() {
        let host = vifu_game_runtime::HostDescriptor {
            engine: "godot".to_string(),
            adapter_version: Some("1".to_string()),
            capabilities: vec![],
            locale: None,
            binding_manifest_hash: None,
        };
        assert_eq!(host.engine, "godot");
    }
}
