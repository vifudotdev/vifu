use std::path::Path as FsPath;
use std::time::{Duration, Instant};

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tracing::warn;
use uuid::Uuid;
use vifu_core::config::{AgentProviderAuthDefinition, AgentProviderDefinition, AgentProvidersFile};
use vifu_core::protocol::validate_identifier;

use crate::auth::{
    bearer_token, decrypt_secret_json, encrypt_secret_json, hash_agent_gateway_credential,
    hash_api_key, is_secret_match,
};
use crate::config::DeploymentMode;
use crate::db::{
    self, CanvasNodePatch, EndpointPatch, NewCanvasEdge, NewCanvasNode, NewEndpoint, NewProject,
    ProfilePatch, ProjectPatch,
};
use crate::error::ApiError;
use crate::models::{
    slugify, validate_slug, AgentEndpoint, ApiKeyAgentScope, ApiKeyPermissions, ApiKeyRecord,
    Capabilities, CreateApiKey, CreateBinding, CreateCanvasEdge, CreateCanvasNode, CreateEndpoint,
    CreateProfile, CreateProject, CreatedApiKey, EndpointRoute, ImportProviderConnections,
    ProviderAdapter, ProviderAdapterField, ProviderConnection, ProviderConnectionSecret,
    RegisterAgentGateway, UpdateApiKey, UpdateBinding, UpdateCanvasNode, UpdateEndpoint,
    UpdateProfile, UpdateProject, UpsertProviderConnection,
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
    let slug = project_slug(input.slug.as_deref(), name)?;
    let description = optional_text("description", input.description.as_deref(), 4096)?;
    let mut binding_ids = input.binding_ids;
    let agent_ids = validate_agent_ids(&input.agent_ids)?;
    let gateway_id = match input
        .gateway_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(value) => required_identifier("agent gateway id", value)?.to_string(),
        None if agent_ids.is_empty() => format!("project-{slug}"),
        None => {
            return Err(ApiError::Invalid(
                "agent gateway id is required when creating a project from detected agents"
                    .to_string(),
            ))
        }
    };
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
                db::ensure_discovered_binding(&state.pool, &gateway_id, &agent.id, &agent.name)
                    .await?;
            binding_ids.push(binding_id);
        }
    }
    binding_ids.sort_unstable();
    binding_ids.dedup();
    let project = db::create_project(
        &state.pool,
        NewProject {
            id: Uuid::new_v4(),
            slug: &slug,
            name,
            description,
            gateway_id: &gateway_id,
            binding_ids: &binding_ids,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(json!({ "project": project }))))
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

pub async fn get_project_canvas(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Result<Json<Value>, ApiError> {
    admin(&state, &headers).await?;
    Ok(Json(json!({
        "canvas": db::get_project_canvas(&state.pool, &slug).await?
    })))
}

pub async fn create_canvas_node(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(input): Json<CreateCanvasNode>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    admin(&state, &headers).await?;
    let kind = required_identifier("node kind", &input.kind)?;
    validate_json_object("position", &input.position, 16 * 1024)?;
    validate_json_object("config", &input.config, 64 * 1024)?;
    validate_json_object("inputs", &input.inputs, 64 * 1024)?;
    validate_json_object("outputs", &input.outputs, 64 * 1024)?;
    let gateway_id = input
        .gateway_id
        .as_deref()
        .map(|value| required_identifier("agent gateway id", value))
        .transpose()?;
    let resource_id = input
        .resource_id
        .as_deref()
        .map(|value| required_identifier("resource id", value))
        .transpose()?;
    let node = db::create_canvas_node(
        &state.pool,
        &slug,
        NewCanvasNode {
            kind,
            position: &input.position,
            profile_id: input.profile_id,
            binding_id: input.binding_id,
            gateway_id,
            resource_id,
            config: &input.config,
            inputs: &input.inputs,
            outputs: &input.outputs,
            exposed: input.exposed.unwrap_or(true),
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(json!({ "node": node }))))
}

pub async fn update_canvas_node(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((slug, id)): Path<(String, Uuid)>,
    Json(input): Json<UpdateCanvasNode>,
) -> Result<Json<Value>, ApiError> {
    admin(&state, &headers).await?;
    if let Some(position) = &input.position {
        validate_json_object("position", position, 16 * 1024)?;
    }
    if let Some(config) = &input.config {
        validate_json_object("config", config, 64 * 1024)?;
    }
    if let Some(inputs) = &input.inputs {
        validate_json_object("inputs", inputs, 64 * 1024)?;
    }
    if let Some(outputs) = &input.outputs {
        validate_json_object("outputs", outputs, 64 * 1024)?;
    }
    let node = db::update_canvas_node(
        &state.pool,
        &slug,
        id,
        CanvasNodePatch {
            position: input.position.as_ref(),
            config: input.config.as_ref(),
            inputs: input.inputs.as_ref(),
            outputs: input.outputs.as_ref(),
            exposed: input.exposed,
        },
    )
    .await?;
    Ok(Json(json!({ "node": node })))
}

pub async fn delete_canvas_node(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((slug, id)): Path<(String, Uuid)>,
) -> Result<StatusCode, ApiError> {
    admin(&state, &headers).await?;
    db::delete_canvas_node(&state.pool, &slug, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn create_canvas_edge(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(input): Json<CreateCanvasEdge>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    admin(&state, &headers).await?;
    let kind = required_identifier("edge kind", &input.kind)?;
    validate_json_object("config", &input.config, 64 * 1024)?;
    let source_handle = input
        .source_handle
        .as_deref()
        .map(|value| required_identifier("source handle", value))
        .transpose()?;
    let target_handle = input
        .target_handle
        .as_deref()
        .map(|value| required_identifier("target handle", value))
        .transpose()?;
    let edge = db::create_canvas_edge(
        &state.pool,
        &slug,
        NewCanvasEdge {
            source_node_id: input.source_node_id,
            source_handle,
            target_node_id: input.target_node_id,
            target_handle,
            kind,
            config: &input.config,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(json!({ "edge": edge }))))
}

pub async fn delete_canvas_edge(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((slug, id)): Path<(String, Uuid)>,
) -> Result<StatusCode, ApiError> {
    admin(&state, &headers).await?;
    db::delete_canvas_edge(&state.pool, &slug, id).await?;
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
    db::get_project(&state.pool, input.project_id).await?;
    let agent_scope = normalize_api_key_agent_scope(input.agent_scope)?;
    let name = input
        .name
        .as_deref()
        .map(|value| required_text("name", value, 128))
        .transpose()?
        .unwrap_or("Project key");
    let created = issue_api_key(
        &state,
        input.project_id,
        name,
        &agent_scope,
        &input.permissions,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(json!({ "apiKey": created }))))
}

async fn issue_api_key(
    state: &AppState,
    project_id: Uuid,
    name: &str,
    agent_scope: &ApiKeyAgentScope,
    permissions: &ApiKeyPermissions,
) -> Result<CreatedApiKey, ApiError> {
    let raw_key = generate_api_key();
    let key_prefix = raw_key.chars().take(18).collect::<String>();
    let key_hash = hash_api_key(&raw_key, &state.config.api_key_pepper);
    let record = db::create_api_key(
        &state.pool,
        db::NewApiKey {
            id: Uuid::new_v4(),
            project_id,
            name,
            agent_scope,
            permissions,
            key_prefix: &key_prefix,
            key_hash: &key_hash,
        },
    )
    .await?;
    Ok(CreatedApiKey {
        record,
        key: raw_key,
    })
}

pub async fn update_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(input): Json<UpdateApiKey>,
) -> Result<Json<Value>, ApiError> {
    admin(&state, &headers).await?;
    let current = db::get_api_key(&state.pool, id).await?;
    if current.revoked_at.is_some() {
        return Err(ApiError::Conflict(
            "revoked API keys cannot be edited".to_string(),
        ));
    }
    if input.project_id.is_none()
        && input.name.is_none()
        && input.agent_scope.is_none()
        && input.permissions.is_none()
    {
        return Err(ApiError::Invalid(
            "at least one API key field is required".to_string(),
        ));
    }
    if let Some(project_id) = input.project_id {
        db::get_project(&state.pool, project_id).await?;
    }
    let project_changed = input
        .project_id
        .is_some_and(|project_id| project_id != current.project_id);
    if project_changed && input.agent_scope.is_none() {
        return Err(ApiError::Invalid(
            "agentScope is required when moving an API key to another project".to_string(),
        ));
    }
    let agent_scope = input
        .agent_scope
        .map(normalize_api_key_agent_scope)
        .transpose()?;
    let name = input
        .name
        .as_deref()
        .map(|value| required_text("name", value, 128))
        .transpose()?;
    let api_key = db::update_api_key(
        &state.pool,
        id,
        db::ApiKeyPatch {
            project_id: input.project_id,
            name,
            agent_scope: agent_scope.as_ref(),
            permissions: input.permissions.as_ref(),
        },
    )
    .await?;
    Ok(Json(json!({ "apiKey": api_key })))
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

pub async fn delete_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    admin(&state, &headers).await?;
    db::delete_api_key(&state.pool, id).await?;
    Ok(StatusCode::NO_CONTENT)
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

pub async fn register_agent_gateway(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<RegisterAgentGateway>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    require_agent_gateway_bootstrap(&state, &headers)?;
    let gateway_id = required_identifier("agent gateway id", &input.gateway_id)?;
    let credential = validate_agent_gateway_credential(&input.credential)?;
    let credential_prefix = credential.chars().take(20).collect::<String>();
    let credential_hash = hash_agent_gateway_credential(credential, &state.config.api_key_pepper);
    let registration = db::register_agent_gateway_credential(
        &state.pool,
        gateway_id,
        &credential_prefix,
        &credential_hash,
    )
    .await?;
    let status = match registration {
        db::AgentGatewayRegistration::Registered => "registered",
        db::AgentGatewayRegistration::Existing => "existing",
    };
    let status_code = if registration == db::AgentGatewayRegistration::Registered {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((
        status_code,
        Json(json!({ "gatewayId": gateway_id, "status": status })),
    ))
}

pub async fn revoke_agent_gateway(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(gateway_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    admin(&state, &headers).await?;
    let gateway_id = required_identifier("agent gateway id", &gateway_id)?;
    let credential = db::revoke_agent_gateway_credential(&state.pool, gateway_id).await?;
    state
        .relay
        .disconnect(gateway_id, "CREDENTIAL_REVOKED")
        .await;
    Ok(Json(json!({ "agentGatewayCredential": credential })))
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

pub async fn list_provider_adapters(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    admin(&state, &headers).await?;
    Ok(Json(json!({ "providerAdapters": provider_adapters() })))
}

pub async fn list_provider_connections(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Result<Json<Value>, ApiError> {
    admin(&state, &headers).await?;
    if let Some(path) = active_provider_registry_file(&state) {
        let project = db::get_project_by_slug(&state.pool, &slug).await?;
        let file = read_provider_registry(&path)?;
        let provider_connections = file
            .providers
            .into_iter()
            .map(|provider| file_provider_connection(project.project.id, provider))
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(Json(json!({ "providerConnections": provider_connections })));
    }
    Ok(Json(json!({
        "providerConnections": db::list_provider_connections(&state.pool, &slug).await?
    })))
}

pub async fn upsert_provider_connection(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((slug, provider_key)): Path<(String, String)>,
    Json(input): Json<UpsertProviderConnection>,
) -> Result<Json<Value>, ApiError> {
    admin(&state, &headers).await?;
    if let Some(path) = active_provider_registry_file(&state) {
        let project = db::get_project_by_slug(&state.pool, &slug).await?;
        let connection =
            save_file_provider_connection(&path, project.project.id, &provider_key, input)?;
        return Ok(Json(json!({ "providerConnection": connection })));
    }
    let secrets = provider_secrets_or_existing(&state, &slug, &provider_key, input.secrets).await?;
    let prepared = prepare_provider_connection(
        &state,
        PreparedProviderSource {
            key: &provider_key,
            name: input.name.as_deref(),
            provider_type: &input.provider_type,
            base_url: &input.base_url,
            config: input.config,
            secrets,
        },
    )?;
    let connection = save_provider_connection(&state, &slug, prepared).await?;
    Ok(Json(json!({ "providerConnection": connection })))
}

pub async fn import_provider_connections(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(input): Json<ImportProviderConnections>,
) -> Result<Json<Value>, ApiError> {
    admin(&state, &headers).await?;
    if input.providers.len() > 100 {
        return Err(ApiError::Invalid(
            "provider import supports at most 100 providers".to_string(),
        ));
    }
    if let Some(path) = active_provider_registry_file(&state) {
        let project = db::get_project_by_slug(&state.pool, &slug).await?;
        let mut connections = Vec::with_capacity(input.providers.len());
        for provider in input.providers {
            connections.push(save_file_provider_connection(
                &path,
                project.project.id,
                &provider.key,
                UpsertProviderConnection {
                    name: provider.name,
                    provider_type: provider.provider_type,
                    base_url: provider.base_url,
                    config: provider.config,
                    secrets: provider.secrets,
                },
            )?);
        }
        return Ok(Json(json!({ "providerConnections": connections })));
    }
    let mut connections = Vec::with_capacity(input.providers.len());
    for provider in input.providers {
        let secrets =
            provider_secrets_or_existing(&state, &slug, &provider.key, provider.secrets).await?;
        let prepared = prepare_provider_connection(
            &state,
            PreparedProviderSource {
                key: &provider.key,
                name: provider.name.as_deref(),
                provider_type: &provider.provider_type,
                base_url: &provider.base_url,
                config: provider.config,
                secrets,
            },
        )?;
        connections.push(save_provider_connection(&state, &slug, prepared).await?);
    }
    Ok(Json(json!({ "providerConnections": connections })))
}

pub async fn delete_project_provider_connection(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((slug, provider_key)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    admin(&state, &headers).await?;
    if let Some(path) = active_provider_registry_file(&state) {
        delete_file_provider_connection(&path, &provider_key)?;
    } else {
        db::delete_provider_connection_by_key(&state.pool, &slug, &provider_key).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_provider_connection(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    admin(&state, &headers).await?;
    db::delete_provider_connection(&state.pool, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn test_project_provider_connection(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((slug, provider_key)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    admin(&state, &headers).await?;
    if let Some(path) = active_provider_registry_file(&state) {
        let project = db::get_project_by_slug(&state.pool, &slug).await?;
        let mut connection =
            get_file_provider_connection(&path, project.project.id, &provider_key)?;
        if connection.provider_type != "openclaw" {
            return Err(ApiError::Invalid("unsupported provider type".to_string()));
        }
        let (status, message) = probe_provider_status(&connection.base_url).await?;
        connection.status = status.to_string();
        connection.last_checked_at = Some(Utc::now());
        return Ok(Json(
            json!({ "providerConnection": connection, "message": message }),
        ));
    }
    let connection =
        db::get_provider_connection_secret_by_key(&state.pool, &slug, &provider_key).await?;
    test_db_provider_connection(&state, connection).await
}

pub async fn test_provider_connection(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    admin(&state, &headers).await?;
    let connection = db::get_provider_connection_secret(&state.pool, id).await?;
    test_db_provider_connection(&state, connection).await
}

async fn test_db_provider_connection(
    state: &AppState,
    connection: ProviderConnectionSecret,
) -> Result<Json<Value>, ApiError> {
    if connection.provider_type != "openclaw" {
        return Err(ApiError::Invalid("unsupported provider type".to_string()));
    }
    let (status, message) = probe_provider_status(&connection.base_url).await?;
    let updated = db::update_provider_connection_status(&state.pool, connection.id, status).await?;
    Ok(Json(
        json!({ "providerConnection": updated, "message": message }),
    ))
}

pub async fn discover_project_provider_agents(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((slug, provider_key)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    admin(&state, &headers).await?;
    if let Some(path) = active_provider_registry_file(&state) {
        let project = db::get_project_by_slug(&state.pool, &slug).await?;
        let (definition, mut connection) =
            get_file_provider_definition_and_connection(&path, project.project.id, &provider_key)?;
        let agents = discover_file_provider_agents(&definition).await?;
        connection.status = "online".to_string();
        connection.last_checked_at = Some(Utc::now());
        return Ok(Json(
            json!({ "providerConnection": connection, "agents": agents }),
        ));
    }
    let connection =
        db::get_provider_connection_secret_by_key(&state.pool, &slug, &provider_key).await?;
    discover_db_provider_agents(&state, connection).await
}

pub async fn discover_provider_agents(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    admin(&state, &headers).await?;
    let connection = db::get_provider_connection_secret(&state.pool, id).await?;
    discover_db_provider_agents(&state, connection).await
}

async fn discover_db_provider_agents(
    state: &AppState,
    connection: ProviderConnectionSecret,
) -> Result<Json<Value>, ApiError> {
    if connection.provider_type != "openclaw" {
        return Err(ApiError::Invalid("unsupported provider type".to_string()));
    }
    let endpoint =
        vifu_core::openclaw::parse_endpoint(&connection.base_url).map_err(ApiError::Invalid)?;
    let secrets = decrypted_provider_secrets(state, &connection)?;
    let token = provider_token(&secrets)?;
    let agents = vifu_core::openclaw::discover_agents(&endpoint, token.as_deref())
        .await
        .map_err(ApiError::AgentGateway)?;
    let updated =
        db::update_provider_connection_status(&state.pool, connection.id, "online").await?;
    Ok(Json(
        json!({ "providerConnection": updated, "agents": agents }),
    ))
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

pub async fn list_openai_models(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let authority = api_request_authority(&state, &headers).await?;
    let endpoints = match authority {
        ApiRequestAuthority::Admin => db::list_enabled_endpoints(&state.pool).await?,
        ApiRequestAuthority::Key(_) => return Err(ApiError::Forbidden),
    };
    Ok(openai_models_response(endpoints))
}

pub async fn list_project_openai_models(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_slug): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let authority = api_request_authority(&state, &headers).await?;
    let project = db::get_project_by_slug(&state.pool, &project_slug).await?;
    let mut endpoints = match &authority {
        ApiRequestAuthority::Admin => {
            db::list_enabled_endpoints_for_project(&state.pool, &project_slug).await?
        }
        ApiRequestAuthority::Key(key) => {
            if key.project_id != project.project.id {
                return Err(ApiError::AgentAccessDenied);
            }
            if !key.permissions.chat_completions_allowed() {
                return Err(ApiError::EndpointAccessDenied);
            }
            db::list_enabled_endpoints_for_project_id(&state.pool, key.project_id).await?
        }
    };
    if let ApiRequestAuthority::Key(key) = authority {
        endpoints.retain(|endpoint| key.agent_scope.allows(endpoint.binding_id));
    }
    Ok(openai_models_response(endpoints))
}

fn openai_models_response(endpoints: Vec<AgentEndpoint>) -> Json<Value> {
    Json(json!({
        "object": "list",
        "data": endpoints
            .into_iter()
            .map(|endpoint| json!({
                "id": endpoint.slug,
                "object": "model",
                "created": endpoint.created_at.timestamp(),
                "owned_by": "vifu",
            }))
            .collect::<Vec<_>>()
    }))
}

pub async fn create_chat_completion(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    create_chat_completion_for_project(state, headers, None, request).await
}

pub async fn create_project_chat_completion(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_slug): Path<String>,
    Json(request): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    create_chat_completion_for_project(state, headers, Some(project_slug), request).await
}

async fn create_chat_completion_for_project(
    state: AppState,
    headers: HeaderMap,
    project_slug: Option<String>,
    mut request: Value,
) -> Result<Json<Value>, ApiError> {
    let authority = api_request_authority(&state, &headers).await?;
    validate_chat_completion_request(&request)?;
    let model = request
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    if request.get("stream").and_then(Value::as_bool) == Some(true) {
        return Err(ApiError::Invalid(
            "streaming chat completions are not supported yet".to_string(),
        ));
    }
    if let Some(object) = request.as_object_mut() {
        object.insert("stream".to_string(), Value::Bool(false));
    }

    let project = match project_slug.as_deref() {
        Some(slug) => Some(db::get_project_by_slug(&state.pool, slug).await?),
        None => None,
    };
    let route = resolve_chat_route(&state, &authority, project.as_ref(), model.as_deref()).await?;
    let request_id = Uuid::new_v4();
    let gateway_session_id = state.relay.session_for(&route.gateway_id).await;
    db::create_trace(
        &state.pool,
        request_id,
        route.endpoint_id,
        project.as_ref().map(|project| project.project.id),
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
            let response = chat_completion_response(request_id, &route.endpoint_slug, output);
            persist_trace(
                &state,
                request_id,
                "completed",
                started_at,
                Some(&response),
                None,
            )
            .await;
            Ok(Json(response))
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

#[derive(Debug)]
enum ApiRequestAuthority {
    Admin,
    Key(ApiKeyRecord),
}

async fn api_request_authority(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<ApiRequestAuthority, ApiError> {
    let token = bearer_token(headers).ok_or(ApiError::Unauthorized)?;
    if is_secret_match(token, &state.config.admin_key) {
        return Ok(ApiRequestAuthority::Admin);
    }
    let key_hash = hash_api_key(token, &state.config.api_key_pepper);
    Ok(ApiRequestAuthority::Key(
        db::active_api_key_by_hash(&state.pool, &key_hash).await?,
    ))
}

async fn resolve_chat_route(
    state: &AppState,
    authority: &ApiRequestAuthority,
    project: Option<&crate::models::ProjectWithBindings>,
    model: Option<&str>,
) -> Result<EndpointRoute, ApiError> {
    match authority {
        ApiRequestAuthority::Admin => {
            let model = model.ok_or_else(|| ApiError::Invalid("model is required".to_string()))?;
            match project {
                Some(project) => {
                    db::resolve_project_model_route(&state.pool, project.project.id, model).await
                }
                None => db::resolve_endpoint_route(&state.pool, model).await,
            }
        }
        ApiRequestAuthority::Key(key) => {
            if !key.permissions.chat_completions_allowed() {
                return Err(ApiError::EndpointAccessDenied);
            }
            let project = project.ok_or(ApiError::Forbidden)?;
            if project.project.id != key.project_id {
                return Err(ApiError::AgentAccessDenied);
            }
            let model = model.ok_or(ApiError::ModelRequired)?;
            match db::resolve_project_model_route(&state.pool, key.project_id, model).await {
                Ok(route) if key.agent_scope.allows(route.binding_id) => Ok(route),
                Ok(_) => Err(ApiError::AgentAccessDenied),
                Err(ApiError::NotFound) => Err(ApiError::AgentAccessDenied),
                Err(error) => Err(error),
            }
        }
    }
}

fn validate_chat_completion_request(request: &Value) -> Result<(), ApiError> {
    let object = request
        .as_object()
        .ok_or_else(|| ApiError::Invalid("request body must be an object".to_string()))?;
    let messages = object
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| ApiError::Invalid("messages must be an array".to_string()))?;
    if messages.is_empty() {
        return Err(ApiError::Invalid("messages must not be empty".to_string()));
    }
    for message in messages {
        let role = message
            .get("role")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if role.is_none() {
            return Err(ApiError::Invalid(
                "each message must include a role".to_string(),
            ));
        }
        if !message
            .as_object()
            .is_some_and(|item| item.contains_key("content"))
        {
            return Err(ApiError::Invalid(
                "each message must include content".to_string(),
            ));
        }
    }
    if serde_json::to_vec(request)
        .map_err(|_| ApiError::Internal)?
        .len()
        > 512 * 1024
    {
        return Err(ApiError::Invalid("request body is too large".to_string()));
    }
    Ok(())
}

fn chat_completion_response(request_id: Uuid, endpoint_slug: &str, output: Value) -> Value {
    if let Value::Object(mut object) = output {
        if object
            .get("choices")
            .and_then(Value::as_array)
            .is_some_and(|choices| !choices.is_empty())
        {
            object.insert(
                "id".to_string(),
                Value::String(chat_completion_id(request_id)),
            );
            object.insert(
                "object".to_string(),
                Value::String("chat.completion".to_string()),
            );
            object.insert(
                "created".to_string(),
                Value::Number(serde_json::Number::from(Utc::now().timestamp())),
            );
            object.insert(
                "model".to_string(),
                Value::String(endpoint_slug.to_string()),
            );
            return Value::Object(object);
        }
        let output = Value::Object(object);
        let content = output
            .get("reply")
            .or_else(|| output.get("text"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| output.to_string());
        return chat_completion_text_response(request_id, endpoint_slug, content);
    }
    let content = output
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| output.to_string());
    chat_completion_text_response(request_id, endpoint_slug, content)
}

fn chat_completion_text_response(request_id: Uuid, endpoint_slug: &str, content: String) -> Value {
    json!({
        "id": chat_completion_id(request_id),
        "object": "chat.completion",
        "created": Utc::now().timestamp(),
        "model": endpoint_slug,
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": content
            },
            "finish_reason": "stop"
        }]
    })
}

fn chat_completion_id(request_id: Uuid) -> String {
    format!("chatcmpl-{request_id}")
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

struct PreparedProviderInput {
    key: String,
    name: String,
    provider_type: String,
    base_url: String,
    config: Value,
    encrypted_secret_json: String,
    secret_keys: Vec<String>,
    display_secret: Option<String>,
}

struct PreparedProviderSource<'a> {
    key: &'a str,
    name: Option<&'a str>,
    provider_type: &'a str,
    base_url: &'a str,
    config: Value,
    secrets: Value,
}

fn provider_adapters() -> Vec<ProviderAdapter> {
    vec![ProviderAdapter {
        id: "openclaw".to_string(),
        name: "OpenClaw".to_string(),
        description: "Local OpenClaw Gateway agents discovered through its HTTP API.".to_string(),
        fields: vec![
            ProviderAdapterField {
                key: "baseUrl".to_string(),
                label: "Gateway URL".to_string(),
                kind: "url".to_string(),
                required: true,
                secret: false,
            },
            ProviderAdapterField {
                key: "token".to_string(),
                label: "Gateway token".to_string(),
                kind: "password".to_string(),
                required: false,
                secret: true,
            },
        ],
    }]
}

fn active_provider_registry_file(state: &AppState) -> Option<std::path::PathBuf> {
    state.config.provider_registry_file.clone()
}

fn read_provider_registry(path: &FsPath) -> Result<AgentProvidersFile, ApiError> {
    vifu_core::config::read_provider_registry_file(path).map_err(ApiError::Invalid)
}

fn write_provider_registry(path: &FsPath, file: &AgentProvidersFile) -> Result<(), ApiError> {
    vifu_core::config::write_provider_registry_file(path, file).map_err(ApiError::Invalid)
}

fn save_file_provider_connection(
    path: &FsPath,
    project_id: Uuid,
    provider_key: &str,
    input: UpsertProviderConnection,
) -> Result<ProviderConnection, ApiError> {
    let key = required_identifier("provider key", provider_key)?.to_string();
    let provider_type = required_identifier("provider type", &input.provider_type)?.to_string();
    let name = optional_text("provider name", input.name.as_deref(), 128)?.map(str::to_string);
    let base_url = required_text("provider base URL", &input.base_url, 2048)?.to_string();
    if provider_type == "openclaw" {
        vifu_core::openclaw::parse_endpoint(&base_url).map_err(ApiError::Invalid)?;
    }
    validate_json_object("config", &input.config, 64 * 1024)?;

    let mut file = read_provider_registry(path)?;
    let existing_index = file
        .providers
        .iter()
        .position(|provider| provider.key == key);
    let existing_auth = existing_index
        .and_then(|index| file.providers.get(index))
        .map(|provider| provider.auth.clone())
        .unwrap_or_default();
    let auth = provider_auth_from_secrets(&existing_auth, &input.secrets)?;
    let definition = AgentProviderDefinition {
        key: key.clone(),
        name,
        provider_type,
        url: base_url,
        enabled: Some(true),
        auth,
        config: input.config,
    };
    if let Some(index) = existing_index {
        file.providers[index] = definition.clone();
    } else {
        file.providers.push(definition.clone());
    }
    write_provider_registry(path, &file)?;
    file_provider_connection(project_id, definition)
}

fn delete_file_provider_connection(path: &FsPath, provider_key: &str) -> Result<(), ApiError> {
    let key = required_identifier("provider key", provider_key)?;
    let mut file = read_provider_registry(path)?;
    let before = file.providers.len();
    file.providers.retain(|provider| provider.key != key);
    if file.providers.len() == before {
        return Err(ApiError::NotFound);
    }
    write_provider_registry(path, &file)
}

fn get_file_provider_connection(
    path: &FsPath,
    project_id: Uuid,
    provider_key: &str,
) -> Result<ProviderConnection, ApiError> {
    let (_, connection) =
        get_file_provider_definition_and_connection(path, project_id, provider_key)?;
    Ok(connection)
}

fn get_file_provider_definition_and_connection(
    path: &FsPath,
    project_id: Uuid,
    provider_key: &str,
) -> Result<(AgentProviderDefinition, ProviderConnection), ApiError> {
    let key = required_identifier("provider key", provider_key)?;
    let file = read_provider_registry(path)?;
    let definition = file
        .providers
        .into_iter()
        .find(|provider| provider.key == key)
        .ok_or(ApiError::NotFound)?;
    let connection = file_provider_connection(project_id, definition.clone())?;
    Ok((definition, connection))
}

fn file_provider_connection(
    project_id: Uuid,
    provider: AgentProviderDefinition,
) -> Result<ProviderConnection, ApiError> {
    let key = required_identifier("provider key", &provider.key)?.to_string();
    let provider_type = required_identifier("provider type", &provider.provider_type)?.to_string();
    let name = optional_text("provider name", provider.name.as_deref(), 128)?
        .unwrap_or(&key)
        .to_string();
    let base_url = required_text("provider base URL", &provider.url, 2048)?.to_string();
    let now = Utc::now();
    Ok(ProviderConnection {
        id: deterministic_provider_connection_id(project_id, &key),
        project_id,
        provider_key: key,
        name,
        provider_type,
        base_url,
        config: provider.config,
        secret_keys: provider_auth_secret_keys(&provider.auth),
        display_secret: provider_auth_display_secret(&provider.auth),
        status: if provider.enabled == Some(false) {
            "disabled".to_string()
        } else {
            "configured".to_string()
        },
        last_checked_at: None,
        created_at: now,
        updated_at: now,
    })
}

fn deterministic_provider_connection_id(project_id: Uuid, provider_key: &str) -> Uuid {
    let mut hasher = Sha256::new();
    hasher.update(project_id.as_bytes());
    hasher.update(provider_key.as_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    Uuid::from_bytes(bytes)
}

fn provider_auth_from_secrets(
    existing: &AgentProviderAuthDefinition,
    secrets: &Value,
) -> Result<AgentProviderAuthDefinition, ApiError> {
    validate_json_object("secrets", secrets, 64 * 1024)?;
    if is_json_object_empty(secrets) {
        return Ok(existing.clone());
    }
    Ok(AgentProviderAuthDefinition {
        token: optional_secret_string(secrets, "token")?,
    })
}

fn provider_auth_secret_keys(auth: &AgentProviderAuthDefinition) -> Vec<String> {
    let mut keys = Vec::new();
    if auth
        .token
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
    {
        keys.push("token".to_string());
    }
    keys
}

fn provider_auth_display_secret(auth: &AgentProviderAuthDefinition) -> Option<String> {
    auth.token
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(mask_secret)
}

fn optional_secret_string(value: &Value, key: &str) -> Result<Option<String>, ApiError> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(item)) => Ok(optional_text(key, Some(item), 4096)?.map(str::to_string)),
        Some(_) => Err(ApiError::Invalid(format!("{key} must be a string"))),
    }
}

fn is_json_object_empty(value: &Value) -> bool {
    value.as_object().is_some_and(serde_json::Map::is_empty)
}

async fn provider_secrets_or_existing(
    state: &AppState,
    slug: &str,
    provider_key: &str,
    secrets: Value,
) -> Result<Value, ApiError> {
    validate_json_object("secrets", &secrets, 64 * 1024)?;
    if !is_json_object_empty(&secrets) {
        return Ok(secrets);
    }
    match db::get_provider_connection_secret_by_key(&state.pool, slug, provider_key).await {
        Ok(connection) => decrypted_provider_secrets(state, &connection),
        Err(ApiError::NotFound) => Ok(secrets),
        Err(error) => Err(error),
    }
}

async fn probe_provider_status(base_url: &str) -> Result<(&'static str, Option<String>), ApiError> {
    let report = vifu_core::openclaw::probe(base_url).await;
    Ok(match report.status {
        vifu_core::openclaw::ProbeStatus::Online => ("online", None),
        vifu_core::openclaw::ProbeStatus::Offline(message) => ("offline", Some(message)),
        vifu_core::openclaw::ProbeStatus::Unsupported(message) => ("unsupported", Some(message)),
    })
}

async fn discover_file_provider_agents(
    provider: &AgentProviderDefinition,
) -> Result<Vec<vifu_core::protocol::AgentDescriptor>, ApiError> {
    if provider.provider_type != "openclaw" {
        return Err(ApiError::Invalid("unsupported provider type".to_string()));
    }
    let endpoint = vifu_core::openclaw::parse_endpoint(&provider.url).map_err(ApiError::Invalid)?;
    let token = vifu_core::config::resolve_provider_token(&provider.key, &provider.auth)
        .map_err(ApiError::Invalid)?;
    vifu_core::openclaw::discover_agents(&endpoint, token.as_deref())
        .await
        .map_err(ApiError::AgentGateway)
}

fn prepare_provider_connection(
    state: &AppState,
    source: PreparedProviderSource<'_>,
) -> Result<PreparedProviderInput, ApiError> {
    let key = required_identifier("provider key", source.key)?.to_string();
    let provider_type = required_identifier("provider type", source.provider_type)?.to_string();
    let name = optional_text("provider name", source.name, 128)?
        .unwrap_or(&key)
        .to_string();
    let base_url = required_text("provider base URL", source.base_url, 2048)?.to_string();
    if provider_type == "openclaw" {
        vifu_core::openclaw::parse_endpoint(&base_url).map_err(ApiError::Invalid)?;
    }
    validate_json_object("config", &source.config, 64 * 1024)?;
    validate_json_object("secrets", &source.secrets, 64 * 1024)?;
    let secret_keys = provider_secret_keys(&source.secrets)?;
    let display_secret = provider_secret_display(&source.secrets)?;
    let encrypted_secret_json = encrypt_secret_json(
        &serde_json::to_string(&source.secrets).map_err(|_| ApiError::Internal)?,
        &state.config.provider_secret_key,
    )?;
    Ok(PreparedProviderInput {
        key,
        name,
        provider_type,
        base_url,
        config: source.config,
        encrypted_secret_json,
        secret_keys,
        display_secret,
    })
}

async fn save_provider_connection(
    state: &AppState,
    project_slug: &str,
    input: PreparedProviderInput,
) -> Result<crate::models::ProviderConnection, ApiError> {
    db::upsert_provider_connection(
        &state.pool,
        project_slug,
        db::NewProviderConnection {
            provider_key: &input.key,
            name: &input.name,
            provider_type: &input.provider_type,
            base_url: &input.base_url,
            config: &input.config,
            encrypted_secret_json: &input.encrypted_secret_json,
            secret_keys: &input.secret_keys,
            display_secret: input.display_secret.as_deref(),
            status: "configured",
        },
    )
    .await
}

fn provider_secret_keys(secrets: &Value) -> Result<Vec<String>, ApiError> {
    let object = secrets
        .as_object()
        .ok_or_else(|| ApiError::Invalid("secrets must be an object".to_string()))?;
    let mut keys = Vec::with_capacity(object.len());
    for (key, value) in object {
        if value.as_str().is_some_and(|item| item.trim().is_empty()) || value.is_null() {
            continue;
        }
        keys.push(required_identifier("secret key", key)?.to_string());
    }
    keys.sort();
    Ok(keys)
}

fn provider_secret_display(secrets: &Value) -> Result<Option<String>, ApiError> {
    if let Some(token) = optional_secret_string(secrets, "token")? {
        return Ok(Some(mask_secret(&token)));
    }
    Ok(None)
}

fn decrypted_provider_secrets(
    state: &AppState,
    connection: &ProviderConnectionSecret,
) -> Result<Value, ApiError> {
    let plaintext = decrypt_secret_json(
        &connection.encrypted_secret_json,
        &state.config.provider_secret_key,
    )?;
    let secrets: Value = serde_json::from_str(&plaintext)
        .map_err(|_| ApiError::Invalid("provider secrets are invalid".to_string()))?;
    validate_json_object("secrets", &secrets, 64 * 1024)?;
    Ok(secrets)
}

fn provider_token(secrets: &Value) -> Result<Option<String>, ApiError> {
    let auth = provider_auth_from_secrets(&AgentProviderAuthDefinition::default(), secrets)?;
    vifu_core::config::resolve_provider_token("provider", &auth).map_err(ApiError::Invalid)
}

fn mask_secret(value: &str) -> String {
    let suffix_rev: String = value.chars().rev().take(4).collect();
    let suffix: String = suffix_rev.chars().rev().collect();
    format!("****{suffix}")
}

async fn admin(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    if bearer_token(headers).is_some_and(|token| is_secret_match(token, &state.config.admin_key)) {
        return Ok(());
    }
    Err(ApiError::Forbidden)
}

fn require_agent_gateway_bootstrap(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    let token = bearer_token(headers).ok_or(ApiError::Unauthorized)?;
    if is_secret_match(token, &state.config.agent_gateway_bootstrap_token) {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

fn normalize_api_key_agent_scope(
    agent_scope: ApiKeyAgentScope,
) -> Result<ApiKeyAgentScope, ApiError> {
    match agent_scope {
        ApiKeyAgentScope::All => Ok(ApiKeyAgentScope::All),
        ApiKeyAgentScope::Selected { mut binding_ids } => {
            binding_ids.sort_unstable();
            binding_ids.dedup();
            if binding_ids.is_empty() || binding_ids.len() > 256 {
                return Err(ApiError::Invalid(
                    "selected agent access requires between 1 and 256 agents".to_string(),
                ));
            }
            Ok(ApiKeyAgentScope::Selected { binding_ids })
        }
    }
}

fn validate_agent_gateway_credential(value: &str) -> Result<&str, ApiError> {
    let value = value.trim();
    let secret = value.strip_prefix("vifu_gw_").ok_or_else(|| {
        ApiError::Invalid("agent gateway credential must start with vifu_gw_".to_string())
    })?;
    if !(48..=256).contains(&value.len())
        || !secret
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(ApiError::Invalid(
            "agent gateway credential is invalid".to_string(),
        ));
    }
    Ok(value)
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

fn project_slug(explicit: Option<&str>, name: &str) -> Result<String, ApiError> {
    match explicit {
        Some(value) => validate_explicit_slug(value),
        None => {
            let value = slugify(name);
            if validate_slug(&value) {
                return Ok(value);
            }
            let fallback = if value.is_empty() {
                let suffix = Uuid::new_v4().simple().to_string();
                format!("project-{}", &suffix[..8])
            } else {
                format!("project-{value}")
            };
            if validate_slug(&fallback) {
                Ok(fallback)
            } else {
                let suffix = Uuid::new_v4().simple().to_string();
                Ok(format!("project-{}", &suffix[..8]))
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
    use super::{patch_text, profile_slug, project_slug, validate_timeout};

    #[test]
    fn derives_profile_slugs() {
        assert_eq!(profile_slug(None, "Town Guide").unwrap(), "town-guide");
    }

    #[test]
    fn derives_project_slugs_without_manual_input() {
        assert_eq!(project_slug(None, "Town Guide").unwrap(), "town-guide");
        assert_eq!(project_slug(None, "AI").unwrap(), "project-ai");
        assert!(project_slug(None, "ゲーム")
            .unwrap()
            .starts_with("project-"));
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
