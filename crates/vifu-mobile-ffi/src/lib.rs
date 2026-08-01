//! UniFFI facade for embedding Vifu Runtime and Gateway utilities in native clients.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::JoinHandle;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;
use vifu_gateway::embedded::EmbeddedRuntimeGatewayProvider;
use vifu_gateway::protocol::{self, AgentDescriptor};
use vifu_gateway::relay::{self, AgentGatewayProvider, AgentGatewayRuntime};
use vifu_gateway::session::{self, SessionSummary};
use vifu_gateway::{config, openclaw};
#[cfg(feature = "local-llama")]
use vifu_provider_llama::{LlamaProvider, LlamaProviderConfig};
use vifu_runtime::{
    AgentDefinition, AgentProvider, CancellationToken, EndpointDefinition, InvocationData,
    InvocationEvent, InvocationEventKind, InvocationHandle, InvocationInput, InvocationOutput,
    InvocationStatus, LocalProviderBinding, ProviderFuture, ProviderRequest, ProviderRequirement,
    ProviderResponse, RuntimeBridge, RuntimeBridgeError, RuntimeError, RuntimeManifest,
    RuntimeRelease, SqliteRuntimeStore, VifuRuntime,
};

#[derive(Debug, Clone, uniffi::Record)]
pub struct VifuRuntimeConfig {
    pub server_url: String,
    pub openclaw_url: String,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct VifuOpenClawEndpoint {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, uniffi::Enum)]
pub enum VifuProbeStatus {
    Online,
    Offline,
    Unsupported,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct VifuOpenClawProbeReport {
    pub endpoint: VifuOpenClawEndpoint,
    pub status: VifuProbeStatus,
    pub message: Option<String>,
}

#[derive(Debug, thiserror::Error, uniffi::Error)]
#[uniffi(flat_error)]
pub enum VifuRuntimeError {
    #[error("{message}")]
    InvalidConfig { message: String },
    #[error("{message}")]
    Runtime { message: String },
}

impl From<String> for VifuRuntimeError {
    fn from(message: String) -> Self {
        Self::Runtime { message }
    }
}

impl From<RuntimeError> for VifuRuntimeError {
    fn from(error: RuntimeError) -> Self {
        Self::Runtime {
            message: error.public_message(),
        }
    }
}

impl From<RuntimeBridgeError> for VifuRuntimeError {
    fn from(error: RuntimeBridgeError) -> Self {
        Self::Runtime {
            message: error.to_string(),
        }
    }
}

impl From<openclaw::Endpoint> for VifuOpenClawEndpoint {
    fn from(endpoint: openclaw::Endpoint) -> Self {
        Self {
            host: endpoint.host,
            port: endpoint.port,
        }
    }
}

#[uniffi::export]
pub fn default_vifu_runtime_config() -> VifuRuntimeConfig {
    VifuRuntimeConfig {
        server_url: config::DEFAULT_SERVER_URL.to_string(),
        openclaw_url: config::DEFAULT_OPENCLAW_URL.to_string(),
    }
}

#[uniffi::export]
pub fn vifu_agent_gateway_websocket_url(server_url: String) -> Result<String, VifuRuntimeError> {
    relay::agent_gateway_websocket_url(&server_url)
        .map_err(|message| VifuRuntimeError::InvalidConfig { message })
}

#[uniffi::export]
pub fn parse_vifu_openclaw_endpoint(
    openclaw_url: String,
) -> Result<VifuOpenClawEndpoint, VifuRuntimeError> {
    openclaw::parse_endpoint(&openclaw_url)
        .map(Into::into)
        .map_err(|message| VifuRuntimeError::InvalidConfig { message })
}

#[uniffi::export]
pub fn probe_vifu_openclaw_gateway(
    openclaw_url: String,
) -> Result<VifuOpenClawProbeReport, VifuRuntimeError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| VifuRuntimeError::Runtime {
            message: error.to_string(),
        })?;
    let report = runtime.block_on(openclaw::probe(&openclaw_url));
    let (status, message) = match report.status {
        openclaw::ProbeStatus::Online => (VifuProbeStatus::Online, None),
        openclaw::ProbeStatus::Offline(message) => (VifuProbeStatus::Offline, Some(message)),
        openclaw::ProbeStatus::Unsupported(message) => {
            (VifuProbeStatus::Unsupported, Some(message))
        }
    };
    Ok(VifuOpenClawProbeReport {
        endpoint: report.endpoint.into(),
        status,
        message,
    })
}

#[derive(Clone, uniffi::Enum)]
pub enum VifuInvocationData {
    Json { json: String },
    Binary { bytes: Vec<u8> },
}

#[derive(Clone, uniffi::Record)]
pub struct VifuProviderRequest {
    pub project_id: String,
    pub endpoint: String,
    pub session_id: String,
    pub agent_id: String,
    pub capability: String,
    pub data: VifuInvocationData,
    pub metadata_json: String,
    pub state_json: String,
    pub state_revision: u64,
}

#[derive(Clone, uniffi::Record)]
pub struct VifuProviderResponse {
    pub data: VifuInvocationData,
    pub metadata_json: String,
    pub state_json: Option<String>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct VifuLlamaProviderConfig {
    pub model_path: String,
    pub context_size: u32,
    pub gpu_layers: u32,
    pub default_max_tokens: u32,
}

#[uniffi::export(callback_interface)]
pub trait VifuAgentProvider: Send + Sync {
    fn invoke(
        &self,
        request: VifuProviderRequest,
    ) -> Result<VifuProviderResponse, VifuRuntimeError>;
}

struct FfiAgentProvider {
    id: String,
    inner: Arc<dyn VifuAgentProvider>,
}

impl AgentProvider for FfiAgentProvider {
    fn supports(&self, _capability: &str) -> bool {
        true
    }

    fn invoke<'a>(
        &'a self,
        request: ProviderRequest,
        cancellation: CancellationToken,
    ) -> ProviderFuture<'a> {
        let provider_id = self.id.clone();
        let inner = Arc::clone(&self.inner);
        Box::pin(async move {
            let ffi_request = VifuProviderRequest {
                project_id: request.project_id,
                endpoint: request.endpoint,
                session_id: request.session_id,
                agent_id: request.agent.id,
                capability: request.capability,
                data: request.data.into(),
                metadata_json: encode_json(&request.metadata)?,
                state_json: encode_json(&request.snapshot.state)?,
                state_revision: request.snapshot.revision,
            };
            let callback = tokio::task::spawn_blocking(move || inner.invoke(ffi_request));
            let response = tokio::select! {
                _ = cancellation.cancelled() => return Err(RuntimeError::Cancelled),
                result = callback => result
                    .map_err(|_error| {
                        RuntimeError::provider(&provider_id, "native provider callback stopped")
                    })?
                    .map_err(|_error| {
                        RuntimeError::provider(&provider_id, "native provider callback failed")
                    })?,
            };
            Ok(ProviderResponse {
                data: response.data.try_into()?,
                metadata: parse_json(&response.metadata_json, "provider metadata")?,
                state: response
                    .state_json
                    .as_deref()
                    .map(|state| parse_json(state, "provider state"))
                    .transpose()?,
            })
        })
    }
}

#[derive(Clone, uniffi::Enum)]
pub enum VifuInvocationState {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl From<InvocationStatus> for VifuInvocationState {
    fn from(status: InvocationStatus) -> Self {
        match status {
            InvocationStatus::Pending => Self::Pending,
            InvocationStatus::Running => Self::Running,
            InvocationStatus::Completed => Self::Completed,
            InvocationStatus::Failed => Self::Failed,
            InvocationStatus::Cancelled => Self::Cancelled,
        }
    }
}

#[derive(Clone, uniffi::Record)]
pub struct VifuInvocationResult {
    pub invocation_id: String,
    pub project_id: String,
    pub endpoint: String,
    pub session_id: String,
    pub agent_id: String,
    pub provider_id: String,
    pub capability: String,
    pub data: VifuInvocationData,
    pub metadata_json: String,
    pub state_revision: u64,
    pub state_json: String,
    pub trace_json: String,
}

impl TryFrom<InvocationOutput> for VifuInvocationResult {
    type Error = RuntimeError;

    fn try_from(output: InvocationOutput) -> Result<Self, Self::Error> {
        Ok(Self {
            invocation_id: output.invocation_id,
            project_id: output.project_id,
            endpoint: output.endpoint,
            session_id: output.session_id,
            agent_id: output.agent,
            provider_id: output.provider,
            capability: output.capability,
            data: output.data.into(),
            metadata_json: encode_json(&output.metadata)?,
            state_revision: output.snapshot.revision,
            state_json: encode_json(&output.snapshot.state)?,
            trace_json: encode_json(&output.trace)?,
        })
    }
}

#[derive(Clone, uniffi::Record)]
pub struct VifuInvocationPoll {
    pub handle: String,
    pub state: VifuInvocationState,
    pub result: Option<VifuInvocationResult>,
    pub error: Option<String>,
}

#[derive(Clone, uniffi::Enum)]
pub enum VifuInvocationEventKind {
    Started,
    OutputDelta,
    Completed,
    Failed,
    Cancelled,
}

impl From<InvocationEventKind> for VifuInvocationEventKind {
    fn from(kind: InvocationEventKind) -> Self {
        match kind {
            InvocationEventKind::Started => Self::Started,
            InvocationEventKind::OutputDelta => Self::OutputDelta,
            InvocationEventKind::Completed => Self::Completed,
            InvocationEventKind::Failed => Self::Failed,
            InvocationEventKind::Cancelled => Self::Cancelled,
        }
    }
}

#[derive(Clone, uniffi::Record)]
pub struct VifuInvocationEvent {
    pub sequence: u64,
    pub kind: VifuInvocationEventKind,
    pub data: Option<VifuInvocationData>,
    pub error: Option<String>,
}

impl From<InvocationEvent> for VifuInvocationEvent {
    fn from(event: InvocationEvent) -> Self {
        Self {
            sequence: event.sequence,
            kind: event.kind.into(),
            data: event.data.map(Into::into),
            error: event.error,
        }
    }
}

#[derive(uniffi::Object)]
pub struct VifuEmbeddedRuntime {
    runtime: VifuRuntime,
    bridge: RuntimeBridge,
    provider_types: RwLock<BTreeMap<String, String>>,
}

#[uniffi::export]
impl VifuEmbeddedRuntime {
    #[uniffi::constructor]
    pub fn new(project_id: String) -> Result<Arc<Self>, VifuRuntimeError> {
        let runtime = VifuRuntime::new(project_id)?;
        Ok(Arc::new(Self {
            bridge: RuntimeBridge::new(runtime.clone()),
            runtime,
            provider_types: RwLock::new(BTreeMap::new()),
        }))
    }

    /// Opens an embedded runtime whose project state survives app restarts.
    #[uniffi::constructor(name = "open")]
    pub fn open(project_id: String, database_path: String) -> Result<Arc<Self>, VifuRuntimeError> {
        let store = Arc::new(SqliteRuntimeStore::open(database_path)?);
        let runtime = VifuRuntime::with_store(project_id, store)?;
        Ok(Arc::new(Self {
            bridge: RuntimeBridge::new(runtime.clone()),
            runtime,
            provider_types: RwLock::new(BTreeMap::new()),
        }))
    }

    pub fn register_provider(
        &self,
        provider_id: String,
        provider: Box<dyn VifuAgentProvider>,
    ) -> Result<(), VifuRuntimeError> {
        self.runtime.register_provider(
            provider_id.clone(),
            Arc::new(FfiAgentProvider {
                id: provider_id.clone(),
                inner: Arc::from(provider),
            }),
        )?;
        self.remember_provider_type(provider_id, "native")?;
        Ok(())
    }

    pub fn register_llama_provider(
        &self,
        provider_id: String,
        config: VifuLlamaProviderConfig,
    ) -> Result<(), VifuRuntimeError> {
        #[cfg(feature = "local-llama")]
        {
            let provider = LlamaProvider::load(LlamaProviderConfig {
                model_path: config.model_path.into(),
                context_size: config.context_size,
                gpu_layers: config.gpu_layers,
                default_max_tokens: config.default_max_tokens,
            })
            .map_err(|error| VifuRuntimeError::InvalidConfig {
                message: error.to_string(),
            })?;
            self.runtime
                .register_provider(provider_id.clone(), Arc::new(provider))?;
            self.remember_provider_type(provider_id, "llama")?;
            Ok(())
        }
        #[cfg(not(feature = "local-llama"))]
        {
            let _ = (provider_id, config);
            Err(VifuRuntimeError::InvalidConfig {
                message: "this Vifu build does not include the local llama provider".to_string(),
            })
        }
    }

    pub fn register_agent(
        &self,
        agent_id: String,
        name: String,
        provider_id: String,
        capabilities: Vec<String>,
        metadata_json: String,
    ) -> Result<(), VifuRuntimeError> {
        self.runtime.register_agent(AgentDefinition {
            id: agent_id,
            name,
            provider: provider_id,
            capabilities,
            metadata: parse_json(&metadata_json, "agent metadata")?,
        })?;
        Ok(())
    }

    pub fn register_endpoint(
        &self,
        name: String,
        agent_id: String,
        capability: String,
        timeout_ms: u64,
    ) -> Result<(), VifuRuntimeError> {
        self.runtime.register_endpoint(EndpointDefinition {
            name,
            agent: agent_id,
            capability,
            timeout_ms,
        })?;
        Ok(())
    }

    pub fn install_runtime_release(
        &self,
        version: u64,
        manifest_json: String,
    ) -> Result<String, VifuRuntimeError> {
        let manifest = RuntimeManifest::from_json(manifest_json.as_bytes())?;
        let release = RuntimeRelease::new(version, manifest)?;
        self.runtime.install_release(&release)?;
        Ok(release.content_hash)
    }

    pub fn activate_runtime_release(&self, version: u64) -> Result<String, VifuRuntimeError> {
        Ok(self.runtime.activate_release(version)?.content_hash)
    }

    pub fn restore_active_runtime_release(&self) -> Result<Option<u64>, VifuRuntimeError> {
        self.runtime
            .restore_active_release()
            .map(|release| release.map(|release| release.version))
            .map_err(Into::into)
    }

    pub fn bootstrap_runtime_release(
        &self,
        manifest_json: String,
    ) -> Result<u64, VifuRuntimeError> {
        let manifest = RuntimeManifest::from_json(manifest_json.as_bytes())?;
        Ok(self.runtime.bootstrap_release(manifest)?.version)
    }

    pub fn current_runtime_manifest(&self) -> Result<Option<String>, VifuRuntimeError> {
        self.runtime
            .current_manifest()?
            .map(|manifest| encode_json(&manifest).map_err(Into::into))
            .transpose()
    }

    pub fn save_local_provider_binding(
        &self,
        provider_id: String,
        configuration_json: String,
    ) -> Result<(), VifuRuntimeError> {
        self.runtime
            .save_local_provider_binding(&LocalProviderBinding {
                provider_id,
                configuration: parse_json(&configuration_json, "provider binding")?,
            })?;
        Ok(())
    }

    pub fn local_provider_bindings(&self) -> Result<String, VifuRuntimeError> {
        encode_json(&self.runtime.local_provider_bindings()?).map_err(Into::into)
    }

    pub fn pending_runtime_traces(&self, limit: u32) -> Result<String, VifuRuntimeError> {
        encode_json(&self.runtime.pending_traces(limit as usize)?).map_err(Into::into)
    }

    pub fn acknowledge_runtime_traces(
        &self,
        trace_ids: Vec<String>,
    ) -> Result<(), VifuRuntimeError> {
        self.runtime
            .acknowledge_traces(&trace_ids)
            .map_err(Into::into)
    }

    pub fn start_invoke(
        &self,
        endpoint: String,
        session_id: String,
        data: VifuInvocationData,
        metadata_json: String,
    ) -> Result<String, VifuRuntimeError> {
        Ok(self
            .runtime
            .start_invoke(InvocationInput {
                endpoint,
                session_id,
                data: data.try_into()?,
                metadata: parse_json(&metadata_json, "invocation metadata")?,
            })?
            .0)
    }

    pub fn poll_invocation(&self, handle: String) -> Result<VifuInvocationPoll, VifuRuntimeError> {
        let poll = self
            .runtime
            .poll_invocation(&InvocationHandle(handle.clone()))?;
        Ok(VifuInvocationPoll {
            handle,
            state: poll.status.into(),
            result: poll.output.map(TryInto::try_into).transpose()?,
            error: poll.error,
        })
    }

    pub fn drain_invocation_events(
        &self,
        handle: String,
    ) -> Result<Vec<VifuInvocationEvent>, VifuRuntimeError> {
        self.runtime
            .drain_invocation_events(&InvocationHandle(handle))
            .map(|events| events.into_iter().map(Into::into).collect())
            .map_err(Into::into)
    }

    pub fn take_invocation(&self, handle: String) -> Result<VifuInvocationPoll, VifuRuntimeError> {
        let poll = self
            .runtime
            .take_invocation(&InvocationHandle(handle.clone()))?;
        Ok(VifuInvocationPoll {
            handle,
            state: poll.status.into(),
            result: poll.output.map(TryInto::try_into).transpose()?,
            error: poll.error,
        })
    }

    pub fn cancel_invocation(&self, handle: String) -> Result<(), VifuRuntimeError> {
        self.runtime
            .cancel_invocation(&InvocationHandle(handle))
            .map_err(Into::into)
    }

    pub fn handle_bridge_frame(
        &self,
        encoded_frame: String,
    ) -> Result<Vec<String>, VifuRuntimeError> {
        self.bridge
            .handle_encoded(&encoded_frame)
            .map_err(Into::into)
    }

    pub fn drain_bridge_frames(&self) -> Result<Vec<String>, VifuRuntimeError> {
        self.bridge.drain_encoded().map_err(Into::into)
    }

    pub fn export_snapshot(&self) -> Result<Vec<u8>, VifuRuntimeError> {
        self.runtime.export_snapshot().map_err(Into::into)
    }

    pub fn restore_snapshot(&self, snapshot: Vec<u8>) -> Result<(), VifuRuntimeError> {
        self.runtime.restore_snapshot(&snapshot).map_err(Into::into)
    }
}

impl VifuEmbeddedRuntime {
    fn remember_provider_type(
        &self,
        provider_id: String,
        provider_type: &str,
    ) -> Result<(), VifuRuntimeError> {
        self.provider_types
            .write()
            .map_err(|_| VifuRuntimeError::Runtime {
                message: "embedded provider registry is unavailable".to_string(),
            })?
            .insert(provider_id, provider_type.to_string());
        Ok(())
    }

    fn prepare_gateway_release(&self) -> Result<RuntimeManifest, VifuRuntimeError> {
        if let Some(manifest) = self.runtime.current_manifest()? {
            return Ok(manifest);
        }
        if let Some(release) = self.runtime.restore_active_release()? {
            return Ok(release.manifest);
        }

        let agents = self.runtime.agent_definitions()?;
        let endpoints = self.runtime.endpoint_definitions()?;
        let provider_types = self
            .provider_types
            .read()
            .map_err(|_| VifuRuntimeError::Runtime {
                message: "embedded provider registry is unavailable".to_string(),
            })?;
        let mut capabilities = BTreeMap::<String, BTreeSet<String>>::new();
        for agent in &agents {
            if !provider_types.contains_key(&agent.provider) {
                return Err(VifuRuntimeError::InvalidConfig {
                    message: format!(
                        "agent {} uses provider {} before it is registered",
                        agent.id, agent.provider
                    ),
                });
            }
            capabilities
                .entry(agent.provider.clone())
                .or_default()
                .extend(agent.capabilities.iter().cloned());
        }
        let providers = provider_types
            .iter()
            .map(|(id, provider_type)| ProviderRequirement {
                id: id.clone(),
                provider_type: provider_type.clone(),
                capabilities: capabilities
                    .remove(id)
                    .unwrap_or_default()
                    .into_iter()
                    .collect(),
                settings: serde_json::json!({}),
                resources: BTreeMap::new(),
            })
            .collect();
        let mut manifest = RuntimeManifest::new(self.runtime.project_id());
        manifest.providers = providers;
        manifest.agents = agents;
        manifest.endpoints = endpoints;
        manifest.metadata = serde_json::json!({ "source": "embedded-runtime" });
        Ok(self.runtime.bootstrap_release(manifest)?.manifest)
    }

    fn gateway_agents(&self, manifest: &RuntimeManifest) -> Vec<AgentDescriptor> {
        let provider_types = self.provider_types.read().ok();
        manifest
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
                if let Some(provider_type) = provider_types
                    .as_ref()
                    .and_then(|types| types.get(&agent.provider))
                {
                    metadata.insert(
                        "localProviderType".to_string(),
                        Value::String(provider_type.clone()),
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
            .collect()
    }

    fn gateway_providers(&self) -> Result<Vec<Arc<dyn AgentGatewayProvider>>, VifuRuntimeError> {
        let providers = self
            .provider_types
            .read()
            .map_err(|_| VifuRuntimeError::Runtime {
                message: "embedded provider registry is unavailable".to_string(),
            })?
            .keys()
            .map(|provider_id| {
                Arc::new(EmbeddedRuntimeGatewayProvider::new(
                    provider_id.clone(),
                    self.runtime.clone(),
                )) as Arc<dyn AgentGatewayProvider>
            })
            .collect();
        Ok(providers)
    }
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct VifuEmbeddedGatewayConfig {
    pub server_url: String,
    pub gateway_id: String,
    pub runtime_database_path: String,
}

#[derive(Clone, uniffi::Record)]
pub struct VifuGeneratedGatewayIdentity {
    pub gateway_id: String,
    pub credential: String,
}

#[uniffi::export]
pub fn generate_vifu_gateway_identity() -> VifuGeneratedGatewayIdentity {
    VifuGeneratedGatewayIdentity {
        gateway_id: session::generate_gateway_id(),
        credential: session::generate_gateway_credential(),
    }
}

#[derive(Debug, Clone, uniffi::Enum)]
pub enum VifuEmbeddedGatewayState {
    Stopped,
    Running,
    Failed,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct VifuEmbeddedGatewayStatus {
    pub state: VifuEmbeddedGatewayState,
    pub last_error: Option<String>,
}

struct EmbeddedGatewayTask {
    shutdown: tokio::sync::oneshot::Sender<()>,
    thread: JoinHandle<()>,
}

#[derive(uniffi::Object)]
pub struct VifuEmbeddedGateway {
    runtime: Arc<VifuEmbeddedRuntime>,
    config: VifuEmbeddedGatewayConfig,
    task: Mutex<Option<EmbeddedGatewayTask>>,
    status: Arc<Mutex<VifuEmbeddedGatewayStatus>>,
}

#[uniffi::export]
impl VifuEmbeddedGateway {
    #[uniffi::constructor]
    pub fn new(
        runtime: Arc<VifuEmbeddedRuntime>,
        config: VifuEmbeddedGatewayConfig,
    ) -> Result<Arc<Self>, VifuRuntimeError> {
        relay::agent_gateway_websocket_url(&config.server_url)
            .map_err(|message| VifuRuntimeError::InvalidConfig { message })?;
        protocol::validate_identifier("agent gateway id", &config.gateway_id)
            .map_err(|message| VifuRuntimeError::InvalidConfig { message })?;
        if config.runtime_database_path.trim().is_empty() {
            return Err(VifuRuntimeError::InvalidConfig {
                message: "runtime database path is required".to_string(),
            });
        }
        Ok(Arc::new(Self {
            runtime,
            config,
            task: Mutex::new(None),
            status: Arc::new(Mutex::new(VifuEmbeddedGatewayStatus {
                state: VifuEmbeddedGatewayState::Stopped,
                last_error: None,
            })),
        }))
    }

    /// Starts the optional network Gateway. The credential remains in memory.
    pub fn start(
        &self,
        gateway_credential: String,
        enrollment_token: Option<String>,
    ) -> Result<(), VifuRuntimeError> {
        session::validate_gateway_credential(&gateway_credential)
            .map_err(|message| VifuRuntimeError::InvalidConfig { message })?;
        let manifest = self.runtime.prepare_gateway_release()?;
        let providers = self.runtime.gateway_providers()?;
        let agents = self.runtime.gateway_agents(&manifest);
        let mut task = self.task.lock().map_err(|_| VifuRuntimeError::Runtime {
            message: "embedded gateway lifecycle is unavailable".to_string(),
        })?;
        if task.as_ref().is_some_and(|task| !task.thread.is_finished()) {
            return Ok(());
        }
        if let Some(finished) = task.take() {
            let _ = finished.thread.join();
        }

        let server_url = self.config.server_url.clone();
        let gateway_id = self.config.gateway_id.clone();
        let runtime_database_path = self.config.runtime_database_path.clone();
        let embedded_runtime = self.runtime.runtime.clone();
        let status = Arc::clone(&self.status);
        let (shutdown, shutdown_receiver) = tokio::sync::oneshot::channel();
        set_gateway_status(&status, VifuEmbeddedGatewayState::Running, None)?;
        let thread = std::thread::Builder::new()
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
                                runtime_database_path: runtime_database_path.as_ref(),
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
                            set_gateway_status(&status, VifuEmbeddedGatewayState::Stopped, None);
                    }
                    Err(error) => {
                        let _ = set_gateway_status(
                            &status,
                            VifuEmbeddedGatewayState::Failed,
                            Some(error),
                        );
                    }
                }
            })
            .map_err(|error| VifuRuntimeError::Runtime {
                message: error.to_string(),
            })?;
        *task = Some(EmbeddedGatewayTask { shutdown, thread });
        Ok(())
    }

    pub fn stop(&self) -> Result<(), VifuRuntimeError> {
        self.stop_inner()
    }

    pub fn status(&self) -> Result<VifuEmbeddedGatewayStatus, VifuRuntimeError> {
        self.status
            .lock()
            .map(|status| status.clone())
            .map_err(|_| VifuRuntimeError::Runtime {
                message: "embedded gateway status is unavailable".to_string(),
            })
    }
}

impl VifuEmbeddedGateway {
    fn stop_inner(&self) -> Result<(), VifuRuntimeError> {
        let task = self
            .task
            .lock()
            .map_err(|_| VifuRuntimeError::Runtime {
                message: "embedded gateway lifecycle is unavailable".to_string(),
            })?
            .take();
        if let Some(task) = task {
            let _ = task.shutdown.send(());
            task.thread.join().map_err(|_| VifuRuntimeError::Runtime {
                message: "embedded gateway worker stopped unexpectedly".to_string(),
            })?;
        }
        set_gateway_status(&self.status, VifuEmbeddedGatewayState::Stopped, None)
    }
}

impl Drop for VifuEmbeddedGateway {
    fn drop(&mut self) {
        let _ = self.stop_inner();
    }
}

fn set_gateway_status(
    status: &Mutex<VifuEmbeddedGatewayStatus>,
    state: VifuEmbeddedGatewayState,
    last_error: Option<String>,
) -> Result<(), VifuRuntimeError> {
    *status.lock().map_err(|_| VifuRuntimeError::Runtime {
        message: "embedded gateway status is unavailable".to_string(),
    })? = VifuEmbeddedGatewayStatus { state, last_error };
    Ok(())
}

impl From<InvocationData> for VifuInvocationData {
    fn from(data: InvocationData) -> Self {
        match data {
            InvocationData::Json(value) => Self::Json {
                json: value.to_string(),
            },
            InvocationData::Binary(bytes) => Self::Binary { bytes },
        }
    }
}

impl TryFrom<VifuInvocationData> for InvocationData {
    type Error = RuntimeError;

    fn try_from(data: VifuInvocationData) -> Result<Self, Self::Error> {
        match data {
            VifuInvocationData::Json { json } => {
                Ok(Self::Json(parse_json(&json, "invocation JSON")?))
            }
            VifuInvocationData::Binary { bytes } => Ok(Self::Binary(bytes)),
        }
    }
}

fn parse_json(json: &str, kind: &str) -> Result<Value, RuntimeError> {
    serde_json::from_str(json)
        .map_err(|error| RuntimeError::InvalidDefinition(format!("{kind} is invalid: {error}")))
}

fn encode_json(value: &impl serde::Serialize) -> Result<String, RuntimeError> {
    serde_json::to_string(value).map_err(|_error| RuntimeError::Internal)
}

uniffi::setup_scaffolding!();

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    struct EchoProvider;

    impl VifuAgentProvider for EchoProvider {
        fn invoke(
            &self,
            request: VifuProviderRequest,
        ) -> Result<VifuProviderResponse, VifuRuntimeError> {
            Ok(VifuProviderResponse {
                data: request.data,
                metadata_json: r#"{"contentType":"application/json"}"#.to_string(),
                state_json: Some(r#"{"turns":1}"#.to_string()),
            })
        }
    }

    struct BlockingProvider;

    impl VifuAgentProvider for BlockingProvider {
        fn invoke(
            &self,
            request: VifuProviderRequest,
        ) -> Result<VifuProviderResponse, VifuRuntimeError> {
            std::thread::sleep(Duration::from_millis(250));
            Ok(VifuProviderResponse {
                data: request.data,
                metadata_json: "{}".to_string(),
                state_json: None,
            })
        }
    }

    fn configured_runtime(project_id: &str) -> Arc<VifuEmbeddedRuntime> {
        let runtime = VifuEmbeddedRuntime::new(project_id.to_string()).unwrap();
        runtime
            .register_provider("native".to_string(), Box::new(EchoProvider))
            .unwrap();
        runtime
            .register_agent(
                "guide".to_string(),
                "Guide".to_string(),
                "native".to_string(),
                vec!["chat".to_string()],
                "{}".to_string(),
            )
            .unwrap();
        runtime
            .register_endpoint(
                "guide".to_string(),
                "guide".to_string(),
                "chat".to_string(),
                500,
            )
            .unwrap();
        runtime
    }

    #[test]
    fn ffi_runtime_round_trips_invocation_and_snapshot() {
        let runtime = configured_runtime("ffi-project");
        let handle = runtime
            .start_invoke(
                "guide".to_string(),
                "player-one".to_string(),
                VifuInvocationData::Json {
                    json: r#"{"message":"hello"}"#.to_string(),
                },
                "{}".to_string(),
            )
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        let result = loop {
            let poll = runtime.poll_invocation(handle.clone()).unwrap();
            if let Some(result) = poll.result {
                break result;
            }
            assert!(Instant::now() < deadline, "FFI invocation did not finish");
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(result.state_revision, 1);
        let events = runtime
            .drain_invocation_events(handle)
            .expect("FFI events should remain available");
        assert!(matches!(
            events.first().map(|event| &event.kind),
            Some(VifuInvocationEventKind::Started)
        ));
        assert!(matches!(
            events.last().map(|event| &event.kind),
            Some(VifuInvocationEventKind::Completed)
        ));

        let snapshot = runtime.export_snapshot().unwrap();
        let restored = configured_runtime("ffi-project");
        restored.restore_snapshot(snapshot).unwrap();
        let restored_snapshot = restored.export_snapshot().unwrap();
        assert!(String::from_utf8_lossy(&restored_snapshot).contains("\"turns\":1"));
    }

    #[test]
    fn embedded_gateway_manifest_preserves_local_provider_identity() {
        let runtime = configured_runtime("gateway-manifest");
        let manifest = runtime.prepare_gateway_release().unwrap();

        assert_eq!(manifest.project_id, "gateway-manifest");
        assert_eq!(manifest.providers.len(), 1);
        assert_eq!(manifest.providers[0].id, "native");
        assert_eq!(manifest.providers[0].provider_type, "native");
        assert_eq!(manifest.agents[0].id, "guide");
        assert_eq!(manifest.endpoints[0].name, "guide");

        let descriptors = runtime.gateway_agents(&manifest);
        assert_eq!(descriptors[0].metadata["providerKey"], "native");
        assert_eq!(descriptors[0].metadata["providerType"], "vifu-runtime");
    }

    #[test]
    fn generated_gateway_identity_is_ready_for_keychain_storage() {
        let identity = generate_vifu_gateway_identity();

        assert!(identity.gateway_id.starts_with("gateway-"));
        session::validate_gateway_credential(&identity.credential).unwrap();
    }

    #[test]
    fn blocking_native_provider_does_not_block_runtime_timeouts() {
        let runtime = VifuEmbeddedRuntime::new("blocking-ffi-project".to_string()).unwrap();
        runtime
            .register_provider("native".to_string(), Box::new(BlockingProvider))
            .unwrap();
        runtime
            .register_agent(
                "guide".to_string(),
                "Guide".to_string(),
                "native".to_string(),
                vec!["chat".to_string()],
                "{}".to_string(),
            )
            .unwrap();
        runtime
            .register_endpoint(
                "guide".to_string(),
                "guide".to_string(),
                "chat".to_string(),
                20,
            )
            .unwrap();

        let started = Instant::now();
        let handle = runtime
            .start_invoke(
                "guide".to_string(),
                "player-one".to_string(),
                VifuInvocationData::Json {
                    json: "{}".to_string(),
                },
                "{}".to_string(),
            )
            .unwrap();
        let deadline = started + Duration::from_millis(150);
        loop {
            let poll = runtime.poll_invocation(handle.clone()).unwrap();
            if matches!(poll.state, VifuInvocationState::Failed) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "blocking callback prevented the runtime timeout"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn ffi_runtime_exposes_transport_neutral_bridge_frames() {
        let runtime = configured_runtime("ffi-bridge-project");
        let response = runtime
            .handle_bridge_frame(
                r#"{"type":"req","id":"invoke-1","method":"runtime.invoke","params":{"endpoint":"guide","sessionId":"player-one","data":{"format":"json","value":{"message":"hello"}},"metadata":{}}}"#
                    .to_string(),
            )
            .unwrap();
        assert_eq!(response.len(), 1);
        assert!(response[0].contains(r#""ok":true"#));

        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let events = runtime.drain_bridge_frames().unwrap();
            if events
                .iter()
                .any(|event| event.contains("runtime.invocation.completed"))
            {
                assert!(events
                    .iter()
                    .any(|event| event.contains(r#""message":"hello""#)));
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
