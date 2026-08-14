//! UniFFI facade for embedding Vifu Runtime and Gateway utilities in native clients.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, RwLock};

use serde_json::Value;
use vifu_gateway::embedded::{
    EmbeddedRuntimeGateway, EmbeddedRuntimeGatewayConfig, EmbeddedRuntimeGatewayState,
};
use vifu_gateway::identity::MachineIdentity;
use vifu_gateway::relay;
use vifu_gateway::{config, openclaw};
#[cfg(feature = "local-llama")]
use vifu_provider_llama::{LlamaProvider, LlamaProviderConfig, LlamaProviderError};
use vifu_runtime::{
    AgentDefinition, AgentProvider, CancellationToken, EndpointDefinition, InvocationData,
    InvocationEvent, InvocationEventKind, InvocationHandle, InvocationInput, InvocationOutput,
    InvocationStatus, LocalProviderBinding, ProviderFuture, ProviderRequest, ProviderRequirement,
    ProviderResponse, ProviderStage, RuntimeBridge, RuntimeBridgeError, RuntimeError,
    RuntimeManifest, RuntimeRelease, SqliteRuntimeStore, VifuRuntime,
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

#[cfg(feature = "local-llama")]
impl From<LlamaProviderError> for VifuRuntimeError {
    fn from(error: LlamaProviderError) -> Self {
        let message = error.to_string();
        match error {
            LlamaProviderError::ModelNotFound
            | LlamaProviderError::InvalidContextSize
            | LlamaProviderError::InvalidConfig(_)
            | LlamaProviderError::ProjectorNotFound => Self::InvalidConfig { message },
            LlamaProviderError::Backend(_)
            | LlamaProviderError::BackendDiscovery(_)
            | LlamaProviderError::Model(_)
            | LlamaProviderError::Multimodal(_) => Self::Runtime { message },
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
    pub provider_id: String,
    pub agent_id: String,
    pub agent_name: String,
    pub agent_capabilities: Vec<String>,
    pub agent_metadata_json: String,
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

#[derive(Debug, Clone, uniffi::Record)]
pub struct VifuWhisperProviderConfig {
    pub model_path: String,
    pub language: Option<String>,
}

#[uniffi::export(callback_interface)]
pub trait VifuAgentProvider: Send + Sync {
    fn invoke(
        &self,
        request: VifuProviderRequest,
    ) -> Result<VifuProviderResponse, VifuRuntimeError>;
}

#[derive(Clone, uniffi::Enum)]
pub enum VifuProviderStage {
    Queue,
    Load,
    Tokenize,
    Prefill,
    FirstToken,
    Decode,
    Validate,
}

impl From<VifuProviderStage> for ProviderStage {
    fn from(stage: VifuProviderStage) -> Self {
        match stage {
            VifuProviderStage::Queue => Self::Queue,
            VifuProviderStage::Load => Self::Load,
            VifuProviderStage::Tokenize => Self::Tokenize,
            VifuProviderStage::Prefill => Self::Prefill,
            VifuProviderStage::FirstToken => Self::FirstToken,
            VifuProviderStage::Decode => Self::Decode,
            VifuProviderStage::Validate => Self::Validate,
        }
    }
}

#[derive(uniffi::Object)]
pub struct VifuProviderInvocation {
    cancellation: CancellationToken,
    events: vifu_runtime::ProviderEventSink,
}

#[uniffi::export]
impl VifuProviderInvocation {
    pub fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }

    pub fn output_delta(&self, data: VifuInvocationData) -> Result<(), VifuRuntimeError> {
        self.events.output_delta(data.try_into()?);
        Ok(())
    }

    pub fn activity(&self) {
        self.events.activity();
    }

    pub fn stage_started(
        &self,
        stage: VifuProviderStage,
        metadata_json: String,
    ) -> Result<(), VifuRuntimeError> {
        self.events.stage_started(
            stage.into(),
            parse_json(&metadata_json, "provider stage metadata")?,
        );
        Ok(())
    }

    pub fn stage_completed(
        &self,
        stage: VifuProviderStage,
        elapsed_ms: u64,
        metadata_json: String,
    ) -> Result<(), VifuRuntimeError> {
        self.events.stage_completed(
            stage.into(),
            elapsed_ms,
            parse_json(&metadata_json, "provider stage metadata")?,
        );
        Ok(())
    }

    pub fn stage_failed(
        &self,
        stage: VifuProviderStage,
        elapsed_ms: u64,
        error: String,
        metadata_json: String,
    ) -> Result<(), VifuRuntimeError> {
        self.events.stage_failed(
            stage.into(),
            elapsed_ms,
            error,
            parse_json(&metadata_json, "provider stage metadata")?,
        );
        Ok(())
    }
}

#[uniffi::export(callback_interface)]
pub trait VifuStreamingAgentProvider: Send + Sync {
    fn invoke(
        &self,
        request: VifuProviderRequest,
        invocation: Arc<VifuProviderInvocation>,
    ) -> Result<VifuProviderResponse, VifuRuntimeError>;
}

struct FfiAgentProvider {
    id: String,
    inner: Arc<dyn VifuAgentProvider>,
}

struct FfiStreamingAgentProvider {
    id: String,
    inner: Arc<dyn VifuStreamingAgentProvider>,
}

impl AgentProvider for FfiStreamingAgentProvider {
    fn supports(&self, _capability: &str) -> bool {
        true
    }

    fn invoke<'a>(
        &'a self,
        request: ProviderRequest,
        cancellation: CancellationToken,
    ) -> ProviderFuture<'a> {
        self.invoke_with_events(
            request,
            cancellation,
            vifu_runtime::ProviderEventSink::discard(),
        )
    }

    fn invoke_with_events<'a>(
        &'a self,
        request: ProviderRequest,
        cancellation: CancellationToken,
        events: vifu_runtime::ProviderEventSink,
    ) -> ProviderFuture<'a> {
        let provider_id = self.id.clone();
        let inner = Arc::clone(&self.inner);
        Box::pin(async move {
            let ffi_request = provider_request_to_ffi(request)?;
            let invocation = Arc::new(VifuProviderInvocation {
                cancellation: cancellation.clone(),
                events,
            });
            let callback =
                tokio::task::spawn_blocking(move || inner.invoke(ffi_request, invocation));
            let response = tokio::select! {
                _ = cancellation.cancelled() => return Err(RuntimeError::Cancelled),
                result = callback => result
                    .map_err(|_error| {
                        RuntimeError::provider(&provider_id, "native streaming provider callback stopped")
                    })?
                    .map_err(|error| {
                        RuntimeError::provider(&provider_id, error.to_string())
                    })?,
            };
            provider_response_from_ffi(response)
        })
    }
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
            let ffi_request = provider_request_to_ffi(request)?;
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
            provider_response_from_ffi(response)
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

    pub fn register_streaming_provider(
        &self,
        provider_id: String,
        provider_type: String,
        provider: Box<dyn VifuStreamingAgentProvider>,
    ) -> Result<(), VifuRuntimeError> {
        let provider_type = provider_type.trim();
        if provider_type.is_empty() {
            return Err(VifuRuntimeError::InvalidConfig {
                message: "provider type is required".to_string(),
            });
        }
        self.runtime.register_provider(
            provider_id.clone(),
            Arc::new(FfiStreamingAgentProvider {
                id: provider_id.clone(),
                inner: Arc::from(provider),
            }),
        )?;
        self.remember_provider_type(provider_id, provider_type)?;
        Ok(())
    }

    pub fn unregister_provider(&self, provider_id: String) -> Result<bool, VifuRuntimeError> {
        let removed = self.runtime.unregister_provider(&provider_id)?;
        if removed {
            self.provider_types
                .write()
                .map_err(|_| VifuRuntimeError::Runtime {
                    message: "embedded provider registry is unavailable".to_string(),
                })?
                .remove(&provider_id);
        }
        Ok(removed)
    }

    pub fn unregister_agent(&self, agent_id: String) -> Result<bool, VifuRuntimeError> {
        self.runtime.unregister_agent(&agent_id).map_err(Into::into)
    }

    pub fn unregister_endpoint(&self, name: String) -> Result<bool, VifuRuntimeError> {
        self.runtime.unregister_endpoint(&name).map_err(Into::into)
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
                max_concurrency: 1,
            })
            .map_err(VifuRuntimeError::from)?;
            self.runtime
                .register_provider(provider_id.clone(), Arc::new(provider))?;
            self.remember_provider_type(provider_id, "llama")?;
            Ok(())
        }
        #[cfg(not(feature = "local-llama"))]
        {
            let _ = (provider_id, config);
            Err(VifuRuntimeError::InvalidConfig {
                message: "local llama is provided by the separate Android llama module".to_string(),
            })
        }
    }

    pub fn register_llama_provider_with_backends(
        &self,
        provider_id: String,
        config: VifuLlamaProviderConfig,
        backend_library_directory: String,
    ) -> Result<(), VifuRuntimeError> {
        #[cfg(feature = "local-llama")]
        {
            let provider = LlamaProvider::load_with_backend_directory(
                LlamaProviderConfig {
                    model_path: config.model_path.into(),
                    context_size: config.context_size,
                    gpu_layers: config.gpu_layers,
                    default_max_tokens: config.default_max_tokens,
                    max_concurrency: 1,
                },
                std::path::Path::new(&backend_library_directory),
            )
            .map_err(VifuRuntimeError::from)?;
            self.runtime
                .register_provider(provider_id.clone(), Arc::new(provider))?;
            self.remember_provider_type(provider_id, "llama")?;
            Ok(())
        }
        #[cfg(not(feature = "local-llama"))]
        {
            let _ = (provider_id, config, backend_library_directory);
            Err(VifuRuntimeError::InvalidConfig {
                message: "local llama is provided by the separate Android llama module".to_string(),
            })
        }
    }

    pub fn register_whisper_provider(
        &self,
        provider_id: String,
        config: VifuWhisperProviderConfig,
    ) -> Result<(), VifuRuntimeError> {
        #[cfg(feature = "local-whisper")]
        {
            let provider = vifu_runtime::LocalWhisperProvider::new(
                provider_id.clone(),
                config.model_path,
                config.language,
            )?;
            self.runtime
                .register_provider(provider_id.clone(), Arc::new(provider))?;
            self.remember_provider_type(provider_id, "local-whisper")?;
            Ok(())
        }
        #[cfg(not(feature = "local-whisper"))]
        {
            let _ = (provider_id, config);
            Err(VifuRuntimeError::InvalidConfig {
                message: "local Whisper is provided by the separate Android Whisper module"
                    .to_string(),
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
        let releases = self.runtime.releases()?;
        if let Some(active_version) = self.runtime.active_release_version()? {
            if let Some(active) = releases
                .iter()
                .find(|release| release.version == active_version)
            {
                if active.manifest == manifest {
                    return Ok(self.runtime.activate_release(active_version)?.manifest);
                }
            }
        }
        if releases.is_empty() {
            return Ok(self.runtime.bootstrap_release(manifest)?.manifest);
        }
        let version = releases
            .iter()
            .map(|release| release.version)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| VifuRuntimeError::Runtime {
                message: "embedded Runtime release version is exhausted".to_string(),
            })?;
        let release = RuntimeRelease::new(version, manifest)?;
        self.runtime.install_release(&release)?;
        Ok(self.runtime.activate_release(version)?.manifest)
    }
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct VifuEmbeddedGatewayConfig {
    pub server_url: String,
    pub runtime_database_path: String,
    pub server_certificate_der: Option<Vec<u8>>,
    pub gateway_metadata_json: String,
}

#[derive(Clone, uniffi::Record)]
pub struct VifuGeneratedGatewayIdentity {
    pub machine_id: String,
    pub public_key: String,
    pub private_key: String,
}

#[uniffi::export]
pub fn generate_vifu_gateway_identity() -> Result<VifuGeneratedGatewayIdentity, VifuRuntimeError> {
    let identity = MachineIdentity::generate()?;
    Ok(VifuGeneratedGatewayIdentity {
        machine_id: identity.machine_id.clone(),
        public_key: identity.public_key.clone(),
        private_key: identity.encoded_private_key().to_string(),
    })
}

#[derive(Debug, Clone, uniffi::Enum)]
pub enum VifuEmbeddedGatewayState {
    Stopped,
    Connecting,
    Connected,
    Reconnecting,
    AuthorizationRequired,
    Degraded,
    Failed,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct VifuEmbeddedGatewayStatus {
    pub state: VifuEmbeddedGatewayState,
    pub last_error: Option<String>,
    pub guest_project: Option<VifuGuestProject>,
    pub authorization: Option<VifuGatewayAuthorization>,
    pub pairing: Option<VifuGatewayPairing>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct VifuGatewayAuthorization {
    pub gateway_id: String,
    pub device_token: String,
    pub generation: u64,
    pub expires_at: String,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct VifuGatewayPairing {
    pub request_id: String,
    pub auth_url: String,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct VifuGuestProject {
    pub project_slug: String,
    pub endpoint_path: String,
    pub claim_token: String,
    pub expires_at: String,
}

#[derive(uniffi::Object)]
pub struct VifuEmbeddedGateway {
    runtime: Arc<VifuEmbeddedRuntime>,
    gateway: EmbeddedRuntimeGateway,
}

#[uniffi::export]
impl VifuEmbeddedGateway {
    #[uniffi::constructor]
    pub fn new(
        runtime: Arc<VifuEmbeddedRuntime>,
        config: VifuEmbeddedGatewayConfig,
    ) -> Result<Arc<Self>, VifuRuntimeError> {
        let mut gateway_config =
            EmbeddedRuntimeGatewayConfig::new(config.server_url, config.runtime_database_path);
        if let Some(certificate_der) = config.server_certificate_der {
            gateway_config = gateway_config.with_server_certificate_der(certificate_der);
        }
        let gateway_metadata = parse_json(&config.gateway_metadata_json, "gateway metadata")?;
        gateway_config = gateway_config
            .with_gateway_metadata(gateway_metadata)
            .map_err(|message| VifuRuntimeError::InvalidConfig { message })?;
        let gateway = EmbeddedRuntimeGateway::new(runtime.runtime.clone(), gateway_config)
            .map_err(|message| VifuRuntimeError::InvalidConfig { message })?;
        Ok(Arc::new(Self { runtime, gateway }))
    }

    /// Starts the optional network Gateway with caller-owned credential storage.
    pub fn start(
        &self,
        machine_private_key: String,
        device_token: Option<String>,
        enrollment_token: Option<String>,
    ) -> Result<(), VifuRuntimeError> {
        self.runtime.prepare_gateway_release()?;
        let identity = MachineIdentity::from_encoded_private_key(&machine_private_key)?;
        self.gateway
            .start(identity, device_token, enrollment_token)
            .map_err(Into::into)
    }

    /// Starts the Gateway with an explicit host consent decision for root invocation content.
    pub fn start_with_monitor_io(
        &self,
        machine_private_key: String,
        device_token: Option<String>,
        enrollment_token: Option<String>,
        capture_monitor_io: bool,
    ) -> Result<(), VifuRuntimeError> {
        self.runtime.prepare_gateway_release()?;
        let identity = MachineIdentity::from_encoded_private_key(&machine_private_key)?;
        self.gateway
            .start_with_monitor_io(identity, device_token, enrollment_token, capture_monitor_io)
            .map_err(Into::into)
    }

    pub fn stop(&self) -> Result<(), VifuRuntimeError> {
        self.gateway.stop().map_err(Into::into)
    }

    pub fn status(&self) -> Result<VifuEmbeddedGatewayStatus, VifuRuntimeError> {
        let status = self.gateway.status()?;
        Ok(VifuEmbeddedGatewayStatus {
            state: status.state.into(),
            last_error: status.last_error,
            guest_project: self.gateway.guest_project()?.map(|guest| VifuGuestProject {
                project_slug: guest.project_slug,
                endpoint_path: guest.endpoint_path,
                claim_token: guest.claim_token,
                expires_at: guest.expires_at,
            }),
            authorization: self.gateway.authorization()?.map(|authorization| {
                VifuGatewayAuthorization {
                    gateway_id: authorization.gateway_id,
                    device_token: authorization.device_token,
                    generation: authorization.generation,
                    expires_at: authorization.expires_at,
                }
            }),
            pairing: self.gateway.pairing()?.map(|pairing| VifuGatewayPairing {
                request_id: pairing.request_id.to_string(),
                auth_url: pairing.auth_url,
            }),
        })
    }
}

impl From<EmbeddedRuntimeGatewayState> for VifuEmbeddedGatewayState {
    fn from(state: EmbeddedRuntimeGatewayState) -> Self {
        match state {
            EmbeddedRuntimeGatewayState::Stopped => Self::Stopped,
            EmbeddedRuntimeGatewayState::Connecting => Self::Connecting,
            EmbeddedRuntimeGatewayState::Connected => Self::Connected,
            EmbeddedRuntimeGatewayState::Reconnecting => Self::Reconnecting,
            EmbeddedRuntimeGatewayState::AuthorizationRequired => Self::AuthorizationRequired,
            EmbeddedRuntimeGatewayState::Degraded => Self::Degraded,
            EmbeddedRuntimeGatewayState::Failed => Self::Failed,
        }
    }
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

fn provider_request_to_ffi(request: ProviderRequest) -> Result<VifuProviderRequest, RuntimeError> {
    let ProviderRequest {
        project_id,
        endpoint,
        session_id,
        agent,
        capability,
        data,
        metadata,
        snapshot,
    } = request;
    Ok(VifuProviderRequest {
        project_id,
        endpoint,
        session_id,
        provider_id: agent.provider,
        agent_id: agent.id,
        agent_name: agent.name,
        agent_capabilities: agent.capabilities,
        agent_metadata_json: encode_json(&agent.metadata)?,
        capability,
        data: data.into(),
        metadata_json: encode_json(&metadata)?,
        state_json: encode_json(&snapshot.state)?,
        state_revision: snapshot.revision,
    })
}

fn provider_response_from_ffi(
    response: VifuProviderResponse,
) -> Result<ProviderResponse, RuntimeError> {
    Ok(ProviderResponse {
        data: response.data.try_into()?,
        metadata: parse_json(&response.metadata_json, "provider metadata")?,
        state: response
            .state_json
            .as_deref()
            .map(|state| parse_json(state, "provider state"))
            .transpose()?,
    })
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

    #[cfg(feature = "local-llama")]
    #[test]
    fn llama_setup_errors_keep_configuration_and_runtime_failures_distinct() {
        assert!(matches!(
            VifuRuntimeError::from(LlamaProviderError::InvalidContextSize),
            VifuRuntimeError::InvalidConfig { .. }
        ));
        assert!(matches!(
            VifuRuntimeError::from(LlamaProviderError::BackendDiscovery("missing".to_string())),
            VifuRuntimeError::Runtime { .. }
        ));
        assert!(matches!(
            VifuRuntimeError::from(LlamaProviderError::Model("rejected".to_string())),
            VifuRuntimeError::Runtime { .. }
        ));
    }

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

    struct StreamingEchoProvider;

    impl VifuStreamingAgentProvider for StreamingEchoProvider {
        fn invoke(
            &self,
            request: VifuProviderRequest,
            invocation: Arc<VifuProviderInvocation>,
        ) -> Result<VifuProviderResponse, VifuRuntimeError> {
            invocation.stage_started(VifuProviderStage::Load, "{}".to_string())?;
            invocation.output_delta(request.data.clone())?;
            invocation.stage_completed(
                VifuProviderStage::Load,
                1,
                r#"{"model":"test"}"#.to_string(),
            )?;
            Ok(VifuProviderResponse {
                data: request.data,
                metadata_json: r#"{"contentType":"application/json"}"#.to_string(),
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
    }

    #[test]
    fn streaming_provider_can_emit_events_and_be_unloaded() {
        let runtime = VifuEmbeddedRuntime::new("streaming-provider".to_string()).unwrap();
        runtime
            .register_streaming_provider(
                "optional-llama".to_string(),
                "llama".to_string(),
                Box::new(StreamingEchoProvider),
            )
            .unwrap();
        runtime
            .register_agent(
                "guide".to_string(),
                "Guide".to_string(),
                "optional-llama".to_string(),
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

        let manifest = runtime.prepare_gateway_release().unwrap();
        assert_eq!(manifest.providers[0].provider_type, "llama");

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
        loop {
            let poll = runtime.poll_invocation(handle.clone()).unwrap();
            if matches!(poll.state, VifuInvocationState::Completed) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "streaming callback did not finish"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        let events = runtime.drain_invocation_events(handle).unwrap();
        assert!(events
            .iter()
            .any(|event| matches!(event.kind, VifuInvocationEventKind::OutputDelta)));

        assert!(runtime.unregister_endpoint("guide".to_string()).unwrap());
        assert!(runtime.unregister_agent("guide".to_string()).unwrap());
        assert!(runtime
            .unregister_provider("optional-llama".to_string())
            .unwrap());
        assert!(!runtime
            .unregister_provider("optional-llama".to_string())
            .unwrap());
    }

    #[test]
    fn embedded_gateway_refreshes_a_persisted_empty_manifest_after_models_are_registered() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "vifu-mobile-release-refresh-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let database = directory.join("runtime.sqlite");

        let empty = VifuEmbeddedRuntime::open(
            "distribution-project".to_string(),
            database.display().to_string(),
        )
        .unwrap();
        assert!(empty.prepare_gateway_release().unwrap().agents.is_empty());
        drop(empty);

        let configured = VifuEmbeddedRuntime::open(
            "distribution-project".to_string(),
            database.display().to_string(),
        )
        .unwrap();
        configured
            .register_provider("native".to_string(), Box::new(EchoProvider))
            .unwrap();
        configured
            .register_agent(
                "guide".to_string(),
                "Guide".to_string(),
                "native".to_string(),
                vec!["chat".to_string()],
                "{}".to_string(),
            )
            .unwrap();
        configured
            .register_endpoint(
                "guide".to_string(),
                "guide".to_string(),
                "chat".to_string(),
                500,
            )
            .unwrap();

        let refreshed = configured.prepare_gateway_release().unwrap();

        assert_eq!(refreshed.agents[0].id, "guide");
        assert_eq!(
            configured.runtime.active_release_version().unwrap(),
            Some(2)
        );
        drop(configured);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn generated_gateway_identity_is_ready_for_keychain_storage() {
        let identity = generate_vifu_gateway_identity().unwrap();

        assert!(identity.machine_id.starts_with("machine-"));
        let restored = MachineIdentity::from_encoded_private_key(&identity.private_key).unwrap();
        assert_eq!(restored.machine_id, identity.machine_id);
        assert_eq!(restored.public_key, identity.public_key);
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
