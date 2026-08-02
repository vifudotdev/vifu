use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::io::{self, IsTerminal, Write};
use std::net::IpAddr;
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{mpsc, oneshot, OwnedSemaphorePermit, Semaphore};
use tokio::task::JoinHandle;
use tokio_tungstenite::connect_async_with_config;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::Message;
use url::Url;
use uuid::Uuid;

use crate::control::{GuestProjectBootstrap, RuntimeControlClient, TraceObservationUploadError};
use crate::gateway_frame;
use crate::openclaw::{self, Endpoint};
use crate::optimization::SessionRouteOverrides;
use crate::protocol::{
    self, canonical_trace_io_summary, AgentDescriptor, AgentGatewayCommand, ApplicationFeedback,
    TraceDeliveryStatus, TraceIoSummary, TraceStageStatus, TraceTelemetry, TraceTelemetryBatch,
    MAX_TRACE_DROPPED_EVENTS, MAX_TRACE_TELEMETRY_EVENTS,
};
use crate::session::{self, GuestProjectSummary, PairingSummary, SessionSummary};
#[cfg(feature = "sqlite")]
use crate::session_store::GatewaySessionPersistence;

use vifu_runtime::{
    AgentDefinition, AgentProvider, CancellationToken, InvocationData, ProviderRequest,
    RuntimeSnapshot, VifuRuntime,
};
pub use vifu_runtime::{ProviderEvent, ProviderEventSink, ProviderStage};
#[cfg(feature = "sqlite")]
use vifu_runtime::{RuntimeStore, SqliteRuntimeStore};

const MAX_CONCURRENT_CALLS: usize = 64;
const OUTBOUND_QUEUE_CAPACITY: usize = 128;
const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(30);
const MAX_PENDING_TELEMETRY_BATCHES: usize = 512;
const MAX_CONCURRENT_TELEMETRY_UPLOADS: usize = 4;
const MAX_CONCURRENT_REJECTION_DELIVERIES: usize = 64;
const TELEMETRY_RETRY_INITIAL_DELAY: Duration = Duration::from_millis(250);
const TELEMETRY_RETRY_MAX_DELAY: Duration = Duration::from_secs(5);

struct OutboundCommand {
    command: AgentGatewayCommand,
    delivery: Option<oneshot::Sender<bool>>,
}

impl OutboundCommand {
    fn best_effort(command: AgentGatewayCommand) -> Self {
        Self {
            command,
            delivery: None,
        }
    }

    fn tracked(command: AgentGatewayCommand) -> (Self, oneshot::Receiver<bool>) {
        let (delivery, receiver) = oneshot::channel();
        (
            Self {
                command,
                delivery: Some(delivery),
            },
            receiver,
        )
    }
}

#[derive(Clone)]
struct PendingTelemetryBatch {
    request_id: Uuid,
    batch: TraceTelemetryBatch,
}

#[derive(Default)]
struct TelemetryBacklogState {
    pending: VecDeque<PendingTelemetryBatch>,
    flushing: bool,
    permanent_drops: u64,
    overflow_drops: u64,
}

type TelemetryBacklog = Arc<Mutex<TelemetryBacklogState>>;

struct ActiveProviderObservation {
    stage: ProviderStage,
    id: Uuid,
    start_offset_ms: u64,
}

struct InvocationTelemetry {
    events: Vec<TraceTelemetry>,
    active: Vec<ActiveProviderObservation>,
    dropped_events: u32,
    root_input_summary: Option<TraceIoSummary>,
    root_output_summary: Option<TraceIoSummary>,
    sealed: bool,
}

impl InvocationTelemetry {
    fn new(provider_key: String, capability: String, model: Option<String>) -> Self {
        Self {
            events: vec![TraceTelemetry::InvocationStarted {
                provider_key,
                capability,
                model,
            }],
            active: Vec::new(),
            dropped_events: 0,
            root_input_summary: None,
            root_output_summary: None,
            sealed: false,
        }
    }

    fn set_root_input_summary(&mut self, summary: TraceIoSummary) {
        if !self.sealed {
            self.root_input_summary = Some(summary);
        }
    }

    fn set_root_output_summary(&mut self, summary: TraceIoSummary) {
        if !self.sealed {
            self.root_output_summary = Some(summary);
        }
    }

    fn push(&mut self, event: TraceTelemetry) {
        if self.sealed {
            return;
        }
        if self.events.len() < MAX_TRACE_TELEMETRY_EVENTS {
            self.events.push(event);
        } else {
            self.dropped_events = self
                .dropped_events
                .saturating_add(1)
                .min(MAX_TRACE_DROPPED_EVENTS);
        }
    }

    fn push_terminal(&mut self, event: TraceTelemetry) {
        if self.events.len() < MAX_TRACE_TELEMETRY_EVENTS {
            self.events.push(event);
            return;
        }
        let observation_id = match &event {
            TraceTelemetry::ProviderStage { observation_id, .. }
            | TraceTelemetry::Delivery { observation_id, .. } => Some(*observation_id),
            TraceTelemetry::InvocationStarted { .. } => None,
        };
        let replacement = observation_id
            .and_then(|observation_id| {
                self.events.iter().position(|candidate| {
                    matches!(
                        candidate,
                        TraceTelemetry::ProviderStage {
                            observation_id: candidate_id,
                            status: TraceStageStatus::Started,
                            ..
                        } if *candidate_id == observation_id
                    )
                })
            })
            .or_else(|| {
                self.events.iter().position(|candidate| {
                    matches!(
                        candidate,
                        TraceTelemetry::ProviderStage {
                            status: TraceStageStatus::Started,
                            ..
                        }
                    )
                })
            })
            .unwrap_or(1.min(self.events.len().saturating_sub(1)));
        self.events[replacement] = event;
        self.dropped_events = self
            .dropped_events
            .saturating_add(1)
            .min(MAX_TRACE_DROPPED_EVENTS);
    }

    fn fail_active(&mut self, end_offset_ms: u64, error: Option<&str>) -> Vec<TraceTelemetry> {
        let active = std::mem::take(&mut self.active);
        active
            .into_iter()
            .map(|observation| {
                let telemetry = TraceTelemetry::ProviderStage {
                    observation_id: observation.id,
                    stage: observation.stage,
                    status: TraceStageStatus::Failed,
                    start_offset_ms: observation.start_offset_ms,
                    end_offset_ms: Some(end_offset_ms.max(observation.start_offset_ms)),
                    elapsed_ms: Some(end_offset_ms.saturating_sub(observation.start_offset_ms)),
                    request_elapsed_ms: None,
                    input_tokens: None,
                    output_tokens: None,
                    resident: None,
                    error: error.map(safe_observer_error),
                };
                self.push_terminal(telemetry.clone());
                telemetry
            })
            .collect()
    }

    fn finish(&mut self) -> TraceTelemetryBatch {
        self.sealed = true;
        self.active.clear();
        TraceTelemetryBatch {
            events: std::mem::take(&mut self.events),
            dropped_events: self.dropped_events,
            root_input_summary: self.root_input_summary.take(),
            root_output_summary: self.root_output_summary.take(),
        }
    }
}

pub struct AgentGatewayRuntime<'a> {
    pub server_url: &'a str,
    pub dashboard_url: Option<&'a str>,
    pub agent_gateway_bootstrap_token: Option<&'a str>,
    pub enrollment_token: Option<String>,
    pub allow_guest_bootstrap: bool,
    pub providers: &'a [Arc<dyn AgentGatewayProvider>],
    pub agents: &'a [AgentDescriptor],
    pub route_overrides: Option<Arc<SessionRouteOverrides>>,
    pub runtime_observer: Option<GatewayRuntimeObserver>,
    pub capture_sender: Option<mpsc::Sender<GatewayCaptureEvent>>,
    pub config_epoch: u64,
    pub provider_models: Option<ProviderModels>,
    pub session_path: Option<&'a Path>,
    pub runtime_database_path: &'a Path,
    pub embedded_runtime: Option<&'a VifuRuntime>,
    pub output_policy: GatewayOutputPolicy,
}

pub type GuestProjectObserver = Arc<dyn Fn(&GuestProjectSummary) + Send + Sync>;
pub type GatewayAuthorizationObserver = Arc<dyn Fn(&GatewayAuthorizationSummary) + Send + Sync>;
pub type GatewayPairingObserver = Arc<dyn Fn(Option<&PairingSummary>) + Send + Sync>;
pub type GatewayRuntimeObserver = Arc<dyn Fn(GatewayRuntimeEvent) + Send + Sync>;
pub type ProviderModels = Arc<HashMap<(String, String), String>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GatewayOutputPolicy {
    Terminal,
    Observer,
}

impl GatewayOutputPolicy {
    fn emits_terminal(self) -> bool {
        matches!(self, Self::Terminal)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GatewayConnectionState {
    Connected,
    Reconnecting,
    AuthorizationRequired,
    Degraded,
}

#[derive(Clone, Debug)]
pub struct GatewayDeliveryObservation {
    pub observation_id: Uuid,
    pub status: TraceDeliveryStatus,
    pub start_offset_ms: u64,
    pub end_offset_ms: u64,
    pub elapsed_ms: u64,
    pub error: Option<String>,
}

/// Payload-safe process-local events used by the live TUI. Raw request and
/// response values travel only through the separately bounded capture queue.
#[derive(Clone, Debug)]
pub enum GatewayRuntimeEvent {
    ConnectionStatus {
        state: GatewayConnectionState,
        message: Option<String>,
    },
    InvocationStarted {
        request_id: Uuid,
        endpoint_id: Uuid,
        profile_id: Uuid,
        binding_id: Uuid,
        agent_id: String,
        agent_name: String,
        provider_key: String,
        capability: String,
        model: Option<String>,
        model_parameters: serde_json::Value,
        timeout_ms: u64,
        started_unix_ms: u64,
    },
    ProviderStage {
        request_id: Uuid,
        observation_id: Uuid,
        stage: ProviderStage,
        status: TraceStageStatus,
        start_offset_ms: u64,
        end_offset_ms: Option<u64>,
        elapsed_ms: Option<u64>,
        request_elapsed_ms: Option<u64>,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        resident: Option<bool>,
        error: Option<String>,
    },
    InvocationFinished {
        request_id: Uuid,
        elapsed_ms: u64,
        terminal: GatewayInvocationTerminal,
        error: Option<String>,
        delivery_observation: Option<GatewayDeliveryObservation>,
    },
    ApplicationFeedback {
        request_id: Uuid,
        observation_id: Uuid,
        start_offset_ms: u64,
        end_offset_ms: u64,
        feedback: ApplicationFeedback,
    },
    CaptureDropped {
        config_epoch: u64,
        request_id: Uuid,
        binding_id: Uuid,
        capability: String,
        provider_key: String,
    },
    InvocationCancelled {
        config_epoch: u64,
        request_id: Uuid,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GatewayInvocationTerminal {
    Delivered,
    ProviderFailed,
    TimedOut,
    DeliveryFailed,
    PreflightFailed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GatewayProviderErrorKind {
    Failed,
    TimedOut,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GatewayProviderError {
    pub kind: GatewayProviderErrorKind,
    pub message: String,
}

impl GatewayProviderError {
    pub fn failed(message: impl Into<String>) -> Self {
        Self {
            kind: GatewayProviderErrorKind::Failed,
            message: message.into(),
        }
    }

    pub fn timed_out(message: impl Into<String>) -> Self {
        Self {
            kind: GatewayProviderErrorKind::TimedOut,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for GatewayProviderError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

/// Raw values used only by Vifu's bounded, process-local optimization worker.
/// Debug output deliberately exposes no payload content.
#[derive(Clone)]
pub enum GatewayCaptureEvent {
    InvocationStarted {
        config_epoch: u64,
        request_id: Uuid,
        binding_id: Uuid,
        agent_id: String,
        provider_key: String,
        capability: String,
        binding: Arc<serde_json::Value>,
        input: Arc<serde_json::Value>,
        timeout_ms: u64,
    },
    InvocationFinished {
        config_epoch: u64,
        request_id: Uuid,
        terminal: GatewayInvocationTerminal,
        output: Option<Arc<serde_json::Value>>,
    },
    InvocationCancelled {
        config_epoch: u64,
        request_id: Uuid,
    },
}

impl std::fmt::Debug for GatewayCaptureEvent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvocationStarted {
                config_epoch,
                request_id,
                binding_id,
                agent_id,
                provider_key,
                capability,
                timeout_ms,
                ..
            } => formatter
                .debug_struct("GatewayCaptureEvent::InvocationStarted")
                .field("config_epoch", config_epoch)
                .field("request_id", request_id)
                .field("binding_id", binding_id)
                .field("agent_id", agent_id)
                .field("provider_key", provider_key)
                .field("capability", capability)
                .field("timeout_ms", timeout_ms)
                .field("binding", &"[REDACTED]")
                .field("input", &"[REDACTED]")
                .finish(),
            Self::InvocationFinished {
                config_epoch,
                request_id,
                terminal,
                output,
            } => formatter
                .debug_struct("GatewayCaptureEvent::InvocationFinished")
                .field("config_epoch", config_epoch)
                .field("request_id", request_id)
                .field("terminal", terminal)
                .field("output", &output.as_ref().map(|_| "[REDACTED]"))
                .finish(),
            Self::InvocationCancelled {
                config_epoch,
                request_id,
            } => formatter
                .debug_struct("GatewayCaptureEvent::InvocationCancelled")
                .field("config_epoch", config_epoch)
                .field("request_id", request_id)
                .finish(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayAuthorizationSummary {
    pub gateway_id: String,
    pub device_token: String,
    pub generation: u64,
    pub expires_at: String,
}

pub trait AgentGatewayProvider: Send + Sync {
    fn id(&self) -> &str;
    fn provider_type(&self) -> &str;
    fn invoke<'a>(
        &'a self,
        agent_id: &'a str,
        binding: &'a serde_json::Value,
        input: &'a serde_json::Value,
        timeout: Duration,
    ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, GatewayProviderError>> + Send + 'a>>;

    fn invoke_with_events<'a>(
        &'a self,
        agent_id: &'a str,
        binding: &'a serde_json::Value,
        input: &'a serde_json::Value,
        timeout: Duration,
        _events: ProviderEventSink,
    ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, GatewayProviderError>> + Send + 'a>>
    {
        self.invoke(agent_id, binding, input, timeout)
    }
}

#[derive(Debug, Clone)]
pub struct OpenClawGatewayProvider {
    id: String,
    endpoint: Endpoint,
    token: Option<String>,
}

impl OpenClawGatewayProvider {
    pub fn new(id: impl Into<String>, endpoint: Endpoint, token: Option<String>) -> Self {
        Self {
            id: id.into(),
            endpoint,
            token,
        }
    }
}

impl AgentGatewayProvider for OpenClawGatewayProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn provider_type(&self) -> &str {
        "openclaw"
    }

    fn invoke<'a>(
        &'a self,
        agent_id: &'a str,
        binding: &'a serde_json::Value,
        input: &'a serde_json::Value,
        timeout: Duration,
    ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, GatewayProviderError>> + Send + 'a>>
    {
        Box::pin(async move {
            openclaw::invoke(
                &self.endpoint,
                self.token.as_deref(),
                agent_id,
                binding,
                input,
                timeout,
            )
            .await
            .map_err(GatewayProviderError::failed)
        })
    }
}

pub struct InProcessGatewayProvider {
    id: String,
    provider: Arc<dyn AgentProvider>,
}

impl InProcessGatewayProvider {
    pub fn new(id: impl Into<String>, provider: Arc<dyn AgentProvider>) -> Result<Self, String> {
        if !["chat", "embedding", "transcription", "tool", "realtime"]
            .iter()
            .any(|capability| provider.supports(capability))
        {
            return Err(
                "in-process provider must support at least one gateway capability".to_string(),
            );
        }
        Ok(Self {
            id: id.into(),
            provider,
        })
    }
}

impl AgentGatewayProvider for InProcessGatewayProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn provider_type(&self) -> &str {
        "vifu-runtime"
    }

    fn invoke<'a>(
        &'a self,
        agent_id: &'a str,
        binding: &'a serde_json::Value,
        input: &'a serde_json::Value,
        timeout: Duration,
    ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, GatewayProviderError>> + Send + 'a>>
    {
        self.invoke_with_events(
            agent_id,
            binding,
            input,
            timeout,
            ProviderEventSink::discard(),
        )
    }

    fn invoke_with_events<'a>(
        &'a self,
        agent_id: &'a str,
        binding: &'a serde_json::Value,
        input: &'a serde_json::Value,
        timeout: Duration,
        events: ProviderEventSink,
    ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, GatewayProviderError>> + Send + 'a>>
    {
        Box::pin(async move {
            let capability = binding_text(binding, "capability").unwrap_or("chat");
            if !self.provider.supports(capability) {
                return Err(GatewayProviderError::failed(format!(
                    "in-process provider does not support capability {capability}"
                )));
            }
            let cancellation = CancellationToken::default();
            let request = ProviderRequest {
                project_id: binding_text(binding, "projectId")
                    .unwrap_or("gateway")
                    .to_string(),
                endpoint: binding_text(binding, "endpoint")
                    .unwrap_or(agent_id)
                    .to_string(),
                session_id: binding_text(binding, "sessionId")
                    .unwrap_or("gateway-session")
                    .to_string(),
                agent: AgentDefinition {
                    id: agent_id.to_string(),
                    name: binding_text(binding, "agentName")
                        .unwrap_or(agent_id)
                        .to_string(),
                    provider: self.id.clone(),
                    capabilities: vec![capability.to_string()],
                    metadata: binding
                        .get("persona")
                        .filter(|value| value.is_object())
                        .cloned()
                        .unwrap_or_else(|| serde_json::json!({})),
                },
                capability: capability.to_string(),
                data: gateway_invocation_data(input).map_err(GatewayProviderError::failed)?,
                metadata: serde_json::json!({
                    "source": "agent-gateway",
                    "binding": binding,
                }),
                snapshot: RuntimeSnapshot::default(),
            };
            let invocation =
                self.provider
                    .invoke_with_events(request, cancellation.clone(), events);
            let response = match tokio::time::timeout(timeout, invocation).await {
                Ok(response) => response.map_err(|error| match error {
                    vifu_runtime::RuntimeError::Timeout(_) => {
                        GatewayProviderError::timed_out(error.public_message())
                    }
                    _ => GatewayProviderError::failed(error.public_message()),
                })?,
                Err(_) => {
                    cancellation.cancel();
                    return Err(GatewayProviderError::timed_out(
                        "in-process provider request timed out",
                    ));
                }
            };
            match response.data {
                InvocationData::Json(value) => Ok(value),
                InvocationData::Binary(_) => Err(GatewayProviderError::failed(
                    "in-process provider returned binary data",
                )),
            }
        })
    }
}

fn gateway_invocation_data(input: &serde_json::Value) -> Result<InvocationData, String> {
    let Some(binary) = input.get("_vifuBinary") else {
        return Ok(InvocationData::Json(input.clone()));
    };
    let encoding = binding_text(binary, "encoding").unwrap_or_default();
    if encoding != "base64" {
        return Err("binary gateway input must use base64 encoding".to_string());
    }
    let data = binding_text(binary, "data")
        .ok_or_else(|| "binary gateway input is missing data".to_string())?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|error| format!("binary gateway input could not be decoded: {error}"))?;
    Ok(InvocationData::Binary(bytes))
}

fn binding_text<'a>(binding: &'a serde_json::Value, name: &str) -> Option<&'a str> {
    binding
        .get(name)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn trace_model_parameters(config: &serde_json::Value) -> serde_json::Value {
    const SAFE_KEYS: [&str; 14] = [
        "backend",
        "contextSize",
        "dimensions",
        "gpuLayers",
        "maxTokens",
        "max_tokens",
        "model",
        "quantization",
        "responseFormat",
        "response_format",
        "temperature",
        "topP",
        "top_p",
        "voice",
    ];
    let Some(config) = config.as_object() else {
        return serde_json::json!({});
    };
    serde_json::Value::Object(
        SAFE_KEYS
            .into_iter()
            .filter_map(|key| {
                config
                    .get(key)
                    .filter(|value| value.is_boolean() || value.is_number() || value.is_string())
                    .map(|value| (key.to_string(), value.clone()))
            })
            .collect(),
    )
}

pub async fn run_agent_gateway(
    runtime: AgentGatewayRuntime<'_>,
    session: &mut SessionSummary,
) -> Result<(), String> {
    run_agent_gateway_inner(
        runtime,
        session,
        SessionPersistence::LegacyFile,
        None,
        None,
        None,
    )
    .await
}

#[cfg(feature = "sqlite")]
pub async fn run_agent_gateway_with_session_persistence(
    runtime: AgentGatewayRuntime<'_>,
    session: &mut SessionSummary,
    persistence: GatewaySessionPersistence,
    guest_project_observer: Option<GuestProjectObserver>,
    authorization_observer: Option<GatewayAuthorizationObserver>,
    pairing_observer: Option<GatewayPairingObserver>,
) -> Result<(), String> {
    run_agent_gateway_inner(
        runtime,
        session,
        SessionPersistence::Sqlite(persistence),
        guest_project_observer,
        authorization_observer,
        pairing_observer,
    )
    .await
}

enum SessionPersistence {
    LegacyFile,
    #[cfg(feature = "sqlite")]
    Sqlite(GatewaySessionPersistence),
}

#[derive(Clone, Copy)]
struct ConnectionObservers<'a> {
    guest_project: Option<&'a GuestProjectObserver>,
    authorization: Option<&'a GatewayAuthorizationObserver>,
    pairing: Option<&'a GatewayPairingObserver>,
}

async fn run_agent_gateway_inner(
    runtime: AgentGatewayRuntime<'_>,
    session: &mut SessionSummary,
    persistence: SessionPersistence,
    guest_project_observer: Option<GuestProjectObserver>,
    authorization_observer: Option<GatewayAuthorizationObserver>,
    pairing_observer: Option<GatewayPairingObserver>,
) -> Result<(), String> {
    let websocket_url = agent_gateway_websocket_url(runtime.server_url)?;
    let mut reconnect_delay = Duration::from_secs(1);
    let telemetry_backlog: TelemetryBacklog =
        Arc::new(Mutex::new(TelemetryBacklogState::default()));
    let telemetry_uploads = Arc::new(Semaphore::new(MAX_CONCURRENT_TELEMETRY_UPLOADS));

    loop {
        match run_connection(
            &websocket_url,
            &runtime,
            session,
            &persistence,
            ConnectionObservers {
                guest_project: guest_project_observer.as_ref(),
                authorization: authorization_observer.as_ref(),
                pairing: pairing_observer.as_ref(),
            },
            &telemetry_backlog,
            &telemetry_uploads,
        )
        .await
        {
            Ok(ConnectionOutcome::Shutdown) => return Ok(()),
            Ok(ConnectionOutcome::Disconnected) => {
                let message = format!(
                    "Agent Gateway disconnected; reconnecting in {}s.",
                    reconnect_delay.as_secs()
                );
                terminal_stderr(runtime.output_policy, &message);
                observe_connection_status(
                    &runtime,
                    GatewayConnectionState::Reconnecting,
                    Some(message),
                );
            }
            Err(AgentGatewayConnectionError::PairingRequired {
                request_id,
                auth_url,
                retry_after,
            }) => {
                let auth_url =
                    pairing_authorization_url(runtime.dashboard_url, &auth_url).unwrap_or(auth_url);
                let changed = session.pairing.as_ref().is_none_or(|pairing| {
                    pairing.request_id != request_id || pairing.auth_url != auth_url
                });
                session.pairing = Some(PairingSummary {
                    request_id,
                    auth_url: auth_url.clone(),
                });
                persist_session(&runtime, session, &persistence)?;
                if let Some(observer) = pairing_observer.as_ref() {
                    observer(session.pairing.as_ref());
                }
                if changed {
                    terminal_stdout(
                        runtime.output_policy,
                        &format!(
                            "\nAuthorization required\n  Dashboard: {}\n  Waiting for approval; this Gateway will reconnect automatically.",
                            terminal_link(&auth_url)
                        ),
                    );
                    observe_connection_status(
                        &runtime,
                        GatewayConnectionState::AuthorizationRequired,
                        Some(
                            "Authorization required; open the Dashboard to approve this Gateway"
                                .to_string(),
                        ),
                    );
                }
                reconnect_delay = retry_after;
            }
            Err(AgentGatewayConnectionError::Failed(error)) => {
                let message = format!(
                    "Agent Gateway connection failed: {}. Retrying in {}s.",
                    sanitize_error(&error),
                    reconnect_delay.as_secs()
                );
                terminal_stderr(runtime.output_policy, &message);
                observe_connection_status(
                    &runtime,
                    GatewayConnectionState::Reconnecting,
                    Some(message),
                );
            }
        }

        tokio::select! {
            _ = tokio::time::sleep(reconnect_delay) => {}
            _ = tokio::signal::ctrl_c() => return Ok(()),
        }
        reconnect_delay = reconnect_delay.saturating_mul(2).min(MAX_RECONNECT_DELAY);
    }
}

fn observe_connection_status(
    runtime: &AgentGatewayRuntime<'_>,
    state: GatewayConnectionState,
    message: Option<String>,
) {
    if let Some(observer) = runtime.runtime_observer.as_ref() {
        observer(GatewayRuntimeEvent::ConnectionStatus { state, message });
    }
}

fn write_terminal_line(
    policy: GatewayOutputPolicy,
    output: &mut impl Write,
    message: &str,
) -> io::Result<()> {
    if policy.emits_terminal() {
        writeln!(output, "{message}")?;
    }
    Ok(())
}

fn terminal_stdout(policy: GatewayOutputPolicy, message: &str) {
    let _ = write_terminal_line(policy, &mut io::stdout().lock(), message);
}

fn terminal_stderr(policy: GatewayOutputPolicy, message: &str) {
    let _ = write_terminal_line(policy, &mut io::stderr().lock(), message);
}

fn terminal_link(url: &str) -> String {
    if io::stdout().is_terminal() {
        format!("\u{1b}]8;;{url}\u{1b}\\{url}\u{1b}]8;;\u{1b}\\")
    } else {
        url.to_string()
    }
}

fn pairing_authorization_url(
    dashboard_url: Option<&str>,
    authorization_url: &str,
) -> Result<String, String> {
    if let Ok(url) = Url::parse(authorization_url) {
        if matches!(url.scheme(), "http" | "https") {
            return Ok(url.to_string());
        }
        return Err("Gateway pairing URL must use HTTP or HTTPS".to_string());
    }
    let dashboard_url = dashboard_url.ok_or_else(|| {
        "Gateway pairing requires gateway.dashboardUrl for a relative authorization URL".to_string()
    })?;
    let mut base = Url::parse(dashboard_url)
        .map_err(|_| "gateway.dashboardUrl must be a valid HTTP or HTTPS URL".to_string())?;
    if !matches!(base.scheme(), "http" | "https") {
        return Err("gateway.dashboardUrl must use HTTP or HTTPS".to_string());
    }
    base.set_path("/");
    base.set_query(None);
    base.set_fragment(None);
    base.join(authorization_url)
        .map(|url| url.to_string())
        .map_err(|_| "Gateway pairing URL is invalid".to_string())
}

fn apply_guest_project(
    runtime: &AgentGatewayRuntime<'_>,
    session: &mut SessionSummary,
    guest: &GuestProjectBootstrap,
    persistence: &SessionPersistence,
    guest_project_observer: Option<&GuestProjectObserver>,
) -> Result<(), String> {
    let guest_summary = guest_project_summary(guest);
    session.guest_project = Some(guest_summary.clone());
    session.resume_session_id = None;
    persist_session(runtime, session, persistence)?;
    print_guest_project(runtime.output_policy, runtime.server_url, guest);
    if let Some(observer) = guest_project_observer {
        observer(&guest_summary);
    }
    Ok(())
}

fn guest_project_summary(guest: &GuestProjectBootstrap) -> GuestProjectSummary {
    GuestProjectSummary {
        project_id: guest.project.id,
        project_slug: guest.project.slug.clone(),
        deployment_id: guest.deployment.id,
        deployment: guest.deployment.name.clone(),
        endpoint_path: guest.endpoint_path.clone(),
        api_key: guest.api_key.clone(),
        claim_token: guest.claim_token.clone(),
        expires_at: guest.expires_at.clone(),
    }
}

fn print_guest_project(
    output_policy: GatewayOutputPolicy,
    server_url: &str,
    guest: &GuestProjectBootstrap,
) {
    if !output_policy.emits_terminal() {
        return;
    }
    let endpoint = guest_endpoint_url(server_url, &guest.endpoint_path)
        .unwrap_or_else(|_| guest.endpoint_path.clone());
    terminal_stdout(
        output_policy,
        &format!(
            "\nProject registered\n  Project:  {}\n  Endpoint: {endpoint}\n  API key:  {}\n  Expires:  {}",
            guest.project.slug, guest.api_key, guest.expires_at
        ),
    );
}

pub fn guest_claim_url(dashboard_url: &str, claim_token: &str) -> Result<String, String> {
    let mut url = Url::parse(dashboard_url.trim())
        .map_err(|_| "gateway.dashboardUrl must be a valid HTTP or HTTPS URL".to_string())?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("gateway.dashboardUrl must be a valid HTTP or HTTPS URL".to_string());
    }
    url.set_path("/pair");
    url.set_query(None);
    let fragment = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("claim_token", claim_token)
        .finish();
    url.set_fragment(Some(&fragment));
    Ok(url.to_string())
}

pub fn guest_endpoint_url(server_url: &str, endpoint_path: &str) -> Result<String, String> {
    let mut url = Url::parse(server_url.trim())
        .map_err(|_| "gateway.serverUrl must be a valid HTTP or HTTPS URL".to_string())?;
    let base_path = url.path().trim_end_matches('/');
    url.set_path(&format!(
        "{base_path}/{}",
        endpoint_path.trim_start_matches('/')
    ));
    Ok(url.to_string().trim_end_matches('/').to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnectionOutcome {
    Disconnected,
    Shutdown,
}

#[derive(Debug, PartialEq, Eq)]
enum AgentGatewayConnectionError {
    PairingRequired {
        request_id: Uuid,
        auth_url: String,
        retry_after: Duration,
    },
    Failed(String),
}

impl From<String> for AgentGatewayConnectionError {
    fn from(value: String) -> Self {
        Self::Failed(value)
    }
}

fn provider_model(
    runtime: &AgentGatewayRuntime<'_>,
    provider_key: &str,
    capability: &str,
) -> Option<String> {
    runtime
        .provider_models
        .as_ref()
        .and_then(|models| models.get(&(provider_key.to_string(), capability.to_string())))
        .cloned()
}

fn safe_trace_telemetry(
    event: ProviderEvent,
    request_elapsed_ms: u64,
    recorder: &mut InvocationTelemetry,
) -> Option<TraceTelemetry> {
    if recorder.sealed {
        return None;
    }
    let (stage, status, elapsed_ms, metadata, error) = match event {
        ProviderEvent::OutputDelta { .. } => return None,
        ProviderEvent::StageStarted { stage, metadata } => {
            (stage, TraceStageStatus::Started, None, metadata, None)
        }
        ProviderEvent::StageCompleted {
            stage,
            elapsed_ms,
            metadata,
        } => (
            stage,
            TraceStageStatus::Completed,
            Some(elapsed_ms),
            metadata,
            None,
        ),
        ProviderEvent::StageFailed {
            stage,
            elapsed_ms,
            error,
            metadata,
        } => (
            stage,
            TraceStageStatus::Failed,
            Some(elapsed_ms),
            metadata,
            Some(safe_observer_error(&error)),
        ),
    };
    let event_offset_ms = request_elapsed_ms;
    let request_elapsed_ms = (stage == ProviderStage::FirstToken
        && status == TraceStageStatus::Completed)
        .then_some(event_offset_ms);
    let terminal_elapsed_ms = elapsed_ms.unwrap_or_default();
    let active_index = recorder
        .active
        .iter()
        .position(|observation| observation.stage == stage);
    let (observation_id, start_offset_ms) = if status == TraceStageStatus::Started {
        if let Some(index) = active_index {
            let observation = &recorder.active[index];
            (observation.id, observation.start_offset_ms)
        } else {
            let observation = ActiveProviderObservation {
                stage,
                id: Uuid::new_v4(),
                start_offset_ms: event_offset_ms,
            };
            let result = (observation.id, observation.start_offset_ms);
            recorder.active.push(observation);
            result
        }
    } else if let Some(index) = active_index {
        let observation = recorder.active.remove(index);
        (observation.id, observation.start_offset_ms)
    } else {
        (
            Uuid::new_v4(),
            event_offset_ms.saturating_sub(terminal_elapsed_ms),
        )
    };
    let end_offset_ms =
        (status != TraceStageStatus::Started).then_some(event_offset_ms.max(start_offset_ms));
    let telemetry = TraceTelemetry::ProviderStage {
        observation_id,
        stage,
        status,
        start_offset_ms,
        end_offset_ms,
        elapsed_ms,
        request_elapsed_ms,
        input_tokens: metadata
            .get("inputTokens")
            .and_then(serde_json::Value::as_u64),
        output_tokens: metadata
            .get("outputTokens")
            .and_then(serde_json::Value::as_u64),
        resident: metadata
            .get("resident")
            .and_then(serde_json::Value::as_bool),
        error,
    };
    recorder.push(telemetry.clone());
    Some(telemetry)
}

fn observe_trace_telemetry(
    observer: Option<&GatewayRuntimeObserver>,
    request_id: Uuid,
    telemetry: &TraceTelemetry,
) {
    let (
        TraceTelemetry::ProviderStage {
            observation_id,
            stage,
            status,
            start_offset_ms,
            end_offset_ms,
            elapsed_ms,
            request_elapsed_ms,
            input_tokens,
            output_tokens,
            resident,
            error,
        },
        Some(observer),
    ) = (telemetry, observer)
    else {
        return;
    };
    observer(GatewayRuntimeEvent::ProviderStage {
        request_id,
        observation_id: *observation_id,
        stage: *stage,
        status: *status,
        start_offset_ms: *start_offset_ms,
        end_offset_ms: *end_offset_ms,
        elapsed_ms: *elapsed_ms,
        request_elapsed_ms: *request_elapsed_ms,
        input_tokens: *input_tokens,
        output_tokens: *output_tokens,
        resident: *resident,
        error: error.clone(),
    });
}

fn try_capture(
    sender: Option<&mpsc::Sender<GatewayCaptureEvent>>,
    event: GatewayCaptureEvent,
) -> bool {
    sender.is_none_or(|sender| sender.try_send(event).is_ok())
}

fn observe_capture_dropped(
    observer: Option<&GatewayRuntimeObserver>,
    config_epoch: u64,
    request_id: Uuid,
    binding_id: Uuid,
    capability: &str,
    provider_key: &str,
) {
    if let Some(observer) = observer {
        observer(GatewayRuntimeEvent::CaptureDropped {
            config_epoch,
            request_id,
            binding_id,
            capability: capability.to_string(),
            provider_key: provider_key.to_string(),
        });
    }
}

struct InvocationDelivery {
    sender: mpsc::Sender<OutboundCommand>,
    observer: Option<GatewayRuntimeObserver>,
    capture_sender: Option<mpsc::Sender<GatewayCaptureEvent>>,
    config_epoch: u64,
    request_id: Uuid,
    binding_id: Uuid,
    capability: String,
    provider_key: String,
    invocation_started: Instant,
    telemetry: Arc<Mutex<InvocationTelemetry>>,
    telemetry_backlog: TelemetryBacklog,
    telemetry_client: RuntimeControlClient,
    telemetry_uploads: Arc<Semaphore>,
    invocation_permit: Option<OwnedSemaphorePermit>,
}

impl InvocationDelivery {
    async fn finish(
        self,
        command: AgentGatewayCommand,
        provider_terminal: GatewayInvocationTerminal,
        error: Option<String>,
        output: Option<Arc<serde_json::Value>>,
    ) {
        if let Some(output) = output.as_deref() {
            self.telemetry
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_root_output_summary(canonical_trace_io_summary(output));
        }
        let terminal_recording =
            (provider_terminal != GatewayInvocationTerminal::Delivered).then(|| {
                let mut telemetry = self
                    .telemetry
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let terminal_events = telemetry.fail_active(
                    duration_millis(self.invocation_started.elapsed()),
                    error.as_deref(),
                );
                let batch = telemetry.finish();
                (terminal_events, batch)
            });
        if let Some((terminal_events, _)) = terminal_recording.as_ref() {
            for telemetry in terminal_events {
                observe_trace_telemetry(self.observer.as_ref(), self.request_id, telemetry);
            }
        }
        let delivery_started = Instant::now();
        let delivery_start_offset_ms = duration_millis(self.invocation_started.elapsed());
        let (message, delivery) = OutboundCommand::tracked(command);
        let delivered = self.sender.send(message).await.is_ok() && delivery.await.unwrap_or(false);
        let delivery_elapsed_ms = duration_millis(delivery_started.elapsed());
        let valid_result = provider_terminal == GatewayInvocationTerminal::Delivered;
        let terminal = if valid_result && !delivered {
            GatewayInvocationTerminal::DeliveryFailed
        } else {
            provider_terminal
        };
        let terminal_error = if valid_result && !delivered {
            Some("Agent Gateway result could not be delivered".to_string())
        } else {
            error
        };
        let delivery_observation = if valid_result {
            let observation = GatewayDeliveryObservation {
                observation_id: Uuid::new_v4(),
                status: if delivered {
                    TraceDeliveryStatus::Delivered
                } else {
                    TraceDeliveryStatus::Failed
                },
                start_offset_ms: delivery_start_offset_ms,
                end_offset_ms: duration_millis(self.invocation_started.elapsed()),
                elapsed_ms: delivery_elapsed_ms,
                error: (!delivered)
                    .then(|| "Agent Gateway result could not be delivered".to_string()),
            };
            let telemetry = TraceTelemetry::Delivery {
                observation_id: observation.observation_id,
                status: observation.status,
                start_offset_ms: observation.start_offset_ms,
                end_offset_ms: observation.end_offset_ms,
                elapsed_ms: observation.elapsed_ms,
                error: observation.error.clone(),
            };
            self.telemetry
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(telemetry);
            Some(observation)
        } else {
            None
        };
        observe_finished(
            self.observer.as_ref(),
            self.request_id,
            duration_millis(self.invocation_started.elapsed()),
            terminal,
            terminal_error,
            delivery_observation,
        );
        if !try_capture(
            self.capture_sender.as_ref(),
            GatewayCaptureEvent::InvocationFinished {
                config_epoch: self.config_epoch,
                request_id: self.request_id,
                terminal,
                output,
            },
        ) {
            observe_capture_dropped(
                self.observer.as_ref(),
                self.config_epoch,
                self.request_id,
                self.binding_id,
                &self.capability,
                &self.provider_key,
            );
        }
        let batch = terminal_recording.map_or_else(
            || {
                self.telemetry
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .finish()
            },
            |(_, batch)| batch,
        );
        let pending = PendingTelemetryBatch {
            request_id: self.request_id,
            batch,
        };
        drop(self.invocation_permit);
        let Ok(telemetry_permit) = self.telemetry_uploads.clone().try_acquire_owned() else {
            enqueue_telemetry_batch(&self.telemetry_backlog, pending);
            trigger_telemetry_flush(
                &self.telemetry_client,
                &self.telemetry_backlog,
                &self.telemetry_uploads,
                self.observer.as_ref(),
            );
            return;
        };
        match send_telemetry_batch(&self.telemetry_client, &pending).await {
            Ok(()) => {}
            Err(error) if error.is_retryable() => {
                enqueue_telemetry_batch(&self.telemetry_backlog, pending);
            }
            Err(error) => record_permanent_telemetry_drop(
                &self.telemetry_backlog,
                self.observer.as_ref(),
                pending.request_id,
                &error,
            ),
        }
        drop(telemetry_permit);
        trigger_telemetry_flush(
            &self.telemetry_client,
            &self.telemetry_backlog,
            &self.telemetry_uploads,
            self.observer.as_ref(),
        );
    }

    fn finish_shed(
        self,
        command: AgentGatewayCommand,
        terminal: GatewayInvocationTerminal,
        error: String,
    ) {
        let terminal_events = {
            let mut telemetry = self
                .telemetry
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            telemetry.fail_active(
                duration_millis(self.invocation_started.elapsed()),
                Some(&error),
            )
        };
        for telemetry in &terminal_events {
            observe_trace_telemetry(self.observer.as_ref(), self.request_id, telemetry);
        }
        let _ = self.sender.try_send(OutboundCommand::best_effort(command));
        observe_finished(
            self.observer.as_ref(),
            self.request_id,
            duration_millis(self.invocation_started.elapsed()),
            terminal,
            Some(error),
            None,
        );
        if !try_capture(
            self.capture_sender.as_ref(),
            GatewayCaptureEvent::InvocationFinished {
                config_epoch: self.config_epoch,
                request_id: self.request_id,
                terminal,
                output: None,
            },
        ) {
            observe_capture_dropped(
                self.observer.as_ref(),
                self.config_epoch,
                self.request_id,
                self.binding_id,
                &self.capability,
                &self.provider_key,
            );
        }
        let batch = self
            .telemetry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .finish();
        drop(self.invocation_permit);
        enqueue_telemetry_batch(
            &self.telemetry_backlog,
            PendingTelemetryBatch {
                request_id: self.request_id,
                batch,
            },
        );
        trigger_telemetry_flush(
            &self.telemetry_client,
            &self.telemetry_backlog,
            &self.telemetry_uploads,
            self.observer.as_ref(),
        );
    }
}

fn dispatch_preflight_failure(
    delivery: InvocationDelivery,
    command: AgentGatewayCommand,
    error: String,
    rejection_delivery_slots: &Arc<Semaphore>,
) -> Option<JoinHandle<()>> {
    match rejection_delivery_slots.clone().try_acquire_owned() {
        Ok(permit) => Some(tokio::spawn(async move {
            let _permit = permit;
            delivery
                .finish(
                    command,
                    GatewayInvocationTerminal::PreflightFailed,
                    Some(error),
                    None,
                )
                .await;
        })),
        Err(_) => {
            delivery.finish_shed(command, GatewayInvocationTerminal::PreflightFailed, error);
            None
        }
    }
}

async fn send_telemetry_batch(
    client: &RuntimeControlClient,
    pending: &PendingTelemetryBatch,
) -> Result<(), TraceObservationUploadError> {
    client
        .upload_trace_observations_classified(pending.request_id, &pending.batch)
        .await
}

fn enqueue_telemetry_batch(backlog: &TelemetryBacklog, pending: PendingTelemetryBatch) {
    let mut backlog = backlog
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if backlog.pending.len() == MAX_PENDING_TELEMETRY_BATCHES {
        backlog.pending.pop_front();
        backlog.overflow_drops = backlog.overflow_drops.saturating_add(1);
    }
    backlog.pending.push_back(pending);
}

fn record_permanent_telemetry_drop(
    backlog: &TelemetryBacklog,
    observer: Option<&GatewayRuntimeObserver>,
    request_id: Uuid,
    error: &TraceObservationUploadError,
) {
    let mut state = backlog
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    state.permanent_drops = state.permanent_drops.saturating_add(1);
    drop(state);
    if let Some(observer) = observer {
        observer(GatewayRuntimeEvent::ConnectionStatus {
            state: GatewayConnectionState::Degraded,
            message: Some(format!(
                "Trace telemetry for invocation {request_id} was rejected and dropped: {error}"
            )),
        });
    }
}

fn observe_finished(
    observer: Option<&GatewayRuntimeObserver>,
    request_id: Uuid,
    elapsed_ms: u64,
    terminal: GatewayInvocationTerminal,
    error: Option<String>,
    delivery_observation: Option<GatewayDeliveryObservation>,
) {
    if let Some(observer) = observer {
        observer(GatewayRuntimeEvent::InvocationFinished {
            request_id,
            elapsed_ms,
            terminal,
            error,
            delivery_observation,
        });
    }
}

fn safe_observer_error(value: &str) -> String {
    let lower = value.to_ascii_lowercase();
    if [
        "authorization",
        "bearer ",
        "basic ",
        "api key",
        "api_key",
        "apikey",
        "access token",
        "access_token",
        "secret",
        "token=",
        "token:",
        "password",
        "credential",
        "cookie",
        "session=",
        "session:",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
    {
        return "Provider failed; sensitive details were redacted".to_string();
    }
    sanitize_error(value)
}

async fn run_connection(
    websocket_url: &str,
    runtime: &AgentGatewayRuntime<'_>,
    session: &mut SessionSummary,
    persistence: &SessionPersistence,
    observers: ConnectionObservers<'_>,
    telemetry_backlog: &TelemetryBacklog,
    telemetry_uploads: &Arc<Semaphore>,
) -> Result<ConnectionOutcome, AgentGatewayConnectionError> {
    let request = websocket_url
        .into_client_request()
        .map_err(|error| error.to_string())?;
    let websocket_config = WebSocketConfig::default()
        .max_message_size(Some(gateway_frame::MAX_GATEWAY_FRAME_BYTES))
        .max_frame_size(Some(gateway_frame::MAX_GATEWAY_FRAME_BYTES));
    let (mut socket, _) = connect_async_with_config(request, Some(websocket_config), false)
        .await
        .map_err(|error| AgentGatewayConnectionError::Failed(error.to_string()))?;

    let challenge = tokio::time::timeout(Duration::from_secs(10), receive_command(&mut socket))
        .await
        .map_err(|_| "server did not send a Gateway challenge in time".to_string())??;
    let AgentGatewayCommand::Challenge {
        nonce,
        timestamp,
        audience,
    } = challenge
    else {
        return Err(
            "server must send a Gateway challenge after WebSocket upgrade"
                .to_string()
                .into(),
        );
    };
    let signed_at = unix_time_ms()?;
    let followup = runtime
        .enrollment_token
        .as_deref()
        .or(runtime.agent_gateway_bootstrap_token)
        .map(str::to_string)
        .or_else(|| {
            session
                .pairing
                .as_ref()
                .map(|pairing| pairing.request_id.to_string())
        });
    let signature_payload = protocol::gateway_signature_payload(
        &audience,
        &nonce,
        timestamp,
        signed_at,
        &session.identity.machine_id,
        followup.as_deref(),
        session.device_token.as_deref(),
    );
    let signature = session.identity.sign(&signature_payload)?;

    send_command(
        &mut socket,
        &AgentGatewayCommand::Hello {
            protocol: protocol::VERSION.to_string(),
            resume_session_id: session.resume_session_id,
            agents: runtime.agents.to_vec(),
            metadata: serde_json::json!({
                "adapter": "vifu",
                "features": [
                    "config-sync-v1",
                    "trace-upload-v1",
                    "embedded-runtime-v1",
                    protocol::APPLICATION_FEEDBACK_FEATURE,
                ],
                "providers": runtime.providers.iter().map(|provider| serde_json::json!({
                    "id": provider.id(),
                    "type": provider.provider_type(),
                })).collect::<Vec<_>>(),
                "version": env!("CARGO_PKG_VERSION")
            }),
            machine: protocol::GatewayMachineProof {
                id: session.identity.machine_id.clone(),
                public_key: session.identity.public_key.clone(),
                signature,
                signed_at,
            },
            auth: protocol::GatewayHelloAuth {
                device_token: session.device_token.clone(),
            },
            followup,
        },
    )
    .await?;

    let welcome = tokio::time::timeout(Duration::from_secs(10), receive_command(&mut socket))
        .await
        .map_err(|_| "server did not accept the agent gateway in time".to_string())??;
    if let AgentGatewayCommand::PairingRequired {
        request_id,
        auth_url,
        retryable: _,
        recommended_next_step: _,
        retry_after_ms,
    } = welcome
    {
        return Err(AgentGatewayConnectionError::PairingRequired {
            request_id,
            auth_url,
            retry_after: Duration::from_millis(retry_after_ms),
        });
    }
    let AgentGatewayCommand::Welcome {
        gateway_id,
        connection_id: _,
        session_id,
        heartbeat_interval_ms: _,
        resumed: _,
        auth,
    } = welcome
    else {
        return Err("server must send welcome after agent gateway hello"
            .to_string()
            .into());
    };
    if session
        .gateway_id
        .as_deref()
        .is_some_and(|stored| stored != gateway_id)
    {
        return Err("server authenticated a different agent gateway identity"
            .to_string()
            .into());
    }
    session.gateway_id = Some(gateway_id);
    if let Some(auth) = auth {
        session.device_token = Some(auth.device_token);
        session.token_generation = Some(auth.generation);
        session.token_expires_at = Some(auth.expires_at);
    }
    if session.device_token.is_none() {
        return Err("server accepted the Gateway without a Device Token"
            .to_string()
            .into());
    }
    session.pairing = None;
    if let Some(observer) = observers.pairing {
        observer(None);
    }
    session.resume_session_id = Some(session_id);
    persist_session(runtime, session, persistence)?;
    if let Some(observer) = observers.authorization {
        observer(&GatewayAuthorizationSummary {
            gateway_id: session.authorized_gateway_id()?.to_string(),
            device_token: session.device_token()?.to_string(),
            generation: session.token_generation.unwrap_or(1),
            expires_at: session.token_expires_at.clone().unwrap_or_default(),
        });
    }

    if runtime.allow_guest_bootstrap
        && runtime.enrollment_token.is_none()
        && runtime.agent_gateway_bootstrap_token.is_none()
        && session.guest_project.is_none()
    {
        let guest = RuntimeControlClient::bootstrap_guest_project(
            runtime.server_url,
            session.device_token()?,
        )
        .await?;
        apply_guest_project(
            runtime,
            session,
            &guest,
            persistence,
            observers.guest_project,
        )?;
    }
    let configuration_sync_error = sync_runtime_state(runtime, session)
        .await
        .err()
        .map(|error| {
            format!(
                "Runtime configuration sync is unavailable: {}",
                sanitize_error(&error)
            )
        });
    if let Some(message) = configuration_sync_error.as_deref() {
        terminal_stderr(runtime.output_policy, message);
    }
    let telemetry_client = RuntimeControlClient::new(runtime.server_url, session.device_token()?)?;
    trigger_telemetry_flush(
        &telemetry_client,
        telemetry_backlog,
        telemetry_uploads,
        runtime.runtime_observer.as_ref(),
    );
    terminal_stdout(runtime.output_policy, "\nStatus: connected");
    observe_connection_status(
        runtime,
        if configuration_sync_error.is_some() {
            GatewayConnectionState::Degraded
        } else {
            GatewayConnectionState::Connected
        },
        configuration_sync_error,
    );

    let (outbound_sender, mut outbound_receiver) =
        mpsc::channel::<OutboundCommand>(OUTBOUND_QUEUE_CAPACITY);
    let semaphore = std::sync::Arc::new(Semaphore::new(MAX_CONCURRENT_CALLS));
    let rejection_delivery_slots = Arc::new(Semaphore::new(MAX_CONCURRENT_REJECTION_DELIVERIES));
    let mut calls = HashMap::<Uuid, JoinHandle<()>>::new();
    let mut configuration_sync = tokio::time::interval(Duration::from_secs(30));
    configuration_sync.tick().await;

    let outcome = loop {
        reap_finished(&mut calls);
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break ConnectionOutcome::Shutdown,
            _ = configuration_sync.tick() => {
                match sync_runtime_state(runtime, session).await {
                    Ok(()) => observe_connection_status(
                        runtime,
                        GatewayConnectionState::Connected,
                        None,
                    ),
                    Err(error) => {
                        let message = format!(
                            "Runtime configuration sync is unavailable: {}",
                            sanitize_error(&error)
                        );
                        terminal_stderr(runtime.output_policy, &message);
                        observe_connection_status(
                            runtime,
                            GatewayConnectionState::Degraded,
                            Some(message),
                        );
                    }
                }
            }
            outbound = outbound_receiver.recv() => {
                let Some(outbound) = outbound else {
                    return Err("agent gateway output queue closed".to_string().into());
                };
                let result = send_command(&mut socket, &outbound.command).await;
                if let Some(delivery) = outbound.delivery {
                    let _ = delivery.send(result.is_ok());
                }
                result?;
            }
            incoming = receive_command(&mut socket) => {
                let incoming = match incoming {
                    Ok(message) => message,
                    Err(error) if error == "server disconnected" => break ConnectionOutcome::Disconnected,
                    Err(error) => return Err(error.into()),
                };
                match incoming {
                    AgentGatewayCommand::Invoke {
                        request_id,
                        channel_id,
                        endpoint_id,
                        profile_id,
                        binding_id,
                        agent_id,
                        binding,
                        input,
                        timeout_ms,
                    } => {
                        if calls.contains_key(&request_id) {
                            let _ = queue_error(
                                &outbound_sender,
                                Some(request_id),
                                Some(channel_id),
                                "DUPLICATE_REQUEST",
                                "The request id is already running.",
                            );
                            continue;
                        }
                        let invocation_started = std::time::Instant::now();
                        let capability = binding_text(&binding, "capability")
                            .unwrap_or("chat")
                            .to_string();
                        let route_key = binding_id.to_string();
                        let profile_key = profile_id.to_string();
                        let selected_provider = selected_provider_key(
                            &binding,
                            &route_key,
                            &profile_key,
                            &agent_id,
                            runtime.route_overrides.as_deref(),
                        )
                        .or_else(|| {
                            (runtime.providers.len() == 1)
                                .then(|| runtime.providers[0].id().to_string())
                        });
                        let provider = resolve_provider(
                            runtime.providers,
                            &binding,
                            &route_key,
                            &profile_key,
                            &agent_id,
                            runtime.route_overrides.as_deref(),
                        )
                        .cloned();
                        let provider_key = provider
                            .as_ref()
                            .map(|provider| provider.id().to_string())
                            .or(selected_provider)
                            .unwrap_or_else(|| "unavailable".to_string());
                        let model = provider_model(runtime, &provider_key, &capability);
                        let telemetry = Arc::new(Mutex::new(InvocationTelemetry::new(
                            provider_key.clone(),
                            capability.clone(),
                            model.clone(),
                        )));
                        let agent_name = binding_text(&binding, "agentName")
                            .or_else(|| binding_text(&binding, "profileName"))
                            .or_else(|| binding_text(&binding, "profileSlug"))
                            .unwrap_or(&agent_id)
                            .to_string();
                        let binding = Arc::new(binding);
                        let input = Arc::new(input);
                        telemetry
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .set_root_input_summary(canonical_trace_io_summary(&input));
                        if let Some(observer) = runtime.runtime_observer.as_ref() {
                            observer(GatewayRuntimeEvent::InvocationStarted {
                                request_id,
                                endpoint_id,
                                profile_id,
                                binding_id,
                                agent_id: agent_id.clone(),
                                agent_name,
                                provider_key: provider_key.clone(),
                                capability: capability.clone(),
                                model: model.clone(),
                                model_parameters: trace_model_parameters(&binding),
                                timeout_ms,
                                started_unix_ms: unix_time_ms()?,
                            });
                        }
                        if !try_capture(
                            runtime.capture_sender.as_ref(),
                            GatewayCaptureEvent::InvocationStarted {
                                config_epoch: runtime.config_epoch,
                                request_id,
                                binding_id,
                                agent_id: agent_id.clone(),
                                provider_key: provider_key.clone(),
                                capability: capability.clone(),
                                binding: Arc::clone(&binding),
                                input: Arc::clone(&input),
                                timeout_ms,
                            },
                        ) {
                            observe_capture_dropped(
                                runtime.runtime_observer.as_ref(),
                                runtime.config_epoch,
                                request_id,
                                binding_id,
                                &capability,
                                &provider_key,
                            );
                        }
                        let permit = match semaphore.clone().try_acquire_owned() {
                            Ok(permit) => permit,
                            Err(_) => {
                                let error = "The agent gateway has reached its concurrent call limit.";
                                let delivery = InvocationDelivery {
                                    sender: outbound_sender.clone(),
                                    observer: runtime.runtime_observer.clone(),
                                    capture_sender: runtime.capture_sender.clone(),
                                    config_epoch: runtime.config_epoch,
                                    request_id,
                                    binding_id,
                                    capability: capability.clone(),
                                    provider_key: provider_key.clone(),
                                    invocation_started,
                                    telemetry: Arc::clone(&telemetry),
                                    telemetry_backlog: Arc::clone(telemetry_backlog),
                                    telemetry_client: telemetry_client.clone(),
                                    telemetry_uploads: Arc::clone(telemetry_uploads),
                                    invocation_permit: None,
                                };
                                if let Some(handle) = dispatch_preflight_failure(
                                    delivery,
                                    agent_gateway_error(
                                        request_id,
                                        channel_id,
                                        "BACKPRESSURE",
                                        error,
                                    ),
                                    error.to_string(),
                                    &rejection_delivery_slots,
                                ) {
                                    calls.insert(request_id, handle);
                                }
                                continue;
                            }
                        };
                        let Some(provider) = provider else {
                            let error = "The requested provider is not connected to this Agent Gateway.";
                            let delivery = InvocationDelivery {
                                sender: outbound_sender.clone(),
                                observer: runtime.runtime_observer.clone(),
                                capture_sender: runtime.capture_sender.clone(),
                                config_epoch: runtime.config_epoch,
                                request_id,
                                binding_id,
                                capability: capability.clone(),
                                provider_key: provider_key.clone(),
                                invocation_started,
                                telemetry: Arc::clone(&telemetry),
                                telemetry_backlog: Arc::clone(telemetry_backlog),
                                telemetry_client: telemetry_client.clone(),
                                telemetry_uploads: Arc::clone(telemetry_uploads),
                                invocation_permit: Some(permit),
                            };
                            let handle = tokio::spawn(async move {
                                delivery
                                    .finish(
                                        agent_gateway_error(
                                            request_id,
                                            channel_id,
                                            "PROVIDER_NOT_AVAILABLE",
                                            error,
                                        ),
                                        GatewayInvocationTerminal::PreflightFailed,
                                        Some(error.to_string()),
                                        None,
                                    )
                                    .await;
                            });
                            calls.insert(request_id, handle);
                            continue;
                        };
                        let sender = outbound_sender.clone();
                        let observer = runtime.runtime_observer.clone();
                        let capture_sender = runtime.capture_sender.clone();
                        let config_epoch = runtime.config_epoch;
                        let capture_capability = capability.clone();
                        let capture_provider_key = provider_key.clone();
                        let provider_telemetry = Arc::clone(&telemetry);
                        let delivery_telemetry = Arc::clone(&telemetry);
                        let delivery_telemetry_backlog = Arc::clone(telemetry_backlog);
                        let delivery_telemetry_client = telemetry_client.clone();
                        let delivery_telemetry_uploads = Arc::clone(telemetry_uploads);
                        let handle = tokio::spawn(async move {
                            let provider_observer = observer.clone();
                            let events = ProviderEventSink::from_fn(move |event| {
                                if matches!(&event, ProviderEvent::OutputDelta { .. }) {
                                    return;
                                }
                                let request_elapsed_ms =
                                    duration_millis(invocation_started.elapsed());
                                let telemetry = safe_trace_telemetry(
                                    event,
                                    request_elapsed_ms,
                                    &mut provider_telemetry
                                        .lock()
                                        .unwrap_or_else(|poisoned| poisoned.into_inner()),
                                );
                                if let Some(telemetry) = telemetry {
                                    observe_trace_telemetry(
                                        provider_observer.as_ref(),
                                        request_id,
                                        &telemetry,
                                    );
                                }
                            });
                            let provider_timeout = Duration::from_millis(timeout_ms.max(1));
                            let result = tokio::time::timeout(
                                provider_timeout,
                                provider.invoke_with_events(
                                    &agent_id,
                                    &binding,
                                    &input,
                                    provider_timeout,
                                    events,
                                ),
                            )
                            .await
                            .unwrap_or_else(|_| {
                                Err(GatewayProviderError::timed_out(format!(
                                    "provider timed out after {}ms",
                                    provider_timeout.as_millis()
                                )))
                            });
                            let (message, provider_terminal, error, output) = match result {
                                Ok(output) => {
                                    let output = Arc::new(output);
                                    (
                                        AgentGatewayCommand::Result {
                                            request_id,
                                            channel_id,
                                            output: output.as_ref().clone(),
                                        },
                                        GatewayInvocationTerminal::Delivered,
                                        None,
                                        Some(output),
                                    )
                                }
                                Err(error) => {
                                    let terminal = match error.kind {
                                        GatewayProviderErrorKind::Failed => {
                                            GatewayInvocationTerminal::ProviderFailed
                                        }
                                        GatewayProviderErrorKind::TimedOut => {
                                            GatewayInvocationTerminal::TimedOut
                                        }
                                    };
                                    let public_error = safe_observer_error(&error.message);
                                    (
                                        agent_gateway_error(
                                            request_id,
                                            channel_id,
                                            "PROVIDER_ERROR",
                                            &error.message,
                                        ),
                                        terminal,
                                        Some(public_error),
                                        None,
                                    )
                                }
                            };
                            InvocationDelivery {
                                sender,
                                observer,
                                capture_sender,
                                config_epoch,
                                request_id,
                                binding_id,
                                capability: capture_capability,
                                provider_key: capture_provider_key,
                                invocation_started,
                                telemetry: delivery_telemetry,
                                telemetry_backlog: delivery_telemetry_backlog,
                                telemetry_client: delivery_telemetry_client,
                                telemetry_uploads: delivery_telemetry_uploads,
                                invocation_permit: Some(permit),
                            }
                            .finish(message, provider_terminal, error, output)
                            .await;
                        });
                        calls.insert(request_id, handle);
                    }
                    AgentGatewayCommand::Cancel { request_id, .. } => {
                        if let Some(call) = calls.remove(&request_id) {
                            call.abort();
                            let _ = try_capture(
                                runtime.capture_sender.as_ref(),
                                GatewayCaptureEvent::InvocationCancelled {
                                    config_epoch: runtime.config_epoch,
                                    request_id,
                                },
                            );
                            if let Some(observer) = runtime.runtime_observer.as_ref() {
                                observer(GatewayRuntimeEvent::InvocationCancelled {
                                    config_epoch: runtime.config_epoch,
                                    request_id,
                                });
                            }
                        }
                    }
                    AgentGatewayCommand::ApplicationFeedback {
                        request_id,
                        observation_id,
                        start_offset_ms,
                        end_offset_ms,
                        feedback,
                    } => {
                        if let Some(observer) = runtime.runtime_observer.as_ref() {
                            observer(GatewayRuntimeEvent::ApplicationFeedback {
                                request_id,
                                observation_id,
                                start_offset_ms,
                                end_offset_ms,
                                feedback,
                            });
                        }
                    }
                    AgentGatewayCommand::Heartbeat { session_id: received } => {
                        if received != session_id {
                            return Err("server heartbeat session does not match".to_string().into());
                        }
                        send_command(
                            &mut socket,
                            &AgentGatewayCommand::HeartbeatAck { session_id },
                        )
                        .await?;
                    }
                    AgentGatewayCommand::RuntimeConfigChanged { .. } => {
                        match sync_runtime_state(runtime, session).await {
                            Ok(()) => observe_connection_status(
                                runtime,
                                GatewayConnectionState::Connected,
                                None,
                            ),
                            Err(error) => {
                                let message = format!(
                                    "Runtime configuration sync is unavailable: {}",
                                    sanitize_error(&error)
                                );
                                terminal_stderr(runtime.output_policy, &message);
                                observe_connection_status(
                                    runtime,
                                    GatewayConnectionState::Degraded,
                                    Some(message),
                                );
                            }
                        }
                    }
                    AgentGatewayCommand::Error {
                        request_id: None,
                        code,
                        message,
                        ..
                    } if code == "SESSION_REPLACED" => {
                        let message = format!(
                            "Agent Gateway session replaced: {}",
                            sanitize_error(&message)
                        );
                        terminal_stderr(runtime.output_policy, &message);
                        observe_connection_status(
                            runtime,
                            GatewayConnectionState::Reconnecting,
                            Some(message),
                        );
                        break ConnectionOutcome::Disconnected;
                    }
                    AgentGatewayCommand::Error {
                        request_id: None,
                        code,
                        ..
                    } if code == "CREDENTIAL_REVOKED" => {
                        break ConnectionOutcome::Disconnected;
                    }
                    AgentGatewayCommand::Error {
                        request_id: None,
                        message,
                        ..
                    } => {
                        return Err(format!(
                            "server rejected agent gateway: {}",
                            sanitize_error(&message)
                        )
                        .into())
                    }
                    _ => {
                        return Err(
                            "server sent an unexpected agent gateway message"
                                .to_string()
                                .into(),
                        )
                    }
                }
            }
        }
    };

    for (_, call) in calls {
        call.abort();
    }
    let _ = socket.close(None).await;
    Ok(outcome)
}

fn trigger_telemetry_flush(
    client: &RuntimeControlClient,
    backlog: &TelemetryBacklog,
    telemetry_uploads: &Arc<Semaphore>,
    observer: Option<&GatewayRuntimeObserver>,
) {
    let should_spawn = {
        let mut state = backlog
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.flushing || state.pending.is_empty() {
            false
        } else {
            state.flushing = true;
            true
        }
    };
    if !should_spawn {
        return;
    }
    let client = client.clone();
    let backlog = Arc::clone(backlog);
    let telemetry_uploads = Arc::clone(telemetry_uploads);
    let observer = observer.cloned();
    tokio::spawn(async move {
        flush_telemetry_backlog(client, backlog, telemetry_uploads, observer).await;
    });
}

async fn flush_telemetry_backlog(
    client: RuntimeControlClient,
    backlog: TelemetryBacklog,
    telemetry_uploads: Arc<Semaphore>,
    observer: Option<GatewayRuntimeObserver>,
) {
    let mut retry_delay = TELEMETRY_RETRY_INITIAL_DELAY;
    loop {
        let Ok(telemetry_permit) = telemetry_uploads.clone().acquire_owned().await else {
            backlog
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .flushing = false;
            return;
        };
        let Some(batch) = ({
            let mut state = backlog
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match state.pending.pop_front() {
                Some(batch) => Some(batch),
                None => {
                    state.flushing = false;
                    None
                }
            }
        }) else {
            drop(telemetry_permit);
            return;
        };
        let result = send_telemetry_batch(&client, &batch).await;
        drop(telemetry_permit);
        if handle_telemetry_flush_result(&backlog, observer.as_ref(), batch, result) {
            retry_delay = TELEMETRY_RETRY_INITIAL_DELAY;
        } else {
            tokio::time::sleep(retry_delay).await;
            retry_delay = retry_delay.saturating_mul(2).min(TELEMETRY_RETRY_MAX_DELAY);
        }
    }
}

fn handle_telemetry_flush_result(
    backlog: &TelemetryBacklog,
    observer: Option<&GatewayRuntimeObserver>,
    batch: PendingTelemetryBatch,
    result: Result<(), TraceObservationUploadError>,
) -> bool {
    match result {
        Ok(()) => true,
        Err(error) if error.is_retryable() => {
            restore_retryable_telemetry(backlog, batch);
            false
        }
        Err(error) => {
            record_permanent_telemetry_drop(backlog, observer, batch.request_id, &error);
            true
        }
    }
}

fn restore_retryable_telemetry(backlog: &TelemetryBacklog, batch: PendingTelemetryBatch) {
    let mut state = backlog
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if state.pending.len() == MAX_PENDING_TELEMETRY_BATCHES {
        state.overflow_drops = state.overflow_drops.saturating_add(1);
    } else {
        state.pending.push_front(batch);
    }
}

fn persist_session(
    runtime: &AgentGatewayRuntime<'_>,
    session: &SessionSummary,
    persistence: &SessionPersistence,
) -> Result<(), String> {
    match persistence {
        SessionPersistence::LegacyFile => match runtime.session_path {
            Some(path) => session::write_session(path, session),
            None => Ok(()),
        },
        #[cfg(feature = "sqlite")]
        SessionPersistence::Sqlite(persistence) => persistence.save(session),
    }
}

#[cfg(feature = "sqlite")]
async fn sync_runtime_state(
    runtime: &AgentGatewayRuntime<'_>,
    session: &SessionSummary,
) -> Result<(), String> {
    let client = RuntimeControlClient::new(runtime.server_url, session.device_token()?)?;
    let configuration = client.configuration().await?;
    if configuration.gateway_id != session.authorized_gateway_id()? {
        return Err("server returned configuration for another Agent Gateway".to_string());
    }
    let store = SqliteRuntimeStore::open(runtime.runtime_database_path)
        .map_err(|error| error.to_string())?;
    for mut deployment in configuration.deployments {
        if deployment.policies.config_sync {
            if deployment.release.is_none() {
                if let Some(embedded) = runtime
                    .embedded_runtime
                    .filter(|embedded| embedded.project_id() == deployment.project_slug)
                {
                    if let Some(manifest) = embedded
                        .current_manifest()
                        .map_err(|error| error.to_string())?
                    {
                        deployment.release = Some(
                            client
                                .bootstrap_runtime_release(deployment.deployment_id, &manifest)
                                .await?,
                        );
                    }
                }
            }
            if let Some(release) = deployment.release.as_ref() {
                release.validate().map_err(|error| error.to_string())?;
                store
                    .save_release(release)
                    .map_err(|error| error.to_string())?;
                store
                    .set_active_release(&deployment.project_slug, release.version)
                    .map_err(|error| error.to_string())?;
                if let Some(embedded) = runtime
                    .embedded_runtime
                    .filter(|embedded| embedded.project_id() == deployment.project_slug)
                {
                    embedded
                        .install_release(release)
                        .map_err(|error| error.to_string())?;
                    embedded
                        .activate_release(release.version)
                        .map_err(|error| error.to_string())?;
                }
            }
        }
        if deployment.policies.trace_mode == "off" {
            continue;
        }
        let traces = store
            .pending_traces(1_000)
            .map_err(|error| error.to_string())?
            .into_iter()
            .filter(|trace| trace.project_id == deployment.project_slug)
            .collect::<Vec<_>>();
        for batch in traces.chunks(100) {
            let acknowledged = client
                .upload_traces(deployment.deployment_id, batch)
                .await?;
            store
                .acknowledge_traces(&acknowledged)
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

fn unix_time_ms() -> Result<u64, String> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system clock is before the Unix epoch".to_string())?
        .as_millis();
    u64::try_from(millis).map_err(|_| "system clock timestamp is too large".to_string())
}

fn duration_millis(duration: Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}

#[cfg(not(feature = "sqlite"))]
async fn sync_runtime_state(
    _runtime: &AgentGatewayRuntime<'_>,
    _session: &SessionSummary,
) -> Result<(), String> {
    Ok(())
}

fn resolve_provider<'a>(
    providers: &'a [Arc<dyn AgentGatewayProvider>],
    binding: &serde_json::Value,
    route_key: &str,
    profile_key: &str,
    agent_id: &str,
    route_overrides: Option<&SessionRouteOverrides>,
) -> Option<&'a Arc<dyn AgentGatewayProvider>> {
    let provider_key =
        selected_provider_key(binding, route_key, profile_key, agent_id, route_overrides);
    match provider_key.as_deref() {
        Some(provider_key) => providers
            .iter()
            .find(|provider| provider.id() == provider_key),
        None if providers.len() == 1 => providers.first(),
        None => None,
    }
}

fn selected_provider_key(
    binding: &serde_json::Value,
    route_key: &str,
    profile_key: &str,
    agent_id: &str,
    route_overrides: Option<&SessionRouteOverrides>,
) -> Option<String> {
    let override_provider_key = route_overrides.and_then(|routes| {
        routes
            .provider_for(route_key)
            .or_else(|| routes.provider_for(profile_key))
            .or_else(|| routes.provider_for(agent_id))
    });
    let binding_provider_key = binding
        .get("providerKey")
        .or_else(|| binding.pointer("/source/providerKey"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    override_provider_key.or_else(|| binding_provider_key.map(str::to_string))
}

pub fn agent_gateway_websocket_url(server_url: &str) -> Result<String, String> {
    let mut url = Url::parse(server_url.trim())
        .map_err(|_| "gateway.serverUrl must be a valid HTTP or HTTPS URL".to_string())?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(
            "gateway.serverUrl must not include credentials, a query, or a fragment".to_string(),
        );
    }
    let websocket_scheme = match url.scheme() {
        "http" if is_local_plaintext_server(&url) => "ws",
        "http" => {
            return Err(
                "Remote gateway.serverUrl values must use https so agent gateway credentials are encrypted"
                    .to_string(),
            );
        }
        "https" => "wss",
        _ => return Err("gateway.serverUrl must use http or https".to_string()),
    };
    url.set_scheme(websocket_scheme)
        .map_err(|_| "could not build agent gateway WebSocket URL".to_string())?;
    let base_path = url.path().trim_end_matches('/');
    let agent_gateway_path = if base_path.is_empty() {
        "/v1/agent-gateway/connect".to_string()
    } else {
        format!("{base_path}/v1/agent-gateway/connect")
    };
    url.set_path(&agent_gateway_path);
    Ok(url.to_string())
}

fn is_local_plaintext_server(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
        || is_single_label_hostname(host)
}

fn is_single_label_hostname(host: &str) -> bool {
    !host.is_empty()
        && !host.contains('.')
        && host
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
}

async fn receive_command<S>(
    socket: &mut tokio_tungstenite::WebSocketStream<S>,
) -> Result<AgentGatewayCommand, String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    loop {
        match socket.next().await {
            Some(Ok(Message::Text(frame))) => return decode_command(frame.as_str()),
            Some(Ok(Message::Ping(payload))) => socket
                .send(Message::Pong(payload))
                .await
                .map_err(|error| error.to_string())?,
            Some(Ok(Message::Pong(_))) | Some(Ok(Message::Frame(_))) => continue,
            Some(Ok(Message::Close(_))) | None => return Err("server disconnected".to_string()),
            Some(Ok(Message::Binary(_))) => {
                return Err("binary agent gateway messages are not supported".to_string());
            }
            Some(Err(error)) => return Err(error.to_string()),
        }
    }
}

async fn send_command<S>(
    socket: &mut tokio_tungstenite::WebSocketStream<S>,
    message: &AgentGatewayCommand,
) -> Result<(), String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    socket
        .send(Message::Text(encode_command(message)?.into()))
        .await
        .map_err(|error| error.to_string())
}

fn decode_command(source: &str) -> Result<AgentGatewayCommand, String> {
    let frame = gateway_frame::decode(source)?;
    protocol::from_gateway_frame(frame)
}

fn encode_command(message: &AgentGatewayCommand) -> Result<String, String> {
    let frame = protocol::to_gateway_frame(message)?;
    gateway_frame::encode(&frame)
}

fn queue_error(
    sender: &mpsc::Sender<OutboundCommand>,
    request_id: Option<Uuid>,
    channel_id: Option<u64>,
    code: &str,
    message: &str,
) -> bool {
    sender
        .try_send(OutboundCommand::best_effort(AgentGatewayCommand::Error {
            request_id,
            channel_id,
            code: code.to_string(),
            message: message.to_string(),
        }))
        .is_ok()
}

fn agent_gateway_error(
    request_id: Uuid,
    channel_id: u64,
    code: &str,
    message: &str,
) -> AgentGatewayCommand {
    AgentGatewayCommand::Error {
        request_id: Some(request_id),
        channel_id: Some(channel_id),
        code: code.to_string(),
        message: sanitize_error(message),
    }
}

fn reap_finished(calls: &mut HashMap<Uuid, JoinHandle<()>>) {
    calls.retain(|_, call| !call.is_finished());
}

fn sanitize_error(value: &str) -> String {
    let output = value
        .chars()
        .take(512)
        .map(|character| {
            if character.is_control() && character != '\n' {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    if output.trim().is_empty() {
        "unknown error".to_string()
    } else {
        output
    }
}

#[cfg(test)]
mod tests {
    use base64::Engine;
    use serde_json::json;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use url::Url;
    use uuid::Uuid;

    use super::{
        agent_gateway_error, agent_gateway_websocket_url, decode_command,
        dispatch_preflight_failure, encode_command, enqueue_telemetry_batch, guest_claim_url,
        handle_telemetry_flush_result, observe_capture_dropped, queue_error, resolve_provider,
        safe_observer_error, safe_trace_telemetry, sanitize_error, trigger_telemetry_flush,
        try_capture, write_terminal_line, AgentGatewayProvider, GatewayCaptureEvent,
        GatewayInvocationTerminal, GatewayOutputPolicy, GatewayRuntimeEvent,
        InProcessGatewayProvider, InvocationDelivery, InvocationTelemetry, OpenClawGatewayProvider,
        PendingTelemetryBatch, RuntimeControlClient, SessionRouteOverrides, TelemetryBacklogState,
        MAX_PENDING_TELEMETRY_BATCHES,
    };
    use crate::control::TraceObservationUploadError;
    use crate::gateway_frame;
    use crate::openclaw::Endpoint;
    use crate::protocol::{
        AgentGatewayCommand, TraceStageStatus, TraceTelemetry, TraceTelemetryBatch,
        AGENT_GATEWAY_HEARTBEAT_EVENT, AGENT_GATEWAY_HELLO_METHOD, AGENT_GATEWAY_HELLO_REQUEST_ID,
        VERSION,
    };
    use vifu_runtime::{
        AgentProvider, CancellationToken, InvocationData, ProviderEvent, ProviderFuture,
        ProviderRequest, ProviderResponse,
    };

    struct PersonaProvider;

    fn pending_telemetry(request_id: Uuid) -> PendingTelemetryBatch {
        PendingTelemetryBatch {
            request_id,
            batch: TraceTelemetryBatch {
                events: vec![TraceTelemetry::InvocationStarted {
                    provider_key: "local-provider".to_string(),
                    capability: "chat".to_string(),
                    model: Some("local-model".to_string()),
                }],
                dropped_events: 0,
                root_input_summary: None,
                root_output_summary: None,
            },
        }
    }

    async fn read_http_json(stream: &mut tokio::net::TcpStream) -> serde_json::Value {
        let mut request = Vec::new();
        let mut chunk = [0_u8; 4096];
        loop {
            let read = stream.read(&mut chunk).await.unwrap();
            assert!(read > 0, "HTTP request ended before its JSON body arrived");
            request.extend_from_slice(&chunk[..read]);
            let Some(header_end) = request
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .map(|index| index + 4)
            else {
                continue;
            };
            let headers = std::str::from_utf8(&request[..header_end]).unwrap();
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .expect("telemetry request should declare its content length");
            if request.len() >= header_end + content_length {
                return serde_json::from_slice(&request[header_end..header_end + content_length])
                    .unwrap();
            }
        }
    }

    #[tokio::test]
    async fn telemetry_worker_drains_burst_and_retries_without_a_new_invocation() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let mut attempts = Vec::new();
            for attempt in 0..10 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let request = read_http_json(&mut stream).await;
                let request_id = request["requestId"].as_str().unwrap().to_string();
                attempts.push(request_id.clone());
                let (status, body) = if attempt == 0 {
                    ("503 Service Unavailable", String::new())
                } else {
                    (
                        "200 OK",
                        json!({ "acceptedRequestId": request_id }).to_string(),
                    )
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await.unwrap();
                stream.shutdown().await.unwrap();
            }
            attempts
        });

        let backlog = Arc::new(std::sync::Mutex::new(TelemetryBacklogState::default()));
        let telemetry_uploads = Arc::new(tokio::sync::Semaphore::new(4));
        let mut occupied_upload_slots = Vec::new();
        for _ in 0..4 {
            occupied_upload_slots.push(telemetry_uploads.clone().acquire_owned().await.unwrap());
        }
        let request_ids = (0..9).map(|_| Uuid::new_v4()).collect::<Vec<_>>();
        for request_id in &request_ids {
            enqueue_telemetry_batch(&backlog, pending_telemetry(*request_id));
        }
        let client =
            RuntimeControlClient::new(&format!("http://{address}"), "device-token").unwrap();

        trigger_telemetry_flush(&client, &backlog, &telemetry_uploads, None);
        trigger_telemetry_flush(&client, &backlog, &telemetry_uploads, None);
        {
            let state = backlog.lock().unwrap();
            assert_eq!(state.pending.len(), request_ids.len());
            assert!(state.flushing);
        }
        drop(occupied_upload_slots);

        let attempts = tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .expect("single telemetry worker should drain without a reconnect")
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let drained = {
                    let state = backlog.lock().unwrap();
                    state.pending.is_empty() && !state.flushing
                };
                if drained {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("telemetry worker should mark the backlog drained");

        assert_eq!(attempts.len(), request_ids.len() + 1);
        assert_eq!(attempts[0], request_ids[0].to_string());
        assert_eq!(attempts[1], attempts[0]);
        assert_eq!(
            attempts[1..],
            request_ids.iter().map(Uuid::to_string).collect::<Vec<_>>()
        );
    }

    #[test]
    fn permanent_telemetry_failure_drops_poisoned_batch_and_continues() {
        let backlog = Arc::new(std::sync::Mutex::new(TelemetryBacklogState {
            flushing: true,
            ..TelemetryBacklogState::default()
        }));
        let first = pending_telemetry(Uuid::new_v4());
        let second = pending_telemetry(Uuid::new_v4());

        assert!(handle_telemetry_flush_result(
            &backlog,
            None,
            first,
            Err(TraceObservationUploadError::Permanent(
                "HTTP 400".to_string()
            )),
        ));
        assert!(handle_telemetry_flush_result(
            &backlog,
            None,
            second,
            Ok(()),
        ));
        let state = backlog.lock().unwrap();
        assert_eq!(state.permanent_drops, 1);
        assert!(state.pending.is_empty());
    }

    #[test]
    fn retryable_telemetry_failures_restore_a_bounded_queue() {
        let backlog = Arc::new(std::sync::Mutex::new(TelemetryBacklogState {
            pending: (0..MAX_PENDING_TELEMETRY_BATCHES)
                .map(|_| pending_telemetry(Uuid::new_v4()))
                .collect(),
            flushing: true,
            ..TelemetryBacklogState::default()
        }));
        let retry = pending_telemetry(Uuid::new_v4());

        assert!(!handle_telemetry_flush_result(
            &backlog,
            None,
            retry,
            Err(TraceObservationUploadError::Retryable(
                "HTTP 503".to_string()
            )),
        ));
        let state = backlog.lock().unwrap();
        assert_eq!(state.pending.len(), MAX_PENDING_TELEMETRY_BATCHES);
        assert_eq!(state.overflow_drops, 1);
        assert!(state.flushing);
    }

    #[tokio::test]
    async fn saturated_telemetry_slots_enqueue_after_result_delivery_releases_invocation_slot() {
        let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
        let backlog = Arc::new(std::sync::Mutex::new(TelemetryBacklogState::default()));
        let invocation_slots = Arc::new(tokio::sync::Semaphore::new(1));
        let invocation_permit = invocation_slots
            .clone()
            .try_acquire_owned()
            .expect("invocation slot should be available");
        let request_id = Uuid::new_v4();
        let delivery = InvocationDelivery {
            sender,
            observer: None,
            capture_sender: None,
            config_epoch: 1,
            request_id,
            binding_id: Uuid::new_v4(),
            capability: "chat".to_string(),
            provider_key: "local-provider".to_string(),
            invocation_started: std::time::Instant::now(),
            telemetry: Arc::new(std::sync::Mutex::new(InvocationTelemetry::new(
                "local-provider".to_string(),
                "chat".to_string(),
                Some("local-model".to_string()),
            ))),
            telemetry_backlog: Arc::clone(&backlog),
            telemetry_client: RuntimeControlClient::new("http://127.0.0.1:1", "test-device-token")
                .unwrap(),
            telemetry_uploads: Arc::new(tokio::sync::Semaphore::new(0)),
            invocation_permit: Some(invocation_permit),
        };
        let finish = tokio::spawn(async move {
            delivery
                .finish(
                    AgentGatewayCommand::Error {
                        request_id: Some(request_id),
                        channel_id: Some(7),
                        code: "PROVIDER_ERROR".to_string(),
                        message: "provider unavailable".to_string(),
                    },
                    GatewayInvocationTerminal::ProviderFailed,
                    Some("provider unavailable".to_string()),
                    None,
                )
                .await;
        });

        let outbound = receiver
            .recv()
            .await
            .expect("result should reach the bounded outbound queue");
        assert_eq!(invocation_slots.available_permits(), 0);
        outbound
            .delivery
            .expect("result delivery should be tracked")
            .send(true)
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), finish)
            .await
            .expect("saturated telemetry must enqueue without waiting")
            .unwrap();

        assert_eq!(invocation_slots.available_permits(), 1);
        assert_eq!(backlog.lock().unwrap().pending.len(), 1);
    }

    #[tokio::test]
    async fn saturated_rejection_delivery_slots_shed_without_spawning_more_tasks() {
        let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
        let backlog = Arc::new(std::sync::Mutex::new(TelemetryBacklogState::default()));
        let telemetry_uploads = Arc::new(tokio::sync::Semaphore::new(0));
        let rejection_slots = Arc::new(tokio::sync::Semaphore::new(1));
        let make_delivery = |request_id| InvocationDelivery {
            sender: sender.clone(),
            observer: None,
            capture_sender: None,
            config_epoch: 1,
            request_id,
            binding_id: Uuid::new_v4(),
            capability: "chat".to_string(),
            provider_key: "local-provider".to_string(),
            invocation_started: std::time::Instant::now(),
            telemetry: Arc::new(std::sync::Mutex::new(InvocationTelemetry::new(
                "local-provider".to_string(),
                "chat".to_string(),
                Some("local-model".to_string()),
            ))),
            telemetry_backlog: Arc::clone(&backlog),
            telemetry_client: RuntimeControlClient::new("http://127.0.0.1:1", "test-device-token")
                .unwrap(),
            telemetry_uploads: Arc::clone(&telemetry_uploads),
            invocation_permit: None,
        };
        let first_request = Uuid::new_v4();
        let first = dispatch_preflight_failure(
            make_delivery(first_request),
            agent_gateway_error(first_request, 1, "BACKPRESSURE", "busy"),
            "busy".to_string(),
            &rejection_slots,
        )
        .expect("first rejection should use the only delivery slot");
        let first_outbound = receiver
            .recv()
            .await
            .expect("first rejection should enter the outbound queue");
        assert_eq!(rejection_slots.available_permits(), 0);

        let second_request = Uuid::new_v4();
        assert!(dispatch_preflight_failure(
            make_delivery(second_request),
            agent_gateway_error(second_request, 2, "BACKPRESSURE", "busy"),
            "busy".to_string(),
            &rejection_slots,
        )
        .is_none());
        let shed_outbound = receiver
            .recv()
            .await
            .expect("shed rejection should use a nonblocking best-effort send");
        assert!(shed_outbound.delivery.is_none());
        assert_eq!(backlog.lock().unwrap().pending.len(), 1);

        first_outbound
            .delivery
            .expect("bounded rejection delivery should be tracked")
            .send(true)
            .unwrap();
        first.await.unwrap();
        assert_eq!(rejection_slots.available_permits(), 1);
        assert_eq!(backlog.lock().unwrap().pending.len(), 2);
    }

    #[test]
    fn saturated_outbound_queue_drops_duplicate_error_without_waiting() {
        let (sender, _receiver) = tokio::sync::mpsc::channel(1);
        sender
            .try_send(super::OutboundCommand::best_effort(
                AgentGatewayCommand::Heartbeat {
                    session_id: Uuid::new_v4(),
                },
            ))
            .unwrap();

        assert!(!queue_error(
            &sender,
            Some(Uuid::new_v4()),
            Some(1),
            "DUPLICATE_REQUEST",
            "request is already running",
        ));
    }

    impl AgentProvider for PersonaProvider {
        fn supports(&self, capability: &str) -> bool {
            capability == "chat"
        }

        fn invoke<'a>(
            &'a self,
            request: ProviderRequest,
            _cancellation: CancellationToken,
        ) -> ProviderFuture<'a> {
            Box::pin(async move { Ok(ProviderResponse::json(request.agent.metadata)) })
        }
    }

    struct EmbeddingProvider;

    impl AgentProvider for EmbeddingProvider {
        fn supports(&self, capability: &str) -> bool {
            capability == "embedding"
        }

        fn invoke<'a>(
            &'a self,
            request: ProviderRequest,
            _cancellation: CancellationToken,
        ) -> ProviderFuture<'a> {
            Box::pin(async move {
                Ok(ProviderResponse::json(json!({
                    "capability": request.capability,
                })))
            })
        }
    }

    struct BinaryTranscriptionProvider;

    #[test]
    fn bounded_capture_queue_reports_saturation_without_forwarding_payloads() {
        let (sender, _receiver) = tokio::sync::mpsc::channel(1);
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        assert!(try_capture(
            Some(&sender),
            GatewayCaptureEvent::InvocationCancelled {
                config_epoch: 7,
                request_id: first,
            },
        ));
        assert!(!try_capture(
            Some(&sender),
            GatewayCaptureEvent::InvocationCancelled {
                config_epoch: 7,
                request_id: second,
            },
        ));

        let observed = Arc::new(std::sync::Mutex::new(Vec::new()));
        let observed_for_callback = Arc::clone(&observed);
        let observer: super::GatewayRuntimeObserver = Arc::new(move |event| {
            observed_for_callback.lock().unwrap().push(event);
        });
        let binding_id = Uuid::new_v4();
        observe_capture_dropped(Some(&observer), 7, second, binding_id, "chat", "local-qwen");

        let events = observed.lock().unwrap();
        assert!(matches!(
            events.as_slice(),
            [GatewayRuntimeEvent::CaptureDropped {
                config_epoch: 7,
                request_id,
                binding_id: received_binding,
                capability,
                provider_key,
            }] if *request_id == second
                && *received_binding == binding_id
                && capability == "chat"
                && provider_key == "local-qwen"
        ));
    }

    #[test]
    fn provider_stage_keeps_one_observation_id_and_real_request_offsets() {
        let mut recorder = InvocationTelemetry::new(
            "local-qwen".to_string(),
            "chat".to_string(),
            Some("qwen".to_string()),
        );
        let started = safe_trace_telemetry(
            ProviderEvent::StageStarted {
                stage: vifu_runtime::ProviderStage::Decode,
                metadata: json!({}),
            },
            10,
            &mut recorder,
        )
        .unwrap();
        let completed = safe_trace_telemetry(
            ProviderEvent::StageCompleted {
                stage: vifu_runtime::ProviderStage::Decode,
                elapsed_ms: 35,
                metadata: json!({"outputTokens": 7}),
            },
            45,
            &mut recorder,
        )
        .unwrap();

        let TraceTelemetry::ProviderStage {
            observation_id: started_id,
            start_offset_ms: started_offset,
            end_offset_ms: None,
            ..
        } = started
        else {
            panic!("expected started provider observation");
        };
        let TraceTelemetry::ProviderStage {
            observation_id: completed_id,
            start_offset_ms: completed_start,
            end_offset_ms: Some(completed_end),
            ..
        } = completed
        else {
            panic!("expected completed provider observation");
        };
        assert_eq!(started_id, completed_id);
        assert_eq!(
            (started_offset, completed_start, completed_end),
            (10, 10, 45)
        );
    }

    #[test]
    fn timeout_finalizes_active_stage_with_its_existing_observation_id() {
        let mut recorder = InvocationTelemetry::new(
            "local-qwen".to_string(),
            "chat".to_string(),
            Some("qwen".to_string()),
        );
        let started = safe_trace_telemetry(
            ProviderEvent::StageStarted {
                stage: vifu_runtime::ProviderStage::Decode,
                metadata: json!({}),
            },
            10,
            &mut recorder,
        )
        .unwrap();
        let TraceTelemetry::ProviderStage { observation_id, .. } = started else {
            panic!("expected started provider observation");
        };

        let terminal = recorder.fail_active(50, Some("provider timed out after 40ms"));
        assert!(matches!(
            terminal.as_slice(),
            [TraceTelemetry::ProviderStage {
                observation_id: terminal_id,
                status: TraceStageStatus::Failed,
                start_offset_ms: 10,
                end_offset_ms: Some(50),
                elapsed_ms: Some(40),
                ..
            }] if *terminal_id == observation_id
        ));
        assert!(recorder.active.is_empty());
        let _batch = recorder.finish();
        assert!(safe_trace_telemetry(
            ProviderEvent::StageCompleted {
                stage: vifu_runtime::ProviderStage::Decode,
                elapsed_ms: 41,
                metadata: json!({}),
            },
            51,
            &mut recorder,
        )
        .is_none());
    }

    #[tokio::test]
    async fn error_responses_do_not_mark_valid_output_delivery() {
        for provider_terminal in [
            GatewayInvocationTerminal::ProviderFailed,
            GatewayInvocationTerminal::TimedOut,
            GatewayInvocationTerminal::PreflightFailed,
        ] {
            let (sender, mut receiver) = tokio::sync::mpsc::channel(4);
            let observed = Arc::new(std::sync::Mutex::new(Vec::new()));
            let observed_for_callback = Arc::clone(&observed);
            let observer: super::GatewayRuntimeObserver = Arc::new(move |event| {
                observed_for_callback.lock().unwrap().push(event);
            });
            let request_id = Uuid::new_v4();
            let delivery = InvocationDelivery {
                sender,
                observer: Some(observer),
                capture_sender: None,
                config_epoch: 1,
                request_id,
                binding_id: Uuid::new_v4(),
                capability: "chat".to_string(),
                provider_key: "local-qwen".to_string(),
                invocation_started: std::time::Instant::now(),
                telemetry: Arc::new(std::sync::Mutex::new(InvocationTelemetry::new(
                    "local-qwen".to_string(),
                    "chat".to_string(),
                    Some("qwen".to_string()),
                ))),
                telemetry_backlog: Arc::new(std::sync::Mutex::new(
                    super::TelemetryBacklogState::default(),
                )),
                telemetry_client: RuntimeControlClient::new(
                    "http://127.0.0.1:1",
                    "test-device-token",
                )
                .unwrap(),
                telemetry_uploads: Arc::new(tokio::sync::Semaphore::new(
                    super::MAX_CONCURRENT_TELEMETRY_UPLOADS,
                )),
                invocation_permit: None,
            };
            let finish = tokio::spawn(async move {
                delivery
                    .finish(
                        AgentGatewayCommand::Error {
                            request_id: Some(request_id),
                            channel_id: Some(7),
                            code: "PROVIDER_ERROR".to_string(),
                            message: "provider unavailable".to_string(),
                        },
                        provider_terminal,
                        Some("provider unavailable".to_string()),
                        None,
                    )
                    .await;
            });

            let outbound = receiver
                .recv()
                .await
                .expect("error response should be queued");
            assert!(matches!(
                outbound.command,
                AgentGatewayCommand::Error { .. }
            ));
            outbound
                .delivery
                .expect("error response should be tracked")
                .send(true)
                .unwrap();
            finish.await.unwrap();
            assert!(
                receiver.try_recv().is_err(),
                "error transport is not Deliver"
            );
            assert!(matches!(
                observed.lock().unwrap().last(),
                Some(GatewayRuntimeEvent::InvocationFinished {
                    terminal,
                    ..
                }) if *terminal == provider_terminal
            ));
        }
    }

    impl AgentProvider for BinaryTranscriptionProvider {
        fn supports(&self, capability: &str) -> bool {
            capability == "transcription"
        }

        fn invoke<'a>(
            &'a self,
            request: ProviderRequest,
            _cancellation: CancellationToken,
        ) -> ProviderFuture<'a> {
            Box::pin(async move {
                let InvocationData::Binary(audio) = request.data else {
                    return Ok(ProviderResponse::json(
                        json!({ "error": "expected binary" }),
                    ));
                };
                Ok(ProviderResponse::json(json!({
                    "capability": request.capability,
                    "bytes": audio.len(),
                })))
            })
        }
    }

    #[test]
    fn routes_calls_to_the_provider_named_by_the_binding() {
        let providers: Vec<Arc<dyn AgentGatewayProvider>> = vec![
            Arc::new(OpenClawGatewayProvider::new(
                "primary",
                Endpoint {
                    host: "127.0.0.1".to_string(),
                    port: 18789,
                },
                None,
            )),
            Arc::new(OpenClawGatewayProvider::new(
                "story",
                Endpoint {
                    host: "127.0.0.1".to_string(),
                    port: 18790,
                },
                None,
            )),
        ];

        let selected = resolve_provider(
            &providers,
            &json!({ "providerKey": "story" }),
            "binding-narrator",
            "profile-narrator",
            "narrator",
            None,
        )
        .expect("story provider must resolve");
        assert_eq!(selected.id(), "story");
        assert!(resolve_provider(
            &providers,
            &json!({}),
            "binding-narrator",
            "profile-narrator",
            "narrator",
            None,
        )
        .is_none());
    }

    #[test]
    fn session_route_override_wins_at_the_invocation_boundary() {
        let providers: Vec<Arc<dyn AgentGatewayProvider>> = vec![
            Arc::new(OpenClawGatewayProvider::new(
                "baseline",
                Endpoint {
                    host: "127.0.0.1".to_string(),
                    port: 18789,
                },
                None,
            )),
            Arc::new(OpenClawGatewayProvider::new(
                "optimized",
                Endpoint {
                    host: "127.0.0.1".to_string(),
                    port: 18790,
                },
                None,
            )),
        ];
        let overrides = SessionRouteOverrides::default();
        overrides
            .activate(std::collections::BTreeMap::from([(
                "planner-binding".to_string(),
                "optimized".to_string(),
            )]))
            .unwrap();

        let selected = resolve_provider(
            &providers,
            &json!({ "providerKey": "baseline" }),
            "planner-binding",
            "planner",
            "planner",
            Some(&overrides),
        )
        .expect("override provider must resolve");

        assert_eq!(selected.id(), "optimized");
    }

    #[tokio::test]
    async fn in_process_provider_receives_the_profile_persona() {
        let provider =
            InProcessGatewayProvider::new("local-qwen", Arc::new(PersonaProvider)).unwrap();

        let output = provider
            .invoke(
                "local-qwen",
                &json!({ "persona": { "instructions": "Choose one safe action." } }),
                &json!({ "messages": [{ "role": "user", "content": "Act" }] }),
                Duration::from_secs(1),
            )
            .await
            .unwrap();

        assert_eq!(output["instructions"], "Choose one safe action.");
    }

    #[tokio::test]
    async fn in_process_provider_routes_the_embedding_capability() {
        let provider =
            InProcessGatewayProvider::new("local-embedding", Arc::new(EmbeddingProvider)).unwrap();

        let output = provider
            .invoke(
                "local-embedding",
                &json!({ "capability": "embedding" }),
                &json!({ "input": ["parsnip", "watering can"] }),
                Duration::from_secs(1),
            )
            .await
            .unwrap();

        assert_eq!(output["capability"], "embedding");
    }

    #[tokio::test]
    async fn in_process_provider_decodes_binary_gateway_input() {
        let provider = InProcessGatewayProvider::new(
            "local-transcriber",
            Arc::new(BinaryTranscriptionProvider),
        )
        .unwrap();

        let output = provider
            .invoke(
                "local-transcriber",
                &json!({ "capability": "transcription" }),
                &json!({
                    "_vifuBinary": {
                        "encoding": "base64",
                        "data": base64::engine::general_purpose::STANDARD.encode(b"abc"),
                    }
                }),
                Duration::from_secs(1),
            )
            .await
            .unwrap();

        assert_eq!(output["capability"], "transcription");
        assert_eq!(output["bytes"], 3);
    }

    #[test]
    fn builds_agent_gateway_websocket_url_from_http_base() {
        assert_eq!(
            agent_gateway_websocket_url("http://127.0.0.1:6790").unwrap(),
            "ws://127.0.0.1:6790/v1/agent-gateway/connect"
        );
        assert_eq!(
            agent_gateway_websocket_url("http://backend:6790").unwrap(),
            "ws://backend:6790/v1/agent-gateway/connect"
        );
        assert_eq!(
            agent_gateway_websocket_url("https://runtime.example.com/api/").unwrap(),
            "wss://runtime.example.com/api/v1/agent-gateway/connect"
        );
    }

    #[test]
    fn rejects_server_urls_with_credentials() {
        let url = format!("https://{}:{}@example.com", "user", "pass");
        assert!(agent_gateway_websocket_url(&url).is_err());
    }

    #[test]
    fn rejects_plaintext_remote_server_urls() {
        let error = agent_gateway_websocket_url("http://relay.example.com").unwrap_err();
        assert!(error.contains("must use https"));
    }

    #[test]
    fn accepts_secure_remote_server_urls() {
        assert_eq!(
            agent_gateway_websocket_url("https://relay.example.com").unwrap(),
            "wss://relay.example.com/v1/agent-gateway/connect"
        );
    }

    #[test]
    fn guest_claim_link_keeps_the_token_in_the_fragment() {
        let claim_token = format!("vifu_gc_{}", "a".repeat(64));
        let link = guest_claim_url("https://dashboard.vifu.ai", &claim_token).unwrap();
        let url = Url::parse(&link).unwrap();

        assert_eq!(url.path(), "/pair");
        assert!(url.query().is_none());
        assert_eq!(
            url.fragment(),
            Some(format!("claim_token={claim_token}").as_str())
        );
    }

    #[test]
    fn guest_claim_link_rejects_non_http_dashboard_urls() {
        assert!(guest_claim_url("vifu://dashboard", "vifu_gc_invalid").is_err());
    }

    #[test]
    fn sanitizes_agent_gateway_errors() {
        assert_eq!(sanitize_error("bad\0token"), "bad token");
        assert_eq!(
            safe_observer_error("Basic cHJpdmF0ZS11c2VyOnByaXZhdGUtcGFzcw=="),
            "Provider failed; sensitive details were redacted"
        );
    }

    #[test]
    fn observer_output_policy_suppresses_terminal_bytes_including_secrets() {
        let mut output = Vec::new();

        write_terminal_line(
            GatewayOutputPolicy::Observer,
            &mut output,
            "API key: vifu_secret",
        )
        .unwrap();

        assert!(output.is_empty());
        write_terminal_line(
            GatewayOutputPolicy::Terminal,
            &mut output,
            "Status: connected",
        )
        .unwrap();
        assert_eq!(output, b"Status: connected\n");
    }

    #[test]
    fn client_transport_codec_round_trips_gateway_frames() {
        let session_id = Uuid::new_v4();
        let command = AgentGatewayCommand::Heartbeat { session_id };
        let encoded = encode_command(&command).unwrap();
        let value: serde_json::Value = serde_json::from_str(&encoded).unwrap();

        assert_eq!(value["type"], "event");
        assert_eq!(value["event"], AGENT_GATEWAY_HEARTBEAT_EVENT);
        assert_eq!(decode_command(&encoded).unwrap(), command);
    }

    #[test]
    fn client_transport_codec_rejects_invalid_frames() {
        assert!(decode_command("").unwrap_err().contains("empty"));
        assert!(decode_command("{")
            .unwrap_err()
            .contains("invalid gateway frame"));
        assert!(
            decode_command(&" ".repeat(gateway_frame::MAX_GATEWAY_FRAME_BYTES + 1))
                .unwrap_err()
                .contains("too large")
        );

        let extra_frame_field = json!({
            "type": "req",
            "id": AGENT_GATEWAY_HELLO_REQUEST_ID,
            "method": AGENT_GATEWAY_HELLO_METHOD,
            "params": {
                "protocol": VERSION,
                "agents": [],
                "metadata": {}
            },
            "extra": true
        })
        .to_string();
        assert!(decode_command(&extra_frame_field)
            .unwrap_err()
            .contains("invalid gateway frame"));

        let null_typed_frame_field = json!({
            "type": "event",
            "event": AGENT_GATEWAY_HEARTBEAT_EVENT,
            "seq": null,
            "payload": {
                "sessionId": Uuid::new_v4()
            }
        })
        .to_string();
        assert!(decode_command(&null_typed_frame_field)
            .unwrap_err()
            .contains("invalid gateway frame"));

        let extra_protocol_payload_field = json!({
            "type": "req",
            "id": AGENT_GATEWAY_HELLO_REQUEST_ID,
            "method": AGENT_GATEWAY_HELLO_METHOD,
            "params": {
                "protocol": VERSION,
                "agents": [],
                "metadata": {},
                "extra": true
            }
        })
        .to_string();
        assert!(decode_command(&extra_protocol_payload_field)
            .unwrap_err()
            .contains("invalid gateway.hello params"));
    }
}
