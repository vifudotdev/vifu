//! Optional llama.cpp provider boundary for Android.

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use vifu_provider_llama::{LlamaProvider, LlamaProviderConfig, LlamaProviderError};
use vifu_runtime::{
    AgentDefinition, AgentProvider, CancellationToken, InvocationData, ProviderEvent,
    ProviderEventSink, ProviderRequest, ProviderResponse, ProviderStage, RuntimeError,
    RuntimeSnapshot,
};

#[derive(Debug, thiserror::Error, uniffi::Error)]
#[uniffi(flat_error)]
pub enum VifuLlamaError {
    #[error("{message}")]
    InvalidConfig { message: String },
    #[error("{message}")]
    Runtime { message: String },
}

impl From<LlamaProviderError> for VifuLlamaError {
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

impl From<RuntimeError> for VifuLlamaError {
    fn from(error: RuntimeError) -> Self {
        match error {
            RuntimeError::InvalidDefinition(message) => Self::InvalidConfig { message },
            error => Self::Runtime {
                message: error.public_message(),
            },
        }
    }
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct VifuLlamaConfig {
    pub model_path: String,
    pub context_size: u32,
    pub gpu_layers: u32,
    pub default_max_tokens: u32,
}

#[derive(Clone, uniffi::Enum)]
pub enum VifuLlamaData {
    Json { json: String },
    Binary { bytes: Vec<u8> },
}

#[derive(Clone, uniffi::Record)]
pub struct VifuLlamaRequest {
    pub project_id: String,
    pub endpoint: String,
    pub session_id: String,
    pub provider_id: String,
    pub agent_id: String,
    pub agent_name: String,
    pub agent_capabilities: Vec<String>,
    pub agent_metadata_json: String,
    pub capability: String,
    pub data: VifuLlamaData,
    pub metadata_json: String,
    pub state_json: String,
    pub state_revision: u64,
}

#[derive(Clone, uniffi::Record)]
pub struct VifuLlamaResponse {
    pub data: VifuLlamaData,
    pub metadata_json: String,
    pub state_json: Option<String>,
}

#[derive(Clone, uniffi::Enum)]
pub enum VifuLlamaStage {
    Queue,
    Load,
    Tokenize,
    Prefill,
    FirstToken,
    Decode,
    Validate,
}

#[uniffi::export(callback_interface)]
pub trait VifuLlamaInvocation: Send + Sync {
    fn is_cancelled(&self) -> bool;
    fn output_delta_json(&self, json: String);
    fn output_delta_binary(&self, bytes: Vec<u8>);
    fn activity(&self);
    fn stage_started(&self, stage: VifuLlamaStage, metadata_json: String);
    fn stage_completed(&self, stage: VifuLlamaStage, elapsed_ms: u64, metadata_json: String);
    fn stage_failed(
        &self,
        stage: VifuLlamaStage,
        elapsed_ms: u64,
        error: String,
        metadata_json: String,
    );
}

#[derive(uniffi::Object)]
pub struct VifuLlamaProvider {
    inner: LlamaProvider,
}

#[uniffi::export]
impl VifuLlamaProvider {
    #[uniffi::constructor]
    pub fn load(config: VifuLlamaConfig) -> Result<Arc<Self>, VifuLlamaError> {
        Ok(Arc::new(Self {
            inner: LlamaProvider::load(config.into())?,
        }))
    }

    #[uniffi::constructor(name = "load_with_backends")]
    pub fn load_with_backends(
        config: VifuLlamaConfig,
        backend_library_directory: String,
    ) -> Result<Arc<Self>, VifuLlamaError> {
        Ok(Arc::new(Self {
            inner: LlamaProvider::load_with_backend_directory(
                config.into(),
                std::path::Path::new(&backend_library_directory),
            )?,
        }))
    }

    pub fn invoke(
        &self,
        request: VifuLlamaRequest,
        invocation: Box<dyn VifuLlamaInvocation>,
    ) -> Result<VifuLlamaResponse, VifuLlamaError> {
        let invocation: Arc<dyn VifuLlamaInvocation> = Arc::from(invocation);
        let cancellation = CancellationToken::default();
        let event_invocation = Arc::clone(&invocation);
        let events = ProviderEventSink::from_fn(move |event| {
            forward_event(event_invocation.as_ref(), event)
        });
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .map_err(|error| VifuLlamaError::Runtime {
                message: error.to_string(),
            })?;
        let provider_request = request.try_into()?;
        let provider_cancellation = cancellation.clone();
        let cancellation_invocation = Arc::clone(&invocation);
        let response = runtime.block_on(async {
            let invoke =
                self.inner
                    .invoke_with_events(provider_request, provider_cancellation, events);
            tokio::pin!(invoke);
            loop {
                tokio::select! {
                    response = &mut invoke => break response,
                    _ = tokio::time::sleep(Duration::from_millis(10)) => {
                        if cancellation_invocation.is_cancelled() {
                            cancellation.cancel();
                            break Err(RuntimeError::Cancelled);
                        }
                    }
                }
            }
        })?;
        response.try_into().map_err(Into::into)
    }
}

impl From<VifuLlamaConfig> for LlamaProviderConfig {
    fn from(config: VifuLlamaConfig) -> Self {
        Self {
            model_path: config.model_path.into(),
            context_size: config.context_size,
            gpu_layers: config.gpu_layers,
            default_max_tokens: config.default_max_tokens,
            max_concurrency: 1,
        }
    }
}

impl TryFrom<VifuLlamaRequest> for ProviderRequest {
    type Error = RuntimeError;

    fn try_from(request: VifuLlamaRequest) -> Result<Self, Self::Error> {
        Ok(Self {
            project_id: request.project_id,
            endpoint: request.endpoint,
            session_id: request.session_id,
            agent: AgentDefinition {
                id: request.agent_id,
                name: request.agent_name,
                provider: request.provider_id,
                capabilities: request.agent_capabilities,
                metadata: parse_json(&request.agent_metadata_json, "agent metadata")?,
            },
            capability: request.capability,
            data: request.data.try_into()?,
            metadata: parse_json(&request.metadata_json, "provider metadata")?,
            snapshot: RuntimeSnapshot {
                revision: request.state_revision,
                state: parse_json(&request.state_json, "provider state")?,
            },
        })
    }
}

impl TryFrom<VifuLlamaData> for InvocationData {
    type Error = RuntimeError;

    fn try_from(data: VifuLlamaData) -> Result<Self, Self::Error> {
        match data {
            VifuLlamaData::Json { json } => Ok(Self::Json(parse_json(&json, "invocation JSON")?)),
            VifuLlamaData::Binary { bytes } => Ok(Self::Binary(bytes)),
        }
    }
}

impl From<InvocationData> for VifuLlamaData {
    fn from(data: InvocationData) -> Self {
        match data {
            InvocationData::Json(value) => Self::Json {
                json: value.to_string(),
            },
            InvocationData::Binary(bytes) => Self::Binary { bytes },
        }
    }
}

impl TryFrom<ProviderResponse> for VifuLlamaResponse {
    type Error = RuntimeError;

    fn try_from(response: ProviderResponse) -> Result<Self, Self::Error> {
        Ok(Self {
            data: response.data.into(),
            metadata_json: encode_json(&response.metadata)?,
            state_json: response.state.as_ref().map(encode_json).transpose()?,
        })
    }
}

fn forward_event(invocation: &dyn VifuLlamaInvocation, event: ProviderEvent) {
    match event {
        ProviderEvent::Activity => invocation.activity(),
        ProviderEvent::OutputDelta { data } => match data {
            InvocationData::Json(value) => invocation.output_delta_json(value.to_string()),
            InvocationData::Binary(bytes) => invocation.output_delta_binary(bytes),
        },
        ProviderEvent::StageStarted { stage, metadata } => {
            invocation.stage_started(stage.into(), metadata.to_string())
        }
        ProviderEvent::StageCompleted {
            stage,
            elapsed_ms,
            metadata,
        } => invocation.stage_completed(stage.into(), elapsed_ms, metadata.to_string()),
        ProviderEvent::StageFailed {
            stage,
            elapsed_ms,
            error,
            metadata,
        } => invocation.stage_failed(stage.into(), elapsed_ms, error, metadata.to_string()),
    }
}

impl From<ProviderStage> for VifuLlamaStage {
    fn from(stage: ProviderStage) -> Self {
        match stage {
            ProviderStage::Queue => Self::Queue,
            ProviderStage::Load => Self::Load,
            ProviderStage::Tokenize => Self::Tokenize,
            ProviderStage::Prefill => Self::Prefill,
            ProviderStage::FirstToken => Self::FirstToken,
            ProviderStage::Decode => Self::Decode,
            ProviderStage::Validate => Self::Validate,
        }
    }
}

fn parse_json(json: &str, kind: &str) -> Result<Value, RuntimeError> {
    serde_json::from_str(json)
        .map_err(|error| RuntimeError::InvalidDefinition(format!("{kind} is invalid: {error}")))
}

fn encode_json(value: &Value) -> Result<String, RuntimeError> {
    serde_json::to_string(value).map_err(|_error| RuntimeError::Internal)
}

uniffi::setup_scaffolding!();
