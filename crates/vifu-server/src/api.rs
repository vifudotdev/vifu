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

use crate::auth::{bearer_token, hash_api_key, is_secret_match};
use crate::config::DeploymentMode;
use crate::db::{self, EndpointPatch, NewEndpoint, NewProject, ProfilePatch, ProjectPatch};
use crate::error::ApiError;
use crate::models::{
    slugify, validate_slug, Capabilities, CreateApiKey, CreateBinding, CreateEndpoint,
    CreateProfile, CreateProject, CreatedApiKey, CreatedProject, InvokeEndpoint, UpdateBinding,
    UpdateEndpoint, UpdateProfile, UpdateProject,
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
    agent_gateways: usize,
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
        agent_gateways: state.relay.connection_count().await,
    }))
}

pub async fn list_projects(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    admin(&state, &headers).await?;
    Ok(Json(
        json!({ "projects": db::list_projects(&state.pool).await? }),
    ))
}

pub async fn create_project(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<CreateProject>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    admin(&state, &headers).await?;
    let name = required_text("name", &input.name, 128)?;
    let slug = profile_slug(input.slug.as_deref(), name)?;
    let description = optional_text("description", input.description.as_deref(), 4096)?;
    let gateway_id = required_identifier("agent gateway id", &input.gateway_id)?;
    let mut binding_ids = input.binding_ids;
    let agent_ids = validate_agent_ids(&input.agent_ids)?;
    if !agent_ids.is_empty() {
        let available_agents = db::list_available_agents(&state.pool).await?;
        for agent_id in agent_ids {
            let agent = available_agents
                .iter()
                .find(|agent| {
                    agent.gateway_id == gateway_id
                        && agent.id == agent_id
                        && agent.status == "connected"
                })
                .ok_or_else(|| {
                    ApiError::Invalid(format!(
                        "agent {agent_id} is not available on agent gateway {gateway_id}"
                    ))
                })?;
            let binding_id =
                db::ensure_discovered_binding(&state.pool, gateway_id, &agent.id, &agent.name)
                    .await?;
            binding_ids.push(binding_id);
        }
    }
    binding_ids.sort_unstable();
    binding_ids.dedup();
    let publishable_key = generate_publishable_project_key();
    let publishable_key_prefix = publishable_key.chars().take(18).collect::<String>();
    let publishable_key_hash = hash_api_key(&publishable_key, &state.config.api_key_pepper);
    let project = db::create_project(
        &state.pool,
        NewProject {
            id: Uuid::new_v4(),
            slug: &slug,
            name,
            description,
            gateway_id,
            publishable_key_prefix: &publishable_key_prefix,
            publishable_key_hash: &publishable_key_hash,
            binding_ids: &binding_ids,
        },
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "project": CreatedProject { project, publishable_key } })),
    ))
}

pub async fn get_project(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    admin(&state, &headers).await?;
    Ok(Json(
        json!({ "project": db::get_project(&state.pool, id).await? }),
    ))
}

pub async fn update_project(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(input): Json<UpdateProject>,
) -> Result<Json<Value>, ApiError> {
    admin(&state, &headers).await?;
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
    let gateway_id = input
        .gateway_id
        .as_deref()
        .map(|value| required_identifier("agent gateway id", value))
        .transpose()?;
    let project = db::update_project(
        &state.pool,
        id,
        ProjectPatch {
            slug: slug.as_deref(),
            name,
            description_changed,
            description,
            gateway_id,
            enabled: input.enabled,
            binding_ids: input.binding_ids.as_deref(),
        },
    )
    .await?;
    Ok(Json(json!({ "project": project })))
}

pub async fn delete_project(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    admin(&state, &headers).await?;
    db::delete_project(&state.pool, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_profiles(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    admin(&state, &headers).await?;
    Ok(Json(
        json!({ "profiles": db::list_profiles(&state.pool).await? }),
    ))
}

pub async fn create_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<CreateProfile>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    admin(&state, &headers).await?;
    let name = required_text("name", &input.name, 128)?;
    let slug = profile_slug(input.slug.as_deref(), name)?;
    let description = optional_text("description", input.description.as_deref(), 4096)?;
    let profile = db::create_profile(&state.pool, Uuid::new_v4(), &slug, name, description).await?;
    Ok((StatusCode::CREATED, Json(json!({ "profile": profile }))))
}

pub async fn get_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    admin(&state, &headers).await?;
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
    admin(&state, &headers).await?;
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
    let profile = db::update_profile(
        &state.pool,
        id,
        ProfilePatch {
            slug: slug.as_deref(),
            name,
            description_changed,
            description,
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
    admin(&state, &headers).await?;
    db::delete_profile(&state.pool, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_bindings(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    admin(&state, &headers).await?;
    Ok(Json(
        json!({ "bindings": db::list_bindings(&state.pool).await? }),
    ))
}

pub async fn create_binding(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<CreateBinding>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    admin(&state, &headers).await?;
    let provider = required_identifier("provider", &input.provider)?;
    if provider != "openclaw" {
        return Err(ApiError::Invalid(
            "openclaw is the only agent runtime provider in this release".to_string(),
        ));
    }
    let gateway_id = required_identifier("agent gateway id", &input.gateway_id)?;
    let agent_id = required_identifier("agent id", &input.agent_id)?;
    validate_json_object("config", &input.config, 64 * 1024)?;
    let binding = db::create_binding(
        &state.pool,
        Uuid::new_v4(),
        input.profile_id,
        provider,
        gateway_id,
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
    admin(&state, &headers).await?;
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
    admin(&state, &headers).await?;
    let gateway_id = input
        .gateway_id
        .as_deref()
        .map(|value| required_identifier("agent gateway id", value))
        .transpose()?;
    let agent_id = input
        .agent_id
        .as_deref()
        .map(|value| required_identifier("agent id", value))
        .transpose()?;
    if let Some(config) = &input.config {
        validate_json_object("config", config, 64 * 1024)?;
    }
    let binding =
        db::update_binding(&state.pool, id, gateway_id, agent_id, input.config.as_ref()).await?;
    Ok(Json(json!({ "binding": binding })))
}

pub async fn delete_binding(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    admin(&state, &headers).await?;
    db::delete_binding(&state.pool, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_endpoints(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    admin(&state, &headers).await?;
    Ok(Json(
        json!({ "endpoints": db::list_endpoints(&state.pool).await? }),
    ))
}

pub async fn create_endpoint(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<CreateEndpoint>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    admin(&state, &headers).await?;
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
    admin(&state, &headers).await?;
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
    admin(&state, &headers).await?;
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
    admin(&state, &headers).await?;
    db::delete_endpoint(&state.pool, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_api_keys(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    admin(&state, &headers).await?;
    Ok(Json(
        json!({ "apiKeys": db::list_api_keys(&state.pool).await? }),
    ))
}

pub async fn create_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<CreateApiKey>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    admin(&state, &headers).await?;
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
    admin(&state, &headers).await?;
    Ok(Json(
        json!({ "apiKey": db::revoke_api_key(&state.pool, id).await? }),
    ))
}

pub async fn list_agent_gateways(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    admin(&state, &headers).await?;
    Ok(Json(json!({
        "agentGateways": db::list_agent_gateway_sessions(&state.pool).await?
    })))
}

pub async fn list_available_agents(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    admin(&state, &headers).await?;
    Ok(Json(json!({
        "agents": db::list_available_agents(&state.pool).await?
    })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceQuery {
    endpoint_id: Option<Uuid>,
    project_id: Option<Uuid>,
    limit: Option<i64>,
}

pub async fn list_traces(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<TraceQuery>,
) -> Result<Json<Value>, ApiError> {
    admin(&state, &headers).await?;
    if query.endpoint_id.is_some() && query.project_id.is_some() {
        return Err(ApiError::Invalid(
            "endpointId and projectId cannot be combined".to_string(),
        ));
    }
    let limit = query.limit.unwrap_or(100).clamp(1, 500);
    Ok(Json(json!({
        "traces": db::list_traces(&state.pool, query.endpoint_id, query.project_id, limit).await?
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
    let gateway_session_id = state.relay.session_for(&route.gateway_id).await;
    db::create_trace(
        &state.pool,
        request_id,
        route.endpoint_id,
        gateway_session_id,
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

async fn admin(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    if bearer_token(headers).is_some_and(|token| is_secret_match(token, &state.config.admin_key)) {
        return Ok(());
    }
    Err(ApiError::Forbidden)
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

fn validate_agent_ids(values: &[String]) -> Result<Vec<String>, ApiError> {
    if values.len() > 256 {
        return Err(ApiError::Invalid(
            "a project supports at most 256 agents".to_string(),
        ));
    }
    let mut agent_ids = Vec::with_capacity(values.len());
    for value in values {
        agent_ids.push(required_identifier("agent id", value)?.to_string());
    }
    agent_ids.sort();
    agent_ids.dedup();
    Ok(agent_ids)
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

fn generate_publishable_project_key() -> String {
    format!(
        "vifu_pk_{}{}",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    )
}

fn relay_error_status(error: &RelayCallError) -> &'static str {
    match error {
        RelayCallError::AgentGatewayUnavailable => "unavailable",
        RelayCallError::Backpressure => "rejected",
        RelayCallError::Timeout => "timed_out",
        RelayCallError::AgentGateway(_) => "failed",
    }
}

fn relay_error_message(error: &RelayCallError) -> String {
    match error {
        RelayCallError::AgentGatewayUnavailable => "agent gateway is not available".to_string(),
        RelayCallError::Backpressure => "agent gateway is busy".to_string(),
        RelayCallError::Timeout => "agent request timed out".to_string(),
        RelayCallError::AgentGateway(message) => message.clone(),
    }
}

fn map_relay_error(error: RelayCallError) -> ApiError {
    match error {
        RelayCallError::AgentGatewayUnavailable => ApiError::AgentGatewayUnavailable,
        RelayCallError::Backpressure => ApiError::Backpressure,
        RelayCallError::Timeout => ApiError::Timeout,
        RelayCallError::AgentGateway(message) => ApiError::AgentGateway(message),
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
