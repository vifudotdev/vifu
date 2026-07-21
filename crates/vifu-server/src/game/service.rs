use std::collections::HashSet;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;
use vifu_game_runtime::{
    canonical_json_bytes, CompileOutput, GameCompiler, GameManifestV1, GameSourceV1,
    HostDescriptor, ValidationIssue, ValidationSeverity, GAME_SCHEMA_VERSION,
};

use crate::db as runtime_db;
use crate::error::ApiError;
use crate::models::ProjectWithBindings;

use super::db;
use super::models::{GameDraft, GameRelease};

const MAX_SOURCE_BYTES: usize = 4 * 1024 * 1024;
const MAX_SOURCE_NODES: usize = 2_000;
const MAX_SOURCE_EDGES: usize = 8_000;
const MAX_RESOURCE_CONTENT_BYTES: usize = 1024 * 1024;

pub async fn ensure_draft(
    pool: &PgPool,
    project: &ProjectWithBindings,
) -> Result<GameDraft, ApiError> {
    match db::get_game_draft(pool, project.project.id).await {
        Ok(draft) => Ok(draft),
        Err(ApiError::NotFound) => {
            let source = GameSourceV1::new(&project.project.name);
            let content_hash = source_hash(&source)?;
            db::ensure_game_draft(pool, project, &source, &content_hash).await
        }
        Err(error) => Err(error),
    }
}

pub fn source_hash(source: &GameSourceV1) -> Result<String, ApiError> {
    let bytes =
        canonical_json_bytes(source).map_err(|error| ApiError::Invalid(error.to_string()))?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

pub fn resource_content_hash(content: &Value) -> Result<String, ApiError> {
    let bytes =
        canonical_json_bytes(content).map_err(|error| ApiError::Invalid(error.to_string()))?;
    if bytes.len() > MAX_RESOURCE_CONTENT_BYTES {
        return Err(ApiError::Invalid(format!(
            "resource content exceeds the {MAX_RESOURCE_CONTENT_BYTES}-byte limit"
        )));
    }
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

pub fn normalize_resource_key(requested: Option<&str>, name: &str) -> Result<String, ApiError> {
    let value = requested.unwrap_or(name).trim().to_ascii_lowercase();
    let normalized: String = if requested.is_some() {
        value
    } else {
        value
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-') {
                    character
                } else {
                    '-'
                }
            })
            .collect()
    };
    let normalized = normalized
        .trim_matches(['.', '_', '-'])
        .chars()
        .take(128)
        .collect::<String>();
    if !valid_resource_key(&normalized) {
        return Err(ApiError::Invalid(
            "resourceKey must contain 2-128 lowercase letters, numbers, dots, underscores, or hyphens"
                .to_string(),
        ));
    }
    Ok(normalized)
}

pub fn validate_resource_kind(kind: &str) -> Result<String, ApiError> {
    let kind = kind.trim().to_ascii_lowercase();
    if !(2..=64).contains(&kind.len())
        || !kind
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Err(ApiError::Invalid(
            "kind must contain 2-64 lowercase letters, numbers, or underscores".to_string(),
        ));
    }
    Ok(kind)
}

pub fn validate_resource_name(name: &str) -> Result<String, ApiError> {
    let name = name.trim();
    if name.is_empty() || name.len() > 128 {
        return Err(ApiError::Invalid(
            "name must contain 1-128 characters".to_string(),
        ));
    }
    Ok(name.to_string())
}

fn valid_resource_key(value: &str) -> bool {
    (2..=128).contains(&value.len())
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        })
        && !value.contains("..")
        && !value.contains("--")
}

pub fn validate_source_limits(source: &GameSourceV1) -> Result<(), ApiError> {
    if source.schema_version != GAME_SCHEMA_VERSION {
        return Err(ApiError::Invalid(format!(
            "schemaVersion must be {GAME_SCHEMA_VERSION}"
        )));
    }
    if source.graph.nodes.len() > MAX_SOURCE_NODES {
        return Err(ApiError::Invalid(format!(
            "game source supports at most {MAX_SOURCE_NODES} nodes"
        )));
    }
    if source.graph.edges.len() > MAX_SOURCE_EDGES {
        return Err(ApiError::Invalid(format!(
            "game source supports at most {MAX_SOURCE_EDGES} edges"
        )));
    }
    let bytes =
        canonical_json_bytes(source).map_err(|error| ApiError::Invalid(error.to_string()))?;
    if bytes.len() > MAX_SOURCE_BYTES {
        return Err(ApiError::Invalid(format!(
            "game source exceeds the {MAX_SOURCE_BYTES}-byte limit"
        )));
    }
    Ok(())
}

pub async fn update_draft(
    pool: &PgPool,
    project: &ProjectWithBindings,
    source: &GameSourceV1,
    expected_revision: Option<u64>,
    expected_hash: Option<&str>,
) -> Result<GameDraft, ApiError> {
    validate_source_limits(source)?;
    ensure_draft(pool, project).await?;
    let hash = source_hash(source)?;
    db::update_game_draft(
        pool,
        project.project.id,
        source,
        &hash,
        expected_revision,
        expected_hash,
    )
    .await
}

pub async fn validate_for_project(
    pool: &PgPool,
    project: &ProjectWithBindings,
    source: &GameSourceV1,
) -> Result<(GameSourceV1, Vec<ValidationIssue>), ApiError> {
    let mut prepared = source.clone();
    let mut issues = GameCompiler::default().validate(&prepared);
    let mut stable_agent_ids = HashSet::new();
    for agent in &mut prepared.agents {
        if !stable_agent_ids.insert(agent.id.clone()) {
            continue;
        }
        let profile_id = match Uuid::parse_str(&agent.profile_id) {
            Ok(profile_id) => profile_id,
            Err(_) => {
                issues.push(
                    ValidationIssue::error(
                        "agent_profile_invalid",
                        format!("Agent `{}` has an invalid profileId", agent.id),
                    )
                    .at_path(format!("/agents/{}/profileId", agent.id)),
                );
                continue;
            }
        };
        let version_id = match agent
            .profile_version_id
            .as_deref()
            .and_then(|version| Uuid::parse_str(version).ok())
        {
            Some(version_id) => version_id,
            None => continue,
        };
        match runtime_db::resolve_profile_route(
            pool,
            project.project.id,
            &profile_id.to_string(),
            "chat",
            None,
            Some(version_id),
        )
        .await
        {
            Ok(route) => {
                agent.execution_descriptor = json!({
                    "profileId": route.profile_id,
                    "profileVersionId": route.profile_version_id,
                    "providerKey": route.provider_key,
                    "capabilityKind": route.capability_kind
                });
            }
            Err(ApiError::NotFound | ApiError::Invalid(_)) => issues.push(
                ValidationIssue::error(
                    "agent_version_unavailable",
                    format!(
                        "Agent `{}` does not resolve to an available chat-capable Profile version",
                        agent.id
                    ),
                )
                .at_path(format!("/agents/{}/profileVersionId", agent.id)),
            ),
            Err(error) => return Err(error),
        }
    }
    for node in prepared
        .graph
        .nodes
        .iter()
        .filter(|node| node.node_type == "tool")
    {
        let agent_id = node
            .config
            .get("agentId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let tool = node
            .config
            .get("tool")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if agent_id.is_empty() || tool.is_empty() {
            continue;
        }
        let Some(agent) = prepared.agents.iter().find(|agent| agent.id == agent_id) else {
            continue;
        };
        let (Ok(profile_id), Some(version_id)) = (
            Uuid::parse_str(&agent.profile_id),
            agent
                .profile_version_id
                .as_deref()
                .and_then(|value| Uuid::parse_str(value).ok()),
        ) else {
            continue;
        };
        match runtime_db::resolve_profile_route(
            pool,
            project.project.id,
            &profile_id.to_string(),
            "tool",
            None,
            Some(version_id),
        )
        .await
        {
            Ok(route) => {
                if route.provider_type != "openclaw"
                    || !profile_tool_is_available(&route.capability_config, tool)
                {
                    issues.push(
                        ValidationIssue::error(
                            "tool_unavailable",
                            format!("Tool `{tool}` is not available to Agent `{agent_id}`"),
                        )
                        .for_node(&node.id)
                        .at_path("/config/tool"),
                    );
                } else if !runtime_db::project_provider_is_assigned(
                    pool,
                    project.project.id,
                    &route.provider_key,
                )
                .await?
                {
                    issues.push(
                        ValidationIssue::error(
                            "tool_provider_unavailable",
                            format!(
                                "Agent `{agent_id}` uses provider `{}` which is not configured for this Project",
                                route.provider_key
                            ),
                        )
                        .for_node(&node.id)
                        .at_path("/config/agentId"),
                    );
                }
            }
            Err(ApiError::NotFound | ApiError::Invalid(_)) => issues.push(
                ValidationIssue::error(
                    "tool_capability_unavailable",
                    format!("Agent `{agent_id}` does not have a callable Tool capability"),
                )
                .for_node(&node.id)
                .at_path("/config/agentId"),
            ),
            Err(error) => return Err(error),
        }
    }
    for resource_reference in &prepared.resources {
        let version_id = match Uuid::parse_str(&resource_reference.version_id) {
            Ok(version_id) => version_id,
            Err(_) => {
                issues.push(
                    ValidationIssue::error(
                        "resource_version_invalid",
                        format!(
                            "Resource `{}` has an invalid versionId",
                            resource_reference.id
                        ),
                    )
                    .at_path(format!("/resources/{}/versionId", resource_reference.id)),
                );
                continue;
            }
        };
        match db::get_game_resource_version(pool, project.project.id, version_id).await {
            Ok(resource) => {
                let path = format!("/resources/{}", resource_reference.id);
                if resource.resource_key != resource_reference.id {
                    issues.push(
                        ValidationIssue::error(
                            "resource_version_mismatch",
                            format!(
                                "Resource `{}` does not match its selected version",
                                resource_reference.id
                            ),
                        )
                        .at_path(path.clone()),
                    );
                }
                if resource.kind != resource_reference.kind {
                    issues.push(
                        ValidationIssue::error(
                            "resource_kind_mismatch",
                            format!(
                                "Resource `{}` kind does not match its selected version",
                                resource_reference.id
                            ),
                        )
                        .at_path(path.clone()),
                    );
                }
                if resource.content_hash != resource_reference.content_hash {
                    issues.push(
                        ValidationIssue::error(
                            "resource_hash_mismatch",
                            format!(
                                "Resource `{}` changed after it was selected",
                                resource_reference.id
                            ),
                        )
                        .at_path(path.clone()),
                    );
                }
                if !resource.approved {
                    issues.push(
                        ValidationIssue::error(
                            "resource_not_approved",
                            format!("Resource `{}` is not approved", resource_reference.id),
                        )
                        .at_path(path),
                    );
                }
            }
            Err(ApiError::NotFound) => issues.push(
                ValidationIssue::error(
                    "resource_version_unavailable",
                    format!(
                        "Resource `{}` does not resolve to a Project resource version",
                        resource_reference.id
                    ),
                )
                .at_path(format!("/resources/{}", resource_reference.id)),
            ),
            Err(error) => return Err(error),
        }
    }
    Ok((prepared, deduplicate_issues(issues)))
}

pub(crate) fn profile_tool_is_available(config: &Value, tool: &str) -> bool {
    config
        .get("tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .any(|entry| entry.get("id").and_then(Value::as_str) == Some(tool))
}

pub async fn compile_for_project(
    pool: &PgPool,
    project: &ProjectWithBindings,
    source: &GameSourceV1,
) -> Result<CompileOutput, ApiError> {
    validate_source_limits(source)?;
    let (prepared, issues) = validate_for_project(pool, project, source).await?;
    let errors: Vec<_> = issues
        .into_iter()
        .filter(|issue| issue.severity == ValidationSeverity::Error)
        .collect();
    if !errors.is_empty() {
        return Err(ApiError::Validation(errors));
    }
    GameCompiler::default()
        .compile(&prepared)
        .map_err(|error| match error {
            vifu_game_runtime::GameRuntimeError::Validation(issues) => ApiError::Validation(issues),
            error => ApiError::Invalid(error.to_string()),
        })
}

pub async fn publish(
    pool: &PgPool,
    project: &ProjectWithBindings,
    expected_revision: u64,
    change_summary: Option<&str>,
) -> Result<GameRelease, ApiError> {
    let draft = ensure_draft(pool, project).await?;
    if draft.revision != expected_revision {
        return Err(ApiError::Conflict(format!(
            "draft revision is {}, not {expected_revision}",
            draft.revision
        )));
    }
    let compiled = compile_for_project(pool, project, &draft.source).await?;
    let backend_resources =
        db::backend_resource_snapshot(pool, project.project.id, &compiled.plan.resources).await?;
    db::publish_game(
        pool,
        project.project.id,
        expected_revision,
        &compiled,
        &backend_resources,
        change_summary,
    )
    .await
}

pub fn validate_host(
    manifest: &GameManifestV1,
    host: &HostDescriptor,
) -> Result<Vec<String>, ApiError> {
    let capabilities: HashSet<_> = host.capabilities.iter().map(String::as_str).collect();
    let missing: Vec<_> = manifest
        .required_host_capabilities
        .iter()
        .filter(|capability| !capabilities.contains(capability.as_str()))
        .cloned()
        .collect();
    if !missing.is_empty() {
        return Err(ApiError::Invalid(format!(
            "host is missing required capabilities: {}",
            missing.join(", ")
        )));
    }
    Ok(manifest
        .optional_host_capabilities
        .iter()
        .filter(|capability| !capabilities.contains(capability.as_str()))
        .cloned()
        .collect())
}

fn deduplicate_issues(issues: Vec<ValidationIssue>) -> Vec<ValidationIssue> {
    let mut seen = HashSet::new();
    issues
        .into_iter()
        .filter(|issue| {
            seen.insert((
                issue.code.clone(),
                issue.node_id.clone(),
                issue.edge_id.clone(),
                issue.path.clone(),
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draft_hash_includes_view_metadata() {
        let left = GameSourceV1::new("Test");
        let mut right = left.clone();
        right.views.insert("canvas".to_string(), json!({"x": 1}));
        assert_ne!(source_hash(&left).unwrap(), source_hash(&right).unwrap());
    }

    #[test]
    fn resource_keys_preserve_logical_namespaces() {
        assert_eq!(
            normalize_resource_key(Some("Story.Opening"), "ignored").unwrap(),
            "story.opening"
        );
        assert!(normalize_resource_key(Some("../private"), "ignored").is_err());
    }

    #[test]
    fn resource_hash_is_canonical() {
        let left = json!({"b": 2, "a": 1});
        let right = json!({"a": 1, "b": 2});
        assert_eq!(
            resource_content_hash(&left).unwrap(),
            resource_content_hash(&right).unwrap()
        );
    }

    #[test]
    fn published_profile_tool_catalog_is_an_allowlist() {
        let config = json!({
            "tools": [
                {"id": "calendar.lookup", "label": "Calendar"},
                {"id": "inventory.read", "label": "Inventory"}
            ]
        });
        assert!(profile_tool_is_available(&config, "calendar.lookup"));
        assert!(!profile_tool_is_available(&config, "calendar.write"));
    }
}
