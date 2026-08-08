use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;
use vifu_runtime::{
    CancellationToken, InvocationData, InvocationInput, RuntimeError, RuntimeManifest,
    RuntimeMonitorIoEvent, VifuRuntime,
};

use crate::identity::MachineIdentity;
use crate::protocol::{canonical_trace_io_summary, AgentDescriptor};
use crate::relay::{self, AgentGatewayRuntime};
use crate::relay::{
    AgentGatewayProvider, GatewayConnectionState, GatewayProviderError, GatewayRuntimeEvent,
    ProviderEventSink,
};
use crate::session::SessionSummary;
#[cfg(feature = "sqlite")]
use crate::session_store::{gateway_session_state_key, GatewaySecretStorage, GatewaySessionStore};

const EMBEDDED_MONITOR_QUEUE_CAPACITY: usize = 256;
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

    fn invocation_input(
        &self,
        agent_id: &str,
        binding: &Value,
        input: &Value,
    ) -> Result<InvocationInput, GatewayProviderError> {
        let agent = self
            .runtime
            .agent_definitions()
            .map_err(|error| GatewayProviderError::failed(error.public_message()))?
            .into_iter()
            .find(|agent| agent.id == agent_id)
            .ok_or_else(|| GatewayProviderError::failed("the embedded agent is not registered"))?;
        if agent.provider != self.provider_id {
            return Err(GatewayProviderError::failed(
                "the embedded agent belongs to another provider",
            ));
        }
        Ok(InvocationInput {
            endpoint: self
                .endpoint_for(agent_id, binding)
                .map_err(GatewayProviderError::failed)?,
            session_id: binding
                .get("sessionId")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .unwrap_or("gateway-session")
                .to_string(),
            data: InvocationData::Json(input.clone()),
            metadata: serde_json::json!({ "source": "agent-gateway" }),
        })
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
    ) -> Pin<Box<dyn Future<Output = Result<Value, GatewayProviderError>> + Send + 'a>> {
        Box::pin(async move {
            let invocation = self
                .runtime
                .invoke(self.invocation_input(agent_id, binding, input)?);
            let output = tokio::time::timeout(timeout, invocation)
                .await
                .map_err(|_| GatewayProviderError::timed_out("embedded runtime request timed out"))?
                .map_err(embedded_runtime_error)?;
            Ok(embedded_runtime_output(output.data))
        })
    }

    fn invoke_with_events_and_cancellation<'a>(
        &'a self,
        agent_id: &'a str,
        binding: &'a Value,
        input: &'a Value,
        _timeout: Duration,
        cancellation: CancellationToken,
        events: ProviderEventSink,
    ) -> Pin<Box<dyn Future<Output = Result<Value, GatewayProviderError>> + Send + 'a>> {
        Box::pin(async move {
            let output = self
                .runtime
                .invoke_with_events_and_cancellation(
                    self.invocation_input(agent_id, binding, input)?,
                    cancellation,
                    events,
                )
                .await
                .map_err(embedded_runtime_error)?;
            Ok(embedded_runtime_output(output.data))
        })
    }
}

fn embedded_runtime_output(output: InvocationData) -> Value {
    match output {
        InvocationData::Json(value) => value,
        InvocationData::Binary(bytes) => serde_json::json!({
            "format": "binary",
            "bytes": bytes,
        }),
    }
}

fn embedded_runtime_error(error: RuntimeError) -> GatewayProviderError {
    match error {
        RuntimeError::Timeout(_) => GatewayProviderError::timed_out(error.public_message()),
        _ => GatewayProviderError::failed(error.public_message()),
    }
}

/// Configuration for exposing an embedded runtime through Vifu Agent Gateway.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmbeddedRuntimeGatewayConfig {
    pub server_url: String,
    pub runtime_database_path: PathBuf,
    pub server_certificate_der: Option<Vec<u8>>,
}

impl EmbeddedRuntimeGatewayConfig {
    pub fn new(server_url: impl Into<String>, runtime_database_path: impl Into<PathBuf>) -> Self {
        Self {
            server_url: server_url.into(),
            runtime_database_path: runtime_database_path.into(),
            server_certificate_der: None,
        }
    }

    /// Trusts exactly the self-signed Vifu Server certificate distributed by pairing.
    pub fn with_server_certificate_der(mut self, certificate_der: Vec<u8>) -> Self {
        self.server_certificate_der = Some(certificate_der);
        self
    }
}

/// Current state of an [`EmbeddedRuntimeGateway`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmbeddedRuntimeGatewayState {
    Stopped,
    Connecting,
    Connected,
    Reconnecting,
    AuthorizationRequired,
    Degraded,
    Failed,
}

/// Observable lifecycle state for an [`EmbeddedRuntimeGateway`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmbeddedRuntimeGatewayStatus {
    pub state: EmbeddedRuntimeGatewayState,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmbeddedGatewayAuthorization {
    pub gateway_id: String,
    pub device_token: String,
    pub generation: u64,
    pub expires_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmbeddedGatewayPairing {
    pub request_id: uuid::Uuid,
    pub auth_url: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmbeddedGuestProject {
    pub project_slug: String,
    pub endpoint_path: String,
    pub claim_token: String,
    pub expires_at: String,
}

struct EmbeddedGatewayTask {
    shutdown: tokio::sync::oneshot::Sender<()>,
    thread: JoinHandle<()>,
}

/// Runs Agent Gateway beside a [`VifuRuntime`] configured from Project Settings.
///
/// The runtime and all locally registered providers remain in process. Starting
/// the gateway only makes the configured agents discoverable through the
/// configured Vifu Server.
pub struct EmbeddedRuntimeGateway {
    runtime: VifuRuntime,
    config: EmbeddedRuntimeGatewayConfig,
    task: Mutex<Option<EmbeddedGatewayTask>>,
    status: Arc<Mutex<EmbeddedRuntimeGatewayStatus>>,
    guest_project: Arc<Mutex<Option<EmbeddedGuestProject>>>,
    authorization: Arc<Mutex<Option<EmbeddedGatewayAuthorization>>>,
    pairing: Arc<Mutex<Option<EmbeddedGatewayPairing>>>,
}

impl EmbeddedRuntimeGateway {
    /// Creates a stopped gateway and validates its network identity.
    pub fn new(runtime: VifuRuntime, config: EmbeddedRuntimeGatewayConfig) -> Result<Self, String> {
        relay::agent_gateway_websocket_url(&config.server_url)?;
        if config
            .server_certificate_der
            .as_ref()
            .is_some_and(Vec::is_empty)
        {
            return Err("the pinned server certificate is empty".to_string());
        }
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
            guest_project: Arc::new(Mutex::new(None)),
            authorization: Arc::new(Mutex::new(None)),
            pairing: Arc::new(Mutex::new(None)),
        })
    }

    /// Starts the network Gateway with host-managed Machine identity and token storage.
    pub fn start(
        &self,
        identity: MachineIdentity,
        device_token: Option<String>,
        enrollment_token: Option<String>,
    ) -> Result<(), String> {
        self.start_with_monitor_io(identity, device_token, enrollment_token, false)
    }

    /// Starts the Gateway and optionally sends bounded root invocation input/output summaries.
    ///
    /// Full invocation content is disabled by default. Hosts must expose their own explicit
    /// consent control before setting `capture_monitor_io` to `true`.
    pub fn start_with_monitor_io(
        &self,
        identity: MachineIdentity,
        device_token: Option<String>,
        enrollment_token: Option<String>,
        capture_monitor_io: bool,
    ) -> Result<(), String> {
        identity.validate()?;
        if let Some(device_token) = device_token.as_deref() {
            crate::session::validate_device_token(device_token)?;
        }
        let manifest = runtime_manifest(&self.runtime)?;
        let (providers, agents, provider_models) = gateway_components(&self.runtime, &manifest);
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
        let runtime_database_path = self.config.runtime_database_path.clone();
        let server_certificate_der = self.config.server_certificate_der.clone();
        let embedded_runtime = self.runtime.clone();
        let (monitor_sender, monitor_receiver) =
            tokio::sync::mpsc::channel(EMBEDDED_MONITOR_QUEUE_CAPACITY);
        let monitor_drops = Arc::new(AtomicU32::new(0));
        install_embedded_monitor_observers(
            &self.runtime,
            monitor_sender,
            Arc::clone(&monitor_drops),
            capture_monitor_io,
        )?;
        let embedded_monitor =
            relay::EmbeddedRuntimeMonitor::new_with_io(monitor_receiver, monitor_drops);
        let status = Arc::clone(&self.status);
        let guest_project = Arc::clone(&self.guest_project);
        #[cfg(feature = "sqlite")]
        let authorization = Arc::clone(&self.authorization);
        #[cfg(feature = "sqlite")]
        let pairing = Arc::clone(&self.pairing);
        let (shutdown, shutdown_receiver) = tokio::sync::oneshot::channel();
        set_gateway_status(&status, EmbeddedRuntimeGatewayState::Connecting, None)?;
        clear_guest_project(&guest_project)?;
        let thread = match std::thread::Builder::new()
            .name("vifu-embedded-gateway".to_string())
            .spawn(move || {
                let result = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|error| error.to_string())
                    .and_then(|tokio| {
                        #[cfg(feature = "sqlite")]
                        let session_store = GatewaySessionStore::open(&runtime_database_path)?;
                        #[cfg(feature = "sqlite")]
                        let session_key = gateway_session_state_key("embedded", &server_url)?;
                        #[cfg(feature = "sqlite")]
                        let mut session = session_store
                            .load(&session_key, Some(&identity), device_token.as_deref())?
                            .unwrap_or_else(|| {
                                SessionSummary::new(
                                    identity.clone(),
                                    SystemTime::now()
                                        .duration_since(UNIX_EPOCH)
                                        .map(|duration| duration.as_secs())
                                        .unwrap_or(1),
                                )
                                .expect("generated Machine identity must be valid")
                            });
                        #[cfg(not(feature = "sqlite"))]
                        let mut session = SessionSummary::new(
                            identity,
                            SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .map(|duration| duration.as_secs())
                                .unwrap_or(1),
                        )?;
                        #[cfg(feature = "sqlite")]
                        let session_persistence =
                            session_store.persistence(session_key, GatewaySecretStorage::External);
                        #[cfg(feature = "sqlite")]
                        session_persistence.save(&session)?;
                        tokio.block_on(async {
                            let guest_project_for_observer = Arc::clone(&guest_project);
                            let guest_project_observer =
                                Arc::new(move |guest: &crate::session::GuestProjectSummary| {
                                    let _ = set_guest_project(&guest_project_for_observer, guest);
                                });
                            if let Some(guest) = session.guest_project.as_ref() {
                                guest_project_observer(guest);
                            }
                            #[cfg(feature = "sqlite")]
                            let authorization_for_observer = Arc::clone(&authorization);
                            #[cfg(feature = "sqlite")]
                            let authorization_observer =
                                Arc::new(move |value: &relay::GatewayAuthorizationSummary| {
                                    if let Ok(mut stored) = authorization_for_observer.lock() {
                                        *stored = Some(EmbeddedGatewayAuthorization {
                                            gateway_id: value.gateway_id.clone(),
                                            device_token: value.device_token.clone(),
                                            generation: value.generation,
                                            expires_at: value.expires_at.clone(),
                                        });
                                    }
                                });
                            #[cfg(feature = "sqlite")]
                            let pairing_for_observer = Arc::clone(&pairing);
                            #[cfg(feature = "sqlite")]
                            let pairing_observer =
                                Arc::new(move |value: Option<&crate::session::PairingSummary>| {
                                    if let Ok(mut stored) = pairing_for_observer.lock() {
                                        *stored = value.map(|value| EmbeddedGatewayPairing {
                                            request_id: value.request_id,
                                            auth_url: value.auth_url.clone(),
                                        });
                                    }
                                });
                            let allow_guest_bootstrap = cfg!(feature = "sqlite")
                                && authorization_token_allows_guest_bootstrap(
                                    enrollment_token.as_deref(),
                                );
                            let connection_status = Arc::clone(&status);
                            let runtime_observer = Arc::new(move |event: GatewayRuntimeEvent| {
                                let GatewayRuntimeEvent::ConnectionStatus { state, message } =
                                    event
                                else {
                                    return;
                                };
                                let state = match state {
                                    GatewayConnectionState::Connected => {
                                        EmbeddedRuntimeGatewayState::Connected
                                    }
                                    GatewayConnectionState::Reconnecting => {
                                        EmbeddedRuntimeGatewayState::Reconnecting
                                    }
                                    GatewayConnectionState::AuthorizationRequired => {
                                        EmbeddedRuntimeGatewayState::AuthorizationRequired
                                    }
                                    GatewayConnectionState::Degraded => {
                                        EmbeddedRuntimeGatewayState::Degraded
                                    }
                                };
                                let _ = set_gateway_status(&connection_status, state, message);
                            });
                            let gateway = AgentGatewayRuntime {
                                server_url: &server_url,
                                server_certificate_der: server_certificate_der.as_deref(),
                                agent_gateway_bootstrap_token: None,
                                enrollment_token,
                                allow_guest_bootstrap,
                                providers: &providers,
                                agents: &agents,
                                route_overrides: None,
                                runtime_observer: Some(runtime_observer),
                                capture_sender: None,
                                config_epoch: 0,
                                provider_models: Some(provider_models),
                                session_path: None,
                                runtime_database_path: &runtime_database_path,
                                embedded_runtime: Some(&embedded_runtime),
                                embedded_monitor: Some(embedded_monitor),
                                output_policy: relay::GatewayOutputPolicy::Terminal,
                            };
                            #[cfg(feature = "sqlite")]
                            let gateway_run = relay::run_agent_gateway_with_session_persistence(
                                gateway,
                                &mut session,
                                session_persistence,
                                Some(guest_project_observer),
                                Some(authorization_observer),
                                Some(pairing_observer),
                            );
                            #[cfg(not(feature = "sqlite"))]
                            let gateway_run = relay::run_agent_gateway(gateway, &mut session);
                            tokio::select! {
                                result = gateway_run => result,
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
                let _ = self.runtime.set_monitor_observer(None);
                let _ = self.runtime.set_monitor_io_observer(None);
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

    /// Returns a temporary guest project created for this Gateway, if any.
    pub fn guest_project(&self) -> Result<Option<EmbeddedGuestProject>, String> {
        self.guest_project
            .lock()
            .map(|guest| guest.clone())
            .map_err(|_| "embedded gateway guest project is unavailable".to_string())
    }

    pub fn authorization(&self) -> Result<Option<EmbeddedGatewayAuthorization>, String> {
        self.authorization
            .lock()
            .map(|authorization| authorization.clone())
            .map_err(|_| "embedded Gateway authorization is unavailable".to_string())
    }

    pub fn pairing(&self) -> Result<Option<EmbeddedGatewayPairing>, String> {
        self.pairing
            .lock()
            .map(|pairing| pairing.clone())
            .map_err(|_| "embedded Gateway pairing is unavailable".to_string())
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
            self.runtime
                .set_monitor_observer(None)
                .map_err(|error| error.public_message())?;
            self.runtime
                .set_monitor_io_observer(None)
                .map_err(|error| error.public_message())?;
        }
        set_gateway_status(&self.status, EmbeddedRuntimeGatewayState::Stopped, None)
    }
}

fn install_embedded_monitor_observers(
    runtime: &VifuRuntime,
    monitor_sender: tokio::sync::mpsc::Sender<relay::EmbeddedRuntimeMonitorEvent>,
    monitor_drops: Arc<AtomicU32>,
    capture_monitor_io: bool,
) -> Result<(), String> {
    let lifecycle_sender = monitor_sender.clone();
    let observer_drops = Arc::clone(&monitor_drops);
    runtime
        .set_monitor_observer(Some(Arc::new(move |event| {
            try_send_embedded_monitor_event(
                &lifecycle_sender,
                relay::EmbeddedRuntimeMonitorEvent::Lifecycle(event),
                &observer_drops,
            );
        })))
        .map_err(|error| error.public_message())?;

    if !capture_monitor_io {
        return runtime
            .set_monitor_io_observer(None)
            .map_err(|error| error.public_message());
    }

    let io_drops = Arc::clone(&monitor_drops);
    if let Err(error) = runtime.set_monitor_io_observer(Some(Arc::new(move |event| {
        let event = match event {
            RuntimeMonitorIoEvent::InvocationInput {
                trace_id, summary, ..
            } => relay::EmbeddedRuntimeMonitorEvent::RootInput {
                trace_id,
                summary: canonical_trace_io_summary(&summary.value),
            },
            RuntimeMonitorIoEvent::InvocationOutput {
                trace_id, summary, ..
            } => relay::EmbeddedRuntimeMonitorEvent::RootOutput {
                trace_id,
                summary: canonical_trace_io_summary(&summary.value),
            },
        };
        try_send_embedded_monitor_event(&monitor_sender, event, &io_drops);
    }))) {
        let _ = runtime.set_monitor_observer(None);
        return Err(error.public_message());
    }
    Ok(())
}

impl Drop for EmbeddedRuntimeGateway {
    fn drop(&mut self) {
        let _ = self.stop_inner();
    }
}

fn try_send_embedded_monitor_event(
    sender: &tokio::sync::mpsc::Sender<relay::EmbeddedRuntimeMonitorEvent>,
    event: relay::EmbeddedRuntimeMonitorEvent,
    dropped_events: &AtomicU32,
) {
    if sender.try_send(event).is_err() {
        let _ = dropped_events.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |dropped| {
            Some(dropped.saturating_add(1))
        });
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
            "embedded gateway requires applied Project Settings or an active release".to_string()
        })
}

fn gateway_components(
    runtime: &VifuRuntime,
    manifest: &RuntimeManifest,
) -> (
    Vec<Arc<dyn AgentGatewayProvider>>,
    Vec<AgentDescriptor>,
    relay::ProviderModels,
) {
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
            if let Some(provider) = manifest
                .providers
                .iter()
                .find(|provider| provider.id == agent.provider)
            {
                metadata.insert("providerSettings".to_string(), provider.settings.clone());
                metadata.insert(
                    "providerResources".to_string(),
                    serde_json::json!(provider.resources),
                );
                metadata.insert(
                    "providerCapabilities".to_string(),
                    serde_json::json!(provider.capabilities),
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
        .collect::<Vec<_>>();
    let provider_models = Arc::new(
        agents
            .iter()
            .flat_map(|agent| {
                let provider = agent
                    .metadata
                    .get("providerKey")
                    .and_then(Value::as_str)
                    .unwrap_or(&agent.id)
                    .to_string();
                let model = agent
                    .metadata
                    .get("model")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                agent
                    .metadata
                    .get("capabilities")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .filter_map(move |capability| {
                        let model = agent
                            .metadata
                            .pointer(&format!("/models/{capability}"))
                            .and_then(Value::as_str)
                            .map(str::to_string)
                            .or_else(|| model.clone())?;
                        Some(((provider.clone(), capability.to_string()), model))
                    })
            })
            .collect(),
    );
    (providers, agents, provider_models)
}

fn set_gateway_status(
    status: &Mutex<EmbeddedRuntimeGatewayStatus>,
    state: EmbeddedRuntimeGatewayState,
    last_error: Option<String>,
) -> Result<(), String> {
    let mut status = status
        .lock()
        .map_err(|_| "embedded gateway status is unavailable".to_string())?;
    status.state = state;
    status.last_error = last_error;
    Ok(())
}

fn set_guest_project(
    guest_project: &Mutex<Option<EmbeddedGuestProject>>,
    guest: &crate::session::GuestProjectSummary,
) -> Result<(), String> {
    *guest_project
        .lock()
        .map_err(|_| "embedded gateway guest project is unavailable".to_string())? =
        Some(EmbeddedGuestProject {
            project_slug: guest.project_slug.clone(),
            endpoint_path: guest.endpoint_path.clone(),
            claim_token: guest.claim_token.clone(),
            expires_at: guest.expires_at.clone(),
        });
    Ok(())
}

fn clear_guest_project(guest_project: &Mutex<Option<EmbeddedGuestProject>>) -> Result<(), String> {
    *guest_project
        .lock()
        .map_err(|_| "embedded gateway guest project is unavailable".to_string())? = None;
    Ok(())
}

fn authorization_token_allows_guest_bootstrap(token: Option<&str>) -> bool {
    token.is_none()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use serde_json::json;
    use vifu_runtime::{
        AgentDefinition, AgentProvider, CancellationToken, EndpointDefinition, InvocationInput,
        ProviderFuture, ProviderRequest, ProviderRequirement, ProviderResponse, RuntimeError,
        RuntimeManifest,
    };

    use super::*;

    #[test]
    fn only_an_unpaired_gateway_can_create_a_guest_project() {
        assert!(authorization_token_allows_guest_bootstrap(None));
        assert!(!authorization_token_allows_guest_bootstrap(Some(
            "vifu_gb_0123456789abcdef"
        )));
        assert!(!authorization_token_allows_guest_bootstrap(Some(
            "vifu_ge_0123456789abcdef"
        )));
    }

    struct EchoProvider;

    impl AgentProvider for EchoProvider {
        fn supports(&self, capability: &str) -> bool {
            capability == "chat"
        }

        fn invoke<'a>(
            &'a self,
            request: ProviderRequest,
            cancellation: CancellationToken,
        ) -> ProviderFuture<'a> {
            self.invoke_with_events(request, cancellation, ProviderEventSink::discard())
        }

        fn invoke_with_events<'a>(
            &'a self,
            request: ProviderRequest,
            _cancellation: CancellationToken,
            events: ProviderEventSink,
        ) -> ProviderFuture<'a> {
            events.activity();
            Box::pin(async move { Ok(ProviderResponse::json(request.data_json()?)) })
        }
    }

    struct CancellationProbeProvider {
        seen: Arc<Mutex<Option<CancellationToken>>>,
    }

    impl AgentProvider for CancellationProbeProvider {
        fn supports(&self, capability: &str) -> bool {
            capability == "chat"
        }

        fn invoke<'a>(
            &'a self,
            _request: ProviderRequest,
            cancellation: CancellationToken,
        ) -> ProviderFuture<'a> {
            *self.seen.lock().unwrap() = Some(cancellation.clone());
            Box::pin(async move {
                cancellation.cancelled().await;
                Err(RuntimeError::Cancelled)
            })
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

    #[tokio::test]
    async fn embedded_monitor_io_requires_explicit_consent() {
        for (capture_monitor_io, expected_io_events) in [(false, 0), (true, 2)] {
            let runtime = runtime();
            let (sender, mut receiver) = tokio::sync::mpsc::channel(32);
            install_embedded_monitor_observers(
                &runtime,
                sender,
                Arc::new(AtomicU32::new(0)),
                capture_monitor_io,
            )
            .unwrap();

            runtime
                .invoke(InvocationInput::json(
                    "mizuki-chat",
                    json!({"messages": [{"role": "user", "content": "private"}]}),
                ))
                .await
                .unwrap();

            let mut io_events = 0;
            while let Ok(event) = receiver.try_recv() {
                if matches!(
                    event,
                    relay::EmbeddedRuntimeMonitorEvent::RootInput { .. }
                        | relay::EmbeddedRuntimeMonitorEvent::RootOutput { .. }
                ) {
                    io_events += 1;
                }
            }
            assert_eq!(io_events, expected_io_events);
        }
    }

    #[tokio::test]
    async fn forwards_real_embedded_provider_activity_to_the_gateway() {
        let provider = EmbeddedRuntimeGatewayProvider::new("local-llama", runtime());
        let forwarded_activity = Arc::new(AtomicUsize::new(0));
        let observed_activity = Arc::clone(&forwarded_activity);
        let output = provider
            .invoke_with_events_and_cancellation(
                "mizuki",
                &json!({}),
                &json!({ "message": "hello" }),
                Duration::from_secs(1),
                CancellationToken::default(),
                ProviderEventSink::from_fn(move |_event| {
                    observed_activity.fetch_add(1, Ordering::Relaxed);
                }),
            )
            .await
            .unwrap();

        assert_eq!(output, json!({ "message": "hello" }));
        assert_eq!(forwarded_activity.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn propagates_gateway_cancellation_into_the_embedded_provider() {
        let seen = Arc::new(Mutex::new(None));
        let forwarded_activity = Arc::new(AtomicUsize::new(0));
        let runtime = VifuRuntime::new("moon-train").unwrap();
        runtime
            .register_provider(
                "local-llama",
                Arc::new(CancellationProbeProvider {
                    seen: Arc::clone(&seen),
                }),
            )
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
        let provider = EmbeddedRuntimeGatewayProvider::new("local-llama", runtime);
        let cancellation = CancellationToken::default();
        let binding = json!({});
        let input = json!({ "message": "hello" });
        let observed_activity = Arc::clone(&forwarded_activity);
        let invocation = provider.invoke_with_events_and_cancellation(
            "mizuki",
            &binding,
            &input,
            Duration::from_secs(1),
            cancellation.clone(),
            crate::relay::ProviderEventSink::from_fn(move |_event| {
                observed_activity.fetch_add(1, Ordering::Relaxed);
            }),
        );
        tokio::pin!(invocation);
        assert!(
            tokio::time::timeout(Duration::from_millis(300), &mut invocation)
                .await
                .is_err()
        );
        assert_eq!(forwarded_activity.load(Ordering::Relaxed), 0);
        cancellation.cancel();
        assert!(invocation.await.is_err());
        assert!(seen
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(CancellationToken::is_cancelled));
    }

    #[tokio::test]
    async fn dropping_a_stopped_gateway_preserves_a_replacement_monitor() {
        let runtime = runtime();
        let observed = Arc::new(AtomicUsize::new(0));
        let observer_count = Arc::clone(&observed);
        runtime
            .set_monitor_observer(Some(Arc::new(move |_| {
                observer_count.fetch_add(1, Ordering::Relaxed);
            })))
            .unwrap();
        let gateway = EmbeddedRuntimeGateway::new(
            runtime.clone(),
            EmbeddedRuntimeGatewayConfig::new(
                "http://127.0.0.1:6790",
                std::env::temp_dir().join(format!("vifu-gateway-{}.sqlite", uuid::Uuid::new_v4())),
            ),
        )
        .unwrap();

        drop(gateway);
        runtime
            .invoke(InvocationInput::json(
                "mizuki-chat",
                json!({ "message": "hello" }),
            ))
            .await
            .unwrap();

        assert!(observed.load(Ordering::Relaxed) >= 2);
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

        let (providers, agents, _provider_models) = gateway_components(&runtime, &manifest);

        assert_eq!(providers[0].id(), "local-llama");
        assert_eq!(agents[0].metadata["providerKey"], "local-llama");
        assert_eq!(agents[0].metadata["providerType"], "vifu-runtime");
        assert_eq!(agents[0].metadata["localProviderType"], "local-llama");
        assert_eq!(agents[0].metadata["providerCapabilities"], json!(["chat"]));
    }

    #[test]
    fn lifecycle_requires_a_manifest_before_starting() {
        let runtime = runtime();
        assert_eq!(
            runtime_manifest(&runtime).unwrap_err(),
            "embedded gateway requires applied Project Settings or an active release"
        );
    }

    #[test]
    fn guest_project_updates_preserve_the_embedded_gateway_lifecycle() {
        let status = Mutex::new(EmbeddedRuntimeGatewayStatus {
            state: EmbeddedRuntimeGatewayState::Connecting,
            last_error: None,
        });
        let guest_project = Mutex::new(None);
        let guest = crate::session::GuestProjectSummary {
            project_id: uuid::Uuid::new_v4(),
            project_slug: "guest-demo".to_string(),
            deployment_id: uuid::Uuid::new_v4(),
            deployment: "development".to_string(),
            endpoint_path: "/guest-demo/v1".to_string(),
            api_key: "vifu_pk_secret".to_string(),
            claim_token: format!("vifu_gc_{}", "a".repeat(64)),
            expires_at: "2026-08-08T00:00:00Z".to_string(),
        };

        set_guest_project(&guest_project, &guest).unwrap();

        let status = status.into_inner().unwrap();
        assert_eq!(status.state, EmbeddedRuntimeGatewayState::Connecting);
        assert_eq!(
            guest_project.into_inner().unwrap().unwrap().project_slug,
            guest.project_slug
        );
    }
}
