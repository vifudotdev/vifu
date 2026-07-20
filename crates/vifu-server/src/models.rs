use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub projects: bool,
    pub profiles: bool,
    pub endpoints: bool,
    pub bindings: bool,
    pub api_keys: bool,
    pub canvas: bool,
    pub agent_gateways: bool,
    pub provider_connections: bool,
    pub traces: bool,
}

impl Capabilities {
    pub fn self_hosted() -> Self {
        Self {
            projects: true,
            profiles: true,
            endpoints: true,
            bindings: true,
            api_keys: true,
            canvas: true,
            agent_gateways: true,
            provider_connections: true,
            traces: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub gateway_id: String,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateProject {
    pub slug: Option<String>,
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProject {
    pub slug: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub gateway_id: Option<String>,
    pub enabled: Option<bool>,
    pub binding_ids: Option<Vec<Uuid>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectWithBindings {
    #[serde(flatten)]
    pub project: Project,
    pub binding_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ProjectCanvasNode {
    pub id: Uuid,
    pub project_id: Uuid,
    pub kind: String,
    pub position: Value,
    pub profile_id: Option<Uuid>,
    pub binding_id: Option<Uuid>,
    pub gateway_id: Option<String>,
    pub resource_id: Option<String>,
    pub config: Value,
    pub inputs: Value,
    pub outputs: Value,
    pub exposed: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ProjectCanvasEdge {
    pub id: Uuid,
    pub project_id: Uuid,
    pub source_node_id: Uuid,
    pub source_handle: Option<String>,
    pub target_node_id: Uuid,
    pub target_handle: Option<String>,
    pub kind: String,
    pub config: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectCanvas {
    pub project: ProjectWithBindings,
    pub nodes: Vec<ProjectCanvasNode>,
    pub edges: Vec<ProjectCanvasEdge>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAdapter {
    pub id: String,
    pub category: String,
    pub name: String,
    pub description: String,
    pub capabilities: Vec<String>,
    pub execution_modes: Vec<String>,
    pub supports_discovery: bool,
    pub fields: Vec<ProviderAdapterField>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAdapterField {
    pub key: String,
    pub label: String,
    pub kind: String,
    pub required: bool,
    pub secret: bool,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConnection {
    pub id: Uuid,
    pub project_id: Uuid,
    pub provider_key: String,
    pub name: String,
    pub provider_type: String,
    pub base_url: String,
    pub config: Value,
    pub secret_keys: Vec<String>,
    pub display_secret: Option<String>,
    pub status: String,
    pub last_checked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
pub struct ProviderConnectionSecret {
    pub id: Uuid,
    pub project_id: Uuid,
    pub provider_key: String,
    pub name: String,
    pub provider_type: String,
    pub base_url: String,
    pub config: Value,
    pub encrypted_secret_json: String,
    pub secret_keys: Vec<String>,
    pub display_secret: Option<String>,
    pub status: String,
    pub last_checked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ProviderStockItem {
    pub id: Uuid,
    pub provider_key: String,
    pub name: String,
    pub provider_type: String,
    pub base_url: String,
    pub config: Value,
    pub secret_keys: Vec<String>,
    pub display_secret: Option<String>,
    pub status: String,
    pub last_checked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
pub struct ProviderStockSecret {
    pub id: Uuid,
    pub provider_key: String,
    pub name: String,
    pub provider_type: String,
    pub base_url: String,
    pub config: Value,
    pub encrypted_secret_json: String,
    pub secret_keys: Vec<String>,
    pub display_secret: Option<String>,
    pub status: String,
    pub last_checked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<ProviderStockSecret> for ProviderStockItem {
    fn from(value: ProviderStockSecret) -> Self {
        Self {
            id: value.id,
            provider_key: value.provider_key,
            name: value.name,
            provider_type: value.provider_type,
            base_url: value.base_url,
            config: value.config,
            secret_keys: value.secret_keys,
            display_secret: value.display_secret,
            status: value.status,
            last_checked_at: value.last_checked_at,
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ProjectProviderAssignment {
    pub project_id: Uuid,
    pub provider_key: String,
    pub created_at: DateTime<Utc>,
}

impl From<ProviderConnectionSecret> for ProviderConnection {
    fn from(value: ProviderConnectionSecret) -> Self {
        Self {
            id: value.id,
            project_id: value.project_id,
            provider_key: value.provider_key,
            name: value.name,
            provider_type: value.provider_type,
            base_url: value.base_url,
            config: value.config,
            secret_keys: value.secret_keys,
            display_secret: value.display_secret,
            status: value.status,
            last_checked_at: value.last_checked_at,
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertProviderConnection {
    pub name: Option<String>,
    pub provider_type: String,
    pub base_url: String,
    #[serde(default = "empty_object")]
    pub config: Value,
    #[serde(default = "empty_object")]
    pub secrets: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportProviderConnections {
    #[serde(default)]
    pub providers: Vec<ImportProviderConnection>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportProviderConnection {
    pub key: String,
    pub name: Option<String>,
    pub provider_type: String,
    pub base_url: String,
    #[serde(default = "empty_object")]
    pub config: Value,
    #[serde(default = "empty_object")]
    pub secrets: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportProjectAgent {
    pub gateway_id: String,
    pub agent_id: String,
    pub provider_key: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateCanvasNode {
    pub kind: String,
    #[serde(default = "empty_object")]
    pub position: Value,
    pub profile_id: Option<Uuid>,
    pub binding_id: Option<Uuid>,
    pub gateway_id: Option<String>,
    pub resource_id: Option<String>,
    #[serde(default = "empty_object")]
    pub config: Value,
    #[serde(default = "empty_object")]
    pub inputs: Value,
    #[serde(default = "empty_object")]
    pub outputs: Value,
    pub exposed: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCanvasNode {
    pub position: Option<Value>,
    pub config: Option<Value>,
    pub inputs: Option<Value>,
    pub outputs: Option<Value>,
    pub exposed: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateCanvasEdge {
    pub source_node_id: Uuid,
    pub source_handle: Option<String>,
    pub target_node_id: Uuid,
    pub target_handle: Option<String>,
    pub kind: String,
    #[serde(default = "empty_object")]
    pub config: Value,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct AgentProfile {
    pub id: Uuid,
    pub project_id: Option<Uuid>,
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub active_version_id: Option<Uuid>,
    pub archived_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateProfile {
    pub project_id: Option<Uuid>,
    pub slug: Option<String>,
    pub name: String,
    pub description: Option<String>,
    #[serde(default = "empty_object")]
    pub persona: Value,
    #[serde(default = "empty_object")]
    pub runtime: Value,
    #[serde(default = "empty_object")]
    pub presentation: Value,
    #[serde(default = "empty_object")]
    pub source: Value,
    #[serde(default)]
    pub capabilities: Vec<ProfileCapabilityDraft>,
    pub change_summary: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProfile {
    pub slug: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct AgentProfileVersion {
    pub id: Uuid,
    pub profile_id: Uuid,
    pub version_number: i32,
    pub persona: Value,
    pub runtime: Value,
    pub presentation: Value,
    pub source: Value,
    pub content_hash: String,
    pub change_summary: Option<String>,
    pub archived_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct AgentProfileCapability {
    pub id: Uuid,
    pub profile_version_id: Uuid,
    pub kind: String,
    pub provider_type: String,
    pub provider_key: String,
    pub resource_id: Option<String>,
    pub config: Value,
    pub input_schema: Value,
    pub output_schema: Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProfileCapabilityDraft {
    pub kind: String,
    pub provider_type: String,
    pub provider_key: String,
    pub resource_id: Option<String>,
    #[serde(default = "empty_object")]
    pub config: Value,
    #[serde(default = "empty_object")]
    pub input_schema: Value,
    #[serde(default = "empty_object")]
    pub output_schema: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateProfileVersion {
    #[serde(default = "empty_object")]
    pub persona: Value,
    #[serde(default = "empty_object")]
    pub runtime: Value,
    #[serde(default = "empty_object")]
    pub presentation: Value,
    #[serde(default = "empty_object")]
    pub source: Value,
    #[serde(default)]
    pub capabilities: Vec<ProfileCapabilityDraft>,
    pub change_summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct AgentProfileRollout {
    pub profile_id: Uuid,
    pub profile_version_id: Uuid,
    pub weight_bps: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetProfileRollout {
    pub allocations: Vec<ProfileRolloutAllocation>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProfileRolloutAllocation {
    pub version_id: Uuid,
    pub weight_bps: i32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TestProfile {
    pub version_id: Option<Uuid>,
    #[serde(default = "default_chat_capability")]
    pub capability: String,
    pub input: Value,
    pub user: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncProfileSource {
    pub change_summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct AgentBinding {
    pub id: Uuid,
    pub profile_id: Uuid,
    pub provider: String,
    pub gateway_id: String,
    pub agent_id: String,
    pub config: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateBinding {
    pub profile_id: Uuid,
    pub provider: String,
    pub gateway_id: String,
    pub agent_id: String,
    #[serde(default = "empty_object")]
    pub config: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateBinding {
    pub gateway_id: Option<String>,
    pub agent_id: Option<String>,
    pub config: Option<Value>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct AgentEndpoint {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub profile_id: Uuid,
    pub binding_id: Uuid,
    pub enabled: bool,
    pub request_timeout_ms: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateEndpoint {
    pub slug: Option<String>,
    pub name: String,
    pub profile_id: Uuid,
    pub binding_id: Uuid,
    pub enabled: Option<bool>,
    pub request_timeout_ms: Option<i32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateEndpoint {
    pub slug: Option<String>,
    pub name: Option<String>,
    pub profile_id: Option<Uuid>,
    pub binding_id: Option<Uuid>,
    pub enabled: Option<bool>,
    pub request_timeout_ms: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "lowercase", deny_unknown_fields)]
pub enum ApiKeyAgentScope {
    All,
    Selected {
        #[serde(rename = "profileIds")]
        profile_ids: Vec<Uuid>,
    },
}

impl Default for ApiKeyAgentScope {
    fn default() -> Self {
        Self::All
    }
}

impl ApiKeyAgentScope {
    pub fn mode(&self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Selected { .. } => "selected",
        }
    }

    pub fn profile_ids(&self) -> &[Uuid] {
        match self {
            Self::All => &[],
            Self::Selected { profile_ids } => profile_ids,
        }
    }

    pub fn allows(&self, profile_id: Uuid) -> bool {
        match self {
            Self::All => true,
            Self::Selected { profile_ids } => profile_ids.contains(&profile_id),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EndpointPermission {
    None,
    Access,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResourcePermission {
    None,
    Read,
    Write,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApiKeyPermissions {
    pub chat_completions: EndpointPermission,
    pub speech: EndpointPermission,
    pub transcriptions: EndpointPermission,
    pub realtime: EndpointPermission,
    pub agents: ResourcePermission,
    pub project: ResourcePermission,
}

impl Default for ApiKeyPermissions {
    fn default() -> Self {
        Self {
            chat_completions: EndpointPermission::Access,
            speech: EndpointPermission::None,
            transcriptions: EndpointPermission::None,
            realtime: EndpointPermission::None,
            agents: ResourcePermission::None,
            project: ResourcePermission::None,
        }
    }
}

impl ApiKeyPermissions {
    pub fn chat_completions_allowed(&self) -> bool {
        self.chat_completions == EndpointPermission::Access
    }

    pub fn speech_allowed(&self) -> bool {
        self.speech == EndpointPermission::Access
    }

    pub fn transcriptions_allowed(&self) -> bool {
        self.transcriptions == EndpointPermission::Access
    }

    pub fn realtime_allowed(&self) -> bool {
        self.realtime == EndpointPermission::Access
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiKeyRecord {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub agent_scope: ApiKeyAgentScope,
    pub permissions: ApiKeyPermissions,
    pub key_prefix: String,
    pub created_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedApiKey {
    #[serde(flatten)]
    pub record: ApiKeyRecord,
    pub key: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateApiKey {
    pub project_id: Uuid,
    pub name: Option<String>,
    #[serde(default)]
    pub agent_scope: ApiKeyAgentScope,
    #[serde(default)]
    pub permissions: ApiKeyPermissions,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateApiKey {
    pub project_id: Option<Uuid>,
    pub name: Option<String>,
    pub agent_scope: Option<ApiKeyAgentScope>,
    pub permissions: Option<ApiKeyPermissions>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegisterAgentGateway {
    pub gateway_id: String,
    pub credential: String,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct AgentGatewayCredential {
    pub gateway_id: String,
    pub credential_prefix: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct AgentGatewaySession {
    pub id: Uuid,
    pub gateway_id: String,
    pub session_id: Uuid,
    pub status: String,
    pub agents: Value,
    pub metadata: Value,
    pub connected_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
    pub disconnected_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AvailableAgent {
    pub gateway_id: String,
    pub id: String,
    pub name: String,
    pub status: String,
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct EndpointTrace {
    pub id: Uuid,
    pub request_id: Uuid,
    pub endpoint_id: Option<Uuid>,
    pub project_id: Option<Uuid>,
    pub gateway_session_id: Option<Uuid>,
    pub profile_id: Option<Uuid>,
    pub profile_version_id: Option<Uuid>,
    pub profile_slug: Option<String>,
    pub profile_name: Option<String>,
    pub profile_version_number: Option<i32>,
    pub operation: String,
    pub provider_key: Option<String>,
    pub capability_kind: Option<String>,
    pub selection_key: Option<String>,
    pub status: String,
    pub latency_ms: Option<i64>,
    pub request: Value,
    pub response: Option<Value>,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct TraceSpan {
    pub id: Uuid,
    pub trace_id: Uuid,
    pub parent_span_id: Option<Uuid>,
    pub name: String,
    pub kind: String,
    pub status: String,
    pub provider_key: Option<String>,
    pub capability_kind: Option<String>,
    pub duration_ms: Option<i64>,
    pub input_summary: Option<Value>,
    pub output_summary: Option<Value>,
    pub attributes: Value,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, FromRow)]
pub struct RealtimeSession {
    pub id: Uuid,
    pub project_id: Uuid,
    pub profile_id: Uuid,
    pub api_key_id: Option<Uuid>,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicAgent {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub version: i32,
    pub capabilities: Vec<String>,
    pub presentation: Value,
}

#[derive(Debug, Clone, FromRow)]
pub struct EndpointRoute {
    pub endpoint_id: Uuid,
    pub endpoint_slug: String,
    pub endpoint_name: String,
    pub request_timeout_ms: i32,
    pub profile_id: Uuid,
    pub binding_id: Uuid,
    pub gateway_id: String,
    pub agent_id: String,
    pub binding_config: Value,
}

#[derive(Debug, Clone, FromRow)]
pub struct ProfileRoute {
    pub profile_id: Uuid,
    pub profile_slug: String,
    pub profile_name: String,
    pub profile_version_id: Uuid,
    pub version_number: i32,
    pub capability_id: Uuid,
    pub capability_kind: String,
    pub provider_type: String,
    pub provider_key: String,
    pub resource_id: Option<String>,
    pub capability_config: Value,
    pub persona: Value,
    pub runtime: Value,
    pub presentation: Value,
    pub source: Value,
}

pub fn slugify(value: &str) -> String {
    let mut slug = String::with_capacity(value.len().min(64));
    let mut separator = false;
    for ch in value.trim().to_ascii_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            if separator && !slug.is_empty() {
                slug.push('-');
            }
            slug.push(ch);
            separator = false;
        } else {
            separator = true;
        }
        if slug.len() >= 64 {
            break;
        }
    }
    slug.trim_matches('-').to_string()
}

pub fn validate_slug(value: &str) -> bool {
    (3..=64).contains(&value.len())
        && value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || (index > 0 && byte == b'-')
        })
        && !value.ends_with('-')
        && !value.contains("--")
}

fn empty_object() -> Value {
    Value::Object(Default::default())
}

fn default_chat_capability() -> String {
    "chat".to_string()
}

#[cfg(test)]
mod tests {
    use super::{slugify, validate_slug};

    #[test]
    fn creates_stable_slugs() {
        assert_eq!(slugify("Town Guide Agent"), "town-guide-agent");
        assert!(validate_slug("town-guide-agent"));
    }

    #[test]
    fn rejects_unsafe_slugs() {
        assert!(!validate_slug("../admin"));
        assert!(!validate_slug("A"));
    }
}
