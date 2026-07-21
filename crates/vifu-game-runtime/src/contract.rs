use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::condition::ConditionExpression;
use crate::GAME_SCHEMA_VERSION;

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GameSourceV1 {
    pub schema_version: u32,
    pub metadata: GameMetadata,
    pub entry_node_id: String,
    pub graph: SourceGraph,
    #[serde(default = "object_schema")]
    pub inputs: Value,
    #[serde(default = "object_schema")]
    pub outputs: Value,
    #[serde(default)]
    pub variables: Vec<GameVariable>,
    #[serde(default)]
    pub agents: Vec<AgentReference>,
    #[serde(default)]
    pub resources: Vec<ResourceReference>,
    #[serde(default)]
    pub presentation_resources: Vec<LogicalPresentationResource>,
    #[serde(default)]
    pub locales: Vec<String>,
    #[serde(default)]
    pub views: BTreeMap<String, Value>,
}

impl GameSourceV1 {
    pub fn new(name: impl Into<String>) -> Self {
        let entry_node_id = "start".to_string();
        Self {
            schema_version: GAME_SCHEMA_VERSION,
            metadata: GameMetadata {
                name: name.into(),
                description: None,
                tags: Vec::new(),
            },
            entry_node_id: entry_node_id.clone(),
            graph: SourceGraph {
                nodes: vec![SourceNode {
                    id: entry_node_id,
                    node_type: "start".to_string(),
                    version: 1,
                    config: json!({}),
                    parent_id: None,
                    label: Some("Start".to_string()),
                    notes: None,
                }],
                edges: Vec::new(),
            },
            inputs: object_schema(),
            outputs: object_schema(),
            variables: Vec::new(),
            agents: Vec::new(),
            resources: Vec::new(),
            presentation_resources: Vec::new(),
            locales: vec!["en".to_string()],
            views: BTreeMap::new(),
        }
    }
}

fn object_schema() -> Value {
    json!({"type": "object", "additionalProperties": true})
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GameMetadata {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceGraph {
    #[serde(default)]
    pub nodes: Vec<SourceNode>,
    #[serde(default)]
    pub edges: Vec<SourceEdge>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceNode {
    pub id: String,
    #[serde(rename = "type")]
    pub node_type: String,
    pub version: u32,
    #[serde(default = "empty_object")]
    pub config: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

fn empty_object() -> Value {
    json!({})
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceEdge {
    pub id: String,
    pub source: PortReference,
    pub target: PortReference,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<ConditionExpression>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PortReference {
    pub node_id: String,
    pub port: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GameVariable {
    pub id: String,
    #[serde(default)]
    pub initial_value: Value,
    #[serde(default)]
    pub public: bool,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentReference {
    pub id: String,
    pub profile_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_version_id: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default = "empty_object")]
    pub execution_descriptor: Value,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourceReference {
    pub id: String,
    pub version_id: String,
    pub kind: String,
    pub content_hash: String,
    #[serde(default)]
    pub approved: bool,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LogicalPresentationResource {
    pub id: String,
    pub kind: String,
    #[serde(default)]
    pub required_capabilities: Vec<String>,
    #[serde(default)]
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<Value>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GamePlanV1 {
    pub schema_version: u32,
    pub entry_node: u32,
    pub nodes: Vec<CompiledNode>,
    pub edges: Vec<CompiledEdge>,
    pub inputs: Value,
    pub outputs: Value,
    pub variables: Vec<GameVariable>,
    pub agents: Vec<PinnedAgent>,
    pub resources: Vec<PinnedResource>,
    pub presentation_resources: Vec<LogicalPresentationResource>,
    pub locales: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledNode {
    pub ordinal: u32,
    pub id: String,
    #[serde(rename = "type")]
    pub node_type: String,
    pub version: u32,
    pub config: Value,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledEdge {
    pub id: String,
    pub source_node: u32,
    pub source_port: String,
    pub target_node: u32,
    pub target_port: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<ConditionExpression>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PinnedAgent {
    pub id: String,
    pub profile_id: String,
    pub profile_version_id: String,
    pub capabilities: Vec<String>,
    pub execution_descriptor: Value,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PinnedResource {
    pub id: String,
    pub version_id: String,
    pub kind: String,
    pub content_hash: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BackendResourceSnapshot {
    pub id: String,
    pub version_id: String,
    pub version: u64,
    pub kind: String,
    pub content_hash: String,
    pub content: Value,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GameManifestV1 {
    pub schema_version: u32,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub commands: BTreeMap<String, Value>,
    pub events: BTreeMap<String, Value>,
    pub inputs: Value,
    pub outputs: Value,
    pub scenes: Vec<ManifestItem>,
    pub characters: Vec<ManifestItem>,
    pub logical_resources: Vec<LogicalPresentationResource>,
    pub required_host_capabilities: Vec<String>,
    pub optional_host_capabilities: Vec<String>,
    pub locales: Vec<String>,
    pub compatibility: ClientCompatibility,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManifestItem {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClientCompatibility {
    pub protocol: String,
    pub minimum_version: u32,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostBindingManifestV1 {
    pub schema_version: u32,
    pub engine: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter_version: Option<String>,
    pub bindings: BTreeMap<String, HostBinding>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostBinding {
    pub kind: String,
    pub value: Value,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GameReleaseBundleV1 {
    pub schema_version: u32,
    pub release_id: String,
    pub project_slug: String,
    pub content_hash: String,
    pub plan: GamePlanV1,
    pub manifest: GameManifestV1,
    pub backend_resources: Vec<BackendResourceSnapshot>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PresentationBundleV1 {
    pub schema_version: u32,
    pub presentation_id: String,
    pub game_content_hash: String,
    pub binding_manifest: HostBindingManifestV1,
    pub asset_version_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Running,
    WaitingInput,
    WaitingEffect,
    WaitingHost,
    Completed,
    Failed,
    Cancelled,
}

impl std::fmt::Display for SessionStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let status = match self {
            Self::Running => "running",
            Self::WaitingInput => "waiting_input",
            Self::WaitingEffect => "waiting_effect",
            Self::WaitingHost => "waiting_host",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        };
        formatter.write_str(status)
    }
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GameSnapshotV1 {
    pub schema_version: u32,
    pub status: SessionStatus,
    pub revision: u64,
    pub current_nodes: Vec<u32>,
    #[serde(default)]
    pub join_arrivals: BTreeMap<u32, BTreeSet<u32>>,
    #[serde(default)]
    pub state: Value,
    #[serde(default)]
    pub node_outputs: BTreeMap<String, Value>,
    #[serde(default)]
    pub conversations: BTreeMap<String, Vec<ConversationMessage>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_effect: Option<EffectRequest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_host_action: Option<PendingHostAction>,
    pub next_event_sequence: u64,
    pub next_effect_sequence: u64,
    pub random_seed: u64,
    pub random_counter: u64,
    pub total_steps: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_output: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<RuntimeFailure>,
}

impl GameSnapshotV1 {
    pub fn initial(entry_node: u32, variables: &[GameVariable], random_seed: u64) -> Self {
        let mut state = serde_json::Map::new();
        for variable in variables {
            state.insert(variable.id.clone(), variable.initial_value.clone());
        }
        Self {
            schema_version: GAME_SCHEMA_VERSION,
            status: SessionStatus::WaitingInput,
            revision: 0,
            current_nodes: vec![entry_node],
            join_arrivals: BTreeMap::new(),
            state: Value::Object(state),
            node_outputs: BTreeMap::new(),
            conversations: BTreeMap::new(),
            pending_effect: None,
            pending_host_action: None,
            next_event_sequence: 1,
            next_effect_sequence: 1,
            random_seed,
            random_counter: 0,
            total_steps: 0,
            public_output: None,
            failure: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConversationMessage {
    pub role: String,
    pub content: Value,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GameCommand {
    pub idempotency_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<u64>,
    #[serde(rename = "type")]
    pub command_type: String,
    #[serde(default)]
    pub data: Value,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GameEvent {
    pub specversion: String,
    pub id: String,
    pub source: String,
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    pub sequence: u64,
    pub data: Value,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EffectRequest {
    pub effect_id: String,
    pub node_id: String,
    pub kind: EffectKind,
    pub descriptor: Value,
    pub input: Value,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectKind {
    Agent,
    Tool,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EffectResult {
    pub effect_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RuntimeFailure>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PendingHostAction {
    pub action_id: String,
    pub node_id: String,
    pub target: String,
    pub action: String,
    pub arguments: Value,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeFailure {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAdvance {
    pub snapshot: GameSnapshotV1,
    pub events: Vec<GameEvent>,
    pub effects: Vec<EffectRequest>,
    #[serde(default)]
    pub node_executions: Vec<NodeExecution>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NodeExecution {
    pub sequence: u64,
    pub ordinal: u32,
    pub node_id: String,
    pub node_type: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostDescriptor {
    pub engine: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter_version: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding_manifest_hash: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_source_has_a_portable_start_node() {
        let source = GameSourceV1::new("New game");
        assert_eq!(source.schema_version, GAME_SCHEMA_VERSION);
        assert_eq!(source.entry_node_id, "start");
        assert_eq!(source.graph.nodes[0].node_type, "start");
    }

    #[test]
    fn snapshot_round_trips_as_camel_case_json() {
        let snapshot = GameSnapshotV1::initial(3, &[], 42);
        let encoded = serde_json::to_string(&snapshot).expect("serialize snapshot");
        assert!(encoded.contains("nextEventSequence"));
        let decoded: GameSnapshotV1 = serde_json::from_str(&encoded).expect("restore snapshot");
        assert_eq!(decoded, snapshot);
    }
}
