use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    #[serde(rename = "apps")]
    pub projects: bool,
    pub profiles: bool,
    pub endpoints: bool,
    pub bindings: bool,
    pub api_keys: bool,
    pub agent_gateways: bool,
    pub provider_connections: bool,
    pub traces: bool,
    pub runtime_extensions: bool,
}

impl Capabilities {
    pub fn self_hosted() -> Self {
        Self {
            projects: true,
            profiles: true,
            endpoints: true,
            bindings: true,
            api_keys: true,
            agent_gateways: true,
            provider_connections: true,
            traces: true,
            runtime_extensions: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: Uuid,
    pub app_id: String,
    #[serde(skip_serializing)]
    pub owner_user_id: Option<String>,
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssignProjectOwner {
    pub owner_user_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectOwnership {
    pub project_id: Uuid,
    pub slug: String,
    pub name: String,
    pub owner_user_id: Option<String>,
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
pub struct RuntimeDeployment {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub is_primary: bool,
    pub config_sync_enabled: bool,
    pub trace_mode: String,
    pub remote_invocation_enabled: bool,
    pub active_release_version: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeDeploymentView {
    #[serde(flatten)]
    pub deployment: RuntimeDeployment,
    pub gateway_ids: Vec<String>,
    pub apply_states: Vec<RuntimeDeploymentApplyState>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeDeploymentApplyState {
    pub deployment_id: Uuid,
    pub gateway_id: String,
    pub release_version: i64,
    pub content_hash: String,
    pub applied_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeDistribution {
    pub id: Uuid,
    pub project_id: Uuid,
    pub deployment_id: Uuid,
    pub name: String,
    pub public_id: String,
    pub status: String,
    pub max_gateways: i64,
    pub created_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateRuntimeDistribution {
    pub name: String,
    pub deployment: Option<String>,
    pub max_gateways: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReportRuntimeReleaseApplied {
    pub deployment_id: Uuid,
    pub release_version: i64,
    pub content_hash: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateRuntimeDeployment {
    pub name: String,
    pub config_sync_enabled: Option<bool>,
    pub trace_mode: Option<String>,
    pub remote_invocation_enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateRuntimeDeployment {
    pub config_sync_enabled: Option<bool>,
    pub trace_mode: Option<String>,
    pub remote_invocation_enabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRuntimeRelease {
    pub id: Uuid,
    pub project_id: Uuid,
    pub version: i64,
    pub content_hash: String,
    pub manifest: Value,
    pub created_by: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportProjectSettings {
    #[serde(alias = "manifest")]
    pub settings: Value,
}

pub type PublishRuntimeRelease = ImportProjectSettings;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BootstrapGatewayRuntimeRelease {
    pub deployment_id: Uuid,
    #[serde(alias = "manifest")]
    pub settings: Value,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRuntimeExtension {
    pub project_id: Uuid,
    pub extension_id: String,
    pub enabled: bool,
    pub active_release_ref: Option<String>,
    pub metadata: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetProjectRuntimeExtension {
    pub extension_id: String,
    pub enabled: Option<bool>,
    pub active_release_ref: Option<String>,
    #[serde(default = "empty_object")]
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRuntimeChannel {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub public_id: Uuid,
    pub launch_key_prefix: String,
    pub allowed_origins: Vec<String>,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateProjectRuntimeChannel {
    pub name: String,
    #[serde(default)]
    pub allowed_origins: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateRuntimeLaunchSession {
    pub channel_id: Uuid,
    pub launch_key: String,
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
    pub source_kind: String,
    pub source_key: String,
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
    pub source_kind: String,
    pub source_key: String,
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
pub struct CustomProvider {
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

impl From<ProviderConnectionSecret> for ProviderConnection {
    fn from(value: ProviderConnectionSecret) -> Self {
        Self {
            id: value.id,
            project_id: value.project_id,
            provider_key: value.provider_key,
            source_kind: value.source_kind,
            source_key: value.source_key,
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderSourceInput {
    pub kind: String,
    pub key: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateProjectProvider {
    pub source: ProviderSourceInput,
    pub name: Option<String>,
    pub base_url: Option<String>,
    #[serde(default = "empty_object")]
    pub config: Value,
    #[serde(default = "empty_object")]
    pub secrets: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportProjectProvider {
    pub provider_key: String,
    pub name: String,
    pub provider_type: String,
    pub base_url: String,
    #[serde(default = "empty_object")]
    pub config: Value,
    #[serde(default = "empty_object")]
    pub secrets: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateProjectProvider {
    pub name: Option<String>,
    pub base_url: Option<String>,
    pub config: Option<Value>,
    pub secrets: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportProjectAgent {
    pub gateway_id: String,
    pub agent_id: String,
    pub provider_key: String,
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportProfileVersion {
    pub archive_id: String,
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportProjectProfile {
    pub archive_id: String,
    pub slug: Option<String>,
    pub name: String,
    pub description: Option<String>,
    pub active_version_id: String,
    pub versions: Vec<ImportProfileVersion>,
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "lowercase", deny_unknown_fields)]
pub enum ApiKeyAgentScope {
    #[default]
    All,
    Selected {
        #[serde(rename = "profileIds")]
        profile_ids: Vec<Uuid>,
    },
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EndpointPermission {
    #[default]
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
    #[serde(default)]
    pub embeddings: EndpointPermission,
    pub speech: EndpointPermission,
    pub transcriptions: EndpointPermission,
    pub realtime: EndpointPermission,
    pub runtime: EndpointPermission,
    pub agents: ResourcePermission,
    pub project: ResourcePermission,
}

impl Default for ApiKeyPermissions {
    fn default() -> Self {
        Self {
            chat_completions: EndpointPermission::Access,
            embeddings: EndpointPermission::Access,
            speech: EndpointPermission::None,
            transcriptions: EndpointPermission::None,
            realtime: EndpointPermission::None,
            runtime: EndpointPermission::None,
            agents: ResourcePermission::None,
            project: ResourcePermission::None,
        }
    }
}

impl ApiKeyPermissions {
    pub fn chat_completions_allowed(&self) -> bool {
        self.chat_completions == EndpointPermission::Access
    }

    pub fn embeddings_allowed(&self) -> bool {
        self.embeddings == EndpointPermission::Access
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

    pub fn runtime_allowed(&self) -> bool {
        self.runtime == EndpointPermission::Access
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClaimGuestProject {
    pub claim_token: String,
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
pub struct AgentGatewayAuthorization {
    pub gateway_id: String,
    pub machine_id: String,
    pub owner_user_id: Option<String>,
    pub status: String,
    pub token_prefix: String,
    pub token_generation: i64,
    pub token_expires_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct AgentGatewayPairingRequest {
    pub id: Uuid,
    pub machine_id: String,
    pub status: String,
    pub owner_user_id: Option<String>,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
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

#[derive(Debug, Clone, FromRow)]
pub struct ArchivedProjectAgentSource {
    pub profile_id: Uuid,
    pub name: String,
    pub gateway_id: String,
    pub agent_id: String,
    pub provider_key: String,
    pub provider_type: String,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct EndpointTrace {
    pub id: Uuid,
    pub request_id: Uuid,
    pub endpoint_id: Option<Uuid>,
    pub project_id: Option<Uuid>,
    pub gateway_session_id: Option<Uuid>,
    pub gateway_id: Option<String>,
    pub gateway_name: Option<String>,
    pub gateway_metadata: Option<Value>,
    pub profile_id: Option<Uuid>,
    pub profile_version_id: Option<Uuid>,
    pub profile_slug: Option<String>,
    pub profile_name: Option<String>,
    pub profile_version_number: Option<i32>,
    pub operation: String,
    pub provider_key: Option<String>,
    pub provider_name: Option<String>,
    pub capability_kind: Option<String>,
    pub selection_key: Option<String>,
    pub status: String,
    pub latency_ms: Option<i64>,
    pub model: Option<String>,
    pub completion_start_ms: Option<i64>,
    pub usage: Option<Value>,
    pub decode_ms: Option<i64>,
    pub app_outcome: Option<String>,
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
    pub observation_type: String,
    pub status: String,
    pub provider_key: Option<String>,
    pub capability_kind: Option<String>,
    pub model: Option<String>,
    pub model_parameters: Option<Value>,
    pub completion_start_ms: Option<i64>,
    pub usage: Option<Value>,
    pub duration_ms: Option<i64>,
    pub input_summary: Option<Value>,
    pub output_summary: Option<Value>,
    pub attributes: Value,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct TraceScore {
    pub id: Uuid,
    pub trace_id: Uuid,
    pub span_id: Option<Uuid>,
    pub name: String,
    pub data_type: String,
    pub value: Value,
    pub source: String,
    pub created_at: DateTime<Utc>,
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
    use serde_json::json;
    use uuid::Uuid;

    use super::{
        slugify, validate_slug, ApiKeyPermissions, BootstrapGatewayRuntimeRelease,
        EndpointPermission, ImportProjectSettings,
    };

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

    #[test]
    fn old_api_key_permissions_do_not_gain_embedding_access() {
        let permissions = serde_json::from_value::<ApiKeyPermissions>(serde_json::json!({
            "chatCompletions": "access",
            "speech": "none",
            "transcriptions": "none",
            "realtime": "none",
            "runtime": "none",
            "agents": "none",
            "project": "none"
        }))
        .unwrap();

        assert_eq!(permissions.embeddings, EndpointPermission::None);
    }

    #[test]
    fn import_project_settings_accepts_settings_payload() {
        let input: ImportProjectSettings = serde_json::from_value(json!({
            "settings": {
                "schemaVersion": 1,
                "projectId": "project",
                "providers": [],
                "agents": [],
                "endpoints": [],
                "metadata": {}
            }
        }))
        .unwrap();

        assert_eq!(input.settings["projectId"], "project");
    }

    #[test]
    fn import_project_settings_accepts_manifest_alias() {
        let input: ImportProjectSettings = serde_json::from_value(json!({
            "manifest": {
                "schemaVersion": 1,
                "projectId": "project",
                "providers": [],
                "agents": [],
                "endpoints": [],
                "metadata": {}
            }
        }))
        .unwrap();

        assert_eq!(input.settings["projectId"], "project");
    }

    #[test]
    fn gateway_bootstrap_accepts_manifest_alias() {
        let deployment_id = Uuid::new_v4();
        let input: BootstrapGatewayRuntimeRelease = serde_json::from_value(json!({
            "deploymentId": deployment_id,
            "manifest": {
                "schemaVersion": 1,
                "projectId": "project",
                "providers": [],
                "agents": [],
                "endpoints": [],
                "metadata": {}
            }
        }))
        .unwrap();

        assert_eq!(input.deployment_id, deployment_id);
        assert_eq!(input.settings["projectId"], "project");
    }
}
