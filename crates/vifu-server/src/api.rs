use std::collections::{BTreeMap, HashSet};
use std::path::Path as FsPath;
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
use vifu_gateway::config::{
    AgentProviderAuthDefinition, AgentProviderDefinition, AgentProvidersFile,
};
use vifu_gateway::protocol::validate_identifier;

use crate::auth::{
    bearer_token, decrypt_secret_json, encrypt_secret_json, hash_agent_gateway_credential,
    hash_api_key, is_secret_match,
};
use crate::config::DeploymentMode;
use crate::db::{self, EndpointPatch, NewEndpoint, NewProject, ProfilePatch, ProjectPatch};
use crate::error::ApiError;
use crate::models::{
    slugify, validate_slug, AgentEndpoint, ApiKeyAgentScope, ApiKeyPermissions, ApiKeyRecord,
    Capabilities, CreateApiKey, CreateBinding, CreateEndpoint, CreateProfile, CreateProfileVersion,
    CreateProject, CreateProjectProvider, CreatedApiKey, CustomProvider, CustomProviderSecret,
    EndpointRoute, ImportProjectAgent, ImportProjectProfile, ImportProjectProvider,
    ProfileCapabilityDraft, ProviderAdapter, ProviderAdapterField, ProviderConnection,
    ProviderConnectionSecret, RegisterAgentGateway, SetProfileRollout, SyncProfileSource,
    TestProfile, UpdateApiKey, UpdateBinding, UpdateEndpoint, UpdateProfile, UpdateProject,
    UpdateProjectProvider, UpsertProviderConnection,
};
use crate::openclaw_device;
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
    let gateway_id = format!("project-{slug}");
    let project = db::create_project(
        &state.pool,
        NewProject {
            id: Uuid::new_v4(),
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
    validate_project_profile_providers(&state, project_id, &input.source, &input.capabilities)
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
            json!({ "profile": profile, "version": profile_version_payload(&state, version).await? }),
        ),
    ))
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

pub async fn list_project_profiles(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_slug): Path<String>,
) -> Result<Json<Value>, ApiError> {
    admin(&state, &headers).await?;
    let project = db::get_project_by_slug(&state.pool, &project_slug).await?;
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
    let project = db::get_project_by_slug(&state.pool, &project_slug).await?;
    input.project_id = Some(project.project.id);
    create_profile(State(state), headers, Json(input)).await
}

pub async fn import_project_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_slug): Path<String>,
    Json(input): Json<ImportProjectProfile>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    admin(&state, &headers).await?;
    let project = db::get_project_by_slug(&state.pool, &project_slug).await?;
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
    admin(&state, &headers).await?;
    let project = db::get_project_by_slug(&state.pool, &project_slug).await?;
    let profile = db::get_project_profile(&state.pool, project.project.id, profile_id).await?;
    Ok(Json(profile_detail_payload(&state, profile).await?))
}

pub async fn update_project_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((project_slug, profile_id)): Path<(String, Uuid)>,
    Json(input): Json<UpdateProfile>,
) -> Result<Json<Value>, ApiError> {
    admin(&state, &headers).await?;
    let project = db::get_project_by_slug(&state.pool, &project_slug).await?;
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
    admin(&state, &headers).await?;
    let project = db::get_project_by_slug(&state.pool, &project_slug).await?;
    db::archive_project_profile(&state.pool, project.project.id, profile_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn create_project_profile_version(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((project_slug, profile_id)): Path<(String, Uuid)>,
    Json(input): Json<CreateProfileVersion>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    admin(&state, &headers).await?;
    let project = db::get_project_by_slug(&state.pool, &project_slug).await?;
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
    admin(&state, &headers).await?;
    let project = db::get_project_by_slug(&state.pool, &project_slug).await?;
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
    admin(&state, &headers).await?;
    let project = db::get_project_by_slug(&state.pool, &project_slug).await?;
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
    admin(&state, &headers).await?;
    let project = db::get_project_by_slug(&state.pool, &project_slug).await?;
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
    admin(&state, &headers).await?;
    let project = db::get_project_by_slug(&state.pool, &project_slug).await?;
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
    admin(&state, &headers).await?;
    let project = db::get_project_by_slug(&state.pool, &project_slug).await?;
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
            request: &request,
        },
    )
    .await?;
    let input_summary = json!({
        "model": profile.slug,
        "messageCount": request.get("messages").and_then(Value::as_array).map_or(0, Vec::len),
        "previewMode": preview_mode,
    });
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

pub async fn list_provider_catalog(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    admin(&state, &headers).await?;
    let custom = if let Some(path) = active_provider_registry_file(&state) {
        read_provider_registry(&path)?
            .providers
            .into_iter()
            .map(file_custom_provider)
            .collect::<Result<Vec<_>, _>>()?
    } else {
        db::list_custom_providers(&state.pool).await?
    };
    Ok(Json(
        json!({ "registry": provider_adapters(), "custom": custom }),
    ))
}

pub async fn list_project_providers(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Result<Json<Value>, ApiError> {
    admin(&state, &headers).await?;
    db::get_project_by_slug(&state.pool, &slug).await?;
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
    admin(&state, &headers).await?;
    db::get_project_by_slug(&state.pool, &slug).await?;
    let mut source =
        resolve_project_provider_source(&state, &input.source.kind, &input.source.key).await?;
    let provider_key = if source.kind == "registry" {
        let key = unique_custom_provider_key(&state, &format!("{}-{}", slug, source.key)).await?;
        let base_url = input
            .base_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ApiError::Invalid("provider base URL is required".to_string()))?;
        upsert_custom_provider_source(
            &state,
            &key,
            input.name.as_deref().unwrap_or(&source.name),
            &source.provider_type,
            base_url,
            input.config.clone(),
            input.secrets.clone(),
        )
        .await?;
        source = resolve_project_provider_source(&state, "custom", &key).await?;
        key
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
    let inherited = source.key == provider_key && input.source.kind == "registry";
    let prepared = prepare_project_provider(
        &state,
        &provider_key,
        input.name.as_deref().unwrap_or(&source.name),
        &source,
        if inherited {
            None
        } else {
            input.base_url.as_deref()
        },
        if inherited { json!({}) } else { input.config },
        if inherited { json!({}) } else { input.secrets },
    )?;
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
    admin(&state, &headers).await?;
    db::get_project_by_slug(&state.pool, &slug).await?;
    let provider_key = required_identifier("provider key", &input.provider_key)?;
    let provider_type = required_identifier("provider type", &input.provider_type)?;
    let name = required_text("provider name", &input.name, 128)?;
    let base_url = required_text("provider base URL", &input.base_url, 2048)?;
    let adapter = provider_adapters()
        .into_iter()
        .find(|adapter| adapter.id == provider_type)
        .ok_or_else(|| ApiError::Invalid(format!("unsupported provider type {provider_type}")))?;
    let source = ProjectProviderSource {
        kind: "registry".to_string(),
        key: adapter.id.clone(),
        name: adapter.name,
        provider_type: adapter.id.clone(),
        base_url: String::new(),
        config: json!({}),
    };
    let prepared = prepare_project_provider(
        &state,
        provider_key,
        name,
        &source,
        Some(base_url),
        input.config,
        json!({}),
    )?;
    let connection = db::upsert_provider_connection(
        &state.pool,
        &slug,
        db::NewProviderConnection {
            provider_key: &prepared.key,
            source_kind: "registry",
            source_key: &adapter.id,
            name: &prepared.name,
            provider_type: &prepared.provider_type,
            base_url: &prepared.base_url,
            config: &prepared.config,
            encrypted_secret_json: &prepared.encrypted_secret_json,
            secret_keys: &prepared.secret_keys,
            display_secret: prepared.display_secret.as_deref(),
            status: "needs_configuration",
        },
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "provider": effective_provider_connection(&state, connection).await?
        })),
    ))
}

pub async fn update_project_provider(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((slug, provider_key)): Path<(String, String)>,
    Json(input): Json<UpdateProjectProvider>,
) -> Result<Json<Value>, ApiError> {
    admin(&state, &headers).await?;
    let provider_key = required_identifier("provider key", &provider_key)?;
    let current =
        db::get_provider_connection_secret_by_key(&state.pool, &slug, provider_key).await?;
    let source =
        resolve_project_provider_source(&state, &current.source_kind, &current.source_key).await?;
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
    let prepared = prepare_project_provider(
        &state,
        provider_key,
        input.name.as_deref().unwrap_or(&current.name),
        &source,
        input.base_url.as_deref().or(Some(&current.base_url)),
        config,
        secrets,
    )?;
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
    admin(&state, &headers).await?;
    let provider_key = required_identifier("provider key", &provider_key)?;
    let project = db::get_project_by_slug(&state.pool, &slug).await?;
    db::unassign_project_provider(&state.pool, project.project.id, provider_key).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn test_project_provider(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((slug, provider_key)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    admin(&state, &headers).await?;
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
    admin(&state, &headers).await?;
    let project = db::get_project_by_slug(&state.pool, &slug).await?;
    let assigned = db::list_provider_connections(&state.pool, &slug)
        .await?
        .into_iter()
        .map(|provider| provider.provider_key)
        .collect::<HashSet<_>>();
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
        .filter(|agent| assigned.contains(&agent.provider_key))
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
    candidates.extend(
        db::list_available_agents(&state.pool)
            .await?
            .into_iter()
            .filter_map(|agent| {
                let provider_key = agent
                    .metadata
                    .get("providerKey")
                    .and_then(Value::as_str)?
                    .to_string();
                if !assigned.contains(&provider_key)
                    || imported.contains(&(provider_key.clone(), agent.id.clone()))
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
            }),
    );
    Ok(Json(json!({ "candidates": candidates })))
}

pub async fn import_project_agent(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(input): Json<ImportProjectAgent>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    admin(&state, &headers).await?;
    let gateway_id = required_identifier("agent gateway id", &input.gateway_id)?;
    let agent_id = required_identifier("agent id", &input.agent_id)?;
    let provider_key = required_identifier("provider key", &input.provider_key)?;
    let project = db::get_project_by_slug(&state.pool, &slug).await?;
    if !db::project_provider_is_assigned(&state.pool, project.project.id, provider_key).await? {
        return Err(ApiError::Conflict(
            "assign the provider to this project before adding its agents".to_string(),
        ));
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
    admin(&state, &headers).await?;
    let project = db::get_project_by_slug(&state.pool, &slug).await?;
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

pub async fn list_trace_spans(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(trace_id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    admin(&state, &headers).await?;
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
    let result = vifu_gateway::providers::elevenlabs_speech(
        &provider.base_url,
        provider.token.as_deref(),
        voice_id,
        &provider_request,
    )
    .await;
    match result {
        Ok(audio) => {
            let response_summary = json!({
                "bytes": audio.body.len(),
                "contentType": audio.content_type,
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
            let mut response = Response::new(Body::from(audio.body));
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_str(&audio.content_type)
                    .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
            );
            Ok(response)
        }
        Err(error) => {
            db::complete_trace_span(
                &state.pool,
                span_id,
                "failed",
                db::elapsed_millis(started_at),
                None,
                Some(&error),
            )
            .await?;
            persist_trace(&state, request_id, "failed", started_at, None, Some(&error)).await;
            Err(ApiError::Provider(error))
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
        &project_slug,
        &route,
        audio,
        &file_name,
        &content_type,
        language.as_deref(),
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

async fn transcribe_profile_audio(
    state: &AppState,
    project_slug: &str,
    route: &crate::models::ProfileRoute,
    audio: Vec<u8>,
    file_name: &str,
    content_type: &str,
    language: Option<&str>,
) -> Result<Value, ApiError> {
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
        "openai-compatible" => {
            let provider =
                resolve_runtime_provider(state, project_slug, &route.provider_key).await?;
            let model = route.resource_id.as_deref().ok_or_else(|| {
                ApiError::Invalid(
                    "transcription capability is missing a provider model".to_string(),
                )
            })?;
            vifu_gateway::providers::openai_audio_transcription(
                &provider.base_url,
                provider.token.as_deref(),
                model,
                audio,
                file_name,
                content_type,
            )
            .await
            .map_err(ApiError::Provider)
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
                            &project_slug,
                            &route,
                            audio,
                            "realtime.wav",
                            "audio/wav",
                            None,
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
            request: &request,
        },
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
            request: &request,
        },
    )
    .await?;
    let input_summary = json!({
        "model": model,
        "messageCount": request.get("messages").and_then(Value::as_array).map_or(0, Vec::len),
    });
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

async fn invoke_profile_chat(
    state: &AppState,
    project_slug: &str,
    route: &crate::models::ProfileRoute,
    request_id: Uuid,
    mut request: Value,
    timeout: Duration,
) -> Result<Value, ApiError> {
    match route.provider_type.as_str() {
        "openclaw" => {
            if route.source.get("managed").and_then(Value::as_bool) == Some(false) {
                vifu_gateway::providers::apply_persona_to_chat_request(
                    &mut request,
                    &route.persona,
                )
                .map_err(ApiError::Invalid)?;
            }
            let gateway_id = profile_gateway_id(route).ok_or_else(|| {
                ApiError::Invalid("OpenClaw capability is missing gatewayId".to_string())
            })?;
            let agent_id = route.resource_id.as_deref().ok_or_else(|| {
                ApiError::Invalid("OpenClaw capability is missing resourceId".to_string())
            })?;
            let mut binding_config = route.capability_config.clone();
            let binding_object = binding_config.as_object_mut().ok_or_else(|| {
                ApiError::Invalid("OpenClaw capability config must be an object".to_string())
            })?;
            binding_object.insert(
                "providerKey".to_string(),
                Value::String(route.provider_key.clone()),
            );
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
            state
                .relay
                .invoke(&endpoint_route, request_id, request, timeout)
                .await
                .map_err(map_relay_error)
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
            vifu_gateway::providers::openai_chat_completion(
                &provider.base_url,
                provider.token.as_deref(),
                model,
                &request,
                &route.persona,
            )
            .await
            .map_err(ApiError::Provider)
        }
        provider => Err(ApiError::Invalid(format!(
            "provider type {provider} does not support chat"
        ))),
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
        let source =
            resolve_project_provider_source(state, &connection.source_kind, &connection.source_key)
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
            merge_json_objects(
                &custom_provider_secrets(state, &connection.source_key).await?,
                &overrides,
            )?,
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
            description: "Run speech-to-text locally with models stored in ~/.vifu/models."
                .to_string(),
            capabilities: vec!["transcription".to_string()],
            execution_modes: vec!["server".to_string()],
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
            description: "Use ElevenLabs voices for speech synthesis.".to_string(),
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

fn active_provider_registry_file(state: &AppState) -> Option<std::path::PathBuf> {
    state.config.provider_registry_file.clone()
}

async fn custom_provider(state: &AppState, provider_key: &str) -> Result<CustomProvider, ApiError> {
    if let Some(path) = active_provider_registry_file(state) {
        let file = read_provider_registry(&path)?;
        let provider = file
            .providers
            .into_iter()
            .find(|provider| provider.key == provider_key)
            .ok_or(ApiError::NotFound)?;
        return file_custom_provider(provider);
    }
    Ok(
        db::get_custom_provider_secret_by_key(&state.pool, provider_key)
            .await?
            .into(),
    )
}

fn read_provider_registry(path: &FsPath) -> Result<AgentProvidersFile, ApiError> {
    vifu_gateway::config::read_provider_registry_file(path).map_err(ApiError::Invalid)
}

fn write_provider_registry(path: &FsPath, file: &AgentProvidersFile) -> Result<(), ApiError> {
    vifu_gateway::config::write_provider_registry_file(path, file).map_err(ApiError::Invalid)
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
        vifu_gateway::openclaw::parse_endpoint(&base_url).map_err(ApiError::Invalid)?;
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
        provider_key: key.clone(),
        source_kind: "custom".to_string(),
        source_key: key,
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

fn file_custom_provider(provider: AgentProviderDefinition) -> Result<CustomProvider, ApiError> {
    Ok(provider_connection_to_custom_provider(
        file_provider_connection(Uuid::nil(), provider)?,
    ))
}

fn provider_connection_to_custom_provider(connection: ProviderConnection) -> CustomProvider {
    CustomProvider {
        id: connection.id,
        provider_key: connection.provider_key,
        name: connection.name,
        provider_type: connection.provider_type,
        base_url: connection.base_url,
        config: connection.config,
        secret_keys: connection.secret_keys,
        display_secret: connection.display_secret,
        status: connection.status,
        last_checked_at: connection.last_checked_at,
        created_at: connection.created_at,
        updated_at: connection.updated_at,
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

async fn resolve_project_provider_source(
    state: &AppState,
    kind: &str,
    key: &str,
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
            let provider = custom_provider(state, key).await?;
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

async fn unique_custom_provider_key(state: &AppState, requested: &str) -> Result<String, ApiError> {
    let existing = if let Some(path) = active_provider_registry_file(state) {
        read_provider_registry(&path)?
            .providers
            .into_iter()
            .map(|provider| provider.key)
            .collect::<HashSet<_>>()
    } else {
        db::list_custom_providers(&state.pool)
            .await?
            .into_iter()
            .map(|provider| provider.provider_key)
            .collect::<HashSet<_>>()
    };
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
        "could not allocate a custom provider key".to_string(),
    ))
}

async fn upsert_custom_provider_source(
    state: &AppState,
    provider_key: &str,
    name: &str,
    provider_type: &str,
    base_url: &str,
    config: Value,
    secrets: Value,
) -> Result<(), ApiError> {
    if let Some(path) = active_provider_registry_file(state) {
        save_file_provider_connection(
            &path,
            Uuid::nil(),
            provider_key,
            UpsertProviderConnection {
                name: Some(name.to_string()),
                provider_type: provider_type.to_string(),
                base_url: base_url.to_string(),
                config,
                secrets,
            },
        )?;
        return Ok(());
    }
    let prepared = prepare_provider_connection(
        state,
        PreparedProviderSource {
            key: provider_key,
            name: Some(name),
            provider_type,
            base_url,
            config,
            secrets,
        },
    )?;
    save_custom_provider(state, prepared).await?;
    Ok(())
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
    if effective_base.is_empty() {
        return Err(ApiError::Invalid(
            "provider base URL is required".to_string(),
        ));
    }
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
    let source = match custom_provider(state, &connection.source_key).await {
        Ok(source) => source,
        Err(ApiError::NotFound) => {
            connection.status = "missing_source".to_string();
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

async fn custom_provider_secrets(state: &AppState, key: &str) -> Result<Value, ApiError> {
    if let Some(path) = active_provider_registry_file(state) {
        let definition = read_provider_registry(&path)?
            .providers
            .into_iter()
            .find(|provider| provider.key == key)
            .ok_or(ApiError::NotFound)?;
        let token = vifu_gateway::config::resolve_provider_token(&definition.key, &definition.auth)
            .map_err(ApiError::Invalid)?;
        return Ok(token.map_or_else(|| json!({}), |token| json!({ "token": token })));
    }
    let provider = db::get_custom_provider_secret_by_key(&state.pool, key).await?;
    decrypted_custom_provider_secrets(state, &provider)
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
        agent.metadata.get("providerKey").and_then(Value::as_str) == Some(provider_key)
            && agent.status == "connected"
    }) {
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
    let resolved = resolve_runtime_provider(state, project_slug, &connection.provider_key).await?;
    let (status, message) = probe_runtime_provider(
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
            let result = vifu_gateway::config::default_home_dir()
                .and_then(|home| vifu_gateway::providers::resolve_local_model_path(&home, model))
                .and_then(|path| {
                    if path.is_file() {
                        Ok(())
                    } else {
                        Err(format!(
                            "Whisper model {model} is not installed in ~/.vifu/models"
                        ))
                    }
                });
            probe_result(result)
        }
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
    let base_url = required_text("provider base URL", source.base_url, 2048)?.to_string();
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

async fn save_custom_provider(
    state: &AppState,
    input: PreparedProviderInput,
) -> Result<CustomProvider, ApiError> {
    db::upsert_custom_provider(
        &state.pool,
        db::NewProviderConnection {
            provider_key: &input.key,
            source_kind: "custom",
            source_key: &input.key,
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

fn decrypted_custom_provider_secrets(
    state: &AppState,
    provider: &CustomProviderSecret,
) -> Result<Value, ApiError> {
    let plaintext = decrypt_secret_json(
        &provider.encrypted_secret_json,
        &state.config.provider_secret_key,
    )?;
    let secrets: Value = serde_json::from_str(&plaintext)
        .map_err(|_| ApiError::Invalid("provider secrets are invalid".to_string()))?;
    validate_json_object("secrets", &secrets, 64 * 1024)?;
    Ok(secrets)
}

fn provider_token(secrets: &Value) -> Result<Option<String>, ApiError> {
    let auth = provider_auth_from_secrets(&AgentProviderAuthDefinition::default(), secrets)?;
    vifu_gateway::config::resolve_provider_token("provider", &auth).map_err(ApiError::Invalid)
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
            "chat" | "speech" | "transcription" | "realtime" | "tool"
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

fn relay_error_status(error: &RelayCallError) -> &'static str {
    match error {
        RelayCallError::AgentGatewayUnavailable => "unavailable",
        RelayCallError::Backpressure => "rejected",
        RelayCallError::Timeout => "timed_out",
        RelayCallError::AgentGateway(_) => "failed",
    }
}

fn api_error_trace_status(error: &ApiError) -> &'static str {
    match error {
        ApiError::AgentGatewayUnavailable => "unavailable",
        ApiError::Backpressure => "rejected",
        ApiError::Timeout => "timed_out",
        _ => "failed",
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
    use serde_json::json;

    use super::{
        api_error_trace_status, merge_json_objects, patch_text, profile_slug, project_slug,
        validate_profile_version_input, validate_timeout,
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
}
