use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{AgentDefinition, EndpointDefinition, RuntimeError};

pub const RUNTIME_MANIFEST_SCHEMA_VERSION: u32 = 1;

const MAX_PROVIDERS: usize = 64;
const MAX_AGENTS: usize = 1_024;
const MAX_ENDPOINTS: usize = 2_048;
const MAX_MANIFEST_BYTES: usize = 4 * 1024 * 1024;
const SENSITIVE_KEYS: &[&str] = &[
    "apikey",
    "authorization",
    "credential",
    "password",
    "privatekey",
    "secret",
    "token",
];

/// A logical provider required by a runtime release.
///
/// Device credentials and filesystem paths are intentionally excluded. Hosts
/// resolve this logical requirement to a local provider installation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderRequirement {
    pub id: String,
    pub provider_type: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub settings: Value,
    #[serde(default)]
    pub resources: BTreeMap<String, String>,
}

/// Portable runtime configuration shared by embedded and connected runtimes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeManifest {
    pub schema_version: u32,
    pub project_id: String,
    #[serde(default)]
    pub providers: Vec<ProviderRequirement>,
    #[serde(default)]
    pub agents: Vec<AgentDefinition>,
    #[serde(default)]
    pub endpoints: Vec<EndpointDefinition>,
    #[serde(default)]
    pub metadata: Value,
}

impl RuntimeManifest {
    pub fn new(project_id: impl Into<String>) -> Self {
        Self {
            schema_version: RUNTIME_MANIFEST_SCHEMA_VERSION,
            project_id: project_id.into(),
            providers: Vec::new(),
            agents: Vec::new(),
            endpoints: Vec::new(),
            metadata: Value::Object(Default::default()),
        }
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, RuntimeError> {
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(RuntimeError::InvalidDefinition(
                "runtime manifest is too large".to_string(),
            ));
        }
        let manifest = serde_json::from_slice::<Self>(bytes)
            .map_err(|error| RuntimeError::InvalidDefinition(error.to_string()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn to_json(&self) -> Result<Vec<u8>, RuntimeError> {
        self.validate()?;
        serde_json::to_vec(self).map_err(|error| RuntimeError::Snapshot(error.to_string()))
    }

    pub fn content_hash(&self) -> Result<String, RuntimeError> {
        let bytes = self.to_json()?;
        let digest = Sha256::digest(bytes);
        let mut encoded = String::with_capacity(7 + digest.len() * 2);
        encoded.push_str("sha256:");
        for byte in digest {
            use std::fmt::Write;
            write!(&mut encoded, "{byte:02x}").map_err(|_error| RuntimeError::Internal)?;
        }
        Ok(encoded)
    }

    pub fn validate(&self) -> Result<(), RuntimeError> {
        if self.schema_version != RUNTIME_MANIFEST_SCHEMA_VERSION {
            return Err(RuntimeError::InvalidDefinition(format!(
                "unsupported runtime manifest schema version {}",
                self.schema_version
            )));
        }
        validate_portable_identifier("project", &self.project_id)?;
        validate_count("providers", self.providers.len(), MAX_PROVIDERS)?;
        validate_count("agents", self.agents.len(), MAX_AGENTS)?;
        validate_count("endpoints", self.endpoints.len(), MAX_ENDPOINTS)?;

        let mut provider_ids = HashSet::with_capacity(self.providers.len());
        for provider in &self.providers {
            validate_portable_identifier("provider", &provider.id)?;
            validate_portable_identifier("provider type", &provider.provider_type)?;
            if !provider_ids.insert(provider.id.as_str()) {
                return Err(duplicate("provider", &provider.id));
            }
            validate_capabilities(&provider.capabilities)?;
            validate_portable_provider_value(&provider.settings, "settings")?;
            for (name, reference) in &provider.resources {
                validate_portable_identifier("resource", name)?;
                validate_resource_reference(reference)?;
            }
        }

        let mut agent_ids = HashSet::with_capacity(self.agents.len());
        for agent in &self.agents {
            validate_portable_identifier("agent", &agent.id)?;
            validate_portable_identifier("provider", &agent.provider)?;
            if agent.name.trim().is_empty() {
                return Err(RuntimeError::InvalidDefinition(
                    "agent name is required".to_string(),
                ));
            }
            if !provider_ids.contains(agent.provider.as_str()) {
                return Err(RuntimeError::ProviderNotFound(agent.provider.clone()));
            }
            if !agent_ids.insert(agent.id.as_str()) {
                return Err(duplicate("agent", &agent.id));
            }
            validate_capabilities(&agent.capabilities)?;
            validate_portable_value(&agent.metadata, "agent metadata")?;
        }

        let mut endpoint_names = HashSet::with_capacity(self.endpoints.len());
        for endpoint in &self.endpoints {
            validate_portable_identifier("endpoint", &endpoint.name)?;
            validate_portable_identifier("agent", &endpoint.agent)?;
            validate_portable_identifier("capability", &endpoint.capability)?;
            if !agent_ids.contains(endpoint.agent.as_str()) {
                return Err(RuntimeError::AgentNotFound(endpoint.agent.clone()));
            }
            if !endpoint_names.insert(endpoint.name.as_str()) {
                return Err(duplicate("endpoint", &endpoint.name));
            }
            if !(1..=120_000).contains(&endpoint.timeout_ms) {
                return Err(RuntimeError::InvalidDefinition(
                    "endpoint timeout must be between 1 and 120000 milliseconds".to_string(),
                ));
            }
            let agent = self
                .agents
                .iter()
                .find(|agent| agent.id == endpoint.agent)
                .ok_or_else(|| RuntimeError::AgentNotFound(endpoint.agent.clone()))?;
            if !agent
                .capabilities
                .iter()
                .any(|capability| capability == &endpoint.capability)
            {
                return Err(RuntimeError::CapabilityUnavailable {
                    provider: agent.provider.clone(),
                    capability: endpoint.capability.clone(),
                });
            }
        }

        validate_portable_value(&self.metadata, "manifest metadata")?;

        if self.to_unchecked_json()?.len() > MAX_MANIFEST_BYTES {
            return Err(RuntimeError::InvalidDefinition(
                "runtime manifest is too large".to_string(),
            ));
        }
        Ok(())
    }

    fn to_unchecked_json(&self) -> Result<Vec<u8>, RuntimeError> {
        serde_json::to_vec(self).map_err(|error| RuntimeError::Snapshot(error.to_string()))
    }
}

/// One immutable, content-addressed project runtime release.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeRelease {
    pub version: u64,
    pub content_hash: String,
    pub manifest: RuntimeManifest,
}

impl RuntimeRelease {
    pub fn new(version: u64, manifest: RuntimeManifest) -> Result<Self, RuntimeError> {
        if version == 0 {
            return Err(RuntimeError::InvalidDefinition(
                "runtime release version must be greater than zero".to_string(),
            ));
        }
        manifest.validate()?;
        let content_hash = manifest.content_hash()?;
        Ok(Self {
            version,
            content_hash,
            manifest,
        })
    }

    pub fn validate(&self) -> Result<(), RuntimeError> {
        if self.version == 0 {
            return Err(RuntimeError::InvalidDefinition(
                "runtime release version must be greater than zero".to_string(),
            ));
        }
        self.manifest.validate()?;
        if self.content_hash != self.manifest.content_hash()? {
            return Err(RuntimeError::InvalidDefinition(
                "runtime release content hash does not match its manifest".to_string(),
            ));
        }
        Ok(())
    }
}

/// Local-only provider configuration kept outside portable releases.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalProviderBinding {
    pub provider_id: String,
    #[serde(default)]
    pub configuration: Value,
}

/// Redacted invocation metadata waiting to be uploaded to an optional server.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeTraceRecord {
    pub id: String,
    pub project_id: String,
    pub invocation_id: String,
    pub endpoint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability: Option<String>,
    pub status: String,
    pub duration_ms: u64,
    pub created_at_ms: u64,
}

impl RuntimeTraceRecord {
    pub fn validate(&self) -> Result<(), RuntimeError> {
        validate_trace_identifier("trace", &self.id)?;
        validate_portable_identifier("project", &self.project_id)?;
        validate_trace_identifier("invocation", &self.invocation_id)?;
        validate_portable_identifier("endpoint", &self.endpoint)?;
        for (kind, value) in [
            ("agent", self.agent.as_deref()),
            ("provider", self.provider.as_deref()),
            ("capability", self.capability.as_deref()),
        ] {
            if let Some(value) = value {
                validate_portable_identifier(kind, value)?;
            }
        }
        if !matches!(self.status.as_str(), "completed" | "cancelled" | "error") {
            return Err(RuntimeError::InvalidDefinition(
                "runtime trace status is invalid".to_string(),
            ));
        }
        if self.duration_ms > 24 * 60 * 60 * 1_000 {
            return Err(RuntimeError::InvalidDefinition(
                "runtime trace duration is invalid".to_string(),
            ));
        }
        if self.created_at_ms == 0 {
            return Err(RuntimeError::InvalidDefinition(
                "runtime trace timestamp is invalid".to_string(),
            ));
        }
        Ok(())
    }
}

fn validate_count(kind: &str, count: usize, maximum: usize) -> Result<(), RuntimeError> {
    if count > maximum {
        return Err(RuntimeError::InvalidDefinition(format!(
            "runtime manifest contains too many {kind}"
        )));
    }
    Ok(())
}

fn validate_capabilities(capabilities: &[String]) -> Result<(), RuntimeError> {
    if capabilities.is_empty() {
        return Err(RuntimeError::InvalidDefinition(
            "at least one capability is required".to_string(),
        ));
    }
    for capability in capabilities {
        validate_portable_identifier("capability", capability)?;
    }
    Ok(())
}

fn validate_resource_reference(reference: &str) -> Result<(), RuntimeError> {
    if reference.is_empty() || reference.len() > 512 {
        return Err(RuntimeError::InvalidDefinition(
            "resource reference must be between 1 and 512 bytes".to_string(),
        ));
    }
    if is_absolute_or_file_path(reference) {
        return Err(RuntimeError::InvalidDefinition(
            "runtime manifests cannot contain filesystem paths".to_string(),
        ));
    }
    Ok(())
}

fn validate_portable_provider_value(value: &Value, path: &str) -> Result<(), RuntimeError> {
    validate_portable_value(value, &format!("provider {path}"))
}

fn validate_portable_value(value: &Value, path: &str) -> Result<(), RuntimeError> {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                let normalized = key
                    .chars()
                    .filter(|character| character.is_ascii_alphanumeric())
                    .flat_map(char::to_lowercase)
                    .collect::<String>();
                if SENSITIVE_KEYS
                    .iter()
                    .any(|sensitive| normalized == *sensitive)
                {
                    return Err(RuntimeError::InvalidDefinition(format!(
                        "runtime manifest {path} contains a credential field"
                    )));
                }
                validate_portable_value(value, path)?;
            }
        }
        Value::Array(values) => {
            for value in values {
                validate_portable_value(value, path)?;
            }
        }
        Value::String(value) if is_absolute_or_file_path(value) => {
            return Err(RuntimeError::InvalidDefinition(format!(
                "runtime manifest {path} contains a filesystem path"
            )));
        }
        _ => {}
    }
    Ok(())
}

fn is_absolute_or_file_path(value: &str) -> bool {
    value.starts_with('/')
        || value.starts_with("file://")
        || (value.len() >= 3
            && value.as_bytes()[0].is_ascii_alphabetic()
            && value.as_bytes()[1] == b':'
            && matches!(value.as_bytes()[2], b'/' | b'\\'))
}

fn validate_portable_identifier(kind: &str, value: &str) -> Result<(), RuntimeError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(RuntimeError::InvalidDefinition(format!(
            "{kind} must be a portable identifier"
        )));
    }
    Ok(())
}

fn validate_trace_identifier(kind: &str, value: &str) -> Result<(), RuntimeError> {
    if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        return Err(RuntimeError::InvalidDefinition(format!(
            "{kind} must be a valid trace identifier"
        )));
    }
    Ok(())
}

fn duplicate(kind: &str, id: &str) -> RuntimeError {
    RuntimeError::InvalidDefinition(format!("duplicate {kind} {id}"))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn manifest() -> RuntimeManifest {
        RuntimeManifest {
            schema_version: RUNTIME_MANIFEST_SCHEMA_VERSION,
            project_id: "moon-train".to_string(),
            providers: vec![ProviderRequirement {
                id: "local-model".to_string(),
                provider_type: "openai-compatible".to_string(),
                capabilities: vec!["chat".to_string()],
                settings: json!({ "model": "small-chat" }),
                resources: BTreeMap::from([("model".to_string(), "model:small-chat".to_string())]),
            }],
            agents: vec![AgentDefinition {
                id: "guide".to_string(),
                name: "Guide".to_string(),
                provider: "local-model".to_string(),
                capabilities: vec!["chat".to_string()],
                metadata: json!({ "persona": "A calm guide." }),
            }],
            endpoints: vec![EndpointDefinition {
                name: "guide".to_string(),
                agent: "guide".to_string(),
                capability: "chat".to_string(),
                timeout_ms: 30_000,
            }],
            metadata: json!({ "title": "Last Train to the Moon" }),
        }
    }

    #[test]
    fn content_hash_is_stable_for_the_same_manifest() {
        let manifest = manifest();
        assert_eq!(
            manifest.content_hash().unwrap(),
            manifest.content_hash().unwrap()
        );
    }

    #[test]
    fn provider_credentials_are_rejected() {
        let mut manifest = manifest();
        manifest.providers[0].settings = json!({ "apiKey": "not-portable" });
        let error = manifest.validate().unwrap_err();
        assert!(error.to_string().contains("credential field"));
    }

    #[test]
    fn provider_filesystem_paths_are_rejected() {
        let mut manifest = manifest();
        manifest.providers[0]
            .resources
            .insert("model".to_string(), "/Users/example/model.gguf".to_string());
        let error = manifest.validate().unwrap_err();
        assert!(error.to_string().contains("filesystem paths"));
    }

    #[test]
    fn metadata_credentials_are_rejected() {
        let mut manifest = manifest();
        manifest.agents[0].metadata = json!({ "credentials": { "token": "not-portable" } });
        let error = manifest.validate().unwrap_err();
        assert!(error.to_string().contains("credential field"));
    }

    #[test]
    fn endpoint_timeouts_are_bounded() {
        let mut manifest = manifest();
        manifest.endpoints[0].timeout_ms = 0;
        let error = manifest.validate().unwrap_err();
        assert!(error.to_string().contains("endpoint timeout"));
    }

    #[test]
    fn trace_records_reject_unknown_statuses() {
        let trace = RuntimeTraceRecord {
            id: "trace-1".to_string(),
            project_id: "moon-train".to_string(),
            invocation_id: "invocation-1".to_string(),
            endpoint: "guide".to_string(),
            agent: Some("guide".to_string()),
            provider: Some("local-model".to_string()),
            capability: Some("chat".to_string()),
            status: "pending".to_string(),
            duration_ms: 1,
            created_at_ms: 1,
        };
        let error = trace.validate().unwrap_err();
        assert!(error.to_string().contains("trace status"));
    }
}
