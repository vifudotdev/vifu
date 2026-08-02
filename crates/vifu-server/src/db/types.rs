use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

use crate::models::{ApiKeyAgentScope, ApiKeyPermissions, ProfileCapabilityDraft};

pub struct NewProjectRuntimeChannel<'a> {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: &'a str,
    pub public_id: Uuid,
    pub launch_key_prefix: &'a str,
    pub launch_key_hash: &'a [u8],
    pub allowed_origins: &'a [String],
}

pub struct NewProject<'a> {
    pub id: Uuid,
    pub owner_user_id: Option<&'a str>,
    pub slug: &'a str,
    pub name: &'a str,
    pub description: Option<&'a str>,
    pub gateway_id: &'a str,
    pub binding_ids: &'a [Uuid],
}

pub struct NewRuntimeDeployment<'a> {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: &'a str,
    pub is_primary: bool,
    pub config_sync_enabled: bool,
    pub trace_mode: &'a str,
    pub remote_invocation_enabled: bool,
}

pub struct RuntimeDeploymentPatch<'a> {
    pub config_sync_enabled: Option<bool>,
    pub trace_mode: Option<&'a str>,
    pub remote_invocation_enabled: Option<bool>,
}

pub struct NewProjectRuntimeRelease<'a> {
    pub id: Uuid,
    pub project_id: Uuid,
    pub version: i64,
    pub content_hash: &'a str,
    pub manifest: &'a Value,
    pub created_by: Option<&'a str>,
}

pub struct ProjectPatch<'a> {
    pub slug: Option<&'a str>,
    pub name: Option<&'a str>,
    pub description_changed: bool,
    pub description: Option<&'a str>,
    pub gateway_id: Option<&'a str>,
    pub enabled: Option<bool>,
    pub binding_ids: Option<&'a [Uuid]>,
}

pub struct NewProviderConnection<'a> {
    pub provider_key: &'a str,
    pub source_kind: &'a str,
    pub source_key: &'a str,
    pub name: &'a str,
    pub provider_type: &'a str,
    pub base_url: &'a str,
    pub config: &'a Value,
    pub encrypted_secret_json: &'a str,
    pub secret_keys: &'a [String],
    pub display_secret: Option<&'a str>,
    pub status: &'a str,
}

pub struct ProfilePatch<'a> {
    pub slug: Option<&'a str>,
    pub name: Option<&'a str>,
    pub description_changed: bool,
    pub description: Option<&'a str>,
}

pub struct NewProfileVersion<'a> {
    pub persona: &'a Value,
    pub runtime: &'a Value,
    pub presentation: &'a Value,
    pub source: &'a Value,
    pub capabilities: &'a [ProfileCapabilityDraft],
    pub change_summary: Option<&'a str>,
}

pub struct NewEndpoint<'a> {
    pub id: Uuid,
    pub slug: &'a str,
    pub name: &'a str,
    pub profile_id: Uuid,
    pub binding_id: Uuid,
    pub enabled: bool,
    pub request_timeout_ms: i32,
}

pub struct EndpointPatch<'a> {
    pub slug: Option<&'a str>,
    pub name: Option<&'a str>,
    pub profile_id: Option<Uuid>,
    pub binding_id: Option<Uuid>,
    pub enabled: Option<bool>,
    pub request_timeout_ms: Option<i32>,
}

pub struct NewApiKey<'a> {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: &'a str,
    pub agent_scope: &'a ApiKeyAgentScope,
    pub permissions: &'a ApiKeyPermissions,
    pub key_prefix: &'a str,
    pub key_hash: &'a [u8],
}

pub struct ApiKeyPatch<'a> {
    pub project_id: Option<Uuid>,
    pub name: Option<&'a str>,
    pub agent_scope: Option<&'a ApiKeyAgentScope>,
    pub permissions: Option<&'a ApiKeyPermissions>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentGatewayRegistration {
    Registered,
    Existing,
}

pub struct NewAgentGatewayEnrollment<'a> {
    pub id: Uuid,
    pub project_id: Uuid,
    pub owner_user_id: &'a str,
    pub deployment_id: Uuid,
    pub token_hash: &'a [u8],
    pub expires_at: DateTime<Utc>,
}

pub struct NewAgentGatewayAuthorization<'a> {
    pub gateway_id: &'a str,
    pub machine_id: &'a str,
    pub owner_user_id: Option<&'a str>,
    pub token_prefix: &'a str,
    pub token_hash: &'a [u8],
    pub token_expires_at: DateTime<Utc>,
}

pub struct RotatedAgentGatewayAuthorization<'a> {
    pub gateway_id: &'a str,
    pub token_prefix: &'a str,
    pub token_hash: &'a [u8],
    pub token_expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct AgentGatewayEnrollmentAssignment {
    pub project_id: Uuid,
    pub deployment_id: Uuid,
    pub owner_user_id: String,
}

pub struct NewGuestProject<'a> {
    pub project_id: Uuid,
    pub gateway_id: &'a str,
    pub claim_token_hash: &'a [u8],
    pub expires_at: DateTime<Utc>,
}

pub struct NewTrace<'a> {
    pub request_id: Uuid,
    pub endpoint_id: Option<Uuid>,
    pub project_id: Option<Uuid>,
    pub gateway_session_id: Option<Uuid>,
    pub profile_id: Option<Uuid>,
    pub profile_version_id: Option<Uuid>,
    pub operation: &'a str,
    pub provider_key: Option<&'a str>,
    pub capability_kind: Option<&'a str>,
    pub selection_key: Option<&'a str>,
    pub request: &'a Value,
}

pub struct NewUploadedRuntimeTrace<'a> {
    pub id: Uuid,
    pub request_id: Uuid,
    pub project_id: Uuid,
    pub operation: &'a str,
    pub provider_key: Option<&'a str>,
    pub capability_kind: Option<&'a str>,
    pub status: &'a str,
    pub latency_ms: i64,
    pub request: &'a Value,
    pub created_at: DateTime<Utc>,
}

pub struct NewTraceSpan<'a> {
    pub trace_id: Uuid,
    pub parent_span_id: Option<Uuid>,
    pub name: &'a str,
    pub kind: &'a str,
    pub observation_type: &'a str,
    pub provider_key: Option<&'a str>,
    pub capability_kind: Option<&'a str>,
    pub model: Option<&'a str>,
    pub model_parameters: Option<&'a Value>,
    pub input_summary: Option<&'a Value>,
    pub attributes: &'a Value,
}

pub struct RuntimeTraceObservation<'a> {
    pub id: Uuid,
    pub trace_id: Uuid,
    pub parent_span_id: Option<Uuid>,
    pub name: &'a str,
    pub kind: &'a str,
    pub observation_type: &'a str,
    pub provider_key: Option<&'a str>,
    pub capability_kind: Option<&'a str>,
    pub model: Option<&'a str>,
    pub status: &'a str,
    pub duration_ms: Option<i64>,
    pub attributes: &'a Value,
    pub error: Option<&'a str>,
}

pub struct NewTraceScore<'a> {
    pub trace_id: Uuid,
    pub span_id: Option<Uuid>,
    pub name: &'a str,
    pub data_type: &'a str,
    pub value: &'a Value,
    pub source: &'a str,
}

#[derive(Debug, Clone)]
pub struct TraceFeedbackTarget {
    pub trace_id: Uuid,
    pub project_id: Uuid,
    pub profile_id: Option<Uuid>,
    pub parent_span_id: Option<Uuid>,
    pub gateway_session_id: Option<Uuid>,
    pub capability_kind: Option<String>,
    pub trace_created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct RuntimeTraceTarget {
    pub trace_id: Uuid,
    pub parent_span_id: Option<Uuid>,
    pub provider_key: Option<String>,
    pub capability_kind: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceIdentity {
    pub project_id: Option<Uuid>,
    pub profile_id: Option<Uuid>,
}

pub fn elapsed_millis(started_at: std::time::Instant) -> i64 {
    let millis = started_at.elapsed().as_millis();
    i64::try_from(millis).unwrap_or(i64::MAX)
}

pub fn timestamp() -> DateTime<Utc> {
    Utc::now()
}
