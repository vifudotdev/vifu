use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::types::Json;
use sqlx::FromRow;
use uuid::Uuid;
use vifu_game_runtime::{
    BackendResourceSnapshot, EffectRequest, GameEvent, GameManifestV1, GamePlanV1, GameSnapshotV1,
    GameSourceV1, HostBindingManifestV1, HostDescriptor, PendingHostAction, RuntimeAdvance,
    RuntimeFailure, SessionStatus,
};

#[derive(Clone, Debug, FromRow)]
pub struct GameDraftRow {
    pub project_id: Uuid,
    pub source: Json<GameSourceV1>,
    pub revision: i64,
    pub content_hash: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameDraft {
    pub project_id: Uuid,
    pub source: GameSourceV1,
    pub revision: u64,
    pub content_hash: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl TryFrom<GameDraftRow> for GameDraft {
    type Error = crate::error::ApiError;

    fn try_from(row: GameDraftRow) -> Result<Self, Self::Error> {
        Ok(Self {
            project_id: row.project_id,
            source: row.source.0,
            revision: u64::try_from(row.revision).map_err(|_| Self::Error::Internal)?,
            content_hash: row.content_hash,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateGameSource {
    pub source: GameSourceV1,
    #[serde(default)]
    pub expected_revision: Option<u64>,
    #[serde(default)]
    pub expected_hash: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublishGame {
    pub expected_revision: u64,
    #[serde(default)]
    pub change_summary: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValidateGame {
    #[serde(default)]
    pub source: Option<GameSourceV1>,
}

#[derive(Clone, Debug, FromRow)]
pub struct GameReleaseRow {
    pub id: Uuid,
    pub project_id: Uuid,
    pub release_number: i32,
    pub source_revision: i64,
    pub content_hash: String,
    pub plan: Json<GamePlanV1>,
    pub manifest: Json<GameManifestV1>,
    pub backend_resources: Json<Vec<BackendResourceSnapshot>>,
    pub change_summary: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameRelease {
    pub id: Uuid,
    pub project_id: Uuid,
    pub release_number: u32,
    pub source_revision: u64,
    pub content_hash: String,
    pub plan: GamePlanV1,
    pub manifest: GameManifestV1,
    pub backend_resources: Vec<BackendResourceSnapshot>,
    pub change_summary: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl TryFrom<GameReleaseRow> for GameRelease {
    type Error = crate::error::ApiError;

    fn try_from(row: GameReleaseRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row.id,
            project_id: row.project_id,
            release_number: u32::try_from(row.release_number).map_err(|_| Self::Error::Internal)?,
            source_revision: u64::try_from(row.source_revision)
                .map_err(|_| Self::Error::Internal)?,
            content_hash: row.content_hash,
            plan: row.plan.0,
            manifest: row.manifest.0,
            backend_resources: row.backend_resources.0,
            change_summary: row.change_summary,
            created_at: row.created_at,
        })
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameOverview {
    pub project_id: Uuid,
    pub project_slug: String,
    pub draft_revision: u64,
    pub draft_hash: String,
    pub active_release: Option<GameReleaseSummary>,
    pub unpublished_changes: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameReleaseSummary {
    pub id: Uuid,
    pub release_number: u32,
    pub source_revision: u64,
    pub content_hash: String,
    pub created_at: DateTime<Utc>,
}

impl From<&GameRelease> for GameReleaseSummary {
    fn from(release: &GameRelease) -> Self {
        Self {
            id: release.id,
            release_number: release.release_number,
            source_revision: release.source_revision,
            content_hash: release.content_hash.clone(),
            created_at: release.created_at,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateGameSession {
    pub host: HostDescriptor,
    #[serde(default)]
    pub random_seed: Option<u64>,
}

#[derive(Clone, Debug, FromRow)]
pub struct GameSessionRow {
    pub id: Uuid,
    pub project_id: Uuid,
    pub game_release_id: Option<Uuid>,
    pub source_revision: Option<i64>,
    pub is_preview: bool,
    pub api_key_id: Option<Uuid>,
    pub status: String,
    pub revision: i64,
    pub snapshot: Json<GameSnapshotV1>,
    pub host: Json<HostDescriptor>,
    pub public_output: Option<Json<Value>>,
    pub failure: Option<Json<Value>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameSession {
    pub id: Uuid,
    pub project_id: Uuid,
    pub game_release_id: Option<Uuid>,
    pub source_revision: Option<u64>,
    #[serde(rename = "preview")]
    pub is_preview: bool,
    pub status: String,
    pub revision: u64,
    pub snapshot: GameSnapshotV1,
    pub host: HostDescriptor,
    pub public_output: Option<Value>,
    pub failure: Option<Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

impl TryFrom<GameSessionRow> for GameSession {
    type Error = crate::error::ApiError;

    fn try_from(row: GameSessionRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row.id,
            project_id: row.project_id,
            game_release_id: row.game_release_id,
            source_revision: row
                .source_revision
                .map(u64::try_from)
                .transpose()
                .map_err(|_| Self::Error::Internal)?,
            is_preview: row.is_preview,
            status: row.status,
            revision: u64::try_from(row.revision).map_err(|_| Self::Error::Internal)?,
            snapshot: row.snapshot.0,
            host: row.host.0,
            public_output: row.public_output.map(|output| output.0),
            failure: row.failure.map(|failure| failure.0),
            created_at: row.created_at,
            updated_at: row.updated_at,
            completed_at: row.completed_at,
        })
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicGameSession {
    pub id: Uuid,
    pub game_release_id: Uuid,
    pub status: String,
    pub revision: u64,
    pub public_output: Option<Value>,
    pub outstanding_host_actions: Vec<PendingHostAction>,
    pub failure: Option<Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

impl TryFrom<&GameSession> for PublicGameSession {
    type Error = crate::error::ApiError;

    fn try_from(session: &GameSession) -> Result<Self, Self::Error> {
        Ok(Self {
            id: session.id,
            game_release_id: session.game_release_id.ok_or(Self::Error::NotFound)?,
            status: session.status.clone(),
            revision: session.revision,
            public_output: session.public_output.clone(),
            outstanding_host_actions: session
                .snapshot
                .pending_host_action
                .iter()
                .cloned()
                .collect(),
            failure: session.failure.clone(),
            created_at: session.created_at,
            updated_at: session.updated_at,
            completed_at: session.completed_at,
        })
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicRuntimeAdvance {
    pub status: SessionStatus,
    pub revision: u64,
    pub public_output: Option<Value>,
    pub outstanding_host_actions: Vec<PendingHostAction>,
    pub failure: Option<RuntimeFailure>,
    pub events: Vec<GameEvent>,
}

impl From<&RuntimeAdvance> for PublicRuntimeAdvance {
    fn from(advance: &RuntimeAdvance) -> Self {
        Self {
            status: advance.snapshot.status.clone(),
            revision: advance.snapshot.revision,
            public_output: advance.snapshot.public_output.clone(),
            outstanding_host_actions: advance
                .snapshot
                .pending_host_action
                .iter()
                .cloned()
                .collect(),
            failure: advance.snapshot.failure.clone(),
            events: advance.events.clone(),
        }
    }
}

#[derive(Clone, Debug, FromRow)]
pub struct SessionExecutionRow {
    pub id: Uuid,
    pub project_id: Uuid,
    pub game_release_id: Option<Uuid>,
    pub revision: i64,
    pub snapshot: Json<GameSnapshotV1>,
    pub plan: Json<GamePlanV1>,
}

#[derive(Clone, Debug, FromRow)]
pub struct StoredCommandRow {
    pub result: Option<Json<RuntimeAdvance>>,
}

#[derive(Clone, Debug, FromRow)]
pub struct StoredEventRow {
    pub sequence: i64,
    pub event: Json<vifu_game_runtime::GameEvent>,
}

#[derive(Clone, Debug, FromRow)]
pub struct GameEffectWorkRow {
    pub session_id: Uuid,
    pub effect_id: String,
    pub status: String,
    pub request: Json<EffectRequest>,
    pub result: Option<Json<vifu_game_runtime::EffectResult>>,
    pub project_id: Uuid,
    pub project_slug: String,
    pub trace_id: Option<Uuid>,
    pub parent_span_id: Option<Uuid>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GameEffectTrace {
    pub trace_id: Uuid,
    pub parent_span_id: Uuid,
}

#[derive(Clone, Debug)]
pub struct GameEffectWork {
    pub session_id: Uuid,
    pub effect_id: String,
    pub status: String,
    pub request: EffectRequest,
    pub result: Option<vifu_game_runtime::EffectResult>,
    pub project_id: Uuid,
    pub project_slug: String,
    pub trace_id: Option<Uuid>,
    pub parent_span_id: Option<Uuid>,
}

impl From<GameEffectWorkRow> for GameEffectWork {
    fn from(row: GameEffectWorkRow) -> Self {
        Self {
            session_id: row.session_id,
            effect_id: row.effect_id,
            status: row.status,
            request: row.request.0,
            result: row.result.map(|result| result.0),
            project_id: row.project_id,
            project_slug: row.project_slug,
            trace_id: row.trace_id,
            parent_span_id: row.parent_span_id,
        }
    }
}

impl GameEffectWork {
    pub fn trace_context(&self) -> Option<GameEffectTrace> {
        match (self.trace_id, self.parent_span_id) {
            (Some(trace_id), Some(parent_span_id)) => Some(GameEffectTrace {
                trace_id,
                parent_span_id,
            }),
            _ => None,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunGame {
    pub host: HostDescriptor,
    #[serde(default)]
    pub input: Value,
    #[serde(default)]
    pub idempotency_key: Option<String>,
}

#[derive(Clone, Debug, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameResource {
    pub id: Uuid,
    pub project_id: Uuid,
    pub resource_key: String,
    pub name: String,
    pub kind: String,
    pub content: Value,
    pub version: i64,
    pub content_hash: String,
    pub approved: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateGameResource {
    #[serde(default)]
    pub resource_key: Option<String>,
    pub name: String,
    pub kind: String,
    #[serde(default)]
    pub content: Value,
    #[serde(default)]
    pub approved: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateGameResource {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub content: Option<Value>,
    #[serde(default)]
    pub approved: Option<bool>,
}

#[derive(Clone, Debug, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameAsset {
    pub id: Uuid,
    pub project_id: Uuid,
    pub asset_key: String,
    pub name: String,
    pub kind: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameAssetVersion {
    pub id: Uuid,
    pub project_id: Uuid,
    pub asset_id: Uuid,
    pub content_hash: String,
    pub mime_type: String,
    pub size_bytes: i64,
    #[serde(skip_serializing)]
    pub storage_key: String,
    pub metadata: Value,
    pub provenance: Value,
    pub rights_status: String,
    pub approval_status: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameAssetWithVersions {
    #[serde(flatten)]
    pub asset: GameAsset,
    pub versions: Vec<GameAssetVersion>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateGameAsset {
    #[serde(default)]
    pub asset_key: Option<String>,
    pub name: String,
    pub kind: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApproveGameAssetVersion {
    #[serde(default = "approved_status")]
    pub status: String,
}

fn approved_status() -> String {
    "approved".to_string()
}

#[derive(Clone, Debug, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameBuildJob {
    pub id: Uuid,
    pub project_id: Uuid,
    pub source_revision: i64,
    pub kind: String,
    pub status: String,
    pub input_hash: String,
    pub input: Value,
    pub output: Option<Value>,
    pub error: Option<Value>,
    pub attempts: i32,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateGameBuild {
    #[serde(default)]
    pub kind: Option<String>,
}

#[derive(Clone, Debug, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameAnalyticsCount {
    pub event_type: String,
    pub count: i64,
}

#[derive(Clone, Debug, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameSessionStatusCount {
    pub status: String,
    pub count: i64,
}

#[derive(Clone, Debug, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GamePresentationRelease {
    pub id: Uuid,
    pub project_id: Uuid,
    pub game_release_id: Uuid,
    pub release_number: i32,
    pub content_hash: String,
    pub binding_manifest: Json<HostBindingManifestV1>,
    pub asset_version_ids: Vec<Uuid>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublishGamePresentation {
    #[serde(default)]
    pub game_release_id: Option<Uuid>,
    pub binding_manifest: HostBindingManifestV1,
    #[serde(default)]
    pub asset_version_ids: Vec<Uuid>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use vifu_game_runtime::{EffectKind, GameSnapshotV1, NodeExecution, RuntimeAdvance};

    use super::*;

    fn session(game_release_id: Option<Uuid>, is_preview: bool) -> GameSession {
        let now = Utc::now();
        GameSession {
            id: Uuid::new_v4(),
            project_id: Uuid::new_v4(),
            game_release_id,
            source_revision: is_preview.then_some(1),
            is_preview,
            status: "waiting_input".to_string(),
            revision: 0,
            snapshot: GameSnapshotV1::initial(0, &[], 7),
            host: HostDescriptor {
                engine: "test".to_string(),
                adapter_version: None,
                capabilities: Vec::new(),
                locale: None,
                binding_manifest_hash: None,
            },
            public_output: None,
            failure: None,
            created_at: now,
            updated_at: now,
            completed_at: None,
        }
    }

    #[test]
    fn public_advance_omits_internal_snapshot_and_effect_payloads() {
        let mut snapshot = GameSnapshotV1::initial(0, &[], 7);
        snapshot.state = json!({"privateMemory": "hidden"});
        snapshot.conversations.insert(
            "npc".to_string(),
            vec![vifu_game_runtime::ConversationMessage {
                role: "assistant".to_string(),
                content: json!("private dialogue"),
            }],
        );
        let advance = RuntimeAdvance {
            snapshot,
            events: Vec::new(),
            effects: vec![EffectRequest {
                effect_id: "effect-1".to_string(),
                node_id: "npc".to_string(),
                kind: EffectKind::Agent,
                descriptor: json!({"privateRoute": true}),
                input: json!({"prompt": "hidden"}),
            }],
            node_executions: vec![NodeExecution {
                sequence: 1,
                ordinal: 0,
                node_id: "npc".to_string(),
                node_type: "agent".to_string(),
            }],
        };

        let value = serde_json::to_value(PublicRuntimeAdvance::from(&advance)).unwrap();
        assert!(value.get("state").is_none());
        assert!(value.get("conversations").is_none());
        assert!(value.get("effects").is_none());
        assert!(value.get("nodeExecutions").is_none());
        assert_eq!(value["status"], "waiting_input");
    }

    #[test]
    fn draft_preview_session_cannot_become_a_public_session() {
        let preview = session(None, true);
        assert!(matches!(
            PublicGameSession::try_from(&preview),
            Err(crate::error::ApiError::NotFound)
        ));

        let release_id = Uuid::new_v4();
        let published = session(Some(release_id), false);
        let public = PublicGameSession::try_from(&published).expect("published session");
        assert_eq!(public.game_release_id, release_id);
    }
}
