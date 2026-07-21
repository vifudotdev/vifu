use std::collections::{BTreeMap, BTreeSet, HashSet};

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use vifu_game_runtime::{
    canonical_json_bytes, localization_source_hash, GameCommand, TranslationPackStatus,
    TranslationPackV1, ValidationSeverity,
};

use crate::auth::require_admin;
use crate::db as runtime_db;
use crate::error::ApiError;
use crate::AppState;

use super::db;
use super::models::{
    CreateGameAsset, CreateGameBuild, CreateGameResource, PublicRuntimeAdvance,
    PublishGamePresentation, RunGame, UpdateGameResource,
};
use super::service;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TranslateGameMessages {
    pub provider_key: String,
    pub model: String,
    pub source_locale: String,
    pub target_locales: Vec<String>,
    pub messages: BTreeMap<String, String>,
}

pub async fn translate_messages(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(input): Json<TranslateGameMessages>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&headers, &state.config.admin_key)?;
    runtime_db::get_project_by_slug(&state.pool, &slug).await?;
    let provider_key = required_authoring_value("provider", &input.provider_key, 128)?;
    let model = required_authoring_value("model", &input.model, 256)?;
    let source_locale = required_authoring_value("source locale", &input.source_locale, 32)?;
    if input.messages.is_empty() || input.messages.len() > 500 {
        return Err(ApiError::Invalid(
            "translation requires between 1 and 500 messages".to_string(),
        ));
    }
    if input
        .messages
        .iter()
        .any(|(key, message)| key.trim().is_empty() || message.trim().is_empty())
    {
        return Err(ApiError::Invalid(
            "translation message IDs and source text cannot be empty".to_string(),
        ));
    }
    let source_characters: usize = input.messages.values().map(String::len).sum();
    if source_characters > 100_000 {
        return Err(ApiError::Invalid(
            "translation source exceeds 100,000 characters".to_string(),
        ));
    }
    let mut targets = BTreeSet::new();
    for locale in &input.target_locales {
        let locale = required_authoring_value("target locale", locale, 32)?;
        if locale == source_locale || !targets.insert(locale.to_string()) {
            return Err(ApiError::Invalid(
                "target locales must be unique and different from the source locale".to_string(),
            ));
        }
    }
    if targets.is_empty() || targets.len() > 8 {
        return Err(ApiError::Invalid(
            "translation requires between 1 and 8 target locales".to_string(),
        ));
    }

    let source_hash = localization_source_hash(&input.messages);
    let mut packs = BTreeMap::new();
    for locale in targets {
        let request = translation_model_request(source_locale, &locale, &input.messages)?;
        let response = crate::api::invoke_project_authoring_model(
            &state,
            &slug,
            provider_key,
            model,
            &request,
        )
        .await?;
        let translated = parse_translation_response(&response, &input.messages)?;
        packs.insert(
            locale,
            TranslationPackV1 {
                source_hash: source_hash.clone(),
                status: TranslationPackStatus::Draft,
                messages: translated,
            },
        );
    }
    Ok(Json(json!({"sourceHash": source_hash, "packs": packs})))
}

fn translation_model_request(
    source_locale: &str,
    target_locale: &str,
    messages: &BTreeMap<String, String>,
) -> Result<Value, ApiError> {
    let input = serde_json::to_string(&json!({
        "sourceLocale": source_locale,
        "targetLocale": target_locale,
        "messages": messages,
    }))
    .map_err(|_| ApiError::Internal)?;
    Ok(json!({
        "messages": [
            {
                "role": "system",
                "content": "Translate interactive game dialogue accurately and naturally. Preserve character names, placeholders, punctuation intent, and every source message ID. Return valid json only: one object whose keys exactly match the source message IDs and whose values are translated strings."
            },
            {"role": "user", "content": format!("Translate this input and return json:\n{input}")}
        ],
        "response_format": {"type": "json_object"},
        "stream": false
    }))
}

fn required_authoring_value<'a>(
    label: &str,
    value: &'a str,
    maximum: usize,
) -> Result<&'a str, ApiError> {
    let value = value.trim();
    if value.is_empty() || value.len() > maximum {
        return Err(ApiError::Invalid(format!(
            "{label} must contain between 1 and {maximum} characters"
        )));
    }
    Ok(value)
}

fn parse_translation_response(
    response: &Value,
    source: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, ApiError> {
    let content = response
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::Provider("translation model returned no content".to_string()))?;
    let value: Value = serde_json::from_str(content)
        .map_err(|_| ApiError::Provider("translation model returned invalid JSON".to_string()))?;
    let object = value.as_object().ok_or_else(|| {
        ApiError::Provider("translation model must return a JSON object".to_string())
    })?;
    if object.len() != source.len() || source.keys().any(|key| !object.contains_key(key)) {
        return Err(ApiError::Provider(
            "translation model changed or omitted message IDs".to_string(),
        ));
    }
    object
        .iter()
        .map(|(key, value)| {
            let message = value
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    ApiError::Provider(format!("translation `{key}` is empty or not text"))
                })?;
            Ok((key.clone(), message.to_string()))
        })
        .collect()
}

pub async fn list_resources(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&headers, &state.config.admin_key)?;
    let project = runtime_db::get_project_by_slug(&state.pool, &slug).await?;
    let resources = db::list_game_resources(&state.pool, project.project.id).await?;
    Ok(Json(json!({"resources": resources})))
}

pub async fn create_resource(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(input): Json<CreateGameResource>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    require_admin(&headers, &state.config.admin_key)?;
    let project = runtime_db::get_project_by_slug(&state.pool, &slug).await?;
    let name = service::validate_resource_name(&input.name)?;
    let resource_key = service::normalize_resource_key(input.resource_key.as_deref(), &name)?;
    let kind = service::validate_resource_kind(&input.kind)?;
    let content_hash = service::resource_content_hash(&input.content)?;
    let resource = db::create_game_resource(
        &state.pool,
        db::NewGameResource {
            project_id: project.project.id,
            resource_key: &resource_key,
            name: &name,
            kind: &kind,
            content: &input.content,
            content_hash: &content_hash,
            approved: input.approved,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(json!({"resource": resource}))))
}

pub async fn update_resource(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((slug, resource_id)): Path<(String, Uuid)>,
    Json(input): Json<UpdateGameResource>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    require_admin(&headers, &state.config.admin_key)?;
    let project = runtime_db::get_project_by_slug(&state.pool, &slug).await?;
    let current =
        db::get_game_resource_version(&state.pool, project.project.id, resource_id).await?;
    let name = match input.name {
        Some(name) => service::validate_resource_name(&name)?,
        None => current.name.clone(),
    };
    let kind = match input.kind {
        Some(kind) => service::validate_resource_kind(&kind)?,
        None => current.kind.clone(),
    };
    let content = input.content.unwrap_or_else(|| current.content.clone());
    let content_hash = service::resource_content_hash(&content)?;
    let resource = db::create_game_resource_version(
        &state.pool,
        &current,
        &name,
        &kind,
        &content,
        &content_hash,
        input.approved.unwrap_or(current.approved),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(json!({"resource": resource}))))
}

pub async fn delete_resource(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((slug, resource_id)): Path<(String, Uuid)>,
) -> Result<StatusCode, ApiError> {
    require_admin(&headers, &state.config.admin_key)?;
    let project = runtime_db::get_project_by_slug(&state.pool, &slug).await?;
    let resource =
        db::get_game_resource_version(&state.pool, project.project.id, resource_id).await?;
    let draft = service::ensure_draft(&state.pool, &project).await?;
    if draft
        .source
        .resources
        .iter()
        .any(|reference| reference.id == resource.resource_key)
    {
        return Err(ApiError::Conflict(
            "remove this resource from the Game draft before deleting it".to_string(),
        ));
    }
    db::delete_game_resource(&state.pool, project.project.id, resource_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_assets(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&headers, &state.config.admin_key)?;
    let project = runtime_db::get_project_by_slug(&state.pool, &slug).await?;
    let assets = db::list_game_assets(&state.pool, project.project.id).await?;
    Ok(Json(json!({"assets": assets})))
}

pub async fn create_asset(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(input): Json<CreateGameAsset>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    require_admin(&headers, &state.config.admin_key)?;
    let project = runtime_db::get_project_by_slug(&state.pool, &slug).await?;
    let name = service::validate_resource_name(&input.name)?;
    let asset_key = service::normalize_resource_key(input.asset_key.as_deref(), &name)?;
    let kind = service::validate_resource_kind(&input.kind)?;
    let asset =
        db::create_game_asset(&state.pool, project.project.id, &asset_key, &name, &kind).await?;
    Ok((StatusCode::CREATED, Json(json!({"asset": asset}))))
}

pub async fn delete_asset(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((slug, asset_id)): Path<(String, Uuid)>,
) -> Result<StatusCode, ApiError> {
    require_admin(&headers, &state.config.admin_key)?;
    let project = runtime_db::get_project_by_slug(&state.pool, &slug).await?;
    let in_presentation = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (
            SELECT 1 FROM game_presentation_releases AS presentation
            JOIN game_asset_versions AS version
              ON version.id = ANY(presentation.asset_version_ids)
            WHERE presentation.project_id = $1 AND version.asset_id = $2
         )",
    )
    .bind(project.project.id)
    .bind(asset_id)
    .fetch_one(&state.pool)
    .await?;
    if in_presentation {
        return Err(ApiError::Conflict(
            "asset versions used by a Presentation release cannot be deleted".to_string(),
        ));
    }
    db::delete_game_asset(&state.pool, project.project.id, asset_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn create_build(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(input): Json<CreateGameBuild>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    require_admin(&headers, &state.config.admin_key)?;
    let project = runtime_db::get_project_by_slug(&state.pool, &slug).await?;
    let draft = service::ensure_draft(&state.pool, &project).await?;
    let kind = input.kind.as_deref().unwrap_or("compile");
    if kind != "compile" {
        return Err(ApiError::Invalid(
            "the V1 build kind must be `compile`".to_string(),
        ));
    }
    let compiled = service::compile_for_project(&state.pool, &project, &draft.source).await?;
    let resources =
        db::backend_resource_snapshot(&state.pool, project.project.id, &compiled.plan.resources)
            .await?;
    let output = json!({
        "contentHash": compiled.content_hash,
        "manifest": compiled.manifest,
        "resourceCount": resources.len(),
        "warnings": compiled.warnings,
    });
    let build = db::create_completed_game_build(
        &state.pool,
        project.project.id,
        draft.revision,
        kind,
        &draft.content_hash,
        &output,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(json!({"build": build}))))
}

pub async fn get_build(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((slug, build_id)): Path<(String, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&headers, &state.config.admin_key)?;
    let project = runtime_db::get_project_by_slug(&state.pool, &slug).await?;
    let build = db::get_game_build(&state.pool, project.project.id, build_id).await?;
    Ok(Json(json!({"build": build})))
}

pub async fn cancel_build(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((slug, build_id)): Path<(String, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&headers, &state.config.admin_key)?;
    let project = runtime_db::get_project_by_slug(&state.pool, &slug).await?;
    let build = db::cancel_game_build(&state.pool, project.project.id, build_id).await?;
    Ok(Json(json!({"build": build})))
}

pub async fn preview_game(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(input): Json<RunGame>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    require_admin(&headers, &state.config.admin_key)?;
    let project = runtime_db::get_project_by_slug(&state.pool, &slug).await?;
    let draft = service::ensure_draft(&state.pool, &project).await?;
    let compiled = service::compile_for_project(&state.pool, &project, &draft.source).await?;
    let missing_optional = service::validate_host(&compiled.manifest, &input.host)?;
    let seed_id = Uuid::new_v4();
    let random_seed = db::random_seed_from_session(seed_id);
    let session = db::create_preview_game_session(
        &state.pool,
        project.project.id,
        draft.revision,
        &compiled.plan,
        &input.host,
        random_seed,
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
            "draftRevision": draft.revision,
            "hostEngine": input.host.engine,
            "hostCapabilityCount": input.host.capabilities.len(),
            "preview": true,
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
                .unwrap_or_else(|| format!("preview:{}", Uuid::new_v4())),
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
            "draftRevision": draft.revision,
            "status": advance.snapshot.status,
            "revision": advance.snapshot.revision,
            "eventCount": advance.events.len(),
            "effectCount": advance.effects.len(),
            "preview": true,
        }),
    )
    .await;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "sessionId": session.id,
            "draftRevision": draft.revision,
            "advance": PublicRuntimeAdvance::from(&advance),
            "optionalCapabilityFallbacks": missing_optional,
        })),
    ))
}

pub async fn game_qa(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&headers, &state.config.admin_key)?;
    let project = runtime_db::get_project_by_slug(&state.pool, &slug).await?;
    let draft = service::ensure_draft(&state.pool, &project).await?;
    let (_, issues) = service::validate_for_project(&state.pool, &project, &draft.source).await?;
    let blocker_count = issues
        .iter()
        .filter(|issue| issue.severity == ValidationSeverity::Error)
        .count();
    let warning_count = issues.len().saturating_sub(blocker_count);
    let (required_host_capabilities, optional_host_capabilities) = if blocker_count == 0 {
        let compiled = service::compile_for_project(&state.pool, &project, &draft.source).await?;
        (
            compiled.manifest.required_host_capabilities,
            compiled.manifest.optional_host_capabilities,
        )
    } else {
        (Vec::new(), Vec::new())
    };
    Ok(Json(json!({
        "qa": {
            "draftRevision": draft.revision,
            "ready": blocker_count == 0,
            "blockerCount": blocker_count,
            "warningCount": warning_count,
            "issues": issues,
            "requiredHostCapabilities": required_host_capabilities,
            "optionalHostCapabilities": optional_host_capabilities,
        }
    })))
}

pub async fn game_analytics(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&headers, &state.config.admin_key)?;
    let project = runtime_db::get_project_by_slug(&state.pool, &slug).await?;
    let (events, statuses, total_sessions, average_duration_ms) =
        db::game_analytics(&state.pool, project.project.id).await?;
    let sessions = db::list_published_game_sessions(&state.pool, project.project.id, 25).await?;
    Ok(Json(json!({
        "analytics": {
            "totalSessions": total_sessions,
            "averageDurationMs": average_duration_ms,
            "events": events,
            "sessionStatuses": statuses,
            "recentSessions": sessions,
        }
    })))
}

pub async fn list_sessions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&headers, &state.config.admin_key)?;
    let project = runtime_db::get_project_by_slug(&state.pool, &slug).await?;
    let sessions = db::list_game_sessions(&state.pool, project.project.id, 100).await?;
    Ok(Json(json!({"sessions": sessions})))
}

pub async fn get_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((slug, session_id)): Path<(String, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&headers, &state.config.admin_key)?;
    let project = runtime_db::get_project_by_slug(&state.pool, &slug).await?;
    let session = db::get_game_session(&state.pool, project.project.id, session_id).await?;
    let events = db::list_game_events_after(&state.pool, session.id, 0, 1_000).await?;
    Ok(Json(json!({"session": session, "events": events})))
}

pub async fn publish_presentation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(input): Json<PublishGamePresentation>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    require_admin(&headers, &state.config.admin_key)?;
    let project = runtime_db::get_project_by_slug(&state.pool, &slug).await?;
    let release = match input.game_release_id {
        Some(release_id) => {
            db::get_game_release(&state.pool, project.project.id, release_id).await?
        }
        None => db::active_game_release(&state.pool, project.project.id)
            .await?
            .ok_or_else(|| {
                ApiError::Conflict("publish a Game release before its Presentation".to_string())
            })?,
    };
    if input.binding_manifest.schema_version != 1 || input.binding_manifest.engine.trim().is_empty()
    {
        return Err(ApiError::Invalid(
            "bindingManifest must use schemaVersion 1 and name its host engine".to_string(),
        ));
    }
    let unique_assets: HashSet<_> = input.asset_version_ids.iter().copied().collect();
    if unique_assets.len() != input.asset_version_ids.len() {
        return Err(ApiError::Invalid(
            "assetVersionIds must not contain duplicates".to_string(),
        ));
    }
    validate_presentation_bindings(&release.manifest, &input.binding_manifest, &unique_assets)?;
    for version_id in &input.asset_version_ids {
        let version =
            db::get_game_asset_version(&state.pool, project.project.id, *version_id).await?;
        if version.approval_status != "approved" {
            return Err(ApiError::Invalid(format!(
                "asset version {version_id} is not approved"
            )));
        }
    }
    let hash_input = json!({
        "gameContentHash": release.content_hash,
        "bindingManifest": input.binding_manifest,
        "assetVersionIds": input.asset_version_ids,
    });
    let bytes =
        canonical_json_bytes(&hash_input).map_err(|error| ApiError::Invalid(error.to_string()))?;
    let content_hash = format!("sha256:{:x}", Sha256::digest(bytes));
    let presentation = db::publish_game_presentation(
        &state.pool,
        project.project.id,
        release.id,
        &content_hash,
        &input.binding_manifest,
        &input.asset_version_ids,
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"presentation": presentation})),
    ))
}

fn validate_presentation_bindings(
    manifest: &vifu_game_runtime::GameManifestV1,
    bindings: &vifu_game_runtime::HostBindingManifestV1,
    asset_version_ids: &HashSet<Uuid>,
) -> Result<(), ApiError> {
    let logical_resources = manifest
        .logical_resources
        .iter()
        .map(|resource| resource.id.as_str())
        .collect::<HashSet<_>>();
    let mut referenced_assets = HashSet::new();
    for (logical_id, binding) in &bindings.bindings {
        if !logical_resources.contains(logical_id.as_str()) {
            return Err(ApiError::Invalid(format!(
                "presentation binding `{logical_id}` is not declared by the Game manifest"
            )));
        }
        if binding.kind == "managed-asset-version" {
            let version_id = binding
                .value
                .as_str()
                .ok_or_else(|| {
                    ApiError::Invalid(format!(
                        "managed binding `{logical_id}` must contain an asset version ID"
                    ))
                })?
                .parse::<Uuid>()
                .map_err(|_| {
                    ApiError::Invalid(format!(
                        "managed binding `{logical_id}` has an invalid asset version ID"
                    ))
                })?;
            if !asset_version_ids.contains(&version_id) {
                return Err(ApiError::Invalid(format!(
                    "managed binding `{logical_id}` is missing from assetVersionIds"
                )));
            }
            referenced_assets.insert(version_id);
        }
    }
    if &referenced_assets != asset_version_ids {
        return Err(ApiError::Invalid(
            "assetVersionIds must contain exactly the versions used by managed bindings"
                .to_string(),
        ));
    }
    Ok(())
}

pub async fn list_presentations(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&headers, &state.config.admin_key)?;
    let project = runtime_db::get_project_by_slug(&state.pool, &slug).await?;
    let presentations = db::list_game_presentations(&state.pool, project.project.id).await?;
    Ok(Json(json!({"presentations": presentations})))
}

pub async fn activate_presentation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((slug, presentation_id)): Path<(String, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&headers, &state.config.admin_key)?;
    let project = runtime_db::get_project_by_slug(&state.pool, &slug).await?;
    let presentation =
        db::activate_game_presentation(&state.pool, project.project.id, presentation_id).await?;
    Ok(Json(json!({"presentation": presentation})))
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashSet};

    use serde_json::json;
    use uuid::Uuid;
    use vifu_game_runtime::{
        GameCompiler, GameSourceV1, HostBinding, HostBindingManifestV1,
        LogicalPresentationResource, PortReference, SourceEdge, SourceNode,
    };

    use super::{
        parse_translation_response, translation_model_request, validate_presentation_bindings,
    };
    use crate::error::ApiError;

    fn manifest_with_resource(resource_id: &str) -> vifu_game_runtime::GameManifestV1 {
        let mut source = GameSourceV1::new("Presentation test");
        source.graph.nodes.push(SourceNode {
            id: "end".to_string(),
            node_type: "end".to_string(),
            version: 1,
            config: json!({"endingId": "presentation-test"}),
            parent_id: None,
            label: Some("End".to_string()),
            notes: None,
        });
        source.graph.edges.push(SourceEdge {
            id: "start-end".to_string(),
            source: PortReference {
                node_id: "start".to_string(),
                port: "next".to_string(),
            },
            target: PortReference {
                node_id: "end".to_string(),
                port: "in".to_string(),
            },
            condition: None,
            managed_by: None,
        });
        source
            .presentation_resources
            .push(LogicalPresentationResource {
                id: resource_id.to_string(),
                kind: "image".to_string(),
                required_capabilities: Vec::new(),
                required: false,
                fallback: None,
            });
        GameCompiler::default()
            .compile(&source)
            .expect("test source should compile")
            .manifest
    }

    fn managed_bindings(resource_id: &str, version_id: Uuid) -> HostBindingManifestV1 {
        HostBindingManifestV1 {
            schema_version: 1,
            engine: "web".to_string(),
            adapter_version: Some("vifu-reference-v1".to_string()),
            bindings: BTreeMap::from([(
                resource_id.to_string(),
                HostBinding {
                    kind: "managed-asset-version".to_string(),
                    value: json!(version_id),
                },
            )]),
        }
    }

    #[test]
    fn accepts_declared_managed_asset_binding() {
        let version_id = Uuid::new_v4();
        let result = validate_presentation_bindings(
            &manifest_with_resource("scene.background"),
            &managed_bindings("scene.background", version_id),
            &HashSet::from([version_id]),
        );

        assert!(result.is_ok());
    }

    #[test]
    fn rejects_binding_for_undeclared_logical_resource() {
        let version_id = Uuid::new_v4();
        let result = validate_presentation_bindings(
            &manifest_with_resource("scene.background"),
            &managed_bindings("scene.portrait", version_id),
            &HashSet::from([version_id]),
        );

        assert!(matches!(
            result,
            Err(ApiError::Invalid(message)) if message.contains("not declared")
        ));
    }

    #[test]
    fn rejects_asset_versions_not_referenced_by_managed_bindings() {
        let bound_version_id = Uuid::new_v4();
        let extra_version_id = Uuid::new_v4();
        let result = validate_presentation_bindings(
            &manifest_with_resource("scene.background"),
            &managed_bindings("scene.background", bound_version_id),
            &HashSet::from([bound_version_id, extra_version_id]),
        );

        assert!(matches!(
            result,
            Err(ApiError::Invalid(message)) if message.contains("exactly the versions")
        ));
    }

    #[test]
    fn accepts_translation_objects_only_when_message_ids_are_preserved() {
        let source = BTreeMap::from([
            ("opening.title".to_string(), "月行き最終列車".to_string()),
            ("opening.line".to_string(), "列车到了。".to_string()),
        ]);
        let response = json!({
            "choices": [{
                "message": {
                    "content": "{\"opening.title\":\"月行き最終列車\",\"opening.line\":\"列車が到着した。\"}"
                }
            }]
        });

        let translated = parse_translation_response(&response, &source).expect("valid translation");
        assert_eq!(translated["opening.line"], "列車が到着した。");

        let missing = json!({
            "choices": [{"message": {"content": "{\"opening.title\":\"Last Train\"}"}}]
        });
        assert!(matches!(
            parse_translation_response(&missing, &source),
            Err(ApiError::Provider(message)) if message.contains("omitted")
        ));
    }

    #[test]
    fn translation_json_mode_is_explicit_in_the_prompt() {
        let request = translation_model_request(
            "en",
            "ja",
            &BTreeMap::from([("opening.title".to_string(), "Last Train".to_string())]),
        )
        .expect("translation request");
        let prompt = request
            .pointer("/messages/1/content")
            .and_then(serde_json::Value::as_str)
            .expect("user prompt");

        assert!(prompt.contains("json"));
        assert_eq!(
            request.pointer("/response_format/type"),
            Some(&json!("json_object"))
        );
    }
}
