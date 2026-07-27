use std::time::Instant;

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Path, Query, State, WebSocketUpgrade};
use axum::http::header::ORIGIN;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{Duration, Utc};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use tracing::warn;
use uuid::Uuid;
use vifu_gateway::runtime_extension::{
    RuntimeExtensionDefinition, RuntimeExtensionManifest, RuntimeProfileInvocation,
    MAX_RUNTIME_RPC_BYTES,
};

use crate::auth::{bearer_token, deployment_credential, hash_api_key, Identity, Operation};
use crate::db;
use crate::error::ApiError;
use crate::models::{
    ApiKeyRecord, CreateProjectRuntimeChannel, CreateRuntimeLaunchSession, ProjectRuntimeExtension,
    ProjectWithBindings, SetProjectRuntimeExtension,
};
use crate::AppState;

#[derive(Debug, Deserialize)]
pub struct RuntimeWebSocketQuery {
    token: String,
}

enum RuntimeAuthority {
    Admin,
    Key(ApiKeyRecord),
    Launch { project_id: Uuid },
}

pub async fn list_runtime_extensions(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    require_deployment_operation(&state, &headers, Operation::DeploymentRead).await?;
    let manifests = state
        .config
        .runtime_extensions
        .iter()
        .map(|extension| &extension.manifest)
        .collect::<Vec<_>>();
    Ok(Json(json!({ "runtimeExtensions": manifests })))
}

pub async fn get_project_runtime_extension(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_slug): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let project = authorized_project(&state, &headers, &project_slug, false).await?;
    let extension = db::get_project_runtime_extension(&state.pool, project.project.id).await?;
    Ok(Json(json!({ "runtimeExtension": extension })))
}

pub async fn set_project_runtime_extension(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_slug): Path<String>,
    Json(input): Json<SetProjectRuntimeExtension>,
) -> Result<Json<Value>, ApiError> {
    let project = authorized_project(&state, &headers, &project_slug, true).await?;
    let definition = configured_extension(&state, &input.extension_id)
        .ok_or_else(|| ApiError::Invalid("runtime extension is not configured".to_string()))?;
    let active_release_ref = input
        .active_release_ref
        .as_deref()
        .map(|value| validated_text("activeReleaseRef", value, 512))
        .transpose()?;
    if !input.metadata.is_object() {
        return Err(ApiError::Invalid(
            "runtime extension metadata must be an object".to_string(),
        ));
    }
    if serde_json::to_vec(&input.metadata)
        .map_err(|_| ApiError::Invalid("runtime extension metadata is invalid".to_string()))?
        .len()
        > 64 * 1024
    {
        return Err(ApiError::Invalid(
            "runtime extension metadata is too large".to_string(),
        ));
    }
    let extension = db::set_project_runtime_extension(
        &state.pool,
        project.project.id,
        &definition.manifest.id,
        input.enabled.unwrap_or(true),
        active_release_ref,
        &input.metadata,
    )
    .await?;
    Ok(Json(json!({ "runtimeExtension": extension })))
}

pub async fn delete_project_runtime_extension(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_slug): Path<String>,
) -> Result<axum::http::StatusCode, ApiError> {
    let project = authorized_project(&state, &headers, &project_slug, true).await?;
    db::delete_project_runtime_extension(&state.pool, project.project.id).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

pub async fn list_project_runtime_channels(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_slug): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let project = authorized_project(&state, &headers, &project_slug, false).await?;
    let channels = db::list_project_runtime_channels(&state.pool, project.project.id).await?;
    Ok(Json(json!({ "channels": channels })))
}

pub async fn create_project_runtime_channel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_slug): Path<String>,
    Json(input): Json<CreateProjectRuntimeChannel>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let project = authorized_project(&state, &headers, &project_slug, true).await?;
    let name = validated_text("channel name", &input.name, 128)?;
    let allowed_origins = input
        .allowed_origins
        .iter()
        .map(|origin| validate_origin(origin))
        .collect::<Result<Vec<_>, _>>()?;
    let launch_key = generate_launch_secret("vifu_channel");
    let launch_key_prefix = launch_key.chars().take(20).collect::<String>();
    let launch_key_hash = hash_api_key(&launch_key, &state.config.api_key_pepper);
    let channel = db::create_project_runtime_channel(
        &state.pool,
        db::NewProjectRuntimeChannel {
            id: Uuid::new_v4(),
            project_id: project.project.id,
            name,
            public_id: Uuid::new_v4(),
            launch_key_prefix: &launch_key_prefix,
            launch_key_hash: &launch_key_hash,
            allowed_origins: &allowed_origins,
        },
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "channel": channel, "launchKey": launch_key })),
    ))
}

pub async fn delete_project_runtime_channel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((project_slug, channel_id)): Path<(String, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let project = authorized_project(&state, &headers, &project_slug, true).await?;
    db::delete_project_runtime_channel(&state.pool, project.project.id, channel_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn create_runtime_launch_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_slug): Path<String>,
    Json(input): Json<CreateRuntimeLaunchSession>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let project = db::get_project_by_slug(&state.pool, &project_slug).await?;
    let launch_key_hash = hash_api_key(&input.launch_key, &state.config.api_key_pepper);
    let channel = db::runtime_channel_for_launch(
        &state.pool,
        project.project.id,
        input.channel_id,
        &launch_key_hash,
    )
    .await?;
    validate_request_origin(&headers, &channel.allowed_origins)?;
    let token = generate_launch_secret("vifu_launch");
    let token_hash = hash_api_key(&token, &state.config.api_key_pepper);
    let expires_at = Utc::now() + Duration::minutes(15);
    db::create_runtime_launch_session(
        &state.pool,
        Uuid::new_v4(),
        project.project.id,
        channel.id,
        &token_hash,
        expires_at,
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "session": {
                "token": token,
                "expiresAt": expires_at,
                "projectSlug": project.project.slug,
            }
        })),
    ))
}

pub async fn invoke_project_profile_for_extension(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((extension_id, project_id)): Path<(String, Uuid)>,
    Json(input): Json<RuntimeProfileInvocation>,
) -> Result<Json<Value>, ApiError> {
    let token = bearer_token(&headers).ok_or(ApiError::Unauthorized)?;
    let definition = configured_extension(&state, &extension_id)
        .filter(|extension| extension.authenticates(token))
        .ok_or(ApiError::Unauthorized)?;
    db::get_project_runtime_extension(&state.pool, project_id)
        .await?
        .filter(|attachment| extension_allows_effects(attachment, &definition.manifest.id))
        .ok_or(ApiError::Forbidden)?;
    let project = db::get_project(&state.pool, project_id).await?;
    let request_id = Uuid::new_v4();
    let request_summary = json!({
        "extensionId": definition.manifest.id,
        "profileId": input.profile_id,
        "profileVersionId": input.profile_version_id,
        "capability": input.capability,
        "operationId": input.operation_id,
    });
    let trace_id = db::create_trace(
        &state.pool,
        db::NewTrace {
            request_id,
            endpoint_id: None,
            project_id: Some(project_id),
            gateway_session_id: None,
            profile_id: Some(input.profile_id),
            profile_version_id: Some(input.profile_version_id),
            operation: "runtime.extension.effect",
            provider_key: None,
            capability_kind: Some(&input.capability),
            selection_key: Some(&input.operation_id),
            request: &request_summary,
        },
    )
    .await?;
    let span_id = db::create_trace_span(
        &state.pool,
        db::NewTraceSpan {
            trace_id,
            parent_span_id: None,
            name: "runtime.extension.effect",
            kind: "runtime_extension",
            provider_key: None,
            capability_kind: Some(&input.capability),
            input_summary: Some(&request_summary),
            attributes: &json!({ "extensionId": definition.manifest.id }),
        },
    )
    .await?;
    let started_at = Instant::now();
    match crate::api::invoke_runtime_extension_profile(&state, &project, &input, request_id).await {
        Ok(output) => {
            db::complete_trace_span(
                &state.pool,
                span_id,
                "completed",
                db::elapsed_millis(started_at),
                Some(&output),
                None,
            )
            .await?;
            db::complete_trace(
                &state.pool,
                request_id,
                "completed",
                db::elapsed_millis(started_at),
                Some(&output),
                None,
            )
            .await?;
            Ok(Json(json!({ "output": output })))
        }
        Err(error) => {
            let message = error.to_string();
            db::complete_trace_span(
                &state.pool,
                span_id,
                "failed",
                db::elapsed_millis(started_at),
                None,
                Some(&message),
            )
            .await?;
            db::complete_trace(
                &state.pool,
                request_id,
                "failed",
                db::elapsed_millis(started_at),
                None,
                Some(&message),
            )
            .await?;
            Err(error)
        }
    }
}

pub async fn invoke_project_runtime(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_slug): Path<String>,
    Json(request): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let authority = runtime_authority(&state, deployment_credential(&headers)).await?;
    let (project, attachment, definition) =
        resolve_runtime(&state, &authority, &project_slug).await?;
    let (request_id_value, method) = match validate_rpc_request(&request) {
        Ok(request) => request,
        Err(response) => return Ok(Json(response)),
    };
    if !definition.manifest.allows_method(method) {
        return Ok(Json(rpc_error(
            request_id_value,
            -32601,
            "method_not_found",
            "The runtime method is not available.",
        )));
    }

    let request_id = Uuid::new_v4();
    let trace_id = db::create_trace(
        &state.pool,
        db::NewTrace {
            request_id,
            endpoint_id: None,
            project_id: Some(project.project.id),
            gateway_session_id: None,
            profile_id: None,
            profile_version_id: None,
            operation: "runtime.rpc",
            provider_key: None,
            capability_kind: Some("runtime"),
            selection_key: Some(method),
            request: &request,
        },
    )
    .await?;
    let span_id = db::create_trace_span(
        &state.pool,
        db::NewTraceSpan {
            trace_id,
            parent_span_id: None,
            name: "runtime.extension",
            kind: "runtime_extension",
            provider_key: None,
            capability_kind: Some("runtime.rpc"),
            input_summary: Some(&json!({ "method": method })),
            attributes: &json!({
                "extensionId": definition.manifest.id,
                "releaseRef": attachment.active_release_ref,
            }),
        },
    )
    .await?;
    let started_at = Instant::now();
    let response = definition
        .call_rpc(
            project.project.id,
            &project.project.slug,
            attachment
                .active_release_ref
                .as_deref()
                .expect("resolved runtime has a release"),
            request_id,
            &request,
            state.config.request_timeout,
        )
        .await;

    match response {
        Ok(response) => {
            let failed = response.get("error").is_some();
            let status = if failed { "failed" } else { "completed" };
            db::complete_trace_span(
                &state.pool,
                span_id,
                status,
                db::elapsed_millis(started_at),
                Some(&response),
                None,
            )
            .await?;
            db::complete_trace(
                &state.pool,
                request_id,
                status,
                db::elapsed_millis(started_at),
                Some(&response),
                None,
            )
            .await?;
            Ok(Json(response))
        }
        Err(error) => {
            warn!(
                extension_id = %definition.manifest.id,
                %request_id,
                error = %error,
                "runtime extension request failed"
            );
            db::complete_trace_span(
                &state.pool,
                span_id,
                "failed",
                db::elapsed_millis(started_at),
                None,
                Some("runtime extension unavailable"),
            )
            .await?;
            db::complete_trace(
                &state.pool,
                request_id,
                "failed",
                db::elapsed_millis(started_at),
                None,
                Some("runtime extension unavailable"),
            )
            .await?;
            Ok(Json(rpc_error(
                request_id_value,
                -32002,
                "runtime_extension_unavailable",
                "The project runtime is temporarily unavailable.",
            )))
        }
    }
}

pub async fn connect_project_runtime(
    State(state): State<AppState>,
    Path(project_slug): Path<String>,
    Query(query): Query<RuntimeWebSocketQuery>,
    upgrade: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let authority = runtime_authority(&state, Some(query.token.trim())).await?;
    let (project, attachment, definition) =
        resolve_runtime(&state, &authority, &project_slug).await?;
    let request_id = Uuid::new_v4();
    let upstream = definition
        .connect_rpc_websocket(
            project.project.id,
            &project.project.slug,
            attachment
                .active_release_ref
                .as_deref()
                .expect("resolved runtime has a release"),
            request_id,
        )
        .await
        .map_err(|error| {
            warn!(
                extension_id = %definition.manifest.id,
                %request_id,
                error = %error,
                "runtime extension WebSocket connection failed"
            );
            ApiError::RuntimeExtensionUnavailable
        })?;
    let manifest = definition.manifest.clone();
    Ok(upgrade
        .max_message_size(MAX_RUNTIME_RPC_BYTES)
        .max_frame_size(MAX_RUNTIME_RPC_BYTES)
        .on_upgrade(move |socket| bridge_runtime_socket(socket, upstream, manifest))
        .into_response())
}

async fn bridge_runtime_socket(
    downstream: WebSocket,
    upstream: vifu_gateway::runtime_extension::RuntimeExtensionWebSocket,
    manifest: RuntimeExtensionManifest,
) {
    let (mut downstream_tx, mut downstream_rx) = downstream.split();
    let (mut upstream_tx, mut upstream_rx) = upstream.split();
    loop {
        tokio::select! {
            incoming = downstream_rx.next() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        let parsed = serde_json::from_str::<Value>(text.as_str()).ok().and_then(
                            |value| {
                                let method = validate_rpc_request(&value)
                                    .ok()
                                    .map(|(_, method)| method.to_string())?;
                                Some((value, method))
                            },
                        );
                        match parsed {
                            Some((value, method)) if manifest.allows_method(&method) => {
                                let encoded = match serde_json::to_string(&value) {
                                    Ok(encoded) => encoded,
                                    Err(_) => break,
                                };
                                if upstream_tx
                                    .send(tokio_tungstenite::tungstenite::Message::Text(encoded.into()))
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                            }
                            Some((value, _)) => {
                                let response = rpc_error(
                                    value.get("id").cloned().unwrap_or(Value::Null),
                                    -32601,
                                    "method_not_found",
                                    "The runtime method is not available.",
                                );
                                if downstream_tx
                                    .send(Message::Text(response.to_string().into()))
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                            }
                            None => {
                                let response = rpc_error(
                                    Value::Null,
                                    -32600,
                                    "invalid_request",
                                    "The JSON-RPC request is invalid.",
                                );
                                if downstream_tx
                                    .send(Message::Text(response.to_string().into()))
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                            }
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        if downstream_tx.send(Message::Pong(payload)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    Some(Ok(Message::Binary(_))) => {
                        let _ = downstream_tx.send(Message::Close(None)).await;
                        break;
                    }
                }
            }
            incoming = upstream_rx.next() => {
                match incoming {
                    Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) => {
                        if downstream_tx.send(Message::Text(text.to_string().into())).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(tokio_tungstenite::tungstenite::Message::Binary(bytes))) => {
                        if downstream_tx.send(Message::Binary(bytes.to_vec().into())).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(tokio_tungstenite::tungstenite::Message::Ping(bytes))) => {
                        if upstream_tx.send(tokio_tungstenite::tungstenite::Message::Pong(bytes)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(tokio_tungstenite::tungstenite::Message::Pong(_))) => {}
                    Some(Ok(tokio_tungstenite::tungstenite::Message::Close(_)))
                    | None
                    | Some(Err(_)) => break,
                    Some(Ok(tokio_tungstenite::tungstenite::Message::Frame(_))) => {}
                }
            }
        }
    }
    let _ = downstream_tx.send(Message::Close(None)).await;
    let _ = upstream_tx
        .send(tokio_tungstenite::tungstenite::Message::Close(None))
        .await;
}

async fn resolve_runtime(
    state: &AppState,
    authority: &RuntimeAuthority,
    project_slug: &str,
) -> Result<
    (
        crate::models::ProjectWithBindings,
        ProjectRuntimeExtension,
        RuntimeExtensionDefinition,
    ),
    ApiError,
> {
    let project = db::get_project_by_slug(&state.pool, project_slug).await?;
    if let RuntimeAuthority::Key(key) = authority {
        if key.project_id != project.project.id {
            return Err(ApiError::Forbidden);
        }
        if !key.permissions.runtime_allowed() {
            return Err(ApiError::EndpointAccessDenied);
        }
    }
    if let RuntimeAuthority::Launch { project_id } = authority {
        if *project_id != project.project.id {
            return Err(ApiError::Forbidden);
        }
    }
    let attachment = db::get_project_runtime_extension(&state.pool, project.project.id)
        .await?
        .filter(|extension| extension.enabled)
        .ok_or(ApiError::RuntimeNotPublished)?;
    if attachment.active_release_ref.is_none() {
        return Err(ApiError::RuntimeNotPublished);
    }
    let definition = configured_extension(state, &attachment.extension_id)
        .cloned()
        .ok_or(ApiError::RuntimeExtensionUnavailable)?;
    Ok((project, attachment, definition))
}

async fn runtime_authority(
    state: &AppState,
    token: Option<&str>,
) -> Result<RuntimeAuthority, ApiError> {
    let token = token
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .ok_or(ApiError::Unauthorized)?;
    if matches!(
        state
            .auth
            .authorize_token(token, Operation::DeploymentWrite)
            .await,
        Ok(Identity::DeploymentAdmin)
    ) {
        return Ok(RuntimeAuthority::Admin);
    }
    let key_hash = hash_api_key(token, &state.config.api_key_pepper);
    if let Some(key) = db::active_api_key_by_hash_optional(&state.pool, &key_hash).await? {
        return Ok(RuntimeAuthority::Key(key));
    }
    if let Some(project_id) = db::active_runtime_launch_project(&state.pool, &key_hash).await? {
        return Ok(RuntimeAuthority::Launch { project_id });
    }
    Err(ApiError::Forbidden)
}

async fn require_deployment_operation(
    state: &AppState,
    headers: &HeaderMap,
    operation: Operation,
) -> Result<(), ApiError> {
    match state.auth.authorize(headers, operation).await? {
        Identity::DeploymentAdmin => Ok(()),
        Identity::ActingUser { .. } => Err(ApiError::Forbidden),
    }
}

async fn authorized_project(
    state: &AppState,
    headers: &HeaderMap,
    slug: &str,
    write: bool,
) -> Result<ProjectWithBindings, ApiError> {
    let project = db::get_project_by_slug(&state.pool, slug).await?;
    state
        .auth
        .authorize_project(
            headers,
            if write {
                Operation::ProjectWrite
            } else {
                Operation::ProjectRead
            },
            project.project.owner_user_id.as_deref(),
        )
        .await?;
    Ok(project)
}

fn configured_extension<'a>(
    state: &'a AppState,
    extension_id: &str,
) -> Option<&'a RuntimeExtensionDefinition> {
    state
        .config
        .runtime_extensions
        .iter()
        .find(|extension| extension.manifest.id == extension_id)
}

fn extension_allows_effects(attachment: &ProjectRuntimeExtension, extension_id: &str) -> bool {
    attachment.enabled && attachment.extension_id == extension_id
}

fn validate_rpc_request(request: &Value) -> Result<(Value, &str), Value> {
    let Some(object) = request.as_object() else {
        return Err(rpc_error(
            Value::Null,
            -32600,
            "invalid_request",
            "The JSON-RPC request must be an object.",
        ));
    };
    let id = object.get("id").cloned().unwrap_or(Value::Null);
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err(rpc_error(
            id,
            -32600,
            "invalid_request",
            "The JSON-RPC version must be 2.0.",
        ));
    }
    let Some(method) = object
        .get("method")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|method| !method.is_empty())
    else {
        return Err(rpc_error(
            id,
            -32600,
            "invalid_request",
            "The JSON-RPC method is required.",
        ));
    };
    if vifu_gateway::protocol::validate_identifier("runtime RPC method", method).is_err() {
        return Err(rpc_error(
            id,
            -32600,
            "invalid_request",
            "The JSON-RPC method is invalid.",
        ));
    }
    Ok((id, method))
}

fn rpc_error(id: Value, code: i64, name: &str, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message,
            "data": { "code": name }
        }
    })
}

fn validated_text<'a>(name: &str, value: &'a str, max: usize) -> Result<&'a str, ApiError> {
    let value = value.trim();
    if value.is_empty() || value.len() > max || value.chars().any(char::is_control) {
        return Err(ApiError::Invalid(format!("{name} is invalid")));
    }
    Ok(value)
}

fn validate_origin(origin: &str) -> Result<String, ApiError> {
    let origin = origin.trim().trim_end_matches('/');
    let uri = origin
        .parse::<Uri>()
        .map_err(|_| ApiError::Invalid("allowed origin is invalid".to_string()))?;
    if !matches!(uri.scheme_str(), Some("http" | "https"))
        || uri.authority().is_none()
        || !matches!(uri.path(), "" | "/")
        || uri.query().is_some()
    {
        return Err(ApiError::Invalid(
            "allowed origin must be an HTTP or HTTPS origin".to_string(),
        ));
    }
    Ok(origin.to_string())
}

fn validate_request_origin(
    headers: &HeaderMap,
    allowed_origins: &[String],
) -> Result<(), ApiError> {
    if allowed_origins.is_empty() {
        return Ok(());
    }
    let origin = headers
        .get(ORIGIN)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .ok_or(ApiError::Forbidden)?;
    if allowed_origins.iter().any(|allowed| allowed == origin) {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

fn generate_launch_secret(prefix: &str) -> String {
    format!(
        "{prefix}_{}{}",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    )
}

#[cfg(test)]
mod tests {
    use axum::http::{header::ORIGIN, HeaderMap, HeaderValue};
    use chrono::Utc;
    use serde_json::json;
    use uuid::Uuid;

    use super::{
        extension_allows_effects, validate_origin, validate_request_origin, validate_rpc_request,
    };
    use crate::models::ProjectRuntimeExtension;

    #[test]
    fn accepts_a_json_rpc_request() {
        let request = json!({
            "jsonrpc": "2.0",
            "id": "request-1",
            "method": "session.create",
            "params": {}
        });
        let (id, method) = validate_rpc_request(&request).unwrap();
        assert_eq!(id, "request-1");
        assert_eq!(method, "session.create");
    }

    #[test]
    fn rejects_a_batch_request() {
        let error = validate_rpc_request(&json!([])).unwrap_err();
        assert_eq!(error["error"]["code"], -32600);
    }

    #[test]
    fn launch_origins_are_exact_and_optional() {
        assert_eq!(
            validate_origin("https://play.example.com/").unwrap(),
            "https://play.example.com"
        );
        assert!(validate_origin("https://play.example.com/path").is_err());

        let mut headers = axum::http::HeaderMap::new();
        headers.insert(ORIGIN, HeaderValue::from_static("https://play.example.com"));
        assert!(
            validate_request_origin(&headers, &["https://play.example.com".to_string()]).is_ok()
        );
        assert!(
            validate_request_origin(&headers, &["https://other.example.com".to_string()]).is_err()
        );
        assert!(validate_request_origin(&HeaderMap::new(), &[]).is_ok());
    }

    #[test]
    fn trusted_attached_extensions_can_run_preview_effects_before_publish() {
        let now = Utc::now();
        let attachment = ProjectRuntimeExtension {
            project_id: Uuid::new_v4(),
            extension_id: "vifu.content".to_string(),
            enabled: true,
            active_release_ref: None,
            metadata: json!({}),
            created_at: now,
            updated_at: now,
        };

        assert!(extension_allows_effects(&attachment, "vifu.content"));
        assert!(!extension_allows_effects(&attachment, "other.extension"));
    }
}
