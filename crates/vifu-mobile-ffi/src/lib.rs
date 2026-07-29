//! UniFFI facade for embedding Vifu Runtime and Gateway utilities in native clients.

use std::sync::Arc;

use serde_json::Value;
use vifu_gateway::{config, openclaw, relay};
use vifu_runtime::{
    AgentDefinition, AgentProvider, CancellationToken, EndpointDefinition, InvocationData,
    InvocationHandle, InvocationInput, InvocationOutput, InvocationStatus, ProviderFuture,
    ProviderRequest, ProviderResponse, RuntimeError, VifuRuntime,
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

#[uniffi::export(callback_interface)]
pub trait VifuAgentProvider: Send + Sync {
    fn supports(&self, capability: String) -> bool;

    fn invoke(
        &self,
        request: VifuProviderRequest,
    ) -> Result<VifuProviderResponse, VifuRuntimeError>;
}

struct FfiAgentProvider {
    id: String,
    inner: Box<dyn VifuAgentProvider>,
}

impl AgentProvider for FfiAgentProvider {
    fn supports(&self, capability: &str) -> bool {
        self.inner.supports(capability.to_string())
    }

    fn invoke<'a>(
        &'a self,
        request: ProviderRequest,
        _cancellation: CancellationToken,
    ) -> ProviderFuture<'a> {
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
            let response = self.inner.invoke(ffi_request).map_err(|_error| {
                RuntimeError::provider(&self.id, "native provider callback failed")
            })?;
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

#[derive(uniffi::Object)]
pub struct VifuEmbeddedRuntime {
    runtime: VifuRuntime,
}

#[uniffi::export]
impl VifuEmbeddedRuntime {
    #[uniffi::constructor]
    pub fn new(project_id: String) -> Result<Arc<Self>, VifuRuntimeError> {
        Ok(Arc::new(Self {
            runtime: VifuRuntime::new(project_id)?,
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
                id: provider_id,
                inner: provider,
            }),
        )?;
        Ok(())
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

    pub fn cancel_invocation(&self, handle: String) -> Result<(), VifuRuntimeError> {
        self.runtime
            .cancel_invocation(&InvocationHandle(handle))
            .map_err(Into::into)
    }

    pub fn export_snapshot(&self) -> Result<Vec<u8>, VifuRuntimeError> {
        self.runtime.export_snapshot().map_err(Into::into)
    }

    pub fn restore_snapshot(&self, snapshot: Vec<u8>) -> Result<(), VifuRuntimeError> {
        self.runtime.restore_snapshot(&snapshot).map_err(Into::into)
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
        fn supports(&self, capability: String) -> bool {
            capability == "chat"
        }

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

        let snapshot = runtime.export_snapshot().unwrap();
        let restored = configured_runtime("ffi-project");
        restored.restore_snapshot(snapshot).unwrap();
        let restored_snapshot = restored.export_snapshot().unwrap();
        assert!(String::from_utf8_lossy(&restored_snapshot).contains("\"turns\":1"));
    }
}
