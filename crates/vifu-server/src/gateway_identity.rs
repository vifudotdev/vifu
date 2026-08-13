use std::fmt::Write;

use serde_json::Value;
use sha2::{Digest, Sha256};

use vifu_gateway::protocol::AgentDescriptor;

use crate::models::AvailableAgent;

const HASH_BYTES: usize = 12;
const MAX_IDENTIFIER_LEN: usize = 128;
const MAX_GATEWAY_NAME_CHARS: usize = 128;

pub(crate) fn scoped_provider_key(gateway_id: &str, runtime_provider_key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(gateway_id.as_bytes());
    hasher.update([0]);
    hasher.update(runtime_provider_key.as_bytes());
    let digest = hasher.finalize();
    let mut suffix = String::with_capacity(HASH_BYTES * 2);
    for byte in &digest[..HASH_BYTES] {
        let _ = write!(&mut suffix, "{byte:02x}");
    }
    let separator = "--";
    let prefix_len = MAX_IDENTIFIER_LEN - separator.len() - suffix.len();
    let prefix = runtime_provider_key
        .chars()
        .take(prefix_len)
        .collect::<String>();
    format!("{prefix}{separator}{suffix}")
}

pub(crate) fn scope_available_agent(mut agent: AvailableAgent) -> AvailableAgent {
    let Some(metadata) = agent.metadata.as_object_mut() else {
        return agent;
    };
    let Some(runtime_provider_key) = metadata
        .get("providerKey")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return agent;
    };
    metadata.insert(
        "runtimeProviderKey".to_string(),
        Value::String(runtime_provider_key.clone()),
    );
    metadata.insert(
        "providerKey".to_string(),
        Value::String(scoped_provider_key(
            &agent.gateway_id,
            &runtime_provider_key,
        )),
    );
    agent
}

pub(crate) fn normalized_gateway_metadata(metadata: Value, agents: &[AgentDescriptor]) -> Value {
    let mut metadata = metadata.as_object().cloned().unwrap_or_default();
    let reported_name = metadata
        .get("name")
        .and_then(Value::as_str)
        .and_then(bounded_gateway_name)
        .or_else(|| {
            metadata
                .get("application")
                .and_then(Value::as_object)
                .and_then(|application| application.get("name"))
                .and_then(Value::as_str)
                .and_then(bounded_gateway_name)
        })
        .or_else(|| {
            agents
                .first()
                .and_then(|agent| bounded_gateway_name(&agent.name))
        })
        .unwrap_or_else(|| "Vifu Gateway".to_string());
    metadata.insert("name".to_string(), Value::String(reported_name));
    Value::Object(metadata)
}

fn bounded_gateway_name(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > MAX_GATEWAY_NAME_CHARS {
        return None;
    }
    Some(value.to_string())
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use vifu_gateway::protocol::AgentDescriptor;

    use super::{normalized_gateway_metadata, scoped_provider_key};

    #[test]
    fn provider_identity_is_stable_and_gateway_scoped() {
        let first = scoped_provider_key("gateway-one", "android-llama");
        let repeated = scoped_provider_key("gateway-one", "android-llama");
        let second = scoped_provider_key("gateway-two", "android-llama");

        assert_eq!(first, repeated);
        assert_ne!(first, second);
        assert!(first.starts_with("android-llama--"));
        assert!(first.len() <= 128);
        assert!(vifu_gateway::protocol::validate_identifier("provider key", &first).is_ok());
    }

    #[test]
    fn provider_identity_bounds_long_runtime_keys() {
        let key = scoped_provider_key("gateway-one", &"a".repeat(128));

        assert_eq!(key.len(), 128);
        assert!(vifu_gateway::protocol::validate_identifier("provider key", &key).is_ok());
    }

    #[test]
    fn gateway_metadata_preserves_a_reported_device_name() {
        let metadata = normalized_gateway_metadata(
            json!({
                "name": "Kitchen light",
                "kind": "light",
                "device": { "manufacturer": "Example" },
            }),
            &[],
        );

        assert_eq!(metadata["name"], "Kitchen light");
    }

    #[test]
    fn gateway_metadata_uses_the_agent_name_for_legacy_clients() {
        let metadata = normalized_gateway_metadata(
            json!({ "adapter": "vifu" }),
            &[AgentDescriptor {
                id: "light-agent".to_string(),
                name: "Legacy kitchen light".to_string(),
                metadata: json!({}),
            }],
        );

        assert_eq!(metadata["name"], "Legacy kitchen light");
    }
}
