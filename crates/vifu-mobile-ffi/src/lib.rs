//! UniFFI facade for embedding the Vifu gateway runtime in native clients.

use vifu_core::{config, openclaw, relay};

#[derive(Debug, Clone, uniffi::Record)]
pub struct VifuRuntimeConfig {
    pub server_url: String,
    pub openclaw_url: String,
    pub agent_gateway_token: String,
    pub openclaw_token: Option<String>,
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
        agent_gateway_token: config::DEFAULT_AGENT_GATEWAY_TOKEN.to_string(),
        openclaw_token: None,
    }
}

#[uniffi::export]
pub fn vifu_agent_gateway_websocket_url(server_url: String) -> Result<String, VifuRuntimeError> {
    relay::agent_gateway_websocket_url(&server_url).map_err(|message| {
        VifuRuntimeError::InvalidConfig {
            message,
        }
    })
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

uniffi::setup_scaffolding!();
