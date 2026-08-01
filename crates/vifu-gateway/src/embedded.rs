use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use serde_json::Value;
use vifu_runtime::{InvocationData, InvocationInput, VifuRuntime};

use crate::relay::AgentGatewayProvider;

/// Exposes one logical provider in an embedded [`VifuRuntime`] to Vifu Server.
///
/// The runtime remains fully usable without this adapter. Adding the adapter
/// only makes its locally registered agents reachable through an Agent Gateway.
#[derive(Clone, Debug)]
pub struct EmbeddedRuntimeGatewayProvider {
    provider_id: String,
    runtime: VifuRuntime,
}

impl EmbeddedRuntimeGatewayProvider {
    pub fn new(provider_id: impl Into<String>, runtime: VifuRuntime) -> Self {
        Self {
            provider_id: provider_id.into(),
            runtime,
        }
    }

    fn endpoint_for(&self, agent_id: &str, binding: &Value) -> Result<String, String> {
        let configured = binding
            .get("runtimeEndpoint")
            .or_else(|| binding.pointer("/source/runtimeEndpoint"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let endpoints = self
            .runtime
            .endpoint_definitions()
            .map_err(|error| error.public_message())?;
        if let Some(configured) = configured {
            return endpoints
                .iter()
                .find(|endpoint| endpoint.name == configured && endpoint.agent == agent_id)
                .map(|endpoint| endpoint.name.clone())
                .ok_or_else(|| {
                    "the configured embedded runtime endpoint does not belong to this agent"
                        .to_string()
                });
        }

        let mut matching = endpoints
            .iter()
            .filter(|endpoint| endpoint.agent == agent_id)
            .collect::<Vec<_>>();
        if matching.len() == 1 {
            return Ok(matching[0].name.clone());
        }
        if let Some(endpoint) = matching.iter().find(|endpoint| endpoint.name == agent_id) {
            return Ok(endpoint.name.clone());
        }
        matching.sort_by(|left, right| left.name.cmp(&right.name));
        match matching.len() {
            0 => Err("the embedded agent has no runtime endpoint".to_string()),
            _ => Err(
                "the embedded agent has multiple endpoints; set binding.runtimeEndpoint"
                    .to_string(),
            ),
        }
    }
}

impl AgentGatewayProvider for EmbeddedRuntimeGatewayProvider {
    fn id(&self) -> &str {
        &self.provider_id
    }

    fn provider_type(&self) -> &str {
        "vifu-runtime"
    }

    fn invoke<'a>(
        &'a self,
        agent_id: &'a str,
        binding: &'a Value,
        input: &'a Value,
        timeout: Duration,
    ) -> Pin<Box<dyn Future<Output = Result<Value, String>> + Send + 'a>> {
        Box::pin(async move {
            let agent = self
                .runtime
                .agent_definitions()
                .map_err(|error| error.public_message())?
                .into_iter()
                .find(|agent| agent.id == agent_id)
                .ok_or_else(|| "the embedded agent is not registered".to_string())?;
            if agent.provider != self.provider_id {
                return Err("the embedded agent belongs to another provider".to_string());
            }
            let endpoint = self.endpoint_for(agent_id, binding)?;
            let session_id = binding
                .get("sessionId")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .unwrap_or("gateway-session")
                .to_string();
            let invocation = self.runtime.invoke(InvocationInput {
                endpoint,
                session_id,
                data: InvocationData::Json(input.clone()),
                metadata: serde_json::json!({ "source": "agent-gateway" }),
            });
            let output = tokio::time::timeout(timeout, invocation)
                .await
                .map_err(|_| "embedded runtime request timed out".to_string())?
                .map_err(|error| error.public_message())?;
            match output.data {
                InvocationData::Json(value) => Ok(value),
                InvocationData::Binary(bytes) => Ok(serde_json::json!({
                    "format": "binary",
                    "bytes": bytes,
                })),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;
    use vifu_runtime::{
        AgentDefinition, AgentProvider, CancellationToken, EndpointDefinition, ProviderFuture,
        ProviderRequest, ProviderResponse, RuntimeError,
    };

    use super::*;

    struct EchoProvider;

    impl AgentProvider for EchoProvider {
        fn supports(&self, capability: &str) -> bool {
            capability == "chat"
        }

        fn invoke<'a>(
            &'a self,
            request: ProviderRequest,
            _cancellation: CancellationToken,
        ) -> ProviderFuture<'a> {
            Box::pin(async move { Ok(ProviderResponse::json(request.data_json()?)) })
        }
    }

    trait RequestJson {
        fn data_json(self) -> Result<Value, RuntimeError>;
    }

    impl RequestJson for ProviderRequest {
        fn data_json(self) -> Result<Value, RuntimeError> {
            match self.data {
                InvocationData::Json(value) => Ok(value),
                InvocationData::Binary(_) => {
                    Err(RuntimeError::InvalidDefinition("expected JSON".to_string()))
                }
            }
        }
    }

    fn runtime() -> VifuRuntime {
        let runtime = VifuRuntime::new("moon-train").unwrap();
        runtime
            .register_provider("local-llama", Arc::new(EchoProvider))
            .unwrap();
        runtime
            .register_agent(AgentDefinition {
                id: "mizuki".to_string(),
                name: "Mizuki".to_string(),
                provider: "local-llama".to_string(),
                capabilities: vec!["chat".to_string()],
                metadata: json!({}),
            })
            .unwrap();
        runtime
            .register_endpoint(EndpointDefinition {
                name: "mizuki-chat".to_string(),
                agent: "mizuki".to_string(),
                capability: "chat".to_string(),
                timeout_ms: 1_000,
            })
            .unwrap();
        runtime
    }

    #[tokio::test]
    async fn invokes_the_endpoint_owned_by_the_discovered_agent() {
        let provider = EmbeddedRuntimeGatewayProvider::new("local-llama", runtime());
        let output = provider
            .invoke(
                "mizuki",
                &json!({}),
                &json!({ "message": "hello" }),
                Duration::from_secs(1),
            )
            .await
            .unwrap();
        assert_eq!(output, json!({ "message": "hello" }));
    }

    #[test]
    fn requires_an_explicit_endpoint_when_an_agent_has_more_than_one() {
        let runtime = runtime();
        runtime
            .register_endpoint(EndpointDefinition {
                name: "mizuki-private".to_string(),
                agent: "mizuki".to_string(),
                capability: "chat".to_string(),
                timeout_ms: 1_000,
            })
            .unwrap();
        let provider = EmbeddedRuntimeGatewayProvider::new("local-llama", runtime);
        assert!(provider.endpoint_for("mizuki", &json!({})).is_err());
        assert_eq!(
            provider
                .endpoint_for("mizuki", &json!({ "runtimeEndpoint": "mizuki-private" }))
                .unwrap(),
            "mizuki-private"
        );
    }
}
