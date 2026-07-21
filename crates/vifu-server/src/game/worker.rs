use std::time::Duration;

use serde_json::Value;
use sha2::{Digest, Sha256};
use tracing::{debug, warn};
use uuid::Uuid;
use vifu_game_runtime::{EffectKind, EffectResult, RuntimeFailure};

use crate::AppState;

use super::db;
use super::models::GameEffectWork;

pub fn spawn_effect_worker(state: AppState) {
    tokio::spawn(async move {
        let worker_id = format!("game-effect-{}", Uuid::new_v4());
        loop {
            match process_next_effect(&state, &worker_id).await {
                Ok(true) => {}
                Ok(false) => tokio::time::sleep(Duration::from_millis(250)).await,
                Err(error) => {
                    warn!(error = %error, "game effect worker iteration failed");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }
    });
}

async fn process_next_effect(
    state: &AppState,
    worker_id: &str,
) -> Result<bool, crate::error::ApiError> {
    let Some(work) = db::claim_game_effect(&state.pool, worker_id).await? else {
        return Ok(false);
    };
    let result = match work.result.clone() {
        Some(result) => result,
        None => {
            let result = execute_effect(state, &work).await;
            db::store_game_effect_result(&state.pool, work.session_id, &work.effect_id, &result)
                .await?;
            result
        }
    };
    db::resume_stored_effect(&state.pool, &work, &result).await?;
    debug!(
        session_id = %work.session_id,
        effect_id = %work.effect_id,
        "game effect completed"
    );
    Ok(true)
}

async fn execute_effect(state: &AppState, work: &GameEffectWork) -> EffectResult {
    let profile_id = descriptor_uuid(&work.request.descriptor, "profileId");
    let profile_version_id = descriptor_uuid(&work.request.descriptor, "profileVersionId");
    let (Some(profile_id), Some(profile_version_id)) = (profile_id, profile_version_id) else {
        return failed_effect(
            work,
            "effect_descriptor_invalid",
            "Published effect descriptor is missing its pinned Profile version",
        );
    };
    let request_id = effect_request_id(work.session_id, &work.effect_id);
    let result = match work.request.kind {
        EffectKind::Agent => {
            crate::api::invoke_game_agent(
                state,
                crate::api::GameProviderEffect {
                    project_id: work.project_id,
                    project_slug: &work.project_slug,
                    profile_id,
                    profile_version_id,
                    request_id,
                    effect_id: &work.effect_id,
                    descriptor: &work.request.descriptor,
                    input: work.request.input.clone(),
                    trace_context: work.trace_context(),
                },
            )
            .await
        }
        EffectKind::Tool => {
            if !work.request.input.is_object() {
                return failed_effect(
                    work,
                    "tool_input_invalid",
                    "Tool input must be a JSON object",
                );
            }
            crate::api::invoke_game_tool(
                state,
                crate::api::GameProviderEffect {
                    project_id: work.project_id,
                    project_slug: &work.project_slug,
                    profile_id,
                    profile_version_id,
                    request_id,
                    effect_id: &work.effect_id,
                    descriptor: &work.request.descriptor,
                    input: work.request.input.clone(),
                    trace_context: work.trace_context(),
                },
            )
            .await
        }
    };
    match result {
        Ok(output) => EffectResult {
            effect_id: work.effect_id.clone(),
            output: Some(output),
            error: None,
        },
        Err(error) => failed_effect(
            work,
            match work.request.kind {
                EffectKind::Agent => "agent_effect_failed",
                EffectKind::Tool => "tool_effect_failed",
            },
            error.to_string(),
        ),
    }
}

fn effect_request_id(session_id: Uuid, effect_id: &str) -> Uuid {
    let mut hasher = Sha256::new();
    hasher.update(b"vifu.game.effect.request.v1");
    hasher.update(session_id.as_bytes());
    hasher.update(effect_id.as_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

fn descriptor_uuid(descriptor: &Value, key: &str) -> Option<Uuid> {
    descriptor
        .get(key)
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
}

fn failed_effect(
    work: &GameEffectWork,
    code: impl Into<String>,
    message: impl Into<String>,
) -> EffectResult {
    EffectResult {
        effect_id: work.effect_id.clone(),
        output: None,
        error: Some(RuntimeFailure {
            code: code.into(),
            message: message.into(),
            node_id: Some(work.request.node_id.clone()),
        }),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn reads_only_uuid_descriptors() {
        let id = Uuid::new_v4();
        assert_eq!(
            descriptor_uuid(&json!({"profileId": id}), "profileId"),
            Some(id)
        );
        assert_eq!(
            descriptor_uuid(&json!({"profileId": "not-a-uuid"}), "profileId"),
            None
        );
    }

    #[test]
    fn effect_request_ids_are_stable_and_session_scoped() {
        let session_id = Uuid::new_v4();
        assert_eq!(
            effect_request_id(session_id, "effect-1"),
            effect_request_id(session_id, "effect-1")
        );
        assert_ne!(
            effect_request_id(session_id, "effect-1"),
            effect_request_id(session_id, "effect-2")
        );
        assert_ne!(
            effect_request_id(session_id, "effect-1"),
            effect_request_id(Uuid::new_v4(), "effect-1")
        );
    }
}
