use std::collections::{BTreeMap, HashSet};
use std::path::Path as FsPath;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Multipart, Path, Query, State, WebSocketUpgrade};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use base64::Engine;
use chrono::{Duration as ChronoDuration, Utc};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tracing::warn;
use uuid::Uuid;
use vifu_gateway::protocol::{validate_identifier, MAX_INVOCATION_BODY_BYTES};
use vifu_runtime::{
    AgentDefinition, AgentProvider, EndpointDefinition, HttpCapabilityProvider,
    HttpCapabilityRoute, InvocationData, InvocationInput, ProjectSettings, RuntimeError,
    RuntimeRelease, RuntimeTraceRecord, VifuRuntime,
};

use crate::auth::{
    bearer_token, decrypt_secret_json, deployment_credential, derive_guest_claim_token,
    derive_guest_project_key, encrypt_secret_json, hash_agent_gateway_credential,
    hash_agent_gateway_enrollment, hash_api_key, hash_guest_claim_token, is_secret_match, Identity,
    Operation,
};
use crate::config::DeploymentMode;
use crate::db::{self, EndpointPatch, NewEndpoint, NewProject, ProfilePatch, ProjectPatch};
use crate::error::ApiError;
use crate::models::{
    slugify, validate_slug, AgentEndpoint, AgentGatewaySession, ApiKeyAgentScope,
    ApiKeyPermissions, ApiKeyRecord, AssignProjectOwner, BootstrapGatewayRuntimeRelease,
    Capabilities, ClaimGuestProject, CreateApiKey, CreateBinding, CreateEndpoint, CreateProfile,
    CreateProfileVersion, CreateProject, CreateProjectProvider, CreateRuntimeDeployment,
    CreatedApiKey, CustomProvider, EndpointRoute, ImportProjectAgent, ImportProjectProfile,
    ImportProjectProvider, ImportProjectSettings, ProfileCapabilityDraft, ProjectOwnership,
    ProviderAdapter, ProviderAdapterField, ProviderConnection, ProviderConnectionSecret,
    RegisterAgentGateway, RuntimeDeployment, RuntimeDeploymentView, SetProfileRollout,
    SyncProfileSource, TestProfile, UpdateApiKey, UpdateBinding, UpdateEndpoint, UpdateProfile,
    UpdateProject, UpdateProjectProvider, UpdateRuntimeDeployment,
};
use crate::openclaw_device;
use crate::relay::RelayAgentProvider;
use crate::AppState;

const MAX_CHAT_REQUEST_BYTES: usize = MAX_INVOCATION_BODY_BYTES;
const MAX_CHAT_IMAGES: usize = 16;

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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UploadRuntimeTraces {
    deployment_id: Uuid,
    traces: Vec<RuntimeTraceRecord>,
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

pub async fn exchange_deployment_credential(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<impl Serialize>, ApiError> {
    let access_token = bearer_token(&headers).ok_or(ApiError::Unauthorized)?;
    Ok(Json(state.auth.exchange_access_token(access_token).await?))
}

pub async fn bootstrap_guest_project(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    if !state.config.guest_bootstrap_enabled {
        return Err(ApiError::Forbidden);
    }
    let credential = bearer_token(&headers).ok_or(ApiError::Unauthorized)?;
    let gateway_id = authenticated_agent_gateway(&state, &headers).await?;
    db::prune_expired_guest_projects(&state.pool).await?;
    if let Some((project, expires_at)) =
        db::get_active_guest_project_for_gateway(&state.pool, &gateway_id).await?
    {
        let response = guest_project_response(&state, project, expires_at, credential).await?;
        return Ok((StatusCode::OK, Json(response)));
    }
    if db::count_active_guest_projects(&state.pool).await?
        >= i64::from(state.config.guest_project_limit)
    {
        return Err(ApiError::Conflict(
            "guest project capacity is temporarily unavailable".to_string(),
        ));
    }

    let project_id = Uuid::new_v4();
    let slug = guest_project_slug(&gateway_id);
    let project = db::create_project(
        &state.pool,
        NewProject {
            id: project_id,
            owner_user_id: None,
            slug: &slug,
            name: "Guest project",
            description: None,
            gateway_id: &gateway_id,
            binding_ids: &[],
        },
    )
    .await?;
    let expires_at = Utc::now()
        + ChronoDuration::from_std(state.config.guest_project_ttl)
            .map_err(|_| ApiError::Internal)?;
    let claim_token = derive_guest_claim_token(credential, &state.config.api_key_pepper);
    let claim_token_hash = hash_guest_claim_token(&claim_token, &state.config.api_key_pepper);
    if let Err(error) = db::create_guest_project(
        &state.pool,
        db::NewGuestProject {
            project_id,
            gateway_id: &gateway_id,
            claim_token_hash: &claim_token_hash,
            expires_at,
        },
    )
    .await
    {
        let _ = db::delete_project(&state.pool, project_id).await;
        if let Some((existing, existing_expires_at)) =
            db::get_active_guest_project_for_gateway(&state.pool, &gateway_id).await?
        {
            let response =
                guest_project_response(&state, existing, existing_expires_at, credential).await?;
            return Ok((StatusCode::OK, Json(response)));
        }
        return Err(error);
    }
    let response = guest_project_response(&state, project, expires_at, credential).await?;
    Ok((StatusCode::CREATED, Json(response)))
}

pub async fn claim_guest_project(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<ClaimGuestProject>,
) -> Result<Json<Value>, ApiError> {
    let identity = deployment_identity(&state, &headers, Operation::ProjectWrite).await?;
    let Identity::ActingUser { subject, .. } = identity else {
        return Err(ApiError::Forbidden);
    };
    let claim_token = validate_guest_claim_token(&input.claim_token)?;
    let claim_token_hash = hash_guest_claim_token(claim_token, &state.config.api_key_pepper);
    let project = db::claim_guest_project(&state.pool, &claim_token_hash, &subject).await?;
    Ok(Json(json!({ "project": project })))
}

async fn guest_project_response(
    state: &AppState,
    project: crate::models::ProjectWithBindings,
    expires_at: chrono::DateTime<Utc>,
    gateway_credential: &str,
) -> Result<Value, ApiError> {
    let project_key = ensure_guest_project_key(state, &project, gateway_credential).await?;
    let mut deployment = db::list_runtime_deployments(&state.pool, project.project.id)
        .await?
        .into_iter()
        .find(|deployment| deployment.is_primary)
        .ok_or(ApiError::Internal)?;
    if !deployment.remote_invocation_enabled {
        deployment = db::update_runtime_deployment(
            &state.pool,
            project.project.id,
            &deployment.name,
            db::RuntimeDeploymentPatch {
                config_sync_enabled: None,
                trace_mode: None,
                remote_invocation_enabled: Some(true),
            },
        )
        .await?;
    }
    Ok(json!({
        "project": {
            "id": project.project.id,
            "slug": project.project.slug,
        },
        "deployment": {
            "id": deployment.id,
            "name": deployment.name,
        },
        "endpointPath": format!("/{}/v1", project.project.slug),
        "apiKey": project_key,
        "claimToken": derive_guest_claim_token(
            gateway_credential,
            &state.config.api_key_pepper,
        ),
        "expiresAt": expires_at,
    }))
}

async fn ensure_guest_project_key(
    state: &AppState,
    project: &crate::models::ProjectWithBindings,
    gateway_credential: &str,
) -> Result<String, ApiError> {
    let raw_key = derive_guest_project_key(gateway_credential, &state.config.api_key_pepper);
    let key_prefix = raw_key.chars().take(18).collect::<String>();
    let exists = db::list_api_keys(&state.pool)
        .await?
        .into_iter()
        .any(|key| {
            key.project_id == project.project.id
                && key.key_prefix == key_prefix
                && key.revoked_at.is_none()
        });
    if !exists {
        let key_hash = hash_api_key(&raw_key, &state.config.api_key_pepper);
        let created = db::create_api_key(
            &state.pool,
            db::NewApiKey {
                id: Uuid::new_v4(),
                project_id: project.project.id,
                name: "Guest project key",
                agent_scope: &ApiKeyAgentScope::All,
                permissions: &ApiKeyPermissions::default(),
                key_prefix: &key_prefix,
                key_hash: &key_hash,
            },
        )
        .await;
        if let Err(error) = created {
            let now_exists = db::list_api_keys(&state.pool)
                .await?
                .into_iter()
                .any(|key| {
                    key.project_id == project.project.id
                        && key.key_prefix == key_prefix
                        && key.revoked_at.is_none()
                });
            if !now_exists {
                return Err(error);
            }
        }
    }
    Ok(raw_key)
}

pub async fn verify_admin(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<impl Serialize>, ApiError> {
    deployment_admin(&state, &headers, Operation::DeploymentRead).await?;
    Ok(Json(json!({
        "valid": true,
    })))
}

pub async fn list_project_ownership(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    deployment_admin(&state, &headers, Operation::DeploymentRead).await?;
    let projects = db::list_projects(&state.pool)
        .await?
        .into_iter()
        .map(|project| ProjectOwnership {
            project_id: project.project.id,
            slug: project.project.slug,
            name: project.project.name,
            owner_user_id: project.project.owner_user_id,
        })
        .collect::<Vec<_>>();
    Ok(Json(json!({ "projects": projects })))
}

pub async fn assign_project_owner(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<Uuid>,
    Json(input): Json<AssignProjectOwner>,
) -> Result<Json<Value>, ApiError> {
    deployment_admin(&state, &headers, Operation::DeploymentWrite).await?;
    let owner_user_id = required_text("ownerUserId", &input.owner_user_id, 512)?;
    let project = db::set_project_owner_user_id(&state.pool, project_id, owner_user_id).await?;
    Ok(Json(json!({
        "project": ProjectOwnership {
            project_id: project.project.id,
            slug: project.project.slug,
            name: project.project.name,
            owner_user_id: project.project.owner_user_id,
        }
    })))
}

pub async fn list_projects(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let identity = deployment_identity(&state, &headers, Operation::ProjectRead).await?;
    let projects = match identity {
        Identity::DeploymentAdmin => db::list_projects(&state.pool).await?,
        Identity::ActingUser { subject, .. } => {
            db::list_projects_for_owner_user_id(&state.pool, &subject).await?
        }
    };
    Ok(Json(json!({ "projects": projects })))
}

pub async fn create_project(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<CreateProject>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let identity = deployment_identity(&state, &headers, Operation::ProjectWrite).await?;
    let owner_user_id = match &identity {
        Identity::DeploymentAdmin => None,
        Identity::ActingUser { subject, .. } => Some(subject.as_str()),
    };
    let name = required_text("name", &input.name, 128)?;
    let slug = project_slug(input.slug.as_deref(), name)?;
    let description = optional_text("description", input.description.as_deref(), 4096)?;
    let gateway_id = format!("project-{slug}");
    let project = db::create_project(
        &state.pool,
        NewProject {
            id: Uuid::new_v4(),
            owner_user_id,
            slug: &slug,
            name,
            description,
            gateway_id: &gateway_id,
            binding_ids: &[],
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
    let project = db::get_project(&state.pool, id).await?;
    state
        .auth
        .authorize_project(
            &headers,
            Operation::ProjectRead,
            project.project.owner_user_id.as_deref(),
        )
        .await?;
    Ok(Json(json!({ "project": project })))
}

pub async fn update_project(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(input): Json<UpdateProject>,
) -> Result<Json<Value>, ApiError> {
    let current = db::get_project(&state.pool, id).await?;
    state
        .auth
        .authorize_project(
            &headers,
            Operation::ProjectWrite,
            current.project.owner_user_id.as_deref(),
        )
        .await?;
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
    let project = db::get_project(&state.pool, id).await?;
    state
        .auth
        .authorize_project(
            &headers,
            Operation::ProjectWrite,
            project.project.owner_user_id.as_deref(),
        )
        .await?;
    db::delete_project(&state.pool, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_project_runtime_deployments(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let project = authorized_project_by_slug(&state, &headers, &slug, ProjectAccess::Read).await?;
    Ok(Json(json!({
        "deployments": runtime_deployment_views(&state, project.project.id).await?
    })))
}

pub async fn create_project_runtime_deployment(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(input): Json<CreateRuntimeDeployment>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let project = authorized_project_by_slug(&state, &headers, &slug, ProjectAccess::Write).await?;
    let name = validate_explicit_slug(&input.name)?;
    let trace_mode = validate_trace_mode(input.trace_mode.as_deref().unwrap_or("summary"))?;
    let deployment = db::create_runtime_deployment(
        &state.pool,
        db::NewRuntimeDeployment {
            id: Uuid::new_v4(),
            project_id: project.project.id,
            name: &name,
            is_primary: false,
            config_sync_enabled: input.config_sync_enabled.unwrap_or(true),
            trace_mode,
            remote_invocation_enabled: input.remote_invocation_enabled.unwrap_or(false),
        },
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "deployment": runtime_deployment_view(&state, deployment).await?
        })),
    ))
}

pub async fn update_project_runtime_deployment(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((slug, deployment)): Path<(String, String)>,
    Json(input): Json<UpdateRuntimeDeployment>,
) -> Result<Json<Value>, ApiError> {
    let project = authorized_project_by_slug(&state, &headers, &slug, ProjectAccess::Write).await?;
    let deployment = validate_explicit_slug(&deployment)?;
    let trace_mode = input
        .trace_mode
        .as_deref()
        .map(validate_trace_mode)
        .transpose()?;
    let deployment = db::update_runtime_deployment(
        &state.pool,
        project.project.id,
        &deployment,
        db::RuntimeDeploymentPatch {
            config_sync_enabled: input.config_sync_enabled,
            trace_mode,
            remote_invocation_enabled: input.remote_invocation_enabled,
        },
    )
    .await?;
    notify_runtime_deployments(&state, std::slice::from_ref(&deployment)).await?;
    Ok(Json(json!({
        "deployment": runtime_deployment_view(&state, deployment).await?
    })))
}

pub async fn delete_project_runtime_deployment(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((slug, deployment)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let project = authorized_project_by_slug(&state, &headers, &slug, ProjectAccess::Write).await?;
    let deployment = validate_explicit_slug(&deployment)?;
    let deployment =
        db::get_runtime_deployment(&state.pool, project.project.id, &deployment).await?;
    let gateway_ids = db::list_runtime_deployment_gateway_ids(&state.pool, deployment.id).await?;
    db::delete_runtime_deployment(&state.pool, project.project.id, &deployment.name).await?;
    for gateway_id in gateway_ids {
        state
            .relay
            .notify_runtime_config(&gateway_id, vec![deployment.id])
            .await;
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn promote_project_runtime_deployment(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((slug, deployment)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let project = authorized_project_by_slug(&state, &headers, &slug, ProjectAccess::Write).await?;
    let deployment = validate_explicit_slug(&deployment)?;
    let project_deployments = db::list_runtime_deployments(&state.pool, project.project.id).await?;
    let deployment =
        db::promote_runtime_deployment(&state.pool, project.project.id, &deployment).await?;
    notify_runtime_deployments(&state, &project_deployments).await?;
    Ok(Json(json!({
        "deployment": runtime_deployment_view(&state, deployment).await?
    })))
}

pub async fn list_project_runtime_releases(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let project = authorized_project_by_slug(&state, &headers, &slug, ProjectAccess::Read).await?;
    Ok(Json(json!({
        "releases": db::list_project_runtime_releases(&state.pool, project.project.id).await?
    })))
}

pub async fn get_project_runtime_release(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((slug, version)): Path<(String, i64)>,
) -> Result<Json<Value>, ApiError> {
    let project = authorized_project_by_slug(&state, &headers, &slug, ProjectAccess::Read).await?;
    Ok(Json(json!({
        "release": db::get_project_runtime_release(&state.pool, project.project.id, version).await?
    })))
}

pub async fn publish_project_runtime_release(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(input): Json<ImportProjectSettings>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let project = db::get_project_by_slug(&state.pool, &slug).await?;
    let identity = state
        .auth
        .authorize_project(
            &headers,
            Operation::ProjectWrite,
            project.project.owner_user_id.as_deref(),
        )
        .await?;
    let manifest = serde_json::from_value::<ProjectSettings>(input.settings)
        .map_err(|error| ApiError::Invalid(format!("project settings are invalid: {error}")))?;
    manifest
        .validate()
        .map_err(|error| ApiError::Invalid(error.to_string()))?;
    if manifest.project_id != project.project.slug {
        return Err(ApiError::Invalid(
            "project settings projectId must match the project slug".to_string(),
        ));
    }
    let releases = db::list_project_runtime_releases(&state.pool, project.project.id).await?;
    let content_hash = manifest
        .content_hash()
        .map_err(|error| ApiError::Invalid(error.to_string()))?;
    if let Some(existing) = releases
        .iter()
        .find(|release| release.content_hash == content_hash)
    {
        return Ok((StatusCode::OK, Json(json!({ "release": existing }))));
    }
    let version = releases
        .first()
        .map_or(1, |release| release.version.saturating_add(1));
    let release = RuntimeRelease::new(
        u64::try_from(version).map_err(|_| ApiError::Internal)?,
        manifest,
    )
    .map_err(|error| ApiError::Invalid(error.to_string()))?;
    let manifest = serde_json::to_value(&release.manifest).map_err(|_| ApiError::Internal)?;
    let created_by = match &identity {
        Identity::DeploymentAdmin => None,
        Identity::ActingUser { subject, .. } => Some(subject.as_str()),
    };
    let release = db::create_project_runtime_release(
        &state.pool,
        db::NewProjectRuntimeRelease {
            id: Uuid::new_v4(),
            project_id: project.project.id,
            version,
            content_hash: &release.content_hash,
            manifest: &manifest,
            created_by,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(json!({ "release": release }))))
}

pub async fn activate_project_runtime_release(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((slug, deployment, version)): Path<(String, String, i64)>,
) -> Result<Json<Value>, ApiError> {
    let project = authorized_project_by_slug(&state, &headers, &slug, ProjectAccess::Write).await?;
    let deployment = validate_explicit_slug(&deployment)?;
    let deployment = db::activate_runtime_deployment_release(
        &state.pool,
        project.project.id,
        &deployment,
        version,
    )
    .await?;
    notify_runtime_deployments(&state, std::slice::from_ref(&deployment)).await?;
    Ok(Json(json!({
        "deployment": runtime_deployment_view(&state, deployment).await?
    })))
}

async fn runtime_deployment_views(
    state: &AppState,
    project_id: Uuid,
) -> Result<Vec<RuntimeDeploymentView>, ApiError> {
    let deployments = db::list_runtime_deployments(&state.pool, project_id).await?;
    let mut views = Vec::with_capacity(deployments.len());
    for deployment in deployments {
        views.push(runtime_deployment_view(state, deployment).await?);
    }
    Ok(views)
}

async fn runtime_deployment_view(
    state: &AppState,
    deployment: RuntimeDeployment,
) -> Result<RuntimeDeploymentView, ApiError> {
    let gateway_ids = db::list_runtime_deployment_gateway_ids(&state.pool, deployment.id).await?;
    Ok(RuntimeDeploymentView {
        deployment,
        gateway_ids,
    })
}

async fn notify_runtime_deployments(
    state: &AppState,
    deployments: &[RuntimeDeployment],
) -> Result<(), ApiError> {
    let mut notifications = BTreeMap::<String, Vec<Uuid>>::new();
    for deployment in deployments {
        for gateway_id in
            db::list_runtime_deployment_gateway_ids(&state.pool, deployment.id).await?
        {
            notifications
                .entry(gateway_id)
                .or_default()
                .push(deployment.id);
        }
    }
    for (gateway_id, mut deployment_ids) in notifications {
        deployment_ids.sort_unstable();
        deployment_ids.dedup();
        state
            .relay
            .notify_runtime_config(&gateway_id, deployment_ids)
            .await;
    }
    Ok(())
}

fn validate_trace_mode(value: &str) -> Result<&str, ApiError> {
    match value {
        "off" | "summary" | "full" => Ok(value),
        _ => Err(ApiError::Invalid(
            "traceMode must be off, summary, or full".to_string(),
        )),
    }
}

pub async fn list_profiles(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    deployment_admin(&state, &headers, Operation::DeploymentRead).await?;
    Ok(Json(
        json!({ "profiles": db::list_profiles(&state.pool).await? }),
    ))
}

pub async fn create_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<CreateProfile>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    deployment_admin(&state, &headers, Operation::DeploymentWrite).await?;
    create_profile_record(&state, input).await
}

async fn create_profile_record(
    state: &AppState,
    input: CreateProfile,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let project_id = input
        .project_id
        .ok_or_else(|| ApiError::Invalid("projectId is required".to_string()))?;
    db::get_project(&state.pool, project_id).await?;
    let name = required_text("name", &input.name, 128)?;
    let slug = profile_slug(input.slug.as_deref(), name)?;
    let description = optional_text("description", input.description.as_deref(), 4096)?;
    validate_profile_version_input(
        &input.persona,
        &input.runtime,
        &input.presentation,
        &input.source,
        &input.capabilities,
    )?;
    validate_project_profile_providers(state, project_id, &input.source, &input.capabilities)
        .await?;
    let profile = db::create_profile(
        &state.pool,
        Uuid::new_v4(),
        project_id,
        &slug,
        name,
        description,
    )
    .await?;
    let version = db::create_profile_version(
        &state.pool,
        profile.id,
        db::NewProfileVersion {
            persona: &input.persona,
            runtime: &input.runtime,
            presentation: &input.presentation,
            source: &input.source,
            capabilities: &input.capabilities,
            change_summary: optional_text("change summary", input.change_summary.as_deref(), 1024)?,
        },
    )
    .await?;
    let profile = db::get_profile(&state.pool, profile.id).await?;
    Ok((
        StatusCode::CREATED,
        Json(
            json!({ "profile": profile, "version": profile_version_payload(state, version).await? }),
        ),
    ))
}

pub async fn get_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    deployment_admin(&state, &headers, Operation::DeploymentRead).await?;
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
    deployment_admin(&state, &headers, Operation::DeploymentWrite).await?;
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
    deployment_admin(&state, &headers, Operation::DeploymentWrite).await?;
    db::delete_profile(&state.pool, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_project_profiles(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_slug): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let project =
        authorized_project_by_slug(&state, &headers, &project_slug, ProjectAccess::Read).await?;
    Ok(Json(json!({
        "profiles": db::list_project_profiles(&state.pool, project.project.id).await?
    })))
}

pub async fn create_project_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_slug): Path<String>,
    Json(mut input): Json<CreateProfile>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let project =
        authorized_project_by_slug(&state, &headers, &project_slug, ProjectAccess::Write).await?;
    input.project_id = Some(project.project.id);
    create_profile_record(&state, input).await
}

pub async fn import_project_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_slug): Path<String>,
    Json(input): Json<ImportProjectProfile>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let project =
        authorized_project_by_slug(&state, &headers, &project_slug, ProjectAccess::Write).await?;
    let archive_id = required_text("archiveId", &input.archive_id, 128)?;
    let active_version_id = required_text("activeVersionId", &input.active_version_id, 128)?;
    if input.versions.is_empty() || input.versions.len() > 50 {
        return Err(ApiError::Invalid(
            "an imported profile requires between 1 and 50 versions".to_string(),
        ));
    }
    let name = required_text("name", &input.name, 128)?;
    let slug = profile_slug(input.slug.as_deref(), name)?;
    let description = optional_text("description", input.description.as_deref(), 4096)?;
    let mut archive_version_ids = HashSet::new();
    for version in &input.versions {
        let version_archive_id = required_text("version archiveId", &version.archive_id, 128)?;
        if !archive_version_ids.insert(version_archive_id.to_string()) {
            return Err(ApiError::Invalid(
                "imported profile version archive IDs must be unique".to_string(),
            ));
        }
        validate_profile_version_input(
            &version.persona,
            &version.runtime,
            &version.presentation,
            &version.source,
            &version.capabilities,
        )?;
        optional_text("change summary", version.change_summary.as_deref(), 1024)?;
    }
    if !archive_version_ids.contains(active_version_id) {
        return Err(ApiError::Invalid(
            "activeVersionId must identify an imported profile version".to_string(),
        ));
    }

    // Portable profile imports intentionally keep provider references unresolved.
    // Credentials and provider connections are deployment-owned and are never copied with projects.
    let profile = db::create_profile(
        &state.pool,
        Uuid::new_v4(),
        project.project.id,
        &slug,
        name,
        description,
    )
    .await?;
    let mut version_map = BTreeMap::new();
    for version in input.versions {
        let created = db::create_profile_version(
            &state.pool,
            profile.id,
            db::NewProfileVersion {
                persona: &version.persona,
                runtime: &version.runtime,
                presentation: &version.presentation,
                source: &version.source,
                capabilities: &version.capabilities,
                change_summary: optional_text(
                    "change summary",
                    version.change_summary.as_deref(),
                    1024,
                )?,
            },
        )
        .await?;
        version_map.insert(version.archive_id, created.id);
    }
    let imported_active_id = *version_map
        .get(active_version_id)
        .ok_or(ApiError::Internal)?;
    db::set_profile_rollout(&state.pool, profile.id, &[(imported_active_id, 10_000)]).await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "archiveId": archive_id,
            "profile": db::get_profile(&state.pool, profile.id).await?,
            "versionMap": version_map,
        })),
    ))
}

pub async fn get_project_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((project_slug, profile_id)): Path<(String, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    let project =
        authorized_project_by_slug(&state, &headers, &project_slug, ProjectAccess::Read).await?;
    let profile = db::get_project_profile(&state.pool, project.project.id, profile_id).await?;
    Ok(Json(profile_detail_payload(&state, profile).await?))
}

pub async fn update_project_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((project_slug, profile_id)): Path<(String, Uuid)>,
    Json(input): Json<UpdateProfile>,
) -> Result<Json<Value>, ApiError> {
    let project =
        authorized_project_by_slug(&state, &headers, &project_slug, ProjectAccess::Write).await?;
    db::get_project_profile(&state.pool, project.project.id, profile_id).await?;
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
        profile_id,
        ProfilePatch {
            slug: slug.as_deref(),
            name,
            description_changed,
            description,
        },
    )
    .await?;
    Ok(Json(profile_detail_payload(&state, profile).await?))
}

pub async fn archive_project_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((project_slug, profile_id)): Path<(String, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let project =
        authorized_project_by_slug(&state, &headers, &project_slug, ProjectAccess::Write).await?;
    db::archive_project_profile(&state.pool, project.project.id, profile_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn create_project_profile_version(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((project_slug, profile_id)): Path<(String, Uuid)>,
    Json(input): Json<CreateProfileVersion>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let project =
        authorized_project_by_slug(&state, &headers, &project_slug, ProjectAccess::Write).await?;
    db::get_project_profile(&state.pool, project.project.id, profile_id).await?;
    validate_profile_version_input(
        &input.persona,
        &input.runtime,
        &input.presentation,
        &input.source,
        &input.capabilities,
    )?;
    validate_project_profile_providers(
        &state,
        project.project.id,
        &input.source,
        &input.capabilities,
    )
    .await?;
    let version = db::create_profile_version(
        &state.pool,
        profile_id,
        db::NewProfileVersion {
            persona: &input.persona,
            runtime: &input.runtime,
            presentation: &input.presentation,
            source: &input.source,
            capabilities: &input.capabilities,
            change_summary: optional_text("change summary", input.change_summary.as_deref(), 1024)?,
        },
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(profile_version_payload(&state, version).await?),
    ))
}

pub async fn sync_project_profile_source(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((project_slug, profile_id)): Path<(String, Uuid)>,
    Json(input): Json<SyncProfileSource>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let project =
        authorized_project_by_slug(&state, &headers, &project_slug, ProjectAccess::Write).await?;
    let profile = db::get_project_profile(&state.pool, project.project.id, profile_id).await?;
    let active_version_id = profile.active_version_id.ok_or_else(|| {
        ApiError::Conflict("profile does not have an active version to sync".to_string())
    })?;
    let active_version =
        db::get_profile_version(&state.pool, profile.id, active_version_id).await?;
    let capabilities = db::list_profile_capabilities(&state.pool, active_version.id).await?;
    let source = openclaw_source(&active_version.source, &capabilities)?;
    let mut client =
        connect_openclaw_management_client(&state, &project_slug, &source.provider_key).await?;
    let agents = client.agents().await.map_err(ApiError::Provider)?;
    if !agents.iter().any(|agent| agent.id == source.resource_id) {
        return Err(ApiError::NotFound);
    }
    let persona_files = read_openclaw_persona_files(&mut client, &source.resource_id).await?;
    let tools = match client.tools_catalog(&source.resource_id).await {
        Ok(catalog) => compact_openclaw_tool_catalog(&catalog),
        Err(error) if error.contains("does not advertise required method") => Vec::new(),
        Err(error) => return Err(ApiError::Provider(error)),
    };
    client.close().await.map_err(ApiError::Provider)?;

    let mut persona = active_version.persona.clone();
    let persona_object = persona
        .as_object_mut()
        .ok_or_else(|| ApiError::Invalid("profile persona must be an object".to_string()))?;
    persona_object.insert("files".to_string(), json!(persona_files));

    let mut next_source = active_version.source.clone();
    let source_object = next_source
        .as_object_mut()
        .ok_or_else(|| ApiError::Invalid("profile source must be an object".to_string()))?;
    source_object.insert("type".to_string(), Value::String("openclaw".to_string()));
    source_object.insert(
        "providerKey".to_string(),
        Value::String(source.provider_key.clone()),
    );
    source_object.insert(
        "resourceId".to_string(),
        Value::String(source.resource_id.clone()),
    );
    source_object.insert("managed".to_string(), Value::Bool(true));
    source_object.insert(
        "syncedAt".to_string(),
        Value::String(Utc::now().to_rfc3339()),
    );

    let mut capability_drafts = capabilities
        .into_iter()
        .filter(|capability| capability.kind != "tool")
        .map(|capability| {
            let provider_key = if capability.provider_type == "openclaw" {
                source.provider_key.clone()
            } else {
                capability.provider_key
            };
            ProfileCapabilityDraft {
                kind: capability.kind,
                provider_type: capability.provider_type,
                provider_key,
                resource_id: capability.resource_id,
                config: capability.config,
                input_schema: capability.input_schema,
                output_schema: capability.output_schema,
            }
        })
        .collect::<Vec<_>>();
    if !tools.is_empty() {
        capability_drafts.push(ProfileCapabilityDraft {
            kind: "tool".to_string(),
            provider_type: "openclaw".to_string(),
            provider_key: source.provider_key,
            resource_id: Some(source.resource_id),
            config: json!({ "tools": tools }),
            input_schema: json!({}),
            output_schema: json!({}),
        });
    }

    let version = db::create_profile_version(
        &state.pool,
        profile.id,
        db::NewProfileVersion {
            persona: &persona,
            runtime: &active_version.runtime,
            presentation: &active_version.presentation,
            source: &next_source,
            capabilities: &capability_drafts,
            change_summary: optional_text("change summary", input.change_summary.as_deref(), 1024)?
                .or(Some("Synced from OpenClaw")),
        },
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "profile": db::get_profile(&state.pool, profile.id).await?,
            "version": profile_version_payload(&state, version).await?,
        })),
    ))
}

pub async fn activate_project_profile_version(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((project_slug, profile_id, version_id)): Path<(String, Uuid, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    let project =
        authorized_project_by_slug(&state, &headers, &project_slug, ProjectAccess::Write).await?;
    db::get_project_profile(&state.pool, project.project.id, profile_id).await?;
    sync_managed_profile_version(&state, &project_slug, profile_id, version_id).await?;
    let rollout = db::set_profile_rollout(&state.pool, profile_id, &[(version_id, 10_000)]).await?;
    Ok(Json(json!({
        "profile": db::get_profile(&state.pool, profile_id).await?,
        "rollout": rollout,
    })))
}

pub async fn set_project_profile_rollout(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((project_slug, profile_id)): Path<(String, Uuid)>,
    Json(input): Json<SetProfileRollout>,
) -> Result<Json<Value>, ApiError> {
    let project =
        authorized_project_by_slug(&state, &headers, &project_slug, ProjectAccess::Write).await?;
    db::get_project_profile(&state.pool, project.project.id, profile_id).await?;
    let allocations = input
        .allocations
        .into_iter()
        .map(|allocation| (allocation.version_id, allocation.weight_bps))
        .collect::<Vec<_>>();
    if let [(version_id, 10_000)] = allocations.as_slice() {
        sync_managed_profile_version(&state, &project_slug, profile_id, *version_id).await?;
    }
    let rollout = db::set_profile_rollout(&state.pool, profile_id, &allocations).await?;
    Ok(Json(json!({
        "profile": db::get_profile(&state.pool, profile_id).await?,
        "rollout": rollout,
    })))
}

pub async fn archive_project_profile_version(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((project_slug, profile_id, version_id)): Path<(String, Uuid, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    let project =
        authorized_project_by_slug(&state, &headers, &project_slug, ProjectAccess::Write).await?;
    db::get_project_profile(&state.pool, project.project.id, profile_id).await?;
    let version = db::archive_profile_version(&state.pool, profile_id, version_id).await?;
    Ok(Json(profile_version_payload(&state, version).await?))
}

pub async fn test_project_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((project_slug, profile_id)): Path<(String, Uuid)>,
    Json(input): Json<TestProfile>,
) -> Result<Json<Value>, ApiError> {
    let project =
        authorized_project_by_slug(&state, &headers, &project_slug, ProjectAccess::Write).await?;
    let profile = db::get_project_profile(&state.pool, project.project.id, profile_id).await?;
    if input.capability != "chat" {
        return Err(ApiError::Invalid(
            "the profile test endpoint currently accepts chat input".to_string(),
        ));
    }
    let route = db::resolve_profile_route(
        &state.pool,
        project.project.id,
        &profile.id.to_string(),
        "chat",
        input.user.as_deref(),
        input.version_id,
    )
    .await?;
    let selection_key = input.user.clone();
    let mut request = if input.input.get("messages").is_some() {
        input.input
    } else {
        let content = input
            .input
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| input.input.to_string());
        json!({
            "model": profile.slug,
            "messages": [{ "role": "user", "content": content }],
            "stream": false,
            "user": input.user,
        })
    };
    validate_chat_completion_request(&request)?;
    let preview_mode = if route.provider_type == "openclaw"
        && route.source.get("managed").and_then(Value::as_bool) != Some(false)
        && profile.active_version_id != Some(route.profile_version_id)
    {
        vifu_gateway::providers::apply_persona_to_chat_request(&mut request, &route.persona)
            .map_err(ApiError::Invalid)?;
        Some("persona-overlay")
    } else {
        None
    };
    let request_id = Uuid::new_v4();
    let gateway_id = profile_gateway_id(&route);
    let gateway_session_id = match gateway_id.as_deref() {
        Some(gateway_id) => state.relay.session_for(gateway_id).await,
        None => None,
    };
    let trace_request = chat_trace_request(&request);
    let input_summary = chat_request_summary(&request);
    let trace_id = db::create_trace(
        &state.pool,
        db::NewTrace {
            request_id,
            endpoint_id: None,
            project_id: Some(project.project.id),
            gateway_session_id,
            profile_id: Some(route.profile_id),
            profile_version_id: Some(route.profile_version_id),
            operation: "profile.test",
            provider_key: Some(&route.provider_key),
            capability_kind: Some(&route.capability_kind),
            selection_key: selection_key.as_deref(),
            request: &trace_request,
        },
    )
    .await?;
    let mut input_summary = input_summary;
    input_summary["model"] = Value::String(profile.slug.clone());
    input_summary["previewMode"] = preview_mode
        .map(|mode| Value::String(mode.to_string()))
        .unwrap_or(Value::Null);
    let span_id = db::create_trace_span(
        &state.pool,
        db::NewTraceSpan {
            trace_id,
            parent_span_id: None,
            name: "agent.test",
            kind: "provider",
            provider_key: Some(&route.provider_key),
            capability_kind: Some("chat"),
            input_summary: Some(&input_summary),
            attributes: &json!({
                "profileId": route.profile_id,
                "profileVersionId": route.profile_version_id,
                "version": route.version_number,
                "capabilityId": route.capability_id,
            }),
        },
    )
    .await?;
    let started_at = Instant::now();
    let result = invoke_profile_chat(
        &state,
        project.project.id,
        &project_slug,
        &route,
        request_id,
        request,
        profile_timeout(&route.runtime, state.config.request_timeout),
    )
    .await;
    match result {
        Ok(output) => {
            let response = chat_completion_response(request_id, &profile.slug, output);
            let duration = db::elapsed_millis(started_at);
            db::complete_trace_span(
                &state.pool,
                span_id,
                "completed",
                duration,
                Some(&json!({
                    "choiceCount": response.get("choices").and_then(Value::as_array).map_or(0, Vec::len)
                })),
                None,
            )
            .await?;
            persist_trace(
                &state,
                request_id,
                "completed",
                started_at,
                Some(&response),
                None,
            )
            .await;
            Ok(Json(json!({
                "output": response,
                "profileId": profile.id,
                "versionId": route.profile_version_id,
                "version": route.version_number,
                "providerKey": route.provider_key,
                "latencyMs": duration,
                "previewMode": preview_mode,
            })))
        }
        Err(error) => {
            let message = error.to_string();
            let duration = db::elapsed_millis(started_at);
            db::complete_trace_span(
                &state.pool,
                span_id,
                "failed",
                duration,
                None,
                Some(&message),
            )
            .await?;
            persist_trace(
                &state,
                request_id,
                "failed",
                started_at,
                None,
                Some(&message),
            )
            .await;
            Err(error)
        }
    }
}

async fn profile_detail_payload(
    state: &AppState,
    profile: crate::models::AgentProfile,
) -> Result<Value, ApiError> {
    let versions = db::list_profile_versions(&state.pool, profile.id).await?;
    let mut version_payloads = Vec::with_capacity(versions.len());
    for version in versions {
        version_payloads.push(profile_version_payload(state, version).await?);
    }
    Ok(json!({
        "profile": profile,
        "versions": version_payloads,
        "rollout": db::list_profile_rollout(&state.pool, profile.id).await?,
    }))
}

async fn profile_version_payload(
    state: &AppState,
    version: crate::models::AgentProfileVersion,
) -> Result<Value, ApiError> {
    let capabilities = db::list_profile_capabilities(&state.pool, version.id).await?;
    Ok(json!({ "version": version, "capabilities": capabilities }))
}

pub async fn list_bindings(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    deployment_admin(&state, &headers, Operation::DeploymentRead).await?;
    Ok(Json(
        json!({ "bindings": db::list_bindings(&state.pool).await? }),
    ))
}

pub async fn create_binding(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<CreateBinding>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    deployment_admin(&state, &headers, Operation::DeploymentWrite).await?;
    create_binding_record(&state, input).await
}

async fn create_binding_record(
    state: &AppState,
    input: CreateBinding,
) -> Result<(StatusCode, Json<Value>), ApiError> {
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
    deployment_admin(&state, &headers, Operation::DeploymentRead).await?;
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
    deployment_admin(&state, &headers, Operation::DeploymentWrite).await?;
    update_binding_record(&state, id, input).await
}

async fn update_binding_record(
    state: &AppState,
    id: Uuid,
    input: UpdateBinding,
) -> Result<Json<Value>, ApiError> {
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
    deployment_admin(&state, &headers, Operation::DeploymentWrite).await?;
    db::delete_binding(&state.pool, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_endpoints(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    deployment_admin(&state, &headers, Operation::DeploymentRead).await?;
    Ok(Json(
        json!({ "endpoints": db::list_endpoints(&state.pool).await? }),
    ))
}

pub async fn create_endpoint(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<CreateEndpoint>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    deployment_admin(&state, &headers, Operation::DeploymentWrite).await?;
    create_endpoint_record(&state, input).await
}

async fn create_endpoint_record(
    state: &AppState,
    input: CreateEndpoint,
) -> Result<(StatusCode, Json<Value>), ApiError> {
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
    deployment_admin(&state, &headers, Operation::DeploymentRead).await?;
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
    deployment_admin(&state, &headers, Operation::DeploymentWrite).await?;
    update_endpoint_record(&state, id, input).await
}

async fn update_endpoint_record(
    state: &AppState,
    id: Uuid,
    input: UpdateEndpoint,
) -> Result<Json<Value>, ApiError> {
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
    deployment_admin(&state, &headers, Operation::DeploymentWrite).await?;
    db::delete_endpoint(&state.pool, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_api_keys(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    deployment_admin(&state, &headers, Operation::DeploymentRead).await?;
    Ok(Json(
        json!({ "apiKeys": db::list_api_keys(&state.pool).await? }),
    ))
}

pub async fn create_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<CreateApiKey>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    deployment_admin(&state, &headers, Operation::DeploymentWrite).await?;
    create_api_key_record(&state, input).await
}

async fn create_api_key_record(
    state: &AppState,
    input: CreateApiKey,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    db::get_project(&state.pool, input.project_id).await?;
    let agent_scope = normalize_api_key_agent_scope(input.agent_scope)?;
    let name = input
        .name
        .as_deref()
        .map(|value| required_text("name", value, 128))
        .transpose()?
        .unwrap_or("Project key");
    let created = issue_api_key(
        state,
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
    deployment_admin(&state, &headers, Operation::DeploymentWrite).await?;
    update_api_key_record(&state, id, input, None).await
}

async fn update_api_key_record(
    state: &AppState,
    id: Uuid,
    input: UpdateApiKey,
    required_project_id: Option<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let current = db::get_api_key(&state.pool, id).await?;
    if required_project_id.is_some_and(|project_id| current.project_id != project_id) {
        return Err(ApiError::NotFound);
    }
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
        if required_project_id.is_some_and(|required| project_id != required) {
            return Err(ApiError::Invalid(
                "an API key cannot be moved through a project-scoped route".to_string(),
            ));
        }
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
    deployment_admin(&state, &headers, Operation::DeploymentWrite).await?;
    Ok(Json(
        json!({ "apiKey": db::revoke_api_key(&state.pool, id).await? }),
    ))
}

pub async fn delete_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    deployment_admin(&state, &headers, Operation::DeploymentWrite).await?;
    db::delete_api_key(&state.pool, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_agent_gateways(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    deployment_admin(&state, &headers, Operation::DeploymentRead).await?;
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
        None,
        &credential_prefix,
        &credential_hash,
    )
    .await?;
    Ok(agent_gateway_registration_response(
        gateway_id,
        registration,
    ))
}

pub async fn enroll_agent_gateway(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<RegisterAgentGateway>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let enrollment_token = bearer_token(&headers).ok_or(ApiError::Unauthorized)?;
    validate_agent_gateway_enrollment_token(enrollment_token)?;
    let gateway_id = required_identifier("agent gateway id", &input.gateway_id)?;
    let credential = validate_agent_gateway_credential(&input.credential)?;
    let credential_prefix = credential.chars().take(20).collect::<String>();
    let credential_hash = hash_agent_gateway_credential(credential, &state.config.api_key_pepper);
    let token_hash = hash_agent_gateway_enrollment(enrollment_token, &state.config.api_key_pepper);
    let registration = db::consume_agent_gateway_enrollment(
        &state.pool,
        &token_hash,
        gateway_id,
        &credential_prefix,
        &credential_hash,
    )
    .await?;
    Ok(agent_gateway_registration_response(
        gateway_id,
        registration,
    ))
}

pub async fn get_agent_gateway_runtime_config(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let gateway_id = authenticated_agent_gateway(&state, &headers).await?;
    let deployments = db::list_runtime_deployments_for_gateway(&state.pool, &gateway_id).await?;
    let mut configurations = Vec::with_capacity(deployments.len());
    for deployment in deployments {
        let project = db::get_project(&state.pool, deployment.project_id).await?;
        let release = if deployment.config_sync_enabled {
            match deployment.active_release_version {
                Some(version) => Some(
                    db::get_project_runtime_release(&state.pool, deployment.project_id, version)
                        .await?,
                ),
                None => None,
            }
        } else {
            None
        };
        configurations.push(json!({
            "deploymentId": deployment.id,
            "deployment": deployment.name,
            "projectId": project.project.id,
            "projectSlug": project.project.slug,
            "projectName": project.project.name,
            "isPrimary": deployment.is_primary,
            "policies": {
                "configSync": deployment.config_sync_enabled,
                "traceMode": deployment.trace_mode,
                "remoteInvocation": deployment.remote_invocation_enabled,
            },
            "release": release.map(|release| json!({
                "version": release.version,
                "contentHash": release.content_hash,
                "manifest": release.manifest,
            })),
        }));
    }
    Ok(Json(json!({
        "gatewayId": gateway_id,
        "deployments": configurations,
    })))
}

pub async fn upload_agent_gateway_runtime_traces(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<UploadRuntimeTraces>,
) -> Result<Json<Value>, ApiError> {
    let gateway_id = authenticated_agent_gateway(&state, &headers).await?;
    if input.traces.is_empty() || input.traces.len() > 100 {
        return Err(ApiError::Invalid(
            "runtime trace batches must contain between 1 and 100 records".to_string(),
        ));
    }
    let deployment = db::list_runtime_deployments_for_gateway(&state.pool, &gateway_id)
        .await?
        .into_iter()
        .find(|deployment| deployment.id == input.deployment_id)
        .ok_or(ApiError::Forbidden)?;
    if deployment.trace_mode == "off" {
        return Err(ApiError::Forbidden);
    }
    let project = db::get_project(&state.pool, deployment.project_id).await?;
    let mut accepted = Vec::with_capacity(input.traces.len());
    for trace in input.traces {
        trace
            .validate()
            .map_err(|error| ApiError::Invalid(error.to_string()))?;
        if trace.project_id != project.project.slug {
            return Err(ApiError::Forbidden);
        }
        let created_at_ms = i64::try_from(trace.created_at_ms)
            .map_err(|_| ApiError::Invalid("runtime trace timestamp is invalid".to_string()))?;
        let created_at = chrono::DateTime::<Utc>::from_timestamp_millis(created_at_ms)
            .ok_or_else(|| ApiError::Invalid("runtime trace timestamp is invalid".to_string()))?;
        let latency_ms = i64::try_from(trace.duration_ms)
            .map_err(|_| ApiError::Invalid("runtime trace duration is invalid".to_string()))?;
        let request = json!({
            "source": "embedded-runtime",
            "gatewayId": gateway_id,
            "deploymentId": deployment.id,
            "traceId": trace.id,
            "invocationId": trace.invocation_id,
            "endpoint": trace.endpoint,
            "agent": trace.agent,
        });
        let request_id = runtime_trace_uuid("request", &gateway_id, &trace.id);
        db::create_uploaded_runtime_trace(
            &state.pool,
            db::NewUploadedRuntimeTrace {
                id: runtime_trace_uuid("trace", &gateway_id, &trace.id),
                request_id,
                project_id: project.project.id,
                operation: "runtime.invoke",
                provider_key: trace.provider.as_deref(),
                capability_kind: trace.capability.as_deref(),
                status: &trace.status,
                latency_ms,
                request: &request,
                created_at,
            },
        )
        .await?;
        accepted.push(trace.id);
    }
    Ok(Json(json!({
        "acceptedTraceIds": accepted,
    })))
}

pub async fn bootstrap_agent_gateway_runtime_release(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<BootstrapGatewayRuntimeRelease>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let gateway_id = authenticated_agent_gateway(&state, &headers).await?;
    let deployment = db::list_runtime_deployments_for_gateway(&state.pool, &gateway_id)
        .await?
        .into_iter()
        .find(|deployment| deployment.id == input.deployment_id)
        .ok_or(ApiError::Forbidden)?;
    if !deployment.config_sync_enabled {
        return Err(ApiError::Forbidden);
    }
    let project = db::get_project(&state.pool, deployment.project_id).await?;
    let manifest = serde_json::from_value::<ProjectSettings>(input.settings)
        .map_err(|error| ApiError::Invalid(format!("project settings are invalid: {error}")))?;
    manifest
        .validate()
        .map_err(|error| ApiError::Invalid(error.to_string()))?;
    if manifest.project_id != project.project.slug {
        return Err(ApiError::Invalid(
            "project settings projectId must match the project slug".to_string(),
        ));
    }
    let content_hash = manifest
        .content_hash()
        .map_err(|error| ApiError::Invalid(error.to_string()))?;
    let releases = db::list_project_runtime_releases(&state.pool, project.project.id).await?;
    if let Some(existing) = releases
        .iter()
        .find(|release| release.content_hash == content_hash)
    {
        if deployment.active_release_version.is_none() {
            let activated = db::activate_runtime_deployment_release(
                &state.pool,
                project.project.id,
                &deployment.name,
                existing.version,
            )
            .await?;
            notify_runtime_deployments(&state, std::slice::from_ref(&activated)).await?;
        } else if deployment.active_release_version != Some(existing.version) {
            return Err(ApiError::Conflict(
                "the deployment already uses another runtime release".to_string(),
            ));
        }
        return Ok((StatusCode::OK, Json(json!({ "release": existing }))));
    }
    if !releases.is_empty() || deployment.active_release_version.is_some() {
        return Err(ApiError::Conflict(
            "only an empty deployment can import its first runtime release".to_string(),
        ));
    }

    let release =
        RuntimeRelease::new(1, manifest).map_err(|error| ApiError::Invalid(error.to_string()))?;
    let manifest = serde_json::to_value(&release.manifest).map_err(|_| ApiError::Internal)?;
    let created = db::create_project_runtime_release(
        &state.pool,
        db::NewProjectRuntimeRelease {
            id: Uuid::new_v4(),
            project_id: project.project.id,
            version: 1,
            content_hash: &release.content_hash,
            manifest: &manifest,
            created_by: Some(&gateway_id),
        },
    )
    .await?;
    let activated = db::activate_runtime_deployment_release(
        &state.pool,
        project.project.id,
        &deployment.name,
        created.version,
    )
    .await?;
    notify_runtime_deployments(&state, std::slice::from_ref(&activated)).await?;
    Ok((StatusCode::CREATED, Json(json!({ "release": created }))))
}

fn agent_gateway_registration_response(
    gateway_id: &str,
    registration: db::AgentGatewayRegistration,
) -> (StatusCode, Json<Value>) {
    let status = match registration {
        db::AgentGatewayRegistration::Registered => "registered",
        db::AgentGatewayRegistration::Existing => "existing",
    };
    let status_code = if registration == db::AgentGatewayRegistration::Registered {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    (
        status_code,
        Json(json!({ "gatewayId": gateway_id, "status": status })),
    )
}

pub async fn create_project_agent_gateway_enrollment(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let project = db::get_project_by_slug(&state.pool, &slug).await?;
    let deployment = db::list_runtime_deployments(&state.pool, project.project.id)
        .await?
        .into_iter()
        .find(|deployment| deployment.is_primary)
        .ok_or(ApiError::NotFound)?;
    create_agent_gateway_enrollment_for_deployment(&state, &headers, project, deployment).await
}

pub async fn create_runtime_deployment_agent_gateway_enrollment(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((slug, deployment)): Path<(String, String)>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let project = db::get_project_by_slug(&state.pool, &slug).await?;
    let deployment = validate_explicit_slug(&deployment)?;
    let deployment =
        db::get_runtime_deployment(&state.pool, project.project.id, &deployment).await?;
    create_agent_gateway_enrollment_for_deployment(&state, &headers, project, deployment).await
}

pub async fn assign_runtime_deployment_agent_gateway(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((slug, deployment, gateway_id)): Path<(String, String, String)>,
) -> Result<Json<Value>, ApiError> {
    let project = db::get_project_by_slug(&state.pool, &slug).await?;
    let identity = state
        .auth
        .authorize_project(
            &headers,
            Operation::ProjectWrite,
            project.project.owner_user_id.as_deref(),
        )
        .await?;
    let deployment_name = validate_explicit_slug(&deployment)?;
    let deployment =
        db::get_runtime_deployment(&state.pool, project.project.id, &deployment_name).await?;
    authorize_gateway_owner(&state, &identity, &gateway_id).await?;
    db::assign_runtime_deployment_gateway(
        &state.pool,
        project.project.id,
        deployment.id,
        &gateway_id,
    )
    .await?;
    Ok(Json(json!({ "assigned": true, "gatewayId": gateway_id })))
}

pub async fn unassign_runtime_deployment_agent_gateway(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((slug, deployment, gateway_id)): Path<(String, String, String)>,
) -> Result<StatusCode, ApiError> {
    let project = db::get_project_by_slug(&state.pool, &slug).await?;
    let identity = state
        .auth
        .authorize_project(
            &headers,
            Operation::ProjectWrite,
            project.project.owner_user_id.as_deref(),
        )
        .await?;
    let deployment_name = validate_explicit_slug(&deployment)?;
    let deployment =
        db::get_runtime_deployment(&state.pool, project.project.id, &deployment_name).await?;
    authorize_gateway_owner(&state, &identity, &gateway_id).await?;
    db::unassign_runtime_deployment_gateway(
        &state.pool,
        project.project.id,
        deployment.id,
        &gateway_id,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn authorize_gateway_owner(
    state: &AppState,
    identity: &Identity,
    gateway_id: &str,
) -> Result<(), ApiError> {
    let authorization = db::get_agent_gateway_authorization(&state.pool, gateway_id).await?;
    if authorization.status != "active" {
        return Err(ApiError::Forbidden);
    }
    match identity {
        Identity::DeploymentAdmin => Ok(()),
        Identity::ActingUser { subject, .. }
            if authorization.owner_user_id.as_deref() == Some(subject.as_str()) =>
        {
            Ok(())
        }
        Identity::ActingUser { .. } => Err(ApiError::Forbidden),
    }
}

async fn create_agent_gateway_enrollment_for_deployment(
    state: &AppState,
    headers: &HeaderMap,
    project: crate::models::ProjectWithBindings,
    deployment: RuntimeDeployment,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let identity = state
        .auth
        .authorize_project(
            headers,
            Operation::ProjectWrite,
            project.project.owner_user_id.as_deref(),
        )
        .await?;
    let owner_user_id = match identity {
        Identity::ActingUser { subject, .. } => subject,
        Identity::DeploymentAdmin => project
            .project
            .owner_user_id
            .clone()
            .unwrap_or_else(|| "deployment-admin".to_string()),
    };
    let token = format!(
        "vifu_ge_{}{}",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    );
    let expires_at = Utc::now() + ChronoDuration::minutes(5);
    let token_hash = hash_agent_gateway_enrollment(&token, &state.config.api_key_pepper);
    db::create_agent_gateway_enrollment(
        &state.pool,
        db::NewAgentGatewayEnrollment {
            id: Uuid::new_v4(),
            project_id: project.project.id,
            owner_user_id: &owner_user_id,
            deployment_id: deployment.id,
            token_hash: &token_hash,
            expires_at,
        },
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "enrollmentToken": token,
            "expiresAt": expires_at,
            "deployment": deployment.name,
        })),
    ))
}

pub async fn revoke_agent_gateway(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(gateway_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let gateway_id = required_identifier("agent gateway id", &gateway_id)?;
    let identity = deployment_identity(&state, &headers, Operation::DeploymentWrite).await?;
    let current = db::get_agent_gateway_authorization(&state.pool, gateway_id).await?;
    if let Identity::ActingUser { subject, .. } = identity {
        if current.owner_user_id.as_deref() != Some(subject.as_str()) {
            return Err(ApiError::Forbidden);
        }
    }
    let authorization = db::revoke_agent_gateway_authorization(&state.pool, gateway_id).await?;
    state
        .relay
        .disconnect(gateway_id, "CREDENTIAL_REVOKED")
        .await;
    Ok(Json(json!({ "agentGatewayAuthorization": authorization })))
}

pub async fn get_agent_gateway_pairing(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    deployment_identity(&state, &headers, Operation::DeploymentRead).await?;
    let pairing = db::get_agent_gateway_pairing(&state.pool, id).await?;
    Ok(Json(json!({ "pairing": pairing })))
}

pub async fn list_agent_gateway_pairings(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    deployment_admin(&state, &headers, Operation::DeploymentRead).await?;
    Ok(Json(json!({
        "pairings": db::list_agent_gateway_pairings(&state.pool).await?
    })))
}

pub async fn approve_agent_gateway_pairing(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let identity = deployment_identity(&state, &headers, Operation::ProjectWrite).await?;
    let request = db::get_agent_gateway_pairing(&state.pool, id).await?;
    let authorization =
        db::get_agent_gateway_authorization_for_machine(&state.pool, &request.machine_id).await?;
    let owner_user_id = match identity {
        Identity::ActingUser { subject, .. } => {
            if authorization
                .as_ref()
                .and_then(|value| value.owner_user_id.as_deref())
                .is_some_and(|owner| owner != subject)
            {
                return Err(ApiError::Forbidden);
            }
            Some(subject)
        }
        Identity::DeploymentAdmin => authorization.and_then(|value| value.owner_user_id),
    };
    let pairing =
        db::resolve_agent_gateway_pairing(&state.pool, id, "approved", owner_user_id.as_deref())
            .await?;
    Ok(Json(json!({ "pairing": pairing })))
}

pub async fn reject_agent_gateway_pairing(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let identity = deployment_identity(&state, &headers, Operation::ProjectWrite).await?;
    let request = db::get_agent_gateway_pairing(&state.pool, id).await?;
    if let Identity::ActingUser { subject, .. } = identity {
        let authorization =
            db::get_agent_gateway_authorization_for_machine(&state.pool, &request.machine_id)
                .await?;
        if authorization
            .as_ref()
            .and_then(|value| value.owner_user_id.as_deref())
            .is_some_and(|owner| owner != subject)
        {
            return Err(ApiError::Forbidden);
        }
    }
    let pairing = db::resolve_agent_gateway_pairing(&state.pool, id, "rejected", None).await?;
    Ok(Json(json!({ "pairing": pairing })))
}

pub async fn list_available_agents(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    deployment_admin(&state, &headers, Operation::DeploymentRead).await?;
    Ok(Json(json!({
        "agents": db::list_available_agents(&state.pool).await?
    })))
}

pub async fn list_provider_adapters(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    deployment_read(&state, &headers).await?;
    Ok(Json(json!({ "providerAdapters": provider_adapters() })))
}

pub async fn list_provider_catalog(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    deployment_admin(&state, &headers, Operation::DeploymentRead).await?;
    Ok(Json(
        json!({ "registry": provider_adapters(), "custom": available_provider_catalog(&state, None).await? }),
    ))
}

pub async fn list_project_provider_catalog(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let project = authorized_project_by_slug(&state, &headers, &slug, ProjectAccess::Read).await?;
    Ok(Json(json!({
        "registry": provider_adapters(),
        "custom": available_provider_catalog(&state, Some(&project.project.gateway_id)).await?,
    })))
}

pub async fn list_project_provider_adapters(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Result<Json<Value>, ApiError> {
    authorized_project_by_slug(&state, &headers, &slug, ProjectAccess::Read).await?;
    Ok(Json(json!({ "providerAdapters": provider_adapters() })))
}

pub async fn list_project_providers(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Result<Json<Value>, ApiError> {
    authorized_project_by_slug(&state, &headers, &slug, ProjectAccess::Read).await?;
    let mut providers = Vec::new();
    for connection in db::list_provider_connections(&state.pool, &slug).await? {
        providers.push(effective_provider_connection(&state, connection).await?);
    }
    Ok(Json(json!({ "providers": providers })))
}

pub async fn create_project_provider(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(input): Json<CreateProjectProvider>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    deployment_credential(&headers).ok_or(ApiError::Unauthorized)?;
    let project = db::get_project_by_slug(&state.pool, &slug).await?;
    let identity = state
        .auth
        .authorize_project(
            &headers,
            Operation::ProjectWrite,
            project.project.owner_user_id.as_deref(),
        )
        .await?;
    if input.source.kind == "custom" && matches!(identity, Identity::ActingUser { .. }) {
        return Err(ApiError::Forbidden);
    }
    let source = resolve_project_provider_source(
        &state,
        &input.source.kind,
        &input.source.key,
        Some(&project.project.gateway_id),
    )
    .await?;
    let provider_key = if source.kind == "registry" {
        unique_project_provider_key(&state, &slug, &source.key).await?
    } else {
        match db::get_provider_connection_secret_by_key(&state.pool, &slug, &source.key).await {
            Ok(_) => {
                return Err(ApiError::Conflict(
                    "this provider is already available in the project".to_string(),
                ));
            }
            Err(ApiError::NotFound) => source.key.clone(),
            Err(error) => return Err(error),
        }
    };
    let prepared = if source.kind == "registry" {
        prepare_project_provider(
            &state,
            &provider_key,
            input.name.as_deref().unwrap_or(&source.name),
            &source,
            input.base_url.as_deref(),
            input.config,
            input.secrets,
        )?
    } else {
        prepare_project_provider_assignment(
            &state,
            &provider_key,
            input.name.as_deref().unwrap_or(&source.name),
            &source.provider_type,
        )?
    };
    let connection =
        save_provider_connection(&state, &slug, &source.kind, &source.key, prepared).await?;
    let (provider, message, added_agents) =
        refresh_project_provider(&state, &slug, connection).await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "provider": provider,
            "message": message,
            "addedAgents": added_agents,
        })),
    ))
}

pub async fn import_project_provider(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(input): Json<ImportProjectProvider>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    authorized_project_by_slug(&state, &headers, &slug, ProjectAccess::Write).await?;
    let provider_key = required_identifier("provider key", &input.provider_key)?;
    let provider_type = required_identifier("provider type", &input.provider_type)?;
    let name = required_text("provider name", &input.name, 128)?;
    let source = resolve_project_provider_source(&state, "registry", provider_type, None).await?;
    let prepared = prepare_project_provider(
        &state,
        provider_key,
        name,
        &source,
        Some(&input.base_url),
        input.config,
        input.secrets,
    )?;
    let connection =
        save_provider_connection(&state, &slug, "registry", provider_type, prepared).await?;
    let (provider, message, added_agents) =
        refresh_project_provider(&state, &slug, connection).await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "provider": provider,
            "message": message,
            "addedAgents": added_agents,
        })),
    ))
}

pub async fn update_project_provider(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((slug, provider_key)): Path<(String, String)>,
    Json(input): Json<UpdateProjectProvider>,
) -> Result<Json<Value>, ApiError> {
    let project = authorized_project_by_slug(&state, &headers, &slug, ProjectAccess::Write).await?;
    let provider_key = required_identifier("provider key", &provider_key)?;
    let current =
        db::get_provider_connection_secret_by_key(&state.pool, &slug, provider_key).await?;
    let source = resolve_project_provider_source(
        &state,
        &current.source_kind,
        &current.source_key,
        Some(&project.project.gateway_id),
    )
    .await?;
    let prepared = if current.source_kind == "custom" {
        prepare_project_provider_assignment(
            &state,
            provider_key,
            input.name.as_deref().unwrap_or(&current.name),
            &source.provider_type,
        )?
    } else {
        let current_secrets = decrypted_provider_secrets(&state, &current)?;
        let secrets = match input.secrets {
            Some(secrets) if !is_json_object_empty(&secrets) => {
                merge_json_objects(&current_secrets, &secrets)?
            }
            Some(_) | None => current_secrets,
        };
        let config = match input.config {
            Some(config) => merge_json_objects(&current.config, &config)?,
            None => current.config.clone(),
        };
        prepare_project_provider(
            &state,
            provider_key,
            input.name.as_deref().unwrap_or(&current.name),
            &source,
            input.base_url.as_deref().or(Some(&current.base_url)),
            config,
            secrets,
        )?
    };
    let connection = save_provider_connection(
        &state,
        &slug,
        &current.source_kind,
        &current.source_key,
        prepared,
    )
    .await?;
    let (provider, message, added_agents) =
        refresh_project_provider(&state, &slug, connection).await?;
    Ok(Json(
        json!({ "provider": provider, "message": message, "addedAgents": added_agents }),
    ))
}

pub async fn delete_project_provider(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((slug, provider_key)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let project = authorized_project_by_slug(&state, &headers, &slug, ProjectAccess::Write).await?;
    let provider_key = required_identifier("provider key", &provider_key)?;
    db::unassign_project_provider(&state.pool, project.project.id, provider_key).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn test_project_provider(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((slug, provider_key)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    authorized_project_by_slug(&state, &headers, &slug, ProjectAccess::Write).await?;
    let connection =
        db::get_provider_connection_secret_by_key(&state.pool, &slug, &provider_key).await?;
    let (provider, message, added_agents) =
        refresh_project_provider(&state, &slug, connection.into()).await?;
    Ok(Json(
        json!({ "provider": provider, "message": message, "addedAgents": added_agents }),
    ))
}

pub async fn list_project_agent_candidates(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let project = authorized_project_by_slug(&state, &headers, &slug, ProjectAccess::Read).await?;
    let mut available_provider_keys = db::list_provider_connections(&state.pool, &slug)
        .await?
        .into_iter()
        .map(|provider| provider.provider_key)
        .collect::<HashSet<_>>();
    let available_agents = db::list_available_agents(&state.pool).await?;
    available_provider_keys.extend(
        available_agents
            .iter()
            .filter(|agent| agent.gateway_id == project.project.gateway_id)
            .filter_map(|agent| {
                agent
                    .metadata
                    .get("providerKey")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            }),
    );
    let imported = db::list_project_profile_provider_resources(&state.pool, project.project.id)
        .await?
        .into_iter()
        .collect::<HashSet<_>>();
    let archived = db::list_archived_project_agent_sources(&state.pool, project.project.id).await?;
    let archived_resources = archived
        .iter()
        .map(|agent| (agent.provider_key.clone(), agent.agent_id.clone()))
        .collect::<HashSet<_>>();
    let mut candidates = archived
        .into_iter()
        .filter(|agent| available_provider_keys.contains(&agent.provider_key))
        .map(|agent| {
            json!({
                "profileId": agent.profile_id,
                "gatewayId": agent.gateway_id,
                "id": agent.agent_id,
                "name": agent.name,
                "status": "removed",
                "providerKey": agent.provider_key,
                "providerType": agent.provider_type,
                "metadata": {},
            })
        })
        .collect::<Vec<_>>();
    candidates.extend(available_agents.into_iter().filter_map(|agent| {
        if agent.gateway_id != project.project.gateway_id {
            return None;
        }
        let provider_key = agent
            .metadata
            .get("providerKey")
            .and_then(Value::as_str)?
            .to_string();
        if imported.contains(&(provider_key.clone(), agent.id.clone()))
            || archived_resources.contains(&(provider_key.clone(), agent.id.clone()))
        {
            return None;
        }
        let provider_type = agent
            .metadata
            .get("providerType")
            .and_then(Value::as_str)
            .unwrap_or("openclaw");
        Some(json!({
            "profileId": null,
            "gatewayId": agent.gateway_id,
            "id": agent.id,
            "name": agent.name,
            "status": agent.status,
            "providerKey": provider_key,
            "providerType": provider_type,
            "metadata": agent.metadata,
        }))
    }));
    Ok(Json(json!({ "candidates": candidates })))
}

pub async fn import_project_agent(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(input): Json<ImportProjectAgent>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let project = authorized_project_by_slug(&state, &headers, &slug, ProjectAccess::Write).await?;
    let gateway_id = required_identifier("agent gateway id", &input.gateway_id)?;
    let agent_id = required_identifier("agent id", &input.agent_id)?;
    let provider_key = required_identifier("provider key", &input.provider_key)?;
    if gateway_id != project.project.gateway_id {
        return Err(ApiError::Forbidden);
    }
    let agent = db::list_available_agents(&state.pool)
        .await?
        .into_iter()
        .find(|agent| {
            agent.gateway_id == gateway_id
                && agent.id == agent_id
                && agent.metadata.get("providerKey").and_then(Value::as_str) == Some(provider_key)
        })
        .ok_or(ApiError::NotFound)?;
    let provider_type = agent
        .metadata
        .get("providerType")
        .and_then(Value::as_str)
        .unwrap_or("openclaw");
    let profile = if let Some((profile_id, archived, binding_id)) =
        db::find_project_profile_by_provider_resource(
            &state.pool,
            project.project.id,
            provider_key,
            agent_id,
        )
        .await?
    {
        db::refresh_discovered_binding(&state.pool, binding_id, gateway_id, &agent.name).await?;
        if archived {
            db::restore_project_profile(&state.pool, project.project.id, profile_id).await?
        } else {
            db::assign_project_binding(&state.pool, project.project.id, binding_id).await?;
            db::get_project_profile(&state.pool, project.project.id, profile_id).await?
        }
    } else {
        let binding_id = db::ensure_discovered_binding(
            &state.pool,
            project.project.id,
            gateway_id,
            agent_id,
            &agent.name,
            provider_key,
            provider_type,
        )
        .await?;
        db::assign_project_binding(&state.pool, project.project.id, binding_id).await?;
        let binding = db::get_binding(&state.pool, binding_id).await?;
        db::get_project_profile(&state.pool, project.project.id, binding.profile_id).await?
    };
    Ok((
        StatusCode::CREATED,
        Json(profile_detail_payload(&state, profile).await?),
    ))
}

pub async fn restore_project_agent(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((slug, profile_id)): Path<(String, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    let project = authorized_project_by_slug(&state, &headers, &slug, ProjectAccess::Write).await?;
    let profile = db::restore_project_profile(&state.pool, project.project.id, profile_id).await?;
    Ok(Json(profile_detail_payload(&state, profile).await?))
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
    deployment_admin(&state, &headers, Operation::DeploymentRead).await?;
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

pub async fn list_trace_spans(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(trace_id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    deployment_admin(&state, &headers, Operation::DeploymentRead).await?;
    Ok(Json(json!({
        "spans": db::list_trace_spans(&state.pool, trace_id).await?
    })))
}

pub async fn list_project_bindings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let project = authorized_project_by_slug(&state, &headers, &slug, ProjectAccess::Read).await?;
    let profile_ids = db::list_project_profiles(&state.pool, project.project.id)
        .await?
        .into_iter()
        .map(|profile| profile.id)
        .collect::<HashSet<_>>();
    let bindings = db::list_bindings(&state.pool)
        .await?
        .into_iter()
        .filter(|binding| profile_ids.contains(&binding.profile_id))
        .collect::<Vec<_>>();
    Ok(Json(json!({ "bindings": bindings })))
}

pub async fn create_project_binding(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(input): Json<CreateBinding>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let project = authorized_project_by_slug(&state, &headers, &slug, ProjectAccess::Write).await?;
    db::get_project_profile(&state.pool, project.project.id, input.profile_id).await?;
    create_binding_record(&state, input).await
}

pub async fn get_project_binding(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((slug, id)): Path<(String, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    let project = authorized_project_by_slug(&state, &headers, &slug, ProjectAccess::Read).await?;
    let binding = project_binding(&state, project.project.id, id).await?;
    Ok(Json(json!({ "binding": binding })))
}

pub async fn update_project_binding(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((slug, id)): Path<(String, Uuid)>,
    Json(input): Json<UpdateBinding>,
) -> Result<Json<Value>, ApiError> {
    let project = authorized_project_by_slug(&state, &headers, &slug, ProjectAccess::Write).await?;
    project_binding(&state, project.project.id, id).await?;
    update_binding_record(&state, id, input).await
}

pub async fn delete_project_binding(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((slug, id)): Path<(String, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let project = authorized_project_by_slug(&state, &headers, &slug, ProjectAccess::Write).await?;
    project_binding(&state, project.project.id, id).await?;
    db::delete_binding(&state.pool, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_project_endpoints(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let project = authorized_project_by_slug(&state, &headers, &slug, ProjectAccess::Read).await?;
    let profile_ids = db::list_project_profiles(&state.pool, project.project.id)
        .await?
        .into_iter()
        .map(|profile| profile.id)
        .collect::<HashSet<_>>();
    let endpoints = db::list_endpoints(&state.pool)
        .await?
        .into_iter()
        .filter(|endpoint| profile_ids.contains(&endpoint.profile_id))
        .collect::<Vec<_>>();
    Ok(Json(json!({ "endpoints": endpoints })))
}

pub async fn create_project_endpoint(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(input): Json<CreateEndpoint>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let project = authorized_project_by_slug(&state, &headers, &slug, ProjectAccess::Write).await?;
    db::get_project_profile(&state.pool, project.project.id, input.profile_id).await?;
    project_binding(&state, project.project.id, input.binding_id).await?;
    create_endpoint_record(&state, input).await
}

pub async fn get_project_endpoint(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((slug, id)): Path<(String, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    let project = authorized_project_by_slug(&state, &headers, &slug, ProjectAccess::Read).await?;
    let endpoint = project_endpoint(&state, project.project.id, id).await?;
    Ok(Json(json!({ "endpoint": endpoint })))
}

pub async fn update_project_endpoint(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((slug, id)): Path<(String, Uuid)>,
    Json(input): Json<UpdateEndpoint>,
) -> Result<Json<Value>, ApiError> {
    let project = authorized_project_by_slug(&state, &headers, &slug, ProjectAccess::Write).await?;
    project_endpoint(&state, project.project.id, id).await?;
    if let Some(profile_id) = input.profile_id {
        db::get_project_profile(&state.pool, project.project.id, profile_id).await?;
    }
    if let Some(binding_id) = input.binding_id {
        project_binding(&state, project.project.id, binding_id).await?;
    }
    update_endpoint_record(&state, id, input).await
}

pub async fn delete_project_endpoint(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((slug, id)): Path<(String, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let project = authorized_project_by_slug(&state, &headers, &slug, ProjectAccess::Write).await?;
    project_endpoint(&state, project.project.id, id).await?;
    db::delete_endpoint(&state.pool, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_project_api_keys(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let project = authorized_project_by_slug(&state, &headers, &slug, ProjectAccess::Read).await?;
    let api_keys = db::list_api_keys(&state.pool)
        .await?
        .into_iter()
        .filter(|key| key.project_id == project.project.id)
        .collect::<Vec<_>>();
    Ok(Json(json!({ "apiKeys": api_keys })))
}

pub async fn create_project_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(mut input): Json<CreateApiKey>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let project = authorized_project_by_slug(&state, &headers, &slug, ProjectAccess::Write).await?;
    input.project_id = project.project.id;
    create_api_key_record(&state, input).await
}

pub async fn update_project_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((slug, id)): Path<(String, Uuid)>,
    Json(input): Json<UpdateApiKey>,
) -> Result<Json<Value>, ApiError> {
    let project = authorized_project_by_slug(&state, &headers, &slug, ProjectAccess::Write).await?;
    update_api_key_record(&state, id, input, Some(project.project.id)).await
}

pub async fn revoke_project_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((slug, id)): Path<(String, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    let project = authorized_project_by_slug(&state, &headers, &slug, ProjectAccess::Write).await?;
    let key = db::get_api_key(&state.pool, id).await?;
    if key.project_id != project.project.id {
        return Err(ApiError::NotFound);
    }
    Ok(Json(
        json!({ "apiKey": db::revoke_api_key(&state.pool, id).await? }),
    ))
}

pub async fn delete_project_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((slug, id)): Path<(String, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let project = authorized_project_by_slug(&state, &headers, &slug, ProjectAccess::Write).await?;
    let key = db::get_api_key(&state.pool, id).await?;
    if key.project_id != project.project.id {
        return Err(ApiError::NotFound);
    }
    db::delete_api_key(&state.pool, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_project_agent_gateways(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let project = authorized_project_by_slug(&state, &headers, &slug, ProjectAccess::Read).await?;
    let sessions = db::list_agent_gateway_sessions(&state.pool)
        .await?
        .into_iter()
        .filter(|session| session.gateway_id == project.project.gateway_id)
        .collect::<Vec<_>>();
    Ok(Json(json!({ "agentGateways": sessions })))
}

pub async fn list_project_available_agents(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let project = authorized_project_by_slug(&state, &headers, &slug, ProjectAccess::Read).await?;
    let agents = db::list_available_agents(&state.pool)
        .await?
        .into_iter()
        .filter(|agent| agent.gateway_id == project.project.gateway_id)
        .collect::<Vec<_>>();
    Ok(Json(json!({ "agents": agents })))
}

pub async fn list_project_traces(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Query(query): Query<TraceQuery>,
) -> Result<Json<Value>, ApiError> {
    let project = authorized_project_by_slug(&state, &headers, &slug, ProjectAccess::Read).await?;
    if query
        .project_id
        .is_some_and(|project_id| project_id != project.project.id)
    {
        return Err(ApiError::Invalid(
            "projectId does not match the project route".to_string(),
        ));
    }
    if let Some(endpoint_id) = query.endpoint_id {
        project_endpoint(&state, project.project.id, endpoint_id).await?;
    }
    let limit = query.limit.unwrap_or(100).clamp(1, 500);
    Ok(Json(json!({
        "traces": db::list_traces(
            &state.pool,
            query.endpoint_id,
            Some(project.project.id),
            limit,
        ).await?
    })))
}

pub async fn list_project_trace_spans(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((slug, trace_id)): Path<(String, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    let project = authorized_project_by_slug(&state, &headers, &slug, ProjectAccess::Read).await?;
    if db::get_trace_project_id(&state.pool, trace_id).await? != Some(project.project.id) {
        return Err(ApiError::NotFound);
    }
    Ok(Json(json!({
        "spans": db::list_trace_spans(&state.pool, trace_id).await?
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
    let allowed_profile_ids = match &authority {
        ApiRequestAuthority::Admin => None,
        ApiRequestAuthority::Key(key) => {
            if key.project_id != project.project.id {
                return Err(ApiError::AgentAccessDenied);
            }
            if !key.permissions.chat_completions_allowed() {
                return Err(ApiError::EndpointAccessDenied);
            }
            match &key.agent_scope {
                ApiKeyAgentScope::All => None,
                ApiKeyAgentScope::Selected { profile_ids } => Some(profile_ids.as_slice()),
            }
        }
    };
    let agents =
        db::list_public_agents(&state.pool, project.project.id, allowed_profile_ids).await?;
    Ok(Json(json!({
        "object": "list",
        "data": agents.into_iter().map(|agent| json!({
            "id": agent.slug,
            "object": "model",
            "owned_by": "vifu",
        })).collect::<Vec<_>>()
    })))
}

pub async fn list_project_agents(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_slug): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let authority = api_request_authority(&state, &headers).await?;
    let project = db::get_project_by_slug(&state.pool, &project_slug).await?;
    let allowed_profile_ids = match &authority {
        ApiRequestAuthority::Admin => None,
        ApiRequestAuthority::Key(key) => {
            if key.project_id != project.project.id {
                return Err(ApiError::AgentAccessDenied);
            }
            if key.permissions.agents == crate::models::ResourcePermission::None {
                return Err(ApiError::EndpointAccessDenied);
            }
            match &key.agent_scope {
                ApiKeyAgentScope::All => None,
                ApiKeyAgentScope::Selected { profile_ids } => Some(profile_ids.as_slice()),
            }
        }
    };
    Ok(Json(json!({
        "object": "list",
        "data": db::list_public_agents(
            &state.pool,
            project.project.id,
            allowed_profile_ids,
        ).await?
    })))
}

#[derive(Debug, Deserialize)]
pub struct SpeechRequest {
    model: String,
    input: String,
    voice: Option<String>,
    response_format: Option<String>,
    user: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RealtimeSessionRequest {
    model: String,
    user: Option<String>,
    expires_in_seconds: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct RealtimeQuery {
    token: String,
}

pub async fn create_project_speech(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_slug): Path<String>,
    Json(input): Json<SpeechRequest>,
) -> Result<Response, ApiError> {
    let model = required_text("model", &input.model, 128)?;
    let text = required_text("input", &input.input, 100_000)?;
    let (authority, project, route) = resolve_authorized_profile_route(
        &state,
        &headers,
        &project_slug,
        model,
        "speech",
        input.user.as_deref(),
        ProfileEndpointPermission::Speech,
    )
    .await?;
    let _authority = authority;
    if route.provider_type != "elevenlabs" {
        return Err(ApiError::Invalid(format!(
            "provider type {} does not support speech",
            route.provider_type
        )));
    }
    let provider = resolve_runtime_provider(&state, &project_slug, &route.provider_key).await?;
    if provider.provider_type != "elevenlabs" {
        return Err(ApiError::Invalid(
            "speech capability does not match its configured provider".to_string(),
        ));
    }
    let voice_id = route
        .resource_id
        .as_deref()
        .ok_or_else(|| ApiError::Invalid("speech capability is missing a voice ID".to_string()))?;
    let provider_request = json!({
        "text": text,
        "model_id": route.capability_config.get("modelId")
            .or_else(|| provider.config.get("modelId"))
            .and_then(Value::as_str)
            .unwrap_or("eleven_multilingual_v2"),
        "voice_settings": route.capability_config.get("voiceSettings")
            .cloned()
            .unwrap_or_else(|| json!({})),
    });
    let request_id = Uuid::new_v4();
    let request_summary = json!({
        "model": model,
        "characters": text.chars().count(),
        "voice": input.voice,
        "responseFormat": input.response_format,
    });
    let trace_id = db::create_trace(
        &state.pool,
        db::NewTrace {
            request_id,
            endpoint_id: None,
            project_id: Some(project.project.id),
            gateway_session_id: None,
            profile_id: Some(route.profile_id),
            profile_version_id: Some(route.profile_version_id),
            operation: "audio.speech",
            provider_key: Some(&route.provider_key),
            capability_kind: Some("speech"),
            selection_key: input.user.as_deref(),
            request: &request_summary,
        },
    )
    .await?;
    let span_id = db::create_trace_span(
        &state.pool,
        db::NewTraceSpan {
            trace_id,
            parent_span_id: None,
            name: "speech.synthesize",
            kind: "provider",
            provider_key: Some(&route.provider_key),
            capability_kind: Some("speech"),
            input_summary: Some(&request_summary),
            attributes: &json!({ "voiceId": voice_id }),
        },
    )
    .await?;
    let started_at = Instant::now();
    let timeout = profile_timeout(&route.runtime, state.config.request_timeout);
    let provider: Arc<dyn AgentProvider> = Arc::new(
        HttpCapabilityProvider::new("http-provider", provider.base_url, provider.token)
            .map_err(map_runtime_error)?
            .with_route(
                "speech",
                HttpCapabilityRoute::ElevenLabsSpeech {
                    voice_id: voice_id.to_string(),
                },
            )
            .map_err(map_runtime_error)?,
    );
    let result = invoke_registered_provider(
        RegisteredInvocation {
            project_id: &project_slug,
            agent_id: route.profile_id,
            agent_name: &route.profile_name,
            endpoint: &route.profile_slug,
            capability: "speech",
            timeout,
            request_id,
        },
        provider,
        InvocationData::Json(provider_request),
    )
    .await
    .and_then(|(data, metadata)| {
        let InvocationData::Binary(body) = data else {
            return Err(ApiError::Internal);
        };
        let content_type = metadata
            .get("contentType")
            .and_then(Value::as_str)
            .unwrap_or("application/octet-stream")
            .to_string();
        Ok((body, content_type))
    });
    match result {
        Ok((audio, content_type)) => {
            let response_summary = json!({
                "bytes": audio.len(),
                "contentType": content_type,
            });
            db::complete_trace_span(
                &state.pool,
                span_id,
                "completed",
                db::elapsed_millis(started_at),
                Some(&response_summary),
                None,
            )
            .await?;
            persist_trace(
                &state,
                request_id,
                "completed",
                started_at,
                Some(&response_summary),
                None,
            )
            .await;
            let mut response = Response::new(Body::from(audio));
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_str(&content_type)
                    .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
            );
            Ok(response)
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
            persist_trace(
                &state,
                request_id,
                api_error_trace_status(&error),
                started_at,
                None,
                Some(&message),
            )
            .await;
            Err(error)
        }
    }
}

pub async fn create_project_transcription(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_slug): Path<String>,
    mut multipart: Multipart,
) -> Result<Json<Value>, ApiError> {
    let authority = api_request_authority(&state, &headers).await?;
    let project = db::get_project_by_slug(&state.pool, &project_slug).await?;
    assert_endpoint_permission(
        &authority,
        project.project.id,
        ProfileEndpointPermission::Transcriptions,
    )?;
    let mut model = None;
    let mut language = None;
    let mut user = None;
    let mut audio = None;
    let mut file_name = "audio.wav".to_string();
    let mut content_type = "audio/wav".to_string();
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|error| ApiError::Invalid(format!("multipart body is invalid: {error}")))?
    {
        let name = field.name().unwrap_or_default().to_string();
        match name.as_str() {
            "file" => {
                if let Some(name) = field.file_name() {
                    file_name = name.chars().take(255).collect();
                }
                if let Some(value) = field.content_type() {
                    content_type = value.chars().take(128).collect();
                }
                let bytes = field.bytes().await.map_err(|error| {
                    ApiError::Invalid(format!("audio file could not be read: {error}"))
                })?;
                if bytes.len() > 25 * 1024 * 1024 {
                    return Err(ApiError::Invalid(
                        "audio file must not exceed 25 MiB".to_string(),
                    ));
                }
                audio = Some(bytes.to_vec());
            }
            "model" | "language" | "user" => {
                let value = field.text().await.map_err(|error| {
                    ApiError::Invalid(format!("multipart field could not be read: {error}"))
                })?;
                match name.as_str() {
                    "model" => model = Some(value),
                    "language" => language = Some(value),
                    "user" => user = Some(value),
                    _ => {}
                }
            }
            _ => {}
        }
    }
    let model = model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or(ApiError::ModelRequired)?;
    let audio = audio.ok_or_else(|| ApiError::Invalid("file is required".to_string()))?;
    let route = resolve_profile_route_for_authority(
        &state,
        &authority,
        &project,
        model,
        "transcription",
        user.as_deref(),
    )
    .await?;
    let request_summary = json!({
        "model": model,
        "bytes": audio.len(),
        "contentType": content_type,
        "language": language,
    });
    let request_id = Uuid::new_v4();
    let trace_id = db::create_trace(
        &state.pool,
        db::NewTrace {
            request_id,
            endpoint_id: None,
            project_id: Some(project.project.id),
            gateway_session_id: None,
            profile_id: Some(route.profile_id),
            profile_version_id: Some(route.profile_version_id),
            operation: "audio.transcriptions",
            provider_key: Some(&route.provider_key),
            capability_kind: Some("transcription"),
            selection_key: user.as_deref(),
            request: &request_summary,
        },
    )
    .await?;
    let span_id = db::create_trace_span(
        &state.pool,
        db::NewTraceSpan {
            trace_id,
            parent_span_id: None,
            name: "audio.transcribe",
            kind: "provider",
            provider_key: Some(&route.provider_key),
            capability_kind: Some("transcription"),
            input_summary: Some(&request_summary),
            attributes: &json!({}),
        },
    )
    .await?;
    let started_at = Instant::now();
    let result = transcribe_profile_audio(
        &state,
        project.project.id,
        &project_slug,
        &route,
        TranscriptionInvocation {
            audio,
            file_name: &file_name,
            content_type: &content_type,
            language: language.as_deref(),
        },
        request_id,
    )
    .await;
    match result {
        Ok(response) => {
            let output_summary = json!({
                "characters": response.get("text").and_then(Value::as_str).map_or(0, str::len)
            });
            db::complete_trace_span(
                &state.pool,
                span_id,
                "completed",
                db::elapsed_millis(started_at),
                Some(&output_summary),
                None,
            )
            .await?;
            persist_trace(
                &state,
                request_id,
                "completed",
                started_at,
                Some(&output_summary),
                None,
            )
            .await;
            Ok(Json(response))
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
            persist_trace(
                &state,
                request_id,
                "failed",
                started_at,
                None,
                Some(&message),
            )
            .await;
            Err(error)
        }
    }
}

pub async fn create_realtime_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_slug): Path<String>,
    Json(input): Json<RealtimeSessionRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let model = required_text("model", &input.model, 128)?;
    let (authority, project, route) = resolve_authorized_profile_route(
        &state,
        &headers,
        &project_slug,
        model,
        "realtime",
        input.user.as_deref(),
        ProfileEndpointPermission::Realtime,
    )
    .await?;
    let expires_in = input.expires_in_seconds.unwrap_or(300).clamp(60, 3600);
    let expires_at = Utc::now() + ChronoDuration::seconds(expires_in);
    let token = format!(
        "vifu_rt_{}{}",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    );
    let token_hash = hash_api_key(&token, &state.config.api_key_pepper);
    let api_key_id = match authority {
        ApiRequestAuthority::Admin => None,
        ApiRequestAuthority::Key(key) => Some(key.id),
    };
    let session = db::create_realtime_session(
        &state.pool,
        Uuid::new_v4(),
        project.project.id,
        route.profile_id,
        api_key_id,
        &token_hash,
        expires_at,
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": session.id,
            "object": "realtime.session",
            "model": route.profile_slug,
            "client_secret": {
                "value": token,
                "expires_at": session.expires_at.timestamp(),
            }
        })),
    ))
}

pub async fn connect_realtime(
    State(state): State<AppState>,
    Path(project_slug): Path<String>,
    Query(query): Query<RealtimeQuery>,
    upgrade: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let project = db::get_project_by_slug(&state.pool, &project_slug).await?;
    let token_hash = hash_api_key(&query.token, &state.config.api_key_pepper);
    let session =
        db::active_realtime_session_by_hash(&state.pool, project.project.id, &token_hash).await?;
    Ok(upgrade
        .on_upgrade(move |socket| run_realtime_socket(state, project_slug, session, socket))
        .into_response())
}

#[derive(Clone, Copy)]
enum ProfileEndpointPermission {
    Embeddings,
    Speech,
    Transcriptions,
    Realtime,
}

async fn resolve_authorized_profile_route(
    state: &AppState,
    headers: &HeaderMap,
    project_slug: &str,
    model: &str,
    capability: &str,
    selection_key: Option<&str>,
    permission: ProfileEndpointPermission,
) -> Result<
    (
        ApiRequestAuthority,
        crate::models::ProjectWithBindings,
        crate::models::ProfileRoute,
    ),
    ApiError,
> {
    let authority = api_request_authority(state, headers).await?;
    let project = db::get_project_by_slug(&state.pool, project_slug).await?;
    assert_endpoint_permission(&authority, project.project.id, permission)?;
    let route = resolve_profile_route_for_authority(
        state,
        &authority,
        &project,
        model,
        capability,
        selection_key,
    )
    .await?;
    Ok((authority, project, route))
}

fn assert_endpoint_permission(
    authority: &ApiRequestAuthority,
    project_id: Uuid,
    permission: ProfileEndpointPermission,
) -> Result<(), ApiError> {
    let ApiRequestAuthority::Key(key) = authority else {
        return Ok(());
    };
    if key.project_id != project_id {
        return Err(ApiError::AgentAccessDenied);
    }
    let allowed = match permission {
        ProfileEndpointPermission::Embeddings => key.permissions.embeddings_allowed(),
        ProfileEndpointPermission::Speech => key.permissions.speech_allowed(),
        ProfileEndpointPermission::Transcriptions => key.permissions.transcriptions_allowed(),
        ProfileEndpointPermission::Realtime => key.permissions.realtime_allowed(),
    };
    if allowed {
        Ok(())
    } else {
        Err(ApiError::EndpointAccessDenied)
    }
}

async fn resolve_profile_route_for_authority(
    state: &AppState,
    authority: &ApiRequestAuthority,
    project: &crate::models::ProjectWithBindings,
    model: &str,
    capability: &str,
    selection_key: Option<&str>,
) -> Result<crate::models::ProfileRoute, ApiError> {
    let route = match db::resolve_profile_route(
        &state.pool,
        project.project.id,
        model,
        capability,
        selection_key,
        None,
    )
    .await
    {
        Ok(route) => route,
        Err(ApiError::NotFound) if matches!(authority, ApiRequestAuthority::Key(_)) => {
            return Err(ApiError::AgentAccessDenied)
        }
        Err(error) => return Err(error),
    };
    if let ApiRequestAuthority::Key(key) = authority {
        if !key.agent_scope.allows(route.profile_id) {
            return Err(ApiError::AgentAccessDenied);
        }
    }
    Ok(route)
}

struct TranscriptionInvocation<'a> {
    audio: Vec<u8>,
    file_name: &'a str,
    content_type: &'a str,
    language: Option<&'a str>,
}

async fn transcribe_profile_audio(
    state: &AppState,
    project_id: Uuid,
    project_slug: &str,
    route: &crate::models::ProfileRoute,
    invocation: TranscriptionInvocation<'_>,
    request_id: Uuid,
) -> Result<Value, ApiError> {
    let TranscriptionInvocation {
        audio,
        file_name,
        content_type,
        language,
    } = invocation;
    match route.provider_type.as_str() {
        "local-whisper" => {
            if content_type != "audio/wav" && !file_name.to_ascii_lowercase().ends_with(".wav") {
                return Err(ApiError::Invalid(
                    "Local Whisper currently accepts WAV audio".to_string(),
                ));
            }
            let model = route.resource_id.as_deref().ok_or_else(|| {
                ApiError::Invalid("transcription capability is missing a model file".to_string())
            })?;
            let home_dir = vifu_gateway::config::default_home_dir().map_err(ApiError::Invalid)?;
            let model_path = vifu_gateway::providers::resolve_local_model_path(&home_dir, model)
                .map_err(ApiError::Invalid)?;
            if !model_path.is_file() {
                return Err(ApiError::Invalid(format!(
                    "Whisper model {} is not installed in ~/.vifu/models",
                    model
                )));
            }
            let language = language.map(str::to_string);
            let text = tokio::task::spawn_blocking(move || {
                vifu_gateway::providers::local_whisper_transcription(
                    &model_path,
                    &audio,
                    language.as_deref(),
                )
            })
            .await
            .map_err(|_| ApiError::Internal)?
            .map_err(ApiError::Provider)?;
            Ok(json!({ "text": text }))
        }
        "vifu-runtime" => {
            let gateway_id = profile_gateway_id(route).ok_or_else(|| {
                ApiError::Invalid(
                    "Gateway transcription capability is missing gatewayId".to_string(),
                )
            })?;
            if !db::runtime_deployment_allows_remote_invocation(
                &state.pool,
                project_id,
                &gateway_id,
            )
            .await?
            {
                return Err(ApiError::Forbidden);
            }
            let agent_id = route.resource_id.as_deref().ok_or_else(|| {
                ApiError::Invalid(
                    "Gateway transcription capability is missing resourceId".to_string(),
                )
            })?;
            let mut binding_config = gateway_binding_config(
                route.capability_config.clone(),
                &route.provider_key,
                "transcription",
                &route.persona,
            )?;
            if let Some(binding) = binding_config.as_object_mut() {
                binding.insert("fileName".to_string(), Value::String(file_name.to_string()));
                binding.insert(
                    "contentType".to_string(),
                    Value::String(content_type.to_string()),
                );
                if let Some(language) = language {
                    binding.insert("language".to_string(), Value::String(language.to_string()));
                }
            }
            let timeout = profile_timeout(&route.runtime, state.config.request_timeout);
            let endpoint_route = EndpointRoute {
                endpoint_id: route.capability_id,
                endpoint_slug: route.profile_slug.clone(),
                endpoint_name: route.profile_name.clone(),
                request_timeout_ms: i32::try_from(timeout.as_millis()).unwrap_or(i32::MAX),
                profile_id: route.profile_id,
                binding_id: route.capability_id,
                gateway_id,
                agent_id: agent_id.to_string(),
                binding_config,
            };
            let provider: Arc<dyn AgentProvider> = Arc::new(RelayAgentProvider::new(
                "agent-gateway",
                "transcription",
                state.relay.clone(),
                endpoint_route,
                request_id,
                timeout,
            ));
            let (output, _) = invoke_registered_provider(
                RegisteredInvocation {
                    project_id: project_slug,
                    agent_id: route.profile_id,
                    agent_name: &route.profile_name,
                    endpoint: &route.profile_slug,
                    capability: "transcription",
                    timeout,
                    request_id,
                },
                provider,
                InvocationData::Binary(audio),
            )
            .await?;
            match output {
                InvocationData::Json(output) => Ok(output),
                InvocationData::Binary(_) => Err(ApiError::Internal),
            }
        }
        "openai-compatible" => {
            let provider =
                resolve_runtime_provider(state, project_slug, &route.provider_key).await?;
            let model = route.resource_id.as_deref().ok_or_else(|| {
                ApiError::Invalid(
                    "transcription capability is missing a provider model".to_string(),
                )
            })?;
            let provider: Arc<dyn AgentProvider> = Arc::new(
                HttpCapabilityProvider::new("http-provider", provider.base_url, provider.token)
                    .map_err(map_runtime_error)?
                    .with_route(
                        "transcription",
                        HttpCapabilityRoute::OpenAiTranscription {
                            model: model.to_string(),
                            file_name: file_name.to_string(),
                            content_type: content_type.to_string(),
                        },
                    )
                    .map_err(map_runtime_error)?,
            );
            let timeout = profile_timeout(&route.runtime, state.config.request_timeout);
            let (output, _) = invoke_registered_provider(
                RegisteredInvocation {
                    project_id: project_slug,
                    agent_id: route.profile_id,
                    agent_name: &route.profile_name,
                    endpoint: &route.profile_slug,
                    capability: "transcription",
                    timeout,
                    request_id,
                },
                provider,
                InvocationData::Binary(audio),
            )
            .await?;
            match output {
                InvocationData::Json(output) => Ok(output),
                InvocationData::Binary(_) => Err(ApiError::Internal),
            }
        }
        provider => Err(ApiError::Invalid(format!(
            "provider type {provider} does not support transcription"
        ))),
    }
}

async fn run_realtime_socket(
    state: AppState,
    project_slug: String,
    session: crate::models::RealtimeSession,
    mut socket: WebSocket,
) {
    let session_id = session.id.to_string();
    if !send_realtime_event(
        &mut socket,
        json!({
            "type": "session.created",
            "event_id": realtime_event_id(),
            "session": {
                "id": session_id,
                "object": "realtime.session",
                "expires_at": session.expires_at.timestamp(),
                "modalities": ["text", "audio"],
            }
        }),
    )
    .await
    {
        return;
    }
    let mut messages = Vec::<Value>::new();
    let mut audio_buffer = Vec::<u8>::new();
    while let Some(message) = socket.next().await {
        let Ok(message) = message else {
            break;
        };
        let Message::Text(text) = message else {
            if matches!(message, Message::Close(_)) {
                break;
            }
            let _ = send_realtime_error(
                &mut socket,
                "invalid_event",
                "Realtime events must be JSON text messages.",
            )
            .await;
            continue;
        };
        let event = match serde_json::from_str::<Value>(&text) {
            Ok(event) if event.is_object() => event,
            _ => {
                let _ = send_realtime_error(
                    &mut socket,
                    "invalid_event",
                    "Realtime event is not valid JSON.",
                )
                .await;
                continue;
            }
        };
        match event.get("type").and_then(Value::as_str) {
            Some("session.update") => {
                let _ = send_realtime_event(
                    &mut socket,
                    json!({
                        "type": "session.updated",
                        "event_id": realtime_event_id(),
                        "session": event.get("session").cloned().unwrap_or_else(|| json!({})),
                    }),
                )
                .await;
            }
            Some("conversation.item.create") => match realtime_item_message(event.get("item")) {
                Some(message) => {
                    messages.push(message);
                    let _ = send_realtime_event(
                        &mut socket,
                        json!({
                            "type": "conversation.item.created",
                            "event_id": realtime_event_id(),
                            "item": event.get("item").cloned().unwrap_or_else(|| json!({})),
                        }),
                    )
                    .await;
                }
                None => {
                    let _ = send_realtime_error(
                        &mut socket,
                        "invalid_item",
                        "Conversation item must contain a text message.",
                    )
                    .await;
                }
            },
            Some("input_audio_buffer.append") => {
                let Some(encoded) = event.get("audio").and_then(Value::as_str) else {
                    let _ = send_realtime_error(
                        &mut socket,
                        "invalid_audio",
                        "input_audio_buffer.append requires audio.",
                    )
                    .await;
                    continue;
                };
                match base64::engine::general_purpose::STANDARD.decode(encoded) {
                    Ok(chunk)
                        if audio_buffer.len().saturating_add(chunk.len()) <= 25 * 1024 * 1024 =>
                    {
                        audio_buffer.extend_from_slice(&chunk);
                    }
                    _ => {
                        let _ = send_realtime_error(
                            &mut socket,
                            "invalid_audio",
                            "Audio buffer is invalid or exceeds 25 MiB.",
                        )
                        .await;
                    }
                }
            }
            Some("input_audio_buffer.clear") => {
                audio_buffer.clear();
                let _ = send_realtime_event(
                    &mut socket,
                    json!({
                        "type": "input_audio_buffer.cleared",
                        "event_id": realtime_event_id(),
                    }),
                )
                .await;
            }
            Some("input_audio_buffer.commit") => {
                if audio_buffer.is_empty() {
                    let _ = send_realtime_error(
                        &mut socket,
                        "empty_audio_buffer",
                        "Audio buffer is empty.",
                    )
                    .await;
                    continue;
                }
                let route = db::resolve_profile_route(
                    &state.pool,
                    session.project_id,
                    &session.profile_id.to_string(),
                    "transcription",
                    Some(&session_id),
                    None,
                )
                .await;
                match route {
                    Ok(route) => {
                        let audio = std::mem::take(&mut audio_buffer);
                        match transcribe_profile_audio(
                            &state,
                            session.project_id,
                            &project_slug,
                            &route,
                            TranscriptionInvocation {
                                audio,
                                file_name: "realtime.wav",
                                content_type: "audio/wav",
                                language: None,
                            },
                            Uuid::new_v4(),
                        )
                        .await
                        {
                            Ok(result) => {
                                let text = result
                                    .get("text")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                                    .to_string();
                                messages.push(json!({ "role": "user", "content": text }));
                                let _ = send_realtime_event(
                                    &mut socket,
                                    json!({
                                        "type": "conversation.item.input_audio_transcription.completed",
                                        "event_id": realtime_event_id(),
                                        "transcript": text,
                                    }),
                                )
                                .await;
                            }
                            Err(error) => {
                                let _ = send_realtime_error(
                                    &mut socket,
                                    "transcription_failed",
                                    &error.to_string(),
                                )
                                .await;
                            }
                        }
                    }
                    Err(_) => {
                        let _ = send_realtime_error(
                            &mut socket,
                            "transcription_unavailable",
                            "This profile does not have a transcription capability.",
                        )
                        .await;
                    }
                }
            }
            Some("response.create") => {
                match invoke_realtime_response(
                    &state,
                    &project_slug,
                    &session,
                    &session_id,
                    &messages,
                )
                .await
                {
                    Ok((response_id, text)) => {
                        messages.push(json!({ "role": "assistant", "content": text }));
                        if !send_realtime_event(
                            &mut socket,
                            json!({
                                "type": "response.created",
                                "event_id": realtime_event_id(),
                                "response": { "id": response_id, "status": "in_progress" },
                            }),
                        )
                        .await
                        {
                            break;
                        }
                        if !send_realtime_event(
                            &mut socket,
                            json!({
                                "type": "response.output_text.delta",
                                "event_id": realtime_event_id(),
                                "response_id": response_id,
                                "delta": text,
                            }),
                        )
                        .await
                        {
                            break;
                        }
                        let _ = send_realtime_event(
                            &mut socket,
                            json!({
                                "type": "response.output_text.done",
                                "event_id": realtime_event_id(),
                                "response_id": response_id,
                                "text": text,
                            }),
                        )
                        .await;
                        let _ = send_realtime_event(
                            &mut socket,
                            json!({
                                "type": "response.done",
                                "event_id": realtime_event_id(),
                                "response": { "id": response_id, "status": "completed" },
                            }),
                        )
                        .await;
                    }
                    Err(error) => {
                        let _ =
                            send_realtime_error(&mut socket, "response_failed", &error.to_string())
                                .await;
                    }
                }
            }
            Some("response.cancel") => {
                let _ = send_realtime_event(
                    &mut socket,
                    json!({
                        "type": "response.cancelled",
                        "event_id": realtime_event_id(),
                    }),
                )
                .await;
            }
            _ => {
                let _ = send_realtime_error(
                    &mut socket,
                    "unknown_event",
                    "Unsupported realtime event type.",
                )
                .await;
            }
        }
    }
}

async fn invoke_realtime_response(
    state: &AppState,
    project_slug: &str,
    session: &crate::models::RealtimeSession,
    selection_key: &str,
    messages: &[Value],
) -> Result<(String, String), ApiError> {
    if messages.is_empty() {
        return Err(ApiError::Invalid(
            "a conversation item is required before response.create".to_string(),
        ));
    }
    let route = db::resolve_profile_route(
        &state.pool,
        session.project_id,
        &session.profile_id.to_string(),
        "realtime",
        Some(selection_key),
        None,
    )
    .await?;
    let request_id = Uuid::new_v4();
    let request = json!({
        "model": route.profile_slug,
        "messages": messages,
        "stream": false,
        "user": selection_key,
    });
    let trace_id = db::create_trace(
        &state.pool,
        db::NewTrace {
            request_id,
            endpoint_id: None,
            project_id: Some(session.project_id),
            gateway_session_id: None,
            profile_id: Some(route.profile_id),
            profile_version_id: Some(route.profile_version_id),
            operation: "realtime.response",
            provider_key: Some(&route.provider_key),
            capability_kind: Some("realtime"),
            selection_key: Some(selection_key),
            request: &json!({ "messageCount": messages.len() }),
        },
    )
    .await?;
    let summary = json!({ "messageCount": messages.len() });
    let span_id = db::create_trace_span(
        &state.pool,
        db::NewTraceSpan {
            trace_id,
            parent_span_id: None,
            name: "realtime.response",
            kind: "provider",
            provider_key: Some(&route.provider_key),
            capability_kind: Some("realtime"),
            input_summary: Some(&summary),
            attributes: &json!({}),
        },
    )
    .await?;
    let started_at = Instant::now();
    let result = invoke_profile_chat(
        state,
        session.project_id,
        project_slug,
        &route,
        request_id,
        request,
        profile_timeout(&route.runtime, state.config.request_timeout),
    )
    .await;
    match result {
        Ok(response) => {
            let text = response
                .pointer("/choices/0/message/content")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| response.to_string());
            db::complete_trace_span(
                &state.pool,
                span_id,
                "completed",
                db::elapsed_millis(started_at),
                Some(&json!({ "characters": text.len() })),
                None,
            )
            .await?;
            persist_trace(
                state,
                request_id,
                "completed",
                started_at,
                Some(&json!({ "characters": text.len() })),
                None,
            )
            .await;
            Ok((format!("resp-{}", Uuid::new_v4().simple()), text))
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
            persist_trace(
                state,
                request_id,
                api_error_trace_status(&error),
                started_at,
                None,
                Some(&message),
            )
            .await;
            Err(error)
        }
    }
}

fn realtime_item_message(item: Option<&Value>) -> Option<Value> {
    let item = item?;
    let role = item.get("role")?.as_str()?;
    let content = item.get("content")?.as_array()?;
    let text = content
        .iter()
        .find_map(|part| {
            part.get("text")
                .or_else(|| part.get("transcript"))
                .and_then(Value::as_str)
        })?
        .trim();
    if text.is_empty() {
        None
    } else {
        Some(json!({ "role": role, "content": text }))
    }
}

async fn send_realtime_event(socket: &mut WebSocket, event: Value) -> bool {
    socket
        .send(Message::Text(event.to_string().into()))
        .await
        .is_ok()
}

async fn send_realtime_error(socket: &mut WebSocket, code: &str, message: &str) -> bool {
    send_realtime_event(
        socket,
        json!({
            "type": "error",
            "event_id": realtime_event_id(),
            "error": { "type": "invalid_request_error", "code": code, "message": message },
        }),
    )
    .await
}

fn realtime_event_id() -> String {
    format!("event-{}", Uuid::new_v4().simple())
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

pub async fn create_project_embeddings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_slug): Path<String>,
    Json(request): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    validate_embedding_request(&request)?;
    let model = request
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or(ApiError::ModelRequired)?;
    let selection_key = request
        .get("user")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let (_authority, project, route) = resolve_authorized_profile_route(
        &state,
        &headers,
        &project_slug,
        model,
        "embedding",
        selection_key,
        ProfileEndpointPermission::Embeddings,
    )
    .await?;
    let request_id = Uuid::new_v4();
    let gateway_id = profile_gateway_id(&route);
    let gateway_session_id = match gateway_id.as_deref() {
        Some(gateway_id) => state.relay.session_for(gateway_id).await,
        None => None,
    };
    let request_summary = embedding_request_summary(&request);
    let trace_id = db::create_trace(
        &state.pool,
        db::NewTrace {
            request_id,
            endpoint_id: None,
            project_id: Some(project.project.id),
            gateway_session_id,
            profile_id: Some(route.profile_id),
            profile_version_id: Some(route.profile_version_id),
            operation: "embeddings",
            provider_key: Some(&route.provider_key),
            capability_kind: Some("embedding"),
            selection_key,
            request: &request_summary,
        },
    )
    .await?;
    let span_id = db::create_trace_span(
        &state.pool,
        db::NewTraceSpan {
            trace_id,
            parent_span_id: None,
            name: "embedding.create",
            kind: "provider",
            provider_key: Some(&route.provider_key),
            capability_kind: Some("embedding"),
            input_summary: Some(&request_summary),
            attributes: &json!({
                "profileId": route.profile_id,
                "profileVersionId": route.profile_version_id,
                "version": route.version_number,
                "capabilityId": route.capability_id,
            }),
        },
    )
    .await?;
    let timeout = profile_timeout(&route.runtime, state.config.request_timeout);
    let started_at = Instant::now();
    let result = invoke_profile_embedding(
        &state,
        project.project.id,
        &project.project.slug,
        &route,
        request_id,
        request,
        timeout,
    )
    .await
    .and_then(|(output, metadata)| {
        embedding_response(&route.profile_slug, output).map(|response| (response, metadata))
    });
    match result {
        Ok((response, metadata)) => {
            let response_summary = embedding_response_summary(&response, &metadata);
            let duration = db::elapsed_millis(started_at);
            db::complete_trace_span(
                &state.pool,
                span_id,
                "completed",
                duration,
                Some(&response_summary),
                None,
            )
            .await?;
            persist_trace(
                &state,
                request_id,
                "completed",
                started_at,
                Some(&response_summary),
                None,
            )
            .await;
            Ok(Json(response))
        }
        Err(error) => {
            let message = error.to_string();
            let duration = db::elapsed_millis(started_at);
            db::complete_trace_span(
                &state.pool,
                span_id,
                "failed",
                duration,
                None,
                Some(&message),
            )
            .await?;
            persist_trace(
                &state,
                request_id,
                api_error_trace_status(&error),
                started_at,
                None,
                Some(&message),
            )
            .await;
            Err(error)
        }
    }
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
    if let Some(project) = project.as_ref() {
        return create_profile_chat_completion(
            &state,
            &authority,
            project,
            request,
            model.as_deref(),
        )
        .await;
    }
    let route = resolve_chat_route(&state, &authority, project.as_ref(), model.as_deref()).await?;
    let request_id = Uuid::new_v4();
    let gateway_session_id = state.relay.session_for(&route.gateway_id).await;
    let trace_request = chat_trace_request(&request);
    db::create_trace(
        &state.pool,
        db::NewTrace {
            request_id,
            endpoint_id: Some(route.endpoint_id),
            project_id: None,
            gateway_session_id,
            profile_id: Some(route.profile_id),
            profile_version_id: None,
            operation: "chat.completions",
            provider_key: Some("agent-gateway"),
            capability_kind: Some("chat"),
            selection_key: None,
            request: &trace_request,
        },
    )
    .await?;

    let timeout = Duration::from_millis(
        u64::try_from(route.request_timeout_ms)
            .unwrap_or(30_000)
            .min(state.config.request_timeout.as_millis() as u64),
    );
    let started_at = Instant::now();
    match invoke_endpoint_chat(&state, &route, request_id, request, timeout).await {
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
            let message = error.to_string();
            persist_trace(
                &state,
                request_id,
                api_error_trace_status(&error),
                started_at,
                None,
                Some(&message),
            )
            .await;
            Err(error)
        }
    }
}

async fn create_profile_chat_completion(
    state: &AppState,
    authority: &ApiRequestAuthority,
    project: &crate::models::ProjectWithBindings,
    request: Value,
    model: Option<&str>,
) -> Result<Json<Value>, ApiError> {
    let model = model.ok_or(ApiError::ModelRequired)?;
    if let ApiRequestAuthority::Key(key) = authority {
        if key.project_id != project.project.id {
            return Err(ApiError::AgentAccessDenied);
        }
        if !key.permissions.chat_completions_allowed() {
            return Err(ApiError::EndpointAccessDenied);
        }
    }
    let selection_key = request
        .get("user")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let route = match db::resolve_profile_route(
        &state.pool,
        project.project.id,
        model,
        "chat",
        selection_key,
        None,
    )
    .await
    {
        Ok(route) => route,
        Err(ApiError::NotFound) if matches!(authority, ApiRequestAuthority::Key(_)) => {
            return Err(ApiError::AgentAccessDenied)
        }
        Err(error) => return Err(error),
    };
    if let ApiRequestAuthority::Key(key) = authority {
        if !key.agent_scope.allows(route.profile_id) {
            return Err(ApiError::AgentAccessDenied);
        }
    }

    let request_id = Uuid::new_v4();
    let gateway_id = profile_gateway_id(&route);
    let gateway_session_id = match gateway_id.as_deref() {
        Some(gateway_id) => state.relay.session_for(gateway_id).await,
        None => None,
    };
    let trace_request = chat_trace_request(&request);
    let input_summary = chat_request_summary(&request);
    let trace_id = db::create_trace(
        &state.pool,
        db::NewTrace {
            request_id,
            endpoint_id: None,
            project_id: Some(project.project.id),
            gateway_session_id,
            profile_id: Some(route.profile_id),
            profile_version_id: Some(route.profile_version_id),
            operation: "chat.completions",
            provider_key: Some(&route.provider_key),
            capability_kind: Some(&route.capability_kind),
            selection_key,
            request: &trace_request,
        },
    )
    .await?;
    let span_id = db::create_trace_span(
        &state.pool,
        db::NewTraceSpan {
            trace_id,
            parent_span_id: None,
            name: "agent.invoke",
            kind: "provider",
            provider_key: Some(&route.provider_key),
            capability_kind: Some("chat"),
            input_summary: Some(&input_summary),
            attributes: &json!({
                "profileId": route.profile_id,
                "profileVersionId": route.profile_version_id,
                "version": route.version_number,
                "capabilityId": route.capability_id,
            }),
        },
    )
    .await?;
    let timeout = profile_timeout(&route.runtime, state.config.request_timeout);
    let started_at = Instant::now();
    match invoke_profile_chat(
        state,
        project.project.id,
        &project.project.slug,
        &route,
        request_id,
        request,
        timeout,
    )
    .await
    {
        Ok(output) => {
            let response = chat_completion_response(request_id, &route.profile_slug, output);
            let duration = db::elapsed_millis(started_at);
            db::complete_trace_span(
                &state.pool,
                span_id,
                "completed",
                duration,
                Some(&json!({
                    "choiceCount": response.get("choices").and_then(Value::as_array).map_or(0, Vec::len)
                })),
                None,
            )
            .await?;
            persist_trace(
                state,
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
            let message = error.to_string();
            let duration = db::elapsed_millis(started_at);
            db::complete_trace_span(
                &state.pool,
                span_id,
                "failed",
                duration,
                None,
                Some(&message),
            )
            .await?;
            persist_trace(
                state,
                request_id,
                api_error_trace_status(&error),
                started_at,
                None,
                Some(&message),
            )
            .await;
            Err(error)
        }
    }
}

pub(crate) async fn invoke_runtime_extension_profile(
    state: &AppState,
    project: &crate::models::ProjectWithBindings,
    input: &vifu_gateway::runtime_extension::RuntimeProfileInvocation,
    request_id: Uuid,
) -> Result<Value, ApiError> {
    let capability = required_identifier("capability", &input.capability)?;
    if !matches!(capability, "chat" | "tool") {
        return Err(ApiError::Invalid(
            "runtime extensions may invoke chat or tool capabilities".to_string(),
        ));
    }
    let operation_id = required_identifier("operation ID", &input.operation_id)?;
    let route = db::resolve_profile_route(
        &state.pool,
        project.project.id,
        &input.profile_id.to_string(),
        capability,
        Some(operation_id),
        Some(input.profile_version_id),
    )
    .await?;
    let timeout = profile_timeout(&route.runtime, state.config.request_timeout);
    match capability {
        "chat" => {
            let request = runtime_agent_request(&route.profile_slug, input.input.clone());
            invoke_profile_chat(
                state,
                project.project.id,
                &project.project.slug,
                &route,
                request_id,
                request,
                timeout,
            )
            .await
            .map(normalize_runtime_agent_output)
        }
        "tool" => {
            let tool = input
                .tool
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| ApiError::Invalid("tool is required".to_string()))?;
            if route.provider_type != "openclaw" {
                return Err(ApiError::Invalid(format!(
                    "provider type {} does not support direct Tool invocation",
                    route.provider_type
                )));
            }
            if !runtime_profile_tool_is_available(&route.capability_config, tool) {
                return Err(ApiError::Invalid(format!(
                    "Tool {tool} is not available in the selected Profile version"
                )));
            }
            let agent_id = route.resource_id.as_deref().ok_or_else(|| {
                ApiError::Invalid("OpenClaw Tool capability is missing an Agent ID".to_string())
            })?;
            let mut client = connect_openclaw_management_client(
                state,
                &project.project.slug,
                &route.provider_key,
            )
            .await?;
            let invocation = client
                .invoke_tool(agent_id, tool, input.input.clone(), &request_id.to_string())
                .await;
            let close = client.close().await;
            match (invocation, close) {
                (Ok(output), Ok(())) => Ok(output),
                (Err(error), _) | (Ok(_), Err(error)) => Err(ApiError::Provider(error)),
            }
        }
        _ => unreachable!("capability was validated"),
    }
}

fn runtime_agent_request(model: &str, input: Value) -> Value {
    if input.get("messages").and_then(Value::as_array).is_some() {
        let mut request = input;
        if let Some(object) = request.as_object_mut() {
            object.insert("model".to_string(), Value::String(model.to_string()));
            object.insert("stream".to_string(), Value::Bool(false));
        }
        return request;
    }
    let content = input
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| serde_json::to_string(&input).unwrap_or_else(|_| "null".to_string()));
    json!({
        "model": model,
        "stream": false,
        "messages": [{"role": "user", "content": content}]
    })
}

fn normalize_runtime_agent_output(response: Value) -> Value {
    if response.get("dialogue").is_some() || response.get("stateChanges").is_some() {
        return response;
    }
    let Some(content) = response
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
    else {
        return response;
    };
    serde_json::from_str::<Value>(content)
        .ok()
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({"dialogue": content}))
}

fn runtime_profile_tool_is_available(config: &Value, tool: &str) -> bool {
    config
        .get("tools")
        .and_then(Value::as_array)
        .is_some_and(|tools| {
            tools.iter().any(|candidate| {
                candidate.as_str() == Some(tool)
                    || candidate.get("name").and_then(Value::as_str) == Some(tool)
            })
        })
}

fn gateway_binding_config(
    mut capability_config: Value,
    provider_key: &str,
    capability: &str,
    persona: &Value,
) -> Result<Value, ApiError> {
    let binding = capability_config.as_object_mut().ok_or_else(|| {
        ApiError::Invalid("Gateway capability config must be an object".to_string())
    })?;
    binding.insert(
        "providerKey".to_string(),
        Value::String(provider_key.to_string()),
    );
    binding.insert(
        "capability".to_string(),
        Value::String(capability.to_string()),
    );
    binding.insert("persona".to_string(), persona.clone());
    Ok(capability_config)
}

async fn invoke_profile_chat(
    state: &AppState,
    project_id: Uuid,
    project_slug: &str,
    route: &crate::models::ProfileRoute,
    request_id: Uuid,
    mut request: Value,
    timeout: Duration,
) -> Result<Value, ApiError> {
    let provider: Arc<dyn AgentProvider> = match route.provider_type.as_str() {
        "openclaw" | "vifu-runtime" => {
            if route.provider_type == "openclaw"
                && route.source.get("managed").and_then(Value::as_bool) == Some(false)
            {
                vifu_gateway::providers::apply_persona_to_chat_request(
                    &mut request,
                    &route.persona,
                )
                .map_err(ApiError::Invalid)?;
            }
            let gateway_id = profile_gateway_id(route).ok_or_else(|| {
                ApiError::Invalid("Gateway capability is missing gatewayId".to_string())
            })?;
            if route.provider_type == "vifu-runtime"
                && !db::runtime_deployment_allows_remote_invocation(
                    &state.pool,
                    project_id,
                    &gateway_id,
                )
                .await?
            {
                return Err(ApiError::Forbidden);
            }
            let agent_id = route.resource_id.as_deref().ok_or_else(|| {
                ApiError::Invalid("Gateway capability is missing resourceId".to_string())
            })?;
            let binding_config = gateway_binding_config(
                route.capability_config.clone(),
                &route.provider_key,
                "chat",
                &route.persona,
            )?;
            let endpoint_route = EndpointRoute {
                endpoint_id: route.capability_id,
                endpoint_slug: route.profile_slug.clone(),
                endpoint_name: route.profile_name.clone(),
                request_timeout_ms: i32::try_from(timeout.as_millis()).unwrap_or(i32::MAX),
                profile_id: route.profile_id,
                binding_id: route.capability_id,
                gateway_id,
                agent_id: agent_id.to_string(),
                binding_config,
            };
            Arc::new(RelayAgentProvider::new(
                "agent-gateway",
                "chat",
                state.relay.clone(),
                endpoint_route,
                request_id,
                timeout,
            ))
        }
        "openai-compatible" => {
            let provider =
                resolve_runtime_provider(state, project_slug, &route.provider_key).await?;
            if provider.provider_type != "openai-compatible" {
                return Err(ApiError::Invalid(
                    "profile capability does not match its configured provider".to_string(),
                ));
            }
            let model = route.resource_id.as_deref().ok_or_else(|| {
                ApiError::Invalid("chat capability is missing a provider model".to_string())
            })?;
            Arc::new(
                HttpCapabilityProvider::new("http-provider", provider.base_url, provider.token)
                    .map_err(map_runtime_error)?
                    .with_route(
                        "chat",
                        HttpCapabilityRoute::OpenAiChat {
                            model: model.to_string(),
                            persona: route.persona.clone(),
                        },
                    )
                    .map_err(map_runtime_error)?,
            )
        }
        provider => {
            return Err(ApiError::Invalid(format!(
                "provider type {provider} does not support chat"
            )))
        }
    };
    let (output, _) = invoke_registered_provider(
        RegisteredInvocation {
            project_id: project_slug,
            agent_id: route.profile_id,
            agent_name: &route.profile_name,
            endpoint: &route.profile_slug,
            capability: "chat",
            timeout,
            request_id,
        },
        provider,
        InvocationData::Json(request),
    )
    .await?;
    match output {
        InvocationData::Json(output) => Ok(output),
        InvocationData::Binary(_) => Err(ApiError::Internal),
    }
}

async fn invoke_profile_embedding(
    state: &AppState,
    project_id: Uuid,
    project_slug: &str,
    route: &crate::models::ProfileRoute,
    request_id: Uuid,
    request: Value,
    timeout: Duration,
) -> Result<(Value, Value), ApiError> {
    let provider: Arc<dyn AgentProvider> = match route.provider_type.as_str() {
        "vifu-runtime" => {
            let gateway_id = profile_gateway_id(route).ok_or_else(|| {
                ApiError::Invalid("Gateway capability is missing gatewayId".to_string())
            })?;
            if !db::runtime_deployment_allows_remote_invocation(
                &state.pool,
                project_id,
                &gateway_id,
            )
            .await?
            {
                return Err(ApiError::Forbidden);
            }
            let agent_id = route.resource_id.as_deref().ok_or_else(|| {
                ApiError::Invalid("embedding capability is missing resourceId".to_string())
            })?;
            let binding_config = gateway_binding_config(
                route.capability_config.clone(),
                &route.provider_key,
                "embedding",
                &route.persona,
            )?;
            let endpoint_route = EndpointRoute {
                endpoint_id: route.capability_id,
                endpoint_slug: route.profile_slug.clone(),
                endpoint_name: route.profile_name.clone(),
                request_timeout_ms: i32::try_from(timeout.as_millis()).unwrap_or(i32::MAX),
                profile_id: route.profile_id,
                binding_id: route.capability_id,
                gateway_id,
                agent_id: agent_id.to_string(),
                binding_config,
            };
            Arc::new(RelayAgentProvider::new(
                "agent-gateway",
                "embedding",
                state.relay.clone(),
                endpoint_route,
                request_id,
                timeout,
            ))
        }
        "openai-compatible" => {
            let provider =
                resolve_runtime_provider(state, project_slug, &route.provider_key).await?;
            if provider.provider_type != "openai-compatible" {
                return Err(ApiError::Invalid(
                    "embedding capability does not match its configured provider".to_string(),
                ));
            }
            let model = route.resource_id.as_deref().ok_or_else(|| {
                ApiError::Invalid("embedding capability is missing a provider model".to_string())
            })?;
            Arc::new(
                HttpCapabilityProvider::new("http-provider", provider.base_url, provider.token)
                    .map_err(map_runtime_error)?
                    .with_route(
                        "embedding",
                        HttpCapabilityRoute::OpenAiEmbedding {
                            model: model.to_string(),
                        },
                    )
                    .map_err(map_runtime_error)?,
            )
        }
        provider => {
            return Err(ApiError::Invalid(format!(
                "provider type {provider} does not support embedding"
            )))
        }
    };
    let (output, metadata) = invoke_registered_provider(
        RegisteredInvocation {
            project_id: project_slug,
            agent_id: route.profile_id,
            agent_name: &route.profile_name,
            endpoint: &route.profile_slug,
            capability: "embedding",
            timeout,
            request_id,
        },
        provider,
        InvocationData::Json(request),
    )
    .await?;
    match output {
        InvocationData::Json(output) => Ok((output, metadata)),
        InvocationData::Binary(_) => Err(ApiError::Internal),
    }
}

async fn invoke_endpoint_chat(
    state: &AppState,
    route: &EndpointRoute,
    request_id: Uuid,
    request: Value,
    timeout: Duration,
) -> Result<Value, ApiError> {
    let provider: Arc<dyn AgentProvider> = Arc::new(RelayAgentProvider::new(
        "agent-gateway",
        "chat",
        state.relay.clone(),
        route.clone(),
        request_id,
        timeout,
    ));
    let (output, _) = invoke_registered_provider(
        RegisteredInvocation {
            project_id: "server",
            agent_id: route.profile_id,
            agent_name: &route.endpoint_name,
            endpoint: &route.endpoint_slug,
            capability: "chat",
            timeout,
            request_id,
        },
        provider,
        InvocationData::Json(request),
    )
    .await?;
    match output {
        InvocationData::Json(output) => Ok(output),
        InvocationData::Binary(_) => Err(ApiError::Internal),
    }
}

struct RegisteredInvocation<'a> {
    project_id: &'a str,
    agent_id: Uuid,
    agent_name: &'a str,
    endpoint: &'a str,
    capability: &'a str,
    timeout: Duration,
    request_id: Uuid,
}

async fn invoke_registered_provider(
    invocation: RegisteredInvocation<'_>,
    provider: Arc<dyn AgentProvider>,
    data: InvocationData,
) -> Result<(InvocationData, Value), ApiError> {
    let RegisteredInvocation {
        project_id,
        agent_id,
        agent_name,
        endpoint,
        capability,
        timeout,
        request_id,
    } = invocation;
    let runtime = VifuRuntime::new(project_id).map_err(map_runtime_error)?;
    runtime
        .register_provider("server-provider", provider)
        .map_err(map_runtime_error)?;
    runtime
        .register_agent(AgentDefinition {
            id: agent_id.to_string(),
            name: agent_name.to_string(),
            provider: "server-provider".to_string(),
            capabilities: vec![capability.to_string()],
            metadata: json!({}),
        })
        .map_err(map_runtime_error)?;
    runtime
        .register_endpoint(EndpointDefinition {
            name: endpoint.to_string(),
            agent: agent_id.to_string(),
            capability: capability.to_string(),
            timeout_ms: u64::try_from(timeout.as_millis())
                .unwrap_or(120_000)
                .clamp(1, 120_000),
        })
        .map_err(map_runtime_error)?;
    runtime
        .invoke(InvocationInput {
            endpoint: endpoint.to_string(),
            session_id: request_id.to_string(),
            data,
            metadata: json!({ "requestId": request_id }),
        })
        .await
        .map(|output| (output.data, output.metadata))
        .map_err(map_runtime_error)
}

fn map_runtime_error(error: RuntimeError) -> ApiError {
    match error {
        RuntimeError::Timeout(_) | RuntimeError::Cancelled => ApiError::Timeout,
        RuntimeError::Unavailable(_) => ApiError::AgentGatewayUnavailable,
        RuntimeError::Backpressure(_) => ApiError::Backpressure,
        RuntimeError::Provider { provider, message } if provider == "agent-gateway" => {
            ApiError::AgentGateway(message)
        }
        RuntimeError::Provider { message, .. } => ApiError::Provider(message),
        RuntimeError::InvalidDefinition(message)
        | RuntimeError::EndpointNotFound(message)
        | RuntimeError::AgentNotFound(message)
        | RuntimeError::ProviderNotFound(message)
        | RuntimeError::Snapshot(message) => ApiError::Invalid(message),
        RuntimeError::CapabilityUnavailable {
            provider,
            capability,
        } => ApiError::Invalid(format!(
            "provider {provider} does not support capability {capability}"
        )),
        RuntimeError::Store(_)
        | RuntimeError::EffectLimitExceeded(_)
        | RuntimeError::InvocationNotFound(_)
        | RuntimeError::Internal => ApiError::Internal,
    }
}

fn profile_gateway_id(route: &crate::models::ProfileRoute) -> Option<String> {
    route
        .capability_config
        .get("gatewayId")
        .or_else(|| route.source.get("gatewayId"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn profile_timeout(runtime: &Value, server_timeout: Duration) -> Duration {
    let configured = runtime
        .get("requestTimeoutMs")
        .and_then(Value::as_u64)
        .unwrap_or(30_000)
        .clamp(500, 120_000);
    Duration::from_millis(configured.min(server_timeout.as_millis() as u64))
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
    if let Some(credential) = deployment_credential(headers) {
        if matches!(
            state
                .auth
                .authorize_token(credential, Operation::DeploymentWrite)
                .await,
            Ok(Identity::DeploymentAdmin)
        ) {
            return Ok(ApiRequestAuthority::Admin);
        }
    }
    let token = bearer_token(headers).ok_or(ApiError::Unauthorized)?;
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
                Ok(route) if key.agent_scope.allows(route.profile_id) => Ok(route),
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
    if chat_image_parts(request).len() > MAX_CHAT_IMAGES {
        return Err(ApiError::Invalid(format!(
            "chat requests support at most {MAX_CHAT_IMAGES} images"
        )));
    }
    if serde_json::to_vec(request)
        .map_err(|_| ApiError::Internal)?
        .len()
        > MAX_CHAT_REQUEST_BYTES
    {
        return Err(ApiError::Invalid("request body is too large".to_string()));
    }
    Ok(())
}

fn chat_image_parts(request: &Value) -> Vec<&Value> {
    request
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|message| message.get("content").and_then(Value::as_array))
        .flatten()
        .filter(|part| part.get("type").and_then(Value::as_str) == Some("image_url"))
        .collect()
}

fn chat_request_summary(request: &Value) -> Value {
    let images = chat_image_parts(request);
    let media_bytes = images
        .iter()
        .filter_map(|part| {
            part.get("image_url")
                .and_then(|image| image.get("url"))
                .and_then(Value::as_str)
        })
        .filter_map(data_url_parts)
        .map(|(_media_type, encoded)| estimated_base64_bytes(encoded))
        .sum::<usize>();
    json!({
        "model": request.get("model").and_then(Value::as_str),
        "messageCount": request.get("messages").and_then(Value::as_array).map_or(0, Vec::len),
        "imageCount": images.len(),
        "mediaBytes": media_bytes,
    })
}

fn chat_trace_request(request: &Value) -> Value {
    let mut trace = request.clone();
    let Some(messages) = trace.get_mut("messages").and_then(Value::as_array_mut) else {
        return trace;
    };
    for part in messages
        .iter_mut()
        .filter_map(|message| message.get_mut("content").and_then(Value::as_array_mut))
        .flatten()
        .filter(|part| part.get("type").and_then(Value::as_str) == Some("image_url"))
    {
        let Some(image) = part.get_mut("image_url").and_then(Value::as_object_mut) else {
            continue;
        };
        let url = image
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        image.insert(
            "url".to_string(),
            Value::String("[image content omitted]".to_string()),
        );
        image.insert("trace".to_string(), chat_image_trace_metadata(&url));
    }
    trace
}

fn chat_image_trace_metadata(url: &str) -> Value {
    if let Some((media_type, encoded)) = data_url_parts(url) {
        let decoded = base64::engine::general_purpose::STANDARD.decode(encoded);
        let digest_input = decoded.as_deref().unwrap_or(encoded.as_bytes());
        return json!({
            "source": "data",
            "mediaType": media_type,
            "encodedBytes": encoded.len(),
            "decodedBytes": decoded.as_ref().ok().map(|bytes| bytes.len()),
            "sha256": base64::engine::general_purpose::STANDARD_NO_PAD.encode(Sha256::digest(digest_input)),
        });
    }
    json!({
        "source": "reference",
        "sha256": base64::engine::general_purpose::STANDARD_NO_PAD.encode(Sha256::digest(url.as_bytes())),
    })
}

fn data_url_parts(url: &str) -> Option<(&str, &str)> {
    let data = url.strip_prefix("data:")?;
    let (metadata, encoded) = data.split_once(',')?;
    let media_type = metadata.split(';').next()?;
    metadata
        .split(';')
        .any(|part| part == "base64")
        .then_some((media_type, encoded))
}

fn estimated_base64_bytes(encoded: &str) -> usize {
    let padding = encoded
        .as_bytes()
        .iter()
        .rev()
        .take_while(|byte| **byte == b'=')
        .take(2)
        .count();
    (encoded.len() / 4 * 3).saturating_sub(padding)
}

fn validate_embedding_request(request: &Value) -> Result<(), ApiError> {
    let object = request
        .as_object()
        .ok_or_else(|| ApiError::Invalid("request body must be an object".to_string()))?;
    required_text(
        "model",
        object.get("model").and_then(Value::as_str).unwrap_or(""),
        128,
    )?;
    let input = object
        .get("input")
        .ok_or_else(|| ApiError::Invalid("input is required".to_string()))?;
    if !valid_embedding_input(input) {
        return Err(ApiError::Invalid(
            "input must be text, text arrays, token arrays, or arrays of token arrays".to_string(),
        ));
    }
    if let Some(format) = object.get("encoding_format") {
        if !matches!(format.as_str(), Some("float" | "base64")) {
            return Err(ApiError::Invalid(
                "encoding_format must be float or base64".to_string(),
            ));
        }
    }
    if let Some(dimensions) = object.get("dimensions") {
        if dimensions.as_u64().is_none_or(|value| value == 0) {
            return Err(ApiError::Invalid(
                "dimensions must be a positive integer".to_string(),
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

fn valid_embedding_input(input: &Value) -> bool {
    if input.is_string() {
        return true;
    }
    let Some(items) = input.as_array() else {
        return false;
    };
    if items.is_empty() {
        return false;
    }
    items.iter().all(Value::is_string)
        || items.iter().all(valid_embedding_token)
        || items.iter().all(|item| {
            item.as_array().is_some_and(|tokens| {
                !tokens.is_empty() && tokens.iter().all(valid_embedding_token)
            })
        })
}

fn valid_embedding_token(value: &Value) -> bool {
    value.as_i64().is_some_and(|token| token >= 0)
}

fn embedding_request_summary(request: &Value) -> Value {
    let input = &request["input"];
    let (input_count, input_format, text_bytes, supplied_tokens) =
        if let Some(text) = input.as_str() {
            (1, "text", text.len(), 0)
        } else if let Some(items) = input.as_array() {
            if items.iter().all(Value::is_string) {
                (
                    items.len(),
                    "text_batch",
                    items.iter().filter_map(Value::as_str).map(str::len).sum(),
                    0,
                )
            } else if items.iter().all(Value::is_number) {
                (1, "tokens", 0, items.len())
            } else {
                (
                    items.len(),
                    "token_batches",
                    0,
                    items.iter().filter_map(Value::as_array).map(Vec::len).sum(),
                )
            }
        } else {
            (0, "invalid", 0, 0)
        };
    json!({
        "model": request.get("model").and_then(Value::as_str),
        "inputCount": input_count,
        "inputFormat": input_format,
        "textBytes": text_bytes,
        "suppliedTokens": supplied_tokens,
        "encodingFormat": request.get("encoding_format").and_then(Value::as_str).unwrap_or("float"),
    })
}

fn embedding_response(model: &str, output: Value) -> Result<Value, ApiError> {
    let mut object = output
        .as_object()
        .cloned()
        .ok_or_else(|| ApiError::Provider("embedding response must be an object".to_string()))?;
    let data = object
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| ApiError::Provider("embedding response is missing data".to_string()))?;
    if data.is_empty()
        || data.iter().any(|item| {
            !item
                .get("embedding")
                .is_some_and(|embedding| embedding.is_array() || embedding.is_string())
        })
    {
        return Err(ApiError::Provider(
            "embedding response data is invalid".to_string(),
        ));
    }
    object.insert("object".to_string(), Value::String("list".to_string()));
    object.insert("model".to_string(), Value::String(model.to_string()));
    Ok(Value::Object(object))
}

fn embedding_response_summary(response: &Value, metadata: &Value) -> Value {
    let data = response
        .get("data")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let dimensions = data
        .first()
        .and_then(|item| item.get("embedding"))
        .and_then(Value::as_array)
        .map(Vec::len);
    json!({
        "model": response.get("model").and_then(Value::as_str),
        "inputCount": data.len(),
        "dimensions": dimensions,
        "usage": response.get("usage").cloned().unwrap_or(Value::Null),
        "provider": metadata,
    })
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

struct ProjectProviderSource {
    kind: String,
    key: String,
    name: String,
    provider_type: String,
    base_url: String,
    config: Value,
}

struct ResolvedRuntimeProvider {
    provider_type: String,
    base_url: String,
    token: Option<String>,
    config: Value,
}

struct OpenClawProfileSource {
    provider_key: String,
    resource_id: String,
}

fn openclaw_source(
    source: &Value,
    capabilities: &[crate::models::AgentProfileCapability],
) -> Result<OpenClawProfileSource, ApiError> {
    if source.get("type").and_then(Value::as_str) != Some("openclaw") {
        return Err(ApiError::Conflict(
            "profile source is not managed by OpenClaw".to_string(),
        ));
    }
    if source.get("managed").and_then(Value::as_bool) == Some(false) {
        return Err(ApiError::Conflict(
            "profile source is not configured for provider-managed persona files".to_string(),
        ));
    }
    let chat = capabilities
        .iter()
        .find(|capability| capability.provider_type == "openclaw");
    let provider_key = source
        .get("providerKey")
        .and_then(Value::as_str)
        .or_else(|| chat.map(|capability| capability.provider_key.as_str()))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            ApiError::Invalid("OpenClaw profile source is missing providerKey".to_string())
        })?;
    let resource_id = source
        .get("resourceId")
        .and_then(Value::as_str)
        .or_else(|| chat.and_then(|capability| capability.resource_id.as_deref()))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            ApiError::Invalid("OpenClaw profile source is missing resourceId".to_string())
        })?;
    required_identifier("provider key", provider_key)?;
    required_text("OpenClaw agent ID", resource_id, 512)?;
    Ok(OpenClawProfileSource {
        provider_key: provider_key.to_string(),
        resource_id: resource_id.to_string(),
    })
}

async fn resolve_openclaw_provider(
    state: &AppState,
    project_slug: &str,
    provider_key: &str,
) -> Result<ResolvedRuntimeProvider, ApiError> {
    let provider = resolve_runtime_provider(state, project_slug, provider_key).await?;
    if provider.provider_type != "openclaw" {
        return Err(ApiError::Invalid(format!(
            "provider {provider_key} is not an OpenClaw connection"
        )));
    }
    Ok(provider)
}

async fn connect_openclaw_management_client(
    state: &AppState,
    project_slug: &str,
    provider_key: &str,
) -> Result<vifu_gateway::openclaw_rpc::OpenClawGatewayClient, ApiError> {
    let provider = resolve_openclaw_provider(state, project_slug, provider_key).await?;
    let endpoint =
        vifu_gateway::openclaw::parse_endpoint(&provider.base_url).map_err(ApiError::Invalid)?;
    let identity = openclaw_device::load_or_create(
        &state.config.provider_home_dir,
        project_slug,
        provider_key,
    )
    .map_err(ApiError::Provider)?;
    vifu_gateway::openclaw_rpc::OpenClawGatewayClient::connect(
        &endpoint,
        provider.token.as_deref(),
        Some(&identity),
    )
    .await
    .map_err(ApiError::Provider)
}

async fn read_openclaw_persona_files(
    client: &mut vifu_gateway::openclaw_rpc::OpenClawGatewayClient,
    agent_id: &str,
) -> Result<BTreeMap<String, String>, ApiError> {
    let files = client
        .list_agent_files(agent_id)
        .await
        .map_err(ApiError::Provider)?;
    if files.len() > 32 {
        return Err(ApiError::Provider(
            "OpenClaw returned too many editable agent files".to_string(),
        ));
    }
    let mut result = BTreeMap::new();
    let mut total_bytes = 0_usize;
    for file in files {
        if file.missing {
            continue;
        }
        let content = match file.content {
            Some(content) => content,
            None => client
                .get_agent_file(agent_id, &file.name)
                .await
                .map_err(ApiError::Provider)?
                .content
                .ok_or_else(|| {
                    ApiError::Provider(format!("OpenClaw did not return content for {}", file.name))
                })?,
        };
        total_bytes = total_bytes.saturating_add(file.name.len() + content.len());
        if total_bytes > 512 * 1024 {
            return Err(ApiError::Provider(
                "OpenClaw agent persona files are too large".to_string(),
            ));
        }
        result.insert(file.name, content);
    }
    Ok(result)
}

fn compact_openclaw_tool_catalog(catalog: &Value) -> Vec<Value> {
    catalog
        .get("groups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|group| {
            group
                .get("tools")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .filter_map(|tool| {
            let id = tool.get("id")?.as_str()?.trim();
            if id.is_empty() {
                return None;
            }
            Some(json!({
                "id": id,
                "label": tool.get("label").and_then(Value::as_str).unwrap_or(id),
                "source": tool.get("source").and_then(Value::as_str).unwrap_or("core"),
                "risk": tool.get("risk").and_then(Value::as_str),
            }))
        })
        .take(256)
        .collect()
}

async fn sync_managed_profile_version(
    state: &AppState,
    project_slug: &str,
    profile_id: Uuid,
    version_id: Uuid,
) -> Result<(), ApiError> {
    let version = db::get_profile_version(&state.pool, profile_id, version_id).await?;
    if version.source.get("type").and_then(Value::as_str) != Some("openclaw")
        || version.source.get("managed").and_then(Value::as_bool) == Some(false)
    {
        return Ok(());
    }
    let capabilities = db::list_profile_capabilities(&state.pool, version.id).await?;
    let source = openclaw_source(&version.source, &capabilities)?;
    let desired = persona_file_map(&version.persona)?;
    let mut client =
        connect_openclaw_management_client(state, project_slug, &source.provider_key).await?;
    let listed = client
        .list_agent_files(&source.resource_id)
        .await
        .map_err(ApiError::Provider)?;
    let allowed = listed
        .iter()
        .map(|file| file.name.as_str())
        .collect::<std::collections::HashSet<_>>();
    if let Some(name) = desired.keys().find(|name| !allowed.contains(name.as_str())) {
        return Err(ApiError::Invalid(format!(
            "{name} is not an editable OpenClaw agent file"
        )));
    }

    let mut original = BTreeMap::new();
    for file in listed.iter().filter(|file| !file.missing) {
        let current = client
            .get_agent_file(&source.resource_id, &file.name)
            .await
            .map_err(ApiError::Provider)?
            .content
            .unwrap_or_default();
        if !desired.contains_key(&file.name) && !current.is_empty() {
            return Err(ApiError::Conflict(format!(
                "version does not define {}; sync the source before activating it",
                file.name
            )));
        }
        original.insert(file.name.clone(), current);
    }

    let mut written = Vec::<String>::new();
    for (name, content) in &desired {
        if original.get(name).is_some_and(|current| current == content) {
            continue;
        }
        if let Err(error) = client
            .set_agent_file(&source.resource_id, name, content)
            .await
        {
            for written_name in written.into_iter().rev() {
                if let Some(previous) = original.get(&written_name) {
                    let _ = client
                        .set_agent_file(&source.resource_id, &written_name, previous)
                        .await;
                }
            }
            return Err(ApiError::Provider(format!(
                "OpenClaw persona activation failed: {error}"
            )));
        }
        written.push(name.clone());
    }
    client.close().await.map_err(ApiError::Provider)
}

fn persona_file_map(persona: &Value) -> Result<BTreeMap<String, String>, ApiError> {
    let Some(files) = persona.get("files") else {
        return Ok(BTreeMap::new());
    };
    let files = files
        .as_object()
        .ok_or_else(|| ApiError::Invalid("persona.files must be an object".to_string()))?;
    let mut result = BTreeMap::new();
    for (name, content) in files {
        let content = content
            .as_str()
            .ok_or_else(|| ApiError::Invalid(format!("persona file {name} must contain text")))?;
        result.insert(name.clone(), content.to_string());
    }
    Ok(result)
}

async fn resolve_runtime_provider(
    state: &AppState,
    project_slug: &str,
    provider_key: &str,
) -> Result<ResolvedRuntimeProvider, ApiError> {
    let connection =
        db::get_provider_connection_secret_by_key(&state.pool, project_slug, provider_key).await?;
    let overrides = decrypted_provider_secrets(state, &connection)?;
    let (provider_type, base_url, config, secrets) = if connection.source_kind == "custom" {
        let project = db::get_project_by_slug(&state.pool, project_slug).await?;
        let source = resolve_project_provider_source(
            state,
            &connection.source_kind,
            &connection.source_key,
            Some(&project.project.gateway_id),
        )
        .await?;
        let base_url = if connection.base_url.trim().is_empty() {
            source.base_url
        } else {
            connection.base_url.clone()
        };
        (
            source.provider_type,
            base_url,
            merge_json_objects(&source.config, &connection.config)?,
            overrides,
        )
    } else {
        (
            connection.provider_type.clone(),
            connection.base_url.clone(),
            connection.config.clone(),
            overrides,
        )
    };
    Ok(ResolvedRuntimeProvider {
        provider_type,
        base_url,
        token: provider_token(&secrets)?,
        config,
    })
}

fn provider_adapters() -> Vec<ProviderAdapter> {
    vec![
        ProviderAdapter {
            id: "openclaw".to_string(),
            category: "local".to_string(),
            name: "OpenClaw".to_string(),
            description: "Discover and manage agents through an OpenClaw Gateway.".to_string(),
            capabilities: vec!["chat".to_string(), "tool".to_string()],
            execution_modes: vec!["gateway".to_string()],
            supports_discovery: true,
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
        },
        ProviderAdapter {
            id: "local-whisper".to_string(),
            category: "local".to_string(),
            name: "Local Whisper".to_string(),
            description:
                "Run speech-to-text through a local Agent Gateway provider registered in providers.json."
                    .to_string(),
            capabilities: vec!["transcription".to_string()],
            execution_modes: vec!["gateway".to_string()],
            supports_discovery: false,
            fields: vec![ProviderAdapterField {
                key: "model".to_string(),
                label: "Model file".to_string(),
                kind: "text".to_string(),
                required: true,
                secret: false,
            }],
        },
        ProviderAdapter {
            id: "elevenlabs".to_string(),
            category: "cloud".to_string(),
            name: "ElevenLabs".to_string(),
            description: "Use ElevenLabs voices from this project provider.".to_string(),
            capabilities: vec!["speech".to_string()],
            execution_modes: vec!["server".to_string()],
            supports_discovery: false,
            fields: vec![
                ProviderAdapterField {
                    key: "baseUrl".to_string(),
                    label: "API URL".to_string(),
                    kind: "url".to_string(),
                    required: true,
                    secret: false,
                },
                ProviderAdapterField {
                    key: "token".to_string(),
                    label: "API key".to_string(),
                    kind: "password".to_string(),
                    required: true,
                    secret: true,
                },
            ],
        },
        ProviderAdapter {
            id: "openai-compatible".to_string(),
            category: "custom".to_string(),
            name: "OpenAI-compatible".to_string(),
            description: "Connect any provider that implements the OpenAI HTTP API.".to_string(),
            capabilities: vec![
                "chat".to_string(),
                "embedding".to_string(),
                "transcription".to_string(),
                "realtime".to_string(),
            ],
            execution_modes: vec!["server".to_string()],
            supports_discovery: false,
            fields: vec![
                ProviderAdapterField {
                    key: "baseUrl".to_string(),
                    label: "API base URL".to_string(),
                    kind: "url".to_string(),
                    required: true,
                    secret: false,
                },
                ProviderAdapterField {
                    key: "token".to_string(),
                    label: "API key".to_string(),
                    kind: "password".to_string(),
                    required: false,
                    secret: true,
                },
            ],
        },
    ]
}

async fn available_provider_catalog(
    state: &AppState,
    gateway_id: Option<&str>,
) -> Result<Vec<CustomProvider>, ApiError> {
    let mut providers = BTreeMap::new();
    for session in db::list_agent_gateway_sessions(&state.pool).await? {
        if session.status != "connected" {
            continue;
        }
        if gateway_id.is_some_and(|expected| expected != session.gateway_id) {
            continue;
        }
        collect_session_declared_providers(&mut providers, &session)?;
        collect_session_agent_providers(&mut providers, &session)?;
    }
    Ok(providers.into_values().collect())
}

async fn available_provider(
    state: &AppState,
    gateway_id: Option<&str>,
    provider_key: &str,
) -> Result<CustomProvider, ApiError> {
    available_provider_catalog(state, gateway_id)
        .await?
        .into_iter()
        .find(|provider| provider.provider_key == provider_key)
        .ok_or(ApiError::NotFound)
}

fn collect_session_declared_providers(
    providers: &mut BTreeMap<String, CustomProvider>,
    session: &AgentGatewaySession,
) -> Result<(), ApiError> {
    let Some(items) = session.metadata.get("providers").and_then(Value::as_array) else {
        return Ok(());
    };
    for item in items {
        let Some(provider_key) = json_text(item, &["id", "key", "providerKey"]) else {
            continue;
        };
        let provider_type = json_text(item, &["type", "providerType"]).unwrap_or("vifu-runtime");
        let name = json_text(item, &["name"]).unwrap_or(provider_key);
        upsert_available_provider(
            providers,
            session,
            provider_key,
            provider_type,
            name,
            json_text(item, &["localProviderType"]),
            provider_capabilities(item),
        )?;
    }
    Ok(())
}

fn collect_session_agent_providers(
    providers: &mut BTreeMap<String, CustomProvider>,
    session: &AgentGatewaySession,
) -> Result<(), ApiError> {
    let Some(items) = session.agents.as_array() else {
        return Ok(());
    };
    for item in items {
        let metadata = item
            .get("metadata")
            .cloned()
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}));
        let Some(provider_key) = metadata.get("providerKey").and_then(Value::as_str) else {
            continue;
        };
        let provider_type = metadata
            .get("providerType")
            .and_then(Value::as_str)
            .unwrap_or("vifu-runtime");
        let provider_name = metadata
            .get("providerName")
            .and_then(Value::as_str)
            .unwrap_or(provider_key);
        upsert_available_provider(
            providers,
            session,
            provider_key,
            provider_type,
            provider_name,
            metadata.get("localProviderType").and_then(Value::as_str),
            provider_capabilities(&metadata),
        )?;
    }
    Ok(())
}

fn upsert_available_provider(
    providers: &mut BTreeMap<String, CustomProvider>,
    session: &AgentGatewaySession,
    provider_key: &str,
    provider_type: &str,
    name: &str,
    local_provider_type: Option<&str>,
    capabilities: Vec<String>,
) -> Result<(), ApiError> {
    let provider_key = required_identifier("provider key", provider_key)?.to_string();
    let provider_type = required_identifier("provider type", provider_type)?.to_string();
    let name = optional_text("provider name", Some(name), 128)?
        .unwrap_or(&provider_key)
        .to_string();
    let provider = providers
        .entry(provider_key.clone())
        .or_insert_with(|| CustomProvider {
            id: deterministic_provider_connection_id(Uuid::nil(), &provider_key),
            provider_key: provider_key.clone(),
            name: name.clone(),
            provider_type: provider_type.clone(),
            base_url: String::new(),
            config: json!({
                "gatewayId": session.gateway_id.clone(),
                "source": "agent-gateway",
            }),
            secret_keys: Vec::new(),
            display_secret: None,
            status: "online".to_string(),
            last_checked_at: Some(session.last_seen_at),
            created_at: session.connected_at,
            updated_at: session.last_seen_at,
        });
    if provider.name == provider.provider_key && name != provider.provider_key {
        provider.name = name;
    }
    provider.provider_type = provider_type;
    provider.last_checked_at = Some(session.last_seen_at);
    provider.updated_at = session.last_seen_at;
    if let Some(local_provider_type) = local_provider_type
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if let Some(config) = provider.config.as_object_mut() {
            config.insert(
                "localProviderType".to_string(),
                Value::String(local_provider_type.to_string()),
            );
        }
    }
    merge_provider_capabilities(&mut provider.config, capabilities);
    Ok(())
}

fn json_text<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
    })
}

fn provider_capabilities(value: &Value) -> Vec<String> {
    let mut capabilities = HashSet::new();
    if let Some(items) = value.get("capabilities").and_then(Value::as_array) {
        for item in items {
            if let Some(capability) = item.as_str().map(str::trim).filter(|text| !text.is_empty()) {
                capabilities.insert(capability.to_string());
            }
        }
    }
    let mut capabilities = capabilities.into_iter().collect::<Vec<_>>();
    capabilities.sort();
    capabilities
}

fn merge_provider_capabilities(config: &mut Value, capabilities: Vec<String>) {
    if capabilities.is_empty() && config.get("capabilities").is_none() {
        return;
    }
    let mut merged = config
        .get("capabilities")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| item.as_str().map(str::trim))
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect::<HashSet<_>>();
    merged.extend(capabilities);
    let mut merged = merged.into_iter().collect::<Vec<_>>();
    merged.sort();
    if let Some(object) = config.as_object_mut() {
        object.insert("capabilities".to_string(), json!(merged));
    }
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

async fn resolve_project_provider_source(
    state: &AppState,
    kind: &str,
    key: &str,
    gateway_id: Option<&str>,
) -> Result<ProjectProviderSource, ApiError> {
    let kind = required_identifier("provider source kind", kind)?;
    let key = required_identifier("provider source key", key)?;
    match kind {
        "registry" => {
            let adapter = provider_adapters()
                .into_iter()
                .find(|adapter| adapter.id == key)
                .ok_or(ApiError::NotFound)?;
            Ok(ProjectProviderSource {
                kind: kind.to_string(),
                key: key.to_string(),
                name: adapter.name,
                provider_type: adapter.id,
                base_url: String::new(),
                config: json!({}),
            })
        }
        "custom" => {
            let provider = available_provider(state, gateway_id, key).await?;
            Ok(ProjectProviderSource {
                kind: kind.to_string(),
                key: key.to_string(),
                name: provider.name,
                provider_type: provider.provider_type,
                base_url: provider.base_url,
                config: provider.config,
            })
        }
        _ => Err(ApiError::Invalid(
            "provider source kind must be registry or custom".to_string(),
        )),
    }
}

async fn unique_project_provider_key(
    state: &AppState,
    project_slug: &str,
    requested: &str,
) -> Result<String, ApiError> {
    let existing = db::list_provider_connections(&state.pool, project_slug)
        .await?
        .into_iter()
        .map(|provider| provider.provider_key)
        .collect::<HashSet<_>>();
    let base = slugify(requested);
    if !existing.contains(&base) {
        return Ok(base);
    }
    for suffix in 2..1000 {
        let suffix = format!("-{suffix}");
        let available = 64_usize.saturating_sub(suffix.len());
        let candidate = format!(
            "{}{}",
            base.chars().take(available).collect::<String>(),
            suffix
        );
        if !existing.contains(&candidate) {
            return Ok(candidate);
        }
    }
    Err(ApiError::Conflict(
        "could not allocate a project provider key".to_string(),
    ))
}

fn prepare_project_provider(
    state: &AppState,
    provider_key: &str,
    name: &str,
    source: &ProjectProviderSource,
    base_url: Option<&str>,
    config: Value,
    secrets: Value,
) -> Result<PreparedProviderInput, ApiError> {
    validate_json_object("config", &config, 64 * 1024)?;
    let base_override = base_url.map(str::trim).unwrap_or("");
    let effective_base = if base_override.is_empty() {
        source.base_url.as_str()
    } else {
        base_override
    };
    let effective_config = merge_json_objects(&source.config, &config)?;
    let mut prepared = prepare_provider_connection(
        state,
        PreparedProviderSource {
            key: provider_key,
            name: Some(name),
            provider_type: &source.provider_type,
            base_url: effective_base,
            config: effective_config,
            secrets,
        },
    )?;
    prepared.base_url = if source.kind == "custom" && base_override == source.base_url {
        String::new()
    } else {
        base_override.to_string()
    };
    prepared.config = config;
    Ok(prepared)
}

fn prepare_project_provider_assignment(
    state: &AppState,
    provider_key: &str,
    name: &str,
    provider_type: &str,
) -> Result<PreparedProviderInput, ApiError> {
    prepare_project_provider_assignment_with_secret_key(
        &state.config.provider_secret_key,
        provider_key,
        name,
        provider_type,
    )
}

fn prepare_project_provider_assignment_with_secret_key(
    provider_secret_key: &str,
    provider_key: &str,
    name: &str,
    provider_type: &str,
) -> Result<PreparedProviderInput, ApiError> {
    let key = required_identifier("provider key", provider_key)?.to_string();
    let provider_type = required_identifier("provider type", provider_type)?.to_string();
    let name = optional_text("provider name", Some(name), 128)?
        .unwrap_or(&key)
        .to_string();
    let encrypted_secret_json = encrypt_secret_json("{}", provider_secret_key)?;
    Ok(PreparedProviderInput {
        key,
        name,
        provider_type,
        base_url: String::new(),
        config: json!({}),
        encrypted_secret_json,
        secret_keys: Vec::new(),
        display_secret: None,
    })
}

fn merge_json_objects(base: &Value, overrides: &Value) -> Result<Value, ApiError> {
    let mut merged = base
        .as_object()
        .cloned()
        .ok_or_else(|| ApiError::Invalid("provider config must be an object".to_string()))?;
    let overrides = overrides
        .as_object()
        .ok_or_else(|| ApiError::Invalid("provider config must be an object".to_string()))?;
    for (key, value) in overrides {
        merged.insert(key.clone(), value.clone());
    }
    Ok(Value::Object(merged))
}

async fn effective_provider_connection(
    state: &AppState,
    mut connection: ProviderConnection,
) -> Result<ProviderConnection, ApiError> {
    if connection.source_kind != "custom" {
        return Ok(connection);
    }
    let project = db::get_project(&state.pool, connection.project_id).await?;
    let source = match available_provider(
        state,
        Some(&project.project.gateway_id),
        &connection.source_key,
    )
    .await
    {
        Ok(source) => source,
        Err(ApiError::NotFound) => {
            if connection.status != "offline" {
                connection.status = "missing_source".to_string();
            }
            return Ok(connection);
        }
        Err(error) => return Err(error),
    };
    if connection.base_url.trim().is_empty() {
        connection.base_url = source.base_url;
    }
    connection.provider_type = source.provider_type;
    connection.config = merge_json_objects(&source.config, &connection.config)?;
    connection.secret_keys.extend(source.secret_keys);
    connection.secret_keys.sort();
    connection.secret_keys.dedup();
    Ok(connection)
}

async fn reconcile_project_provider_agents(
    state: &AppState,
    project_slug: &str,
    provider_key: &str,
) -> Result<usize, ApiError> {
    let project = db::get_project_by_slug(&state.pool, project_slug).await?;
    let agents = db::list_available_agents(&state.pool).await?;
    let mut added = 0_usize;
    for agent in agents.into_iter().filter(|agent| {
        agent.gateway_id == project.project.gateway_id
            && agent.metadata.get("providerKey").and_then(Value::as_str) == Some(provider_key)
            && agent.status == "connected"
    }) {
        let provider_type = agent
            .metadata
            .get("providerType")
            .and_then(Value::as_str)
            .unwrap_or("openclaw");
        match db::find_project_profile_by_provider_resource(
            &state.pool,
            project.project.id,
            provider_key,
            &agent.id,
        )
        .await?
        {
            Some((_profile_id, true, binding_id)) => {
                db::refresh_discovered_binding(
                    &state.pool,
                    binding_id,
                    &agent.gateway_id,
                    &agent.name,
                )
                .await?;
            }
            Some((_profile_id, false, binding_id)) => {
                db::refresh_discovered_binding(
                    &state.pool,
                    binding_id,
                    &agent.gateway_id,
                    &agent.name,
                )
                .await?;
                db::assign_project_binding(&state.pool, project.project.id, binding_id).await?;
            }
            None => {
                db::ensure_discovered_binding(
                    &state.pool,
                    project.project.id,
                    &agent.gateway_id,
                    &agent.id,
                    &agent.name,
                    provider_key,
                    provider_type,
                )
                .await?;
                added += 1;
            }
        }
    }
    Ok(added)
}

async fn refresh_project_provider(
    state: &AppState,
    project_slug: &str,
    connection: ProviderConnection,
) -> Result<(ProviderConnection, Option<String>, usize), ApiError> {
    if connection.source_kind == "custom" {
        let project = db::get_project_by_slug(&state.pool, project_slug).await?;
        let (status, message) = match available_provider(
            state,
            Some(&project.project.gateway_id),
            &connection.source_key,
        )
        .await
        {
            Ok(_) => ("online", None),
            Err(ApiError::NotFound) => (
                "offline",
                Some(format!(
                    "provider {} is not reported by gateway {}",
                    connection.source_key, project.project.gateway_id
                )),
            ),
            Err(error) => return Err(error),
        };
        let updated =
            db::update_provider_connection_status(&state.pool, connection.id, status).await?;
        let added_agents =
            reconcile_project_provider_agents(state, project_slug, &connection.provider_key)
                .await?;
        return Ok((
            effective_provider_connection(state, updated).await?,
            message,
            added_agents,
        ));
    }
    let resolved = resolve_runtime_provider(state, project_slug, &connection.provider_key).await?;
    let (status, message) = probe_runtime_provider(
        &state.config.provider_home_dir,
        &resolved.provider_type,
        &resolved.base_url,
        resolved.token.as_deref(),
        &resolved.config,
    )
    .await;
    let updated = db::update_provider_connection_status(&state.pool, connection.id, status).await?;
    let added_agents =
        reconcile_project_provider_agents(state, project_slug, &connection.provider_key).await?;
    Ok((
        effective_provider_connection(state, updated).await?,
        message,
        added_agents,
    ))
}

async fn probe_runtime_provider(
    provider_home_dir: &FsPath,
    provider_type: &str,
    base_url: &str,
    token: Option<&str>,
    config: &Value,
) -> (&'static str, Option<String>) {
    match provider_type {
        "openclaw" => {
            let report = vifu_gateway::openclaw::probe(base_url).await;
            match report.status {
                vifu_gateway::openclaw::ProbeStatus::Online => ("online", None),
                vifu_gateway::openclaw::ProbeStatus::Offline(message) => ("offline", Some(message)),
                vifu_gateway::openclaw::ProbeStatus::Unsupported(message) => {
                    ("unsupported", Some(message))
                }
            }
        }
        "openai-compatible" => {
            probe_result(vifu_gateway::providers::probe_openai_compatible(base_url, token).await)
        }
        "elevenlabs" => {
            probe_result(vifu_gateway::providers::probe_elevenlabs(base_url, token).await)
        }
        "local-whisper" => {
            let model = config.get("model").and_then(Value::as_str).unwrap_or("");
            let result =
                vifu_gateway::providers::resolve_local_model_path(provider_home_dir, model)
                    .and_then(|path| {
                        if path.is_file() {
                            Ok(())
                        } else {
                            Err(format!(
                                "Whisper model {model} is not installed in {}",
                                provider_home_dir.join("models").display()
                            ))
                        }
                    });
            probe_result(result)
        }
        "llama" => ("online", None),
        _ => (
            "unsupported",
            Some(format!("unsupported provider type {provider_type}")),
        ),
    }
}

fn probe_result(result: Result<(), String>) -> (&'static str, Option<String>) {
    match result {
        Ok(()) => ("online", None),
        Err(message) => ("offline", Some(message)),
    }
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
    let base_url = validated_provider_base_url(&provider_type, source.base_url)?;
    if provider_type == "openclaw" {
        vifu_gateway::openclaw::parse_endpoint(&base_url).map_err(ApiError::Invalid)?;
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

fn validated_provider_base_url(provider_type: &str, value: &str) -> Result<String, ApiError> {
    if matches!(provider_type, "llama" | "local-whisper") && value.trim().is_empty() {
        return Ok(String::new());
    }
    Ok(required_text("provider base URL", value, 2048)?.to_string())
}

async fn save_provider_connection(
    state: &AppState,
    project_slug: &str,
    source_kind: &str,
    source_key: &str,
    input: PreparedProviderInput,
) -> Result<crate::models::ProviderConnection, ApiError> {
    db::upsert_provider_connection(
        &state.pool,
        project_slug,
        db::NewProviderConnection {
            provider_key: &input.key,
            source_kind,
            source_key,
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
    optional_secret_string(secrets, "token")
}

fn mask_secret(value: &str) -> String {
    let suffix_rev: String = value.chars().rev().take(4).collect();
    let suffix: String = suffix_rev.chars().rev().collect();
    format!("****{suffix}")
}

#[derive(Clone, Copy)]
enum ProjectAccess {
    Read,
    Write,
}

async fn authorized_project_by_slug(
    state: &AppState,
    headers: &HeaderMap,
    slug: &str,
    access: ProjectAccess,
) -> Result<crate::models::ProjectWithBindings, ApiError> {
    deployment_credential(headers).ok_or(ApiError::Unauthorized)?;
    let project = db::get_project_by_slug(&state.pool, slug).await?;
    state
        .auth
        .authorize_project(
            headers,
            match access {
                ProjectAccess::Read => Operation::ProjectRead,
                ProjectAccess::Write => Operation::ProjectWrite,
            },
            project.project.owner_user_id.as_deref(),
        )
        .await?;
    Ok(project)
}

async fn project_binding(
    state: &AppState,
    project_id: Uuid,
    binding_id: Uuid,
) -> Result<crate::models::AgentBinding, ApiError> {
    let binding = db::get_binding(&state.pool, binding_id).await?;
    db::get_project_profile(&state.pool, project_id, binding.profile_id).await?;
    Ok(binding)
}

async fn project_endpoint(
    state: &AppState,
    project_id: Uuid,
    endpoint_id: Uuid,
) -> Result<AgentEndpoint, ApiError> {
    let endpoint = db::get_endpoint(&state.pool, endpoint_id).await?;
    db::get_project_profile(&state.pool, project_id, endpoint.profile_id).await?;
    project_binding(state, project_id, endpoint.binding_id).await?;
    Ok(endpoint)
}

async fn deployment_identity(
    state: &AppState,
    headers: &HeaderMap,
    operation: Operation,
) -> Result<Identity, ApiError> {
    state.auth.authorize(headers, operation).await
}

async fn deployment_admin(
    state: &AppState,
    headers: &HeaderMap,
    operation: Operation,
) -> Result<(), ApiError> {
    match deployment_identity(state, headers, operation).await? {
        Identity::DeploymentAdmin => Ok(()),
        Identity::ActingUser { .. } => Err(ApiError::Forbidden),
    }
}

async fn deployment_read(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    deployment_identity(state, headers, Operation::DeploymentRead)
        .await
        .map(|_| ())
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

fn runtime_trace_uuid(kind: &str, gateway_id: &str, trace_id: &str) -> Uuid {
    let mut hasher = Sha256::new();
    hasher.update(b"vifu-runtime-trace-v1\0");
    hasher.update(kind.as_bytes());
    hasher.update(b"\0");
    hasher.update(gateway_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(trace_id.as_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
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
        ApiKeyAgentScope::Selected { mut profile_ids } => {
            profile_ids.sort_unstable();
            profile_ids.dedup();
            if profile_ids.is_empty() || profile_ids.len() > 256 {
                return Err(ApiError::Invalid(
                    "selected profile access requires between 1 and 256 profiles".to_string(),
                ));
            }
            Ok(ApiKeyAgentScope::Selected { profile_ids })
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

fn validate_agent_gateway_enrollment_token(value: &str) -> Result<&str, ApiError> {
    let value = value.trim();
    let secret = value
        .strip_prefix("vifu_ge_")
        .ok_or(ApiError::Unauthorized)?;
    if value.len() != 72 || !secret.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ApiError::Unauthorized);
    }
    Ok(value)
}

fn validate_guest_claim_token(value: &str) -> Result<&str, ApiError> {
    let value = value.trim();
    let secret = value
        .strip_prefix("vifu_gc_")
        .ok_or(ApiError::Unauthorized)?;
    if value.len() != 72 || !secret.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ApiError::Unauthorized);
    }
    Ok(value)
}

fn guest_project_slug(gateway_id: &str) -> String {
    let digest = Sha256::digest(gateway_id.as_bytes());
    let mut suffix = String::with_capacity(16);
    for byte in &digest[..8] {
        use std::fmt::Write;
        let _ = write!(&mut suffix, "{byte:02x}");
    }
    format!("guest-{suffix}")
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

async fn validate_project_profile_providers(
    state: &AppState,
    project_id: Uuid,
    source: &Value,
    capabilities: &[ProfileCapabilityDraft],
) -> Result<(), ApiError> {
    let mut provider_keys = capabilities
        .iter()
        .map(|capability| capability.provider_key.as_str())
        .collect::<HashSet<_>>();
    if let Some(provider_key) = source
        .get("providerKey")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        provider_keys.insert(provider_key);
    }
    for provider_key in provider_keys {
        if !db::project_provider_is_assigned(&state.pool, project_id, provider_key).await? {
            return Err(ApiError::Conflict(format!(
                "provider {provider_key} is not assigned to this project"
            )));
        }
    }
    Ok(())
}

fn validate_profile_version_input(
    persona: &Value,
    runtime: &Value,
    presentation: &Value,
    source: &Value,
    capabilities: &[ProfileCapabilityDraft],
) -> Result<(), ApiError> {
    validate_json_object("persona", persona, 512 * 1024)?;
    validate_json_object("runtime", runtime, 128 * 1024)?;
    validate_json_object("presentation", presentation, 256 * 1024)?;
    validate_json_object("source", source, 128 * 1024)?;
    if capabilities.len() > 64 {
        return Err(ApiError::Invalid(
            "a profile version supports at most 64 capabilities".to_string(),
        ));
    }
    if let Some(files) = persona.get("files") {
        let files = files
            .as_object()
            .ok_or_else(|| ApiError::Invalid("persona files must be an object".to_string()))?;
        if files.len() > 64 {
            return Err(ApiError::Invalid(
                "a profile version supports at most 64 persona files".to_string(),
            ));
        }
        for (name, content) in files {
            if name.is_empty() || name.len() > 128 || name.contains('\0') {
                return Err(ApiError::Invalid(
                    "persona file name is invalid".to_string(),
                ));
            }
            if !content.is_string() {
                return Err(ApiError::Invalid(format!(
                    "persona file {name} must contain text"
                )));
            }
        }
    }
    let mut capability_kinds = HashSet::with_capacity(capabilities.len());
    for capability in capabilities {
        if !matches!(
            capability.kind.as_str(),
            "chat" | "embedding" | "speech" | "transcription" | "realtime" | "tool"
        ) {
            return Err(ApiError::Invalid(format!(
                "unsupported profile capability {}",
                capability.kind
            )));
        }
        required_identifier("provider type", &capability.provider_type)?;
        required_identifier("provider key", &capability.provider_key)?;
        if !capability_kinds.insert(capability.kind.as_str()) {
            return Err(ApiError::Invalid(format!(
                "profile capability {} is duplicated",
                capability.kind
            )));
        }
        if let Some(resource_id) = capability.resource_id.as_deref() {
            required_text("resource id", resource_id, 512)?;
        }
        validate_json_object("capability config", &capability.config, 128 * 1024)?;
        validate_json_object(
            "capability input schema",
            &capability.input_schema,
            128 * 1024,
        )?;
        validate_json_object(
            "capability output schema",
            &capability.output_schema,
            128 * 1024,
        )?;
    }
    Ok(())
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
        "vifu_pk_{}{}",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    )
}

fn api_error_trace_status(error: &ApiError) -> &'static str {
    match error {
        ApiError::AgentGatewayUnavailable => "unavailable",
        ApiError::Backpressure => "rejected",
        ApiError::Timeout => "timed_out",
        _ => "failed",
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
    use serde_json::json;

    use super::{
        api_error_trace_status, chat_request_summary, chat_trace_request,
        embedding_request_summary, embedding_response, gateway_binding_config, merge_json_objects,
        patch_text, prepare_project_provider_assignment_with_secret_key, profile_slug,
        project_slug, validate_chat_completion_request, validate_embedding_request,
        validate_profile_version_input, validate_timeout, validated_provider_base_url,
    };
    use crate::error::ApiError;
    use crate::models::ProfileCapabilityDraft;

    #[test]
    fn derives_profile_slugs() {
        assert_eq!(profile_slug(None, "Town Guide").unwrap(), "town-guide");
    }

    #[test]
    fn preserves_transport_failure_statuses_for_profile_traces() {
        assert_eq!(api_error_trace_status(&ApiError::Timeout), "timed_out");
        assert_eq!(
            api_error_trace_status(&ApiError::AgentGatewayUnavailable),
            "unavailable"
        );
        assert_eq!(api_error_trace_status(&ApiError::Backpressure), "rejected");
        assert_eq!(
            api_error_trace_status(&ApiError::Invalid("invalid".to_string())),
            "failed"
        );
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
    fn project_provider_configuration_overrides_its_source() {
        let merged = merge_json_objects(
            &json!({ "model": "source-model", "timeout": 10 }),
            &json!({ "model": "project-model" }),
        )
        .unwrap();

        assert_eq!(merged, json!({ "model": "project-model", "timeout": 10 }));
    }

    #[test]
    fn project_provider_assignment_does_not_store_runtime_settings() {
        let assignment = prepare_project_provider_assignment_with_secret_key(
            "vifu-local-provider-secret-key",
            "local-qwen",
            "Local Qwen",
            "openai-compatible",
        )
        .unwrap();

        assert_eq!(assignment.key, "local-qwen");
        assert_eq!(assignment.name, "Local Qwen");
        assert_eq!(assignment.provider_type, "openai-compatible");
        assert!(assignment.base_url.is_empty());
        assert_eq!(assignment.config, json!({}));
        assert!(assignment.secret_keys.is_empty());
        assert!(assignment.display_secret.is_none());
    }

    #[test]
    fn llama_provider_accepts_an_empty_base_url() {
        assert!(validated_provider_base_url("llama", "").unwrap().is_empty());
    }

    #[test]
    fn gateway_binding_carries_the_profile_persona() {
        let binding = gateway_binding_config(
            json!({ "temperature": 0 }),
            "local-qwen",
            "chat",
            &json!({ "instructions": "Choose one safe action." }),
        )
        .unwrap();

        assert_eq!(
            binding["persona"]["instructions"],
            "Choose one safe action."
        );
        assert_eq!(binding["capability"], "chat");
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

    #[test]
    fn rejects_non_text_persona_files() {
        let error = validate_profile_version_input(
            &json!({ "files": { "SOUL.md": { "unexpected": true } } }),
            &json!({}),
            &json!({}),
            &json!({}),
            &[],
        )
        .unwrap_err();

        assert!(error.to_string().contains("SOUL.md must contain text"));
    }

    #[test]
    fn rejects_duplicate_capability_routes() {
        let capability = ProfileCapabilityDraft {
            kind: "chat".to_string(),
            provider_type: "openai-compatible".to_string(),
            provider_key: "primary-model".to_string(),
            resource_id: Some("model-small".to_string()),
            config: json!({}),
            input_schema: json!({}),
            output_schema: json!({}),
        };
        let error = validate_profile_version_input(
            &json!({}),
            &json!({}),
            &json!({}),
            &json!({}),
            &[capability.clone(), capability],
        )
        .unwrap_err();

        assert!(error.to_string().contains("chat is duplicated"));
    }

    #[test]
    fn accepts_embedding_profile_capabilities() {
        let capability = ProfileCapabilityDraft {
            kind: "embedding".to_string(),
            provider_type: "openai-compatible".to_string(),
            provider_key: "local-embeddings".to_string(),
            resource_id: Some("small-embedding-model".to_string()),
            config: json!({}),
            input_schema: json!({}),
            output_schema: json!({}),
        };

        validate_profile_version_input(
            &json!({}),
            &json!({}),
            &json!({}),
            &json!({}),
            &[capability],
        )
        .unwrap();
    }

    #[test]
    fn validates_and_summarizes_embedding_requests_without_retaining_text() {
        let request = json!({
            "model": "farm-embedding",
            "input": ["parsnip", "watering can"],
            "encoding_format": "float",
        });

        validate_embedding_request(&request).unwrap();
        let summary = embedding_request_summary(&request);

        assert_eq!(summary["inputCount"], 2);
        assert_eq!(summary["textBytes"], 19);
        assert!(!summary.to_string().contains("parsnip"));
    }

    #[test]
    fn normalizes_embedding_responses_to_the_project_profile_model() {
        let response = embedding_response(
            "stardew-valley-farming-0",
            json!({
                "data": [{"object": "embedding", "index": 0, "embedding": [0.6, 0.8]}],
                "model": "physical-model",
                "usage": {"prompt_tokens": 2, "total_tokens": 2}
            }),
        )
        .unwrap();

        assert_eq!(response["object"], "list");
        assert_eq!(response["model"], "stardew-valley-farming-0");
    }

    #[test]
    fn accepts_bounded_multimodal_chat_requests_larger_than_the_text_limit() {
        let image = "A".repeat(600 * 1024);
        let request = json!({
            "model": "farm-vision",
            "messages": [{
                "role": "user",
                "content": [{
                    "type": "image_url",
                    "image_url": { "url": format!("data:image/jpeg;base64,{image}") }
                }]
            }]
        });

        validate_chat_completion_request(&request).unwrap();
    }

    #[test]
    fn omits_image_payloads_from_chat_traces() {
        let request = json!({
            "model": "farm-vision",
            "messages": [{
                "role": "user",
                "content": [
                    { "type": "text", "text": "What is here?" },
                    {
                        "type": "image_url",
                        "image_url": { "url": "data:image/png;base64,AQID" }
                    }
                ]
            }]
        });

        let trace = chat_trace_request(&request);

        assert!(!trace.to_string().contains("AQID"));
        assert_eq!(
            trace["messages"][0]["content"][1]["image_url"]["trace"]["decodedBytes"],
            3
        );
    }

    #[test]
    fn summarizes_multimodal_chat_without_retaining_image_data() {
        let request = json!({
            "model": "farm-vision",
            "messages": [{
                "role": "user",
                "content": [{
                    "type": "image_url",
                    "image_url": { "url": "data:image/png;base64,AQID" }
                }]
            }]
        });

        let summary = chat_request_summary(&request);

        assert_eq!(summary["imageCount"], 1);
        assert_eq!(summary["mediaBytes"], 3);
    }
}
