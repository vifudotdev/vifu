use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;
use vifu_runtime::{InvocationData, InvocationInput, RuntimeManifest, VifuRuntime};

use crate::protocol::{self, AgentDescriptor};
use crate::relay::AgentGatewayProvider;
use crate::relay::{self, AgentGatewayRuntime};
use crate::session::SessionSummary;

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

/// Configuration for exposing an embedded runtime through Vifu Agent Gateway.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmbeddedRuntimeGatewayConfig {
    pub server_url: String,
    pub gateway_id: String,
    pub runtime_database_path: PathBuf,
}

impl EmbeddedRuntimeGatewayConfig {
    pub fn new(
        server_url: impl Into<String>,
        gateway_id: impl Into<String>,
        runtime_database_path: impl Into<PathBuf>,
    ) -> Self {
        Self {
            server_url: server_url.into(),
            gateway_id: gateway_id.into(),
            runtime_database_path: runtime_database_path.into(),
        }
    }
}

/// Current state of an [`EmbeddedRuntimeGateway`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmbeddedRuntimeGatewayState {
    Stopped,
    Running,
    Failed,
}

/// Observable lifecycle state for an [`EmbeddedRuntimeGateway`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmbeddedRuntimeGatewayStatus {
    pub state: EmbeddedRuntimeGatewayState,
    pub last_error: Option<String>,
}

struct EmbeddedGatewayTask {
    shutdown: tokio::sync::oneshot::Sender<()>,
    thread: JoinHandle<()>,
}

/// Runs Agent Gateway beside a manifest-configured [`VifuRuntime`].
///
/// The runtime and all locally registered providers remain in process. Starting
/// the gateway only makes the manifest's agents discoverable through the
/// configured Vifu Server.
pub struct EmbeddedRuntimeGateway {
    runtime: VifuRuntime,
    config: EmbeddedRuntimeGatewayConfig,
    task: Mutex<Option<EmbeddedGatewayTask>>,
    status: Arc<Mutex<EmbeddedRuntimeGatewayStatus>>,
}

impl EmbeddedRuntimeGateway {
    /// Creates a stopped gateway and validates its network identity.
    pub fn new(runtime: VifuRuntime, config: EmbeddedRuntimeGatewayConfig) -> Result<Self, String> {
        relay::agent_gateway_websocket_url(&config.server_url)?;
        protocol::validate_identifier("agent gateway id", &config.gateway_id)?;
        if config.runtime_database_path.as_os_str().is_empty() {
            return Err("runtime database path is required".to_string());
        }
        Ok(Self {
            runtime,
            config,
            task: Mutex::new(None),
            status: Arc::new(Mutex::new(EmbeddedRuntimeGatewayStatus {
                state: EmbeddedRuntimeGatewayState::Stopped,
                last_error: None,
            })),
        })
    }

    /// Starts the network gateway. The credential is kept in memory only.
    pub fn start(
        &self,
        gateway_credential: String,
        enrollment_token: Option<String>,
    ) -> Result<(), String> {
        crate::session::validate_gateway_credential(&gateway_credential)?;
        let manifest = runtime_manifest(&self.runtime)?;
        let (providers, agents) = gateway_components(&self.runtime, &manifest);
        let mut task = self
            .task
            .lock()
            .map_err(|_| "embedded gateway lifecycle is unavailable".to_string())?;
        if task.as_ref().is_some_and(|task| !task.thread.is_finished()) {
            return Ok(());
        }
        if let Some(finished) = task.take() {
            finished
                .thread
                .join()
                .map_err(|_| "embedded gateway worker stopped unexpectedly".to_string())?;
        }

        let server_url = self.config.server_url.clone();
        let gateway_id = self.config.gateway_id.clone();
        let runtime_database_path = self.config.runtime_database_path.clone();
        let embedded_runtime = self.runtime.clone();
        let status = Arc::clone(&self.status);
        let (shutdown, shutdown_receiver) = tokio::sync::oneshot::channel();
        set_gateway_status(&status, EmbeddedRuntimeGatewayState::Running, None)?;
        let thread = match std::thread::Builder::new()
            .name("vifu-embedded-gateway".to_string())
            .spawn(move || {
                let result = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|error| error.to_string())
                    .and_then(|tokio| {
                        let mut session = SessionSummary {
                            gateway_id,
                            gateway_credential,
                            resume_session_id: None,
                            created_at_unix: SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .map(|duration| duration.as_secs())
                                .unwrap_or(1),
                            guest_project: None,
                        };
                        tokio.block_on(async {
                            let gateway = AgentGatewayRuntime {
                                server_url: &server_url,
                                agent_gateway_bootstrap_token: None,
                                enrollment_token,
                                allow_guest_bootstrap: false,
                                providers: &providers,
                                agents: &agents,
                                session_path: None,
                                runtime_database_path: &runtime_database_path,
                                embedded_runtime: Some(&embedded_runtime),
                            };
                            tokio::select! {
                                result = relay::run_agent_gateway(gateway, &mut session) => result,
                                _ = shutdown_receiver => Ok(()),
                            }
                        })
                    });
                match result {
                    Ok(()) => {
                        let _ =
                            set_gateway_status(&status, EmbeddedRuntimeGatewayState::Stopped, None);
                    }
                    Err(error) => {
                        let _ = set_gateway_status(
                            &status,
                            EmbeddedRuntimeGatewayState::Failed,
                            Some(error),
                        );
                    }
                }
            }) {
            Ok(thread) => thread,
            Err(error) => {
                set_gateway_status(
                    &self.status,
                    EmbeddedRuntimeGatewayState::Failed,
                    Some(error.to_string()),
                )?;
                return Err(error.to_string());
            }
        };
        *task = Some(EmbeddedGatewayTask { shutdown, thread });
        Ok(())
    }

    /// Stops the network gateway and waits for its worker thread.
    pub fn stop(&self) -> Result<(), String> {
        self.stop_inner()
    }

    /// Returns the most recent lifecycle state.
    pub fn status(&self) -> Result<EmbeddedRuntimeGatewayStatus, String> {
        self.status
            .lock()
            .map(|status| status.clone())
            .map_err(|_| "embedded gateway status is unavailable".to_string())
    }

    fn stop_inner(&self) -> Result<(), String> {
        let task = self
            .task
            .lock()
            .map_err(|_| "embedded gateway lifecycle is unavailable".to_string())?
            .take();
        if let Some(task) = task {
            let _ = task.shutdown.send(());
            task.thread
                .join()
                .map_err(|_| "embedded gateway worker stopped unexpectedly".to_string())?;
        }
        set_gateway_status(&self.status, EmbeddedRuntimeGatewayState::Stopped, None)
    }
}

impl Drop for EmbeddedRuntimeGateway {
    fn drop(&mut self) {
        let _ = self.stop_inner();
    }
}

fn runtime_manifest(runtime: &VifuRuntime) -> Result<RuntimeManifest, String> {
    if let Some(manifest) = runtime
        .current_manifest()
        .map_err(|error| error.public_message())?
    {
        return Ok(manifest);
    }
    runtime
        .restore_active_release()
        .map_err(|error| error.public_message())?
        .map(|release| release.manifest)
        .ok_or_else(|| {
            "embedded gateway requires an applied runtime manifest or active release".to_string()
        })
}

fn gateway_components(
    runtime: &VifuRuntime,
    manifest: &RuntimeManifest,
) -> (Vec<Arc<dyn AgentGatewayProvider>>, Vec<AgentDescriptor>) {
    let providers = manifest
        .providers
        .iter()
        .map(|provider| {
            Arc::new(EmbeddedRuntimeGatewayProvider::new(
                provider.id.clone(),
                runtime.clone(),
            )) as Arc<dyn AgentGatewayProvider>
        })
        .collect();
    let agents = manifest
        .agents
        .iter()
        .map(|agent| {
            let mut metadata = agent.metadata.as_object().cloned().unwrap_or_default();
            metadata.insert(
                "providerKey".to_string(),
                Value::String(agent.provider.clone()),
            );
            metadata.insert(
                "providerType".to_string(),
                Value::String("vifu-runtime".to_string()),
            );
            if let Some(provider_type) = manifest
                .providers
                .iter()
                .find(|provider| provider.id == agent.provider)
                .map(|provider| provider.provider_type.clone())
            {
                metadata.insert(
                    "localProviderType".to_string(),
                    Value::String(provider_type),
                );
            }
            metadata.insert(
                "capabilities".to_string(),
                serde_json::json!(agent.capabilities),
            );
            AgentDescriptor {
                id: agent.id.clone(),
                name: agent.name.clone(),
                metadata: Value::Object(metadata),
            }
        })
        .collect();
    (providers, agents)
}

fn set_gateway_status(
    status: &Mutex<EmbeddedRuntimeGatewayStatus>,
    state: EmbeddedRuntimeGatewayState,
    last_error: Option<String>,
) -> Result<(), String> {
    *status
        .lock()
        .map_err(|_| "embedded gateway status is unavailable".to_string())? =
        EmbeddedRuntimeGatewayStatus { state, last_error };
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use serde_json::json;
    use vifu_runtime::{
        AgentDefinition, AgentProvider, CancellationToken, EndpointDefinition, ProviderFuture,
        ProviderRequest, ProviderRequirement, ProviderResponse, RuntimeError, RuntimeManifest,
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

    #[test]
    fn manifest_agents_keep_their_local_provider_identity() {
        let runtime = runtime();
        let mut manifest = RuntimeManifest::new("moon-train");
        manifest.providers = vec![ProviderRequirement {
            id: "local-llama".to_string(),
            provider_type: "local-llama".to_string(),
            capabilities: vec!["chat".to_string()],
            settings: json!({}),
            resources: BTreeMap::new(),
        }];
        manifest.agents = runtime.agent_definitions().unwrap();
        manifest.endpoints = runtime.endpoint_definitions().unwrap();

        let (providers, agents) = gateway_components(&runtime, &manifest);

        assert_eq!(providers[0].id(), "local-llama");
        assert_eq!(agents[0].metadata["providerKey"], "local-llama");
        assert_eq!(agents[0].metadata["providerType"], "vifu-runtime");
        assert_eq!(agents[0].metadata["localProviderType"], "local-llama");
    }

    #[test]
    fn lifecycle_requires_a_manifest_before_starting() {
        let runtime = runtime();
        assert_eq!(
            runtime_manifest(&runtime).unwrap_err(),
            "embedded gateway requires an applied runtime manifest or active release"
        );
    }
}
