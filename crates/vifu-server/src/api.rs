use std::time::{Duration, Instant};

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::warn;
use uuid::Uuid;
use vifu::protocol::validate_identifier;

use crate::auth::{bearer_token, hash_api_key, is_secret_match, require_admin};
use crate::config::DeploymentMode;
use crate::db::{self, EndpointPatch, NewEndpoint, ProfilePatch};
use crate::error::ApiError;
use crate::models::{
    slugify, validate_slug, Capabilities, CreateApiKey, CreateBinding, CreateEndpoint,
    CreateProfile, CreatedApiKey, InvokeEndpoint, UpdateBinding, UpdateEndpoint, UpdateProfile,
};
use crate::relay::RelayCallError;
use crate::AppState;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HealthResponse {
    service: &'static str,
    status: &'static str,
    version: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusResponse {
    service: &'static str,
    status: &'static str,
    version: &'static str,
    mode: DeploymentMode,
    capabilities: Capabilities,
    connections: usize,
}

pub async fn health() -> Json<impl Serialize> {
    Json(HealthResponse {
        service: "vifu-server",
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}

pub async fn status(State(state): State<AppState>) -> Result<Json<impl Serialize>, ApiError> {
    db::ready(&state.pool).await?;
    // This binary owns runtime capabilities only. A managed control plane may
    // augment this response after it has authenticated the account session.
    let capabilities = Capabilities::self_hosted();
    Ok(Json(StatusResponse {
        service: "vifu-server",
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        mode: state.config.deployment_mode,
        capabilities,
        connections: state.relay.connection_count().await,
    }))
}

pub async fn list_profiles(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    admin(&state, &headers)?;
    Ok(Json(
        json!({ "profiles": db::list_profiles(&state.pool).await? }),
    ))
}

pub async fn create_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<CreateProfile>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    admin(&state, &headers)?;
    let name = required_text("name", &input.name, 128)?;
    let slug = profile_slug(input.slug.as_deref(), name)?;
    let description = optional_text("description", input.description.as_deref(), 4096)?;
    let instructions = optional_text("instructions", input.instructions.as_deref(), 64 * 1024)?;
    let profile = db::create_profile(
        &state.pool,
        Uuid::new_v4(),
        &slug,
        name,
        description,
        instructions,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(json!({ "profile": profile }))))
}

pub async fn get_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    admin(&state, &headers)?;
    Ok(Json(
        json!({ "profile": db::get_profile(&state.pool, id).await? }),
    ))
}

pub async fn update_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(input): Json<UpdateProfile>,
) -> Result<Json<Value>, ApiError> {
    admin(&state, &headers)?;
    let slug = input
        .slug
        .as_deref()
        .map(validate_explicit_slug)
        .transpose()?;
    let name = input
        .name
        .as_deref()
        .map(|value| required_text("name", value, 128))
        .transpose()?;
    let (description_changed, description) =
        patch_text("description", input.description.as_deref(), 4096)?;
    let (instructions_changed, instructions) =
        patch_text("instructions", input.instructions.as_deref(), 64 * 1024)?;
    let profile = db::update_profile(
        &state.pool,
        id,
        ProfilePatch {
            slug: slug.as_deref(),
            name,
            description_changed,
            description,
            instructions_changed,
            instructions,
        },
    )
    .await?;
    Ok(Json(json!({ "profile": profile })))
}

pub async fn delete_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    admin(&state, &headers)?;
    db::delete_profile(&state.pool, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_bindings(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    admin(&state, &headers)?;
    Ok(Json(
        json!({ "bindings": db::list_bindings(&state.pool).await? }),
    ))
}

pub async fn create_binding(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<CreateBinding>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    admin(&state, &headers)?;
    let provider = required_identifier("provider", &input.provider)?;
    if provider != "openclaw" {
        return Err(ApiError::Invalid(
            "openclaw is the only connector provider in this release".to_string(),
        ));
    }
    let connector_id = required_identifier("connector id", &input.connector_id)?;
    let agent_id = required_identifier("agent id", &input.agent_id)?;
    validate_json_object("config", &input.config, 64 * 1024)?;
    let binding = db::create_binding(
        &state.pool,
        Uuid::new_v4(),
        input.profile_id,
        provider,
        connector_id,
        agent_id,
        &input.config,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(json!({ "binding": binding }))))
}

pub async fn get_binding(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    admin(&state, &headers)?;
    Ok(Json(
        json!({ "binding": db::get_binding(&state.pool, id).await? }),
    ))
}

pub async fn update_binding(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(input): Json<UpdateBinding>,
) -> Result<Json<Value>, ApiError> {
    admin(&state, &headers)?;
    let connector_id = input
        .connector_id
        .as_deref()
        .map(|value| required_identifier("connector id", value))
        .transpose()?;
    let agent_id = input
        .agent_id
        .as_deref()
        .map(|value| required_identifier("agent id", value))
        .transpose()?;
    if let Some(config) = &input.config {
        validate_json_object("config", config, 64 * 1024)?;
    }
    let binding = db::update_binding(
        &state.pool,
        id,
        connector_id,
        agent_id,
        input.config.as_ref(),
    )
    .await?;
    Ok(Json(json!({ "binding": binding })))
}

pub async fn delete_binding(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    admin(&state, &headers)?;
    db::delete_binding(&state.pool, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_endpoints(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    admin(&state, &headers)?;
    Ok(Json(
        json!({ "endpoints": db::list_endpoints(&state.pool).await? }),
    ))
}

pub async fn create_endpoint(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<CreateEndpoint>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    admin(&state, &headers)?;
    let name = required_text("name", &input.name, 128)?;
    let slug = profile_slug(input.slug.as_deref(), name)?;
    let request_timeout_ms = validate_timeout(input.request_timeout_ms.unwrap_or(30_000))?;
    let endpoint = db::create_endpoint(
        &state.pool,
        NewEndpoint {
            id: Uuid::new_v4(),
            slug: &slug,
            name,
            profile_id: input.profile_id,
            binding_id: input.binding_id,
            enabled: input.enabled.unwrap_or(true),
            request_timeout_ms,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(json!({ "endpoint": endpoint }))))
}

pub async fn get_endpoint(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    admin(&state, &headers)?;
    Ok(Json(
        json!({ "endpoint": db::get_endpoint(&state.pool, id).await? }),
    ))
}

pub async fn update_endpoint(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(input): Json<UpdateEndpoint>,
) -> Result<Json<Value>, ApiError> {
    admin(&state, &headers)?;
    let slug = input
        .slug
        .as_deref()
        .map(validate_explicit_slug)
        .transpose()?;
    let name = input
        .name
        .as_deref()
        .map(|value| required_text("name", value, 128))
        .transpose()?;
    let request_timeout_ms = input.request_timeout_ms.map(validate_timeout).transpose()?;
    let endpoint = db::update_endpoint(
        &state.pool,
        id,
        EndpointPatch {
            slug: slug.as_deref(),
            name,
            profile_id: input.profile_id,
            binding_id: input.binding_id,
            enabled: input.enabled,
            request_timeout_ms,
        },
    )
    .await?;
    Ok(Json(json!({ "endpoint": endpoint })))
}

pub async fn delete_endpoint(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    admin(&state, &headers)?;
    db::delete_endpoint(&state.pool, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_api_keys(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    admin(&state, &headers)?;
    Ok(Json(
        json!({ "apiKeys": db::list_api_keys(&state.pool).await? }),
    ))
}

pub async fn create_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<CreateApiKey>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    admin(&state, &headers)?;
    db::get_endpoint(&state.pool, input.endpoint_id).await?;
    let name = input
        .name
        .as_deref()
        .map(|value| required_text("name", value, 128))
        .transpose()?
        .unwrap_or("Default");
    let raw_key = generate_api_key();
    let key_prefix = raw_key.chars().take(18).collect::<String>();
    let key_hash = hash_api_key(&raw_key, &state.config.api_key_pepper);
    let record = db::create_api_key(
        &state.pool,
        Uuid::new_v4(),
        input.endpoint_id,
        name,
        &key_prefix,
        &key_hash,
    )
    .await?;
    let created = CreatedApiKey {
        record,
        key: raw_key,
    };
    Ok((StatusCode::CREATED, Json(json!({ "apiKey": created }))))
}

pub async fn revoke_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    admin(&state, &headers)?;
    Ok(Json(
        json!({ "apiKey": db::revoke_api_key(&state.pool, id).await? }),
    ))
}

pub async fn list_connections(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    admin(&state, &headers)?;
    Ok(Json(json!({
        "connections": db::list_connector_sessions(&state.pool).await?
    })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceQuery {
    endpoint_id: Option<Uuid>,
    limit: Option<i64>,
}

pub async fn list_traces(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<TraceQuery>,
) -> Result<Json<Value>, ApiError> {
    admin(&state, &headers)?;
    let limit = query.limit.unwrap_or(100).clamp(1, 500);
    Ok(Json(json!({
        "traces": db::list_traces(&state.pool, query.endpoint_id, limit).await?
    })))
}

pub async fn invoke_endpoint(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id_or_slug): Path<String>,
    Json(input): Json<InvokeEndpoint>,
) -> Result<Json<Value>, ApiError> {
    let route = db::resolve_endpoint_route(&state.pool, &id_or_slug).await?;
    authorize_endpoint(&state, &headers, route.endpoint_id).await?;
    if input.message.as_deref().is_none_or(str::is_empty) && input.input.is_none() {
        return Err(ApiError::Invalid(
            "message or input must be provided".to_string(),
        ));
    }
    let request = serde_json::to_value(&input).map_err(|_| ApiError::Internal)?;
    if serde_json::to_vec(&request)
        .map_err(|_| ApiError::Internal)?
        .len()
        > 512 * 1024
    {
        return Err(ApiError::Invalid("request body is too large".to_string()));
    }

    let request_id = Uuid::new_v4();
    let connector_session_id = state.relay.session_for(&route.connector_id).await;
    db::create_trace(
        &state.pool,
        request_id,
        route.endpoint_id,
        connector_session_id,
        &request,
    )
    .await?;

    let timeout = Duration::from_millis(
        u64::try_from(route.request_timeout_ms)
            .unwrap_or(30_000)
            .min(state.config.request_timeout.as_millis() as u64),
    );
    let started_at = Instant::now();
    match state
        .relay
        .invoke(&route, request_id, request, timeout)
        .await
    {
        Ok(output) => {
            persist_trace(
                &state,
                request_id,
                "completed",
                started_at,
                Some(&output),
                None,
            )
            .await;
            Ok(Json(json!({
                "requestId": request_id,
                "endpointId": route.endpoint_id,
                "output": output
            })))
        }
        Err(error) => {
            let message = relay_error_message(&error);
            persist_trace(
                &state,
                request_id,
                relay_error_status(&error),
                started_at,
                None,
                Some(&message),
            )
            .await;
            Err(map_relay_error(error))
        }
    }
}

async fn persist_trace(
    state: &AppState,
    request_id: Uuid,
    status: &str,
    started_at: Instant,
    response: Option<&Value>,
    error: Option<&str>,
) {
    if let Err(persist_error) = db::complete_trace(
        &state.pool,
        request_id,
        status,
        db::elapsed_millis(started_at),
        response,
        error,
    )
    .await
    {
        warn!(error = %persist_error, %request_id, "could not complete endpoint trace");
    }
}

fn admin(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    require_admin(headers, &state.config.admin_key)
}

async fn authorize_endpoint(
    state: &AppState,
    headers: &HeaderMap,
    endpoint_id: Uuid,
) -> Result<(), ApiError> {
    let token = bearer_token(headers).ok_or(ApiError::Unauthorized)?;
    if is_secret_match(token, &state.config.admin_key) {
        return Ok(());
    }
    let key_hash = hash_api_key(token, &state.config.api_key_pepper);
    if db::api_key_matches_endpoint(&state.pool, endpoint_id, &key_hash).await? {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

fn profile_slug(explicit: Option<&str>, name: &str) -> Result<String, ApiError> {
    match explicit {
        Some(value) => validate_explicit_slug(value),
        None => {
            let value = slugify(name);
            if validate_slug(&value) {
                Ok(value)
            } else {
                Err(ApiError::Invalid(
                    "name cannot produce a valid slug".to_string(),
                ))
            }
        }
    }
}

fn validate_explicit_slug(value: &str) -> Result<String, ApiError> {
    let value = value.trim().to_ascii_lowercase();
    if validate_slug(&value) {
        Ok(value)
    } else {
        Err(ApiError::Invalid(
            "slug must contain 3-64 lowercase letters, numbers, or single hyphens".to_string(),
        ))
    }
}

fn required_text<'a>(name: &str, value: &'a str, max: usize) -> Result<&'a str, ApiError> {
    let value = value.trim();
    if value.is_empty() || value.len() > max || value.contains('\0') {
        Err(ApiError::Invalid(format!("invalid {name}")))
    } else {
        Ok(value)
    }
}

fn optional_text<'a>(
    name: &str,
    value: Option<&'a str>,
    max: usize,
) -> Result<Option<&'a str>, ApiError> {
    match value.map(str::trim) {
        None | Some("") => Ok(None),
        Some(value) => required_text(name, value, max).map(Some),
    }
}

fn patch_text<'a>(
    name: &str,
    value: Option<&'a str>,
    max: usize,
) -> Result<(bool, Option<&'a str>), ApiError> {
    match value {
        None => Ok((false, None)),
        Some(value) => optional_text(name, Some(value), max).map(|value| (true, value)),
    }
}

fn required_identifier<'a>(name: &str, value: &'a str) -> Result<&'a str, ApiError> {
    let value = value.trim();
    validate_identifier(name, value).map_err(ApiError::Invalid)?;
    Ok(value)
}

fn validate_timeout(value: i32) -> Result<i32, ApiError> {
    if (500..=120_000).contains(&value) {
        Ok(value)
    } else {
        Err(ApiError::Invalid(
            "requestTimeoutMs must be between 500 and 120000".to_string(),
        ))
    }
}

fn validate_json_object(name: &str, value: &Value, max: usize) -> Result<(), ApiError> {
    if !value.is_object() {
        return Err(ApiError::Invalid(format!("{name} must be an object")));
    }
    if serde_json::to_vec(value)
        .map_err(|_| ApiError::Internal)?
        .len()
        > max
    {
        return Err(ApiError::Invalid(format!("{name} is too large")));
    }
    Ok(())
}

fn generate_api_key() -> String {
    format!(
        "vifu_ep_{}{}",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    )
}

fn relay_error_status(error: &RelayCallError) -> &'static str {
    match error {
        RelayCallError::ConnectorUnavailable => "unavailable",
        RelayCallError::Backpressure => "rejected",
        RelayCallError::Timeout => "timed_out",
        RelayCallError::Connector(_) => "failed",
    }
}

fn relay_error_message(error: &RelayCallError) -> String {
    match error {
        RelayCallError::ConnectorUnavailable => "connector is not available".to_string(),
        RelayCallError::Backpressure => "connector is busy".to_string(),
        RelayCallError::Timeout => "agent request timed out".to_string(),
        RelayCallError::Connector(message) => message.clone(),
    }
}

fn map_relay_error(error: RelayCallError) -> ApiError {
    match error {
        RelayCallError::ConnectorUnavailable => ApiError::ConnectorUnavailable,
        RelayCallError::Backpressure => ApiError::Backpressure,
        RelayCallError::Timeout => ApiError::Timeout,
        RelayCallError::Connector(message) => ApiError::Connector(message),
    }
}

pub async fn fallback() -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        Json(json!({ "error": { "code": "NOT_FOUND", "message": "resource not found" } })),
    )
}

#[cfg(test)]
mod tests {
    use super::{patch_text, profile_slug, validate_timeout};

    #[test]
    fn derives_profile_slugs() {
        assert_eq!(profile_slug(None, "Town Guide").unwrap(), "town-guide");
    }

    #[test]
    fn validates_endpoint_timeouts() {
        assert!(validate_timeout(30_000).is_ok());
        assert!(validate_timeout(100).is_err());
    }

    #[test]
    fn distinguishes_cleared_profile_text_from_an_omitted_patch() {
        assert_eq!(patch_text("description", None, 64).unwrap(), (false, None));
        assert_eq!(
            patch_text("description", Some("  "), 64).unwrap(),
            (true, None)
        );
        assert_eq!(
            patch_text("description", Some("  Guide  "), 64).unwrap(),
            (true, Some("Guide"))
        );
    }
}
