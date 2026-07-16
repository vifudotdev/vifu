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
    pub agent_gateways: bool,
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
            agent_gateways: true,
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
#[serde(rename_all = "camelCase")]
pub struct CreateProject {
    pub slug: Option<String>,
    pub name: String,
    pub description: Option<String>,
    pub gateway_id: String,
    #[serde(default)]
    pub binding_ids: Vec<Uuid>,
    #[serde(default)]
    pub agent_ids: Vec<String>,
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
pub struct AgentProfile {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateProfile {
    pub slug: Option<String>,
    pub name: String,
    pub description: Option<String>,
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

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ApiKeyRecord {
    pub id: Uuid,
    pub endpoint_id: Uuid,
    pub name: String,
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
    pub endpoint_id: Uuid,
    pub name: Option<String>,
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
    pub status: String,
    pub latency_ms: Option<i64>,
    pub request: Value,
    pub response: Option<Value>,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvokeEndpoint {
    pub message: Option<String>,
    pub input: Option<Value>,
    pub context: Option<Value>,
    pub metadata: Option<Value>,
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
