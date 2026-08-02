use std::collections::HashMap;
use std::future::Future;
use std::io::{self, IsTerminal};
use std::net::IpAddr;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::{SinkExt, StreamExt};
use tokio::sync::{mpsc, Semaphore};
use tokio::task::JoinHandle;
use tokio_tungstenite::connect_async_with_config;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::Message;
use url::Url;
use uuid::Uuid;

use crate::control::{GuestProjectBootstrap, RuntimeControlClient};
use crate::gateway_frame;
use crate::openclaw::{self, Endpoint};
use crate::protocol::{self, AgentDescriptor, AgentGatewayCommand};
use crate::session::{self, GuestProjectSummary, PairingSummary, SessionSummary};
#[cfg(feature = "sqlite")]
use crate::session_store::GatewaySessionPersistence;

use vifu_runtime::{
    AgentDefinition, AgentProvider, CancellationToken, InvocationData, ProviderRequest,
    RuntimeSnapshot, VifuRuntime,
};
#[cfg(feature = "sqlite")]
use vifu_runtime::{RuntimeStore, SqliteRuntimeStore};

const MAX_CONCURRENT_CALLS: usize = 64;
const OUTBOUND_QUEUE_CAPACITY: usize = 128;
const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(30);

pub struct AgentGatewayRuntime<'a> {
    pub server_url: &'a str,
    pub dashboard_url: Option<&'a str>,
    pub agent_gateway_bootstrap_token: Option<&'a str>,
    pub enrollment_token: Option<String>,
    pub allow_guest_bootstrap: bool,
    pub providers: &'a [Arc<dyn AgentGatewayProvider>],
    pub agents: &'a [AgentDescriptor],
    pub session_path: Option<&'a Path>,
    pub runtime_database_path: &'a Path,
    pub embedded_runtime: Option<&'a VifuRuntime>,
}

pub type GuestProjectObserver = Arc<dyn Fn(&GuestProjectSummary) + Send + Sync>;
pub type GatewayAuthorizationObserver = Arc<dyn Fn(&GatewayAuthorizationSummary) + Send + Sync>;
pub type GatewayPairingObserver = Arc<dyn Fn(Option<&PairingSummary>) + Send + Sync>;

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
    ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, String>> + Send + 'a>>;
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
    ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, String>> + Send + 'a>> {
        Box::pin(openclaw::invoke(
            &self.endpoint,
            self.token.as_deref(),
            agent_id,
            binding,
            input,
            timeout,
        ))
    }
}

pub struct InProcessGatewayProvider {
    id: String,
    provider: Arc<dyn AgentProvider>,
}

impl InProcessGatewayProvider {
    pub fn new(id: impl Into<String>, provider: Arc<dyn AgentProvider>) -> Result<Self, String> {
        if !provider.supports("chat") && !provider.supports("embedding") {
            return Err("in-process provider must support chat or embedding".to_string());
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
    ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, String>> + Send + 'a>> {
        Box::pin(async move {
            let capability = binding_text(binding, "capability").unwrap_or("chat");
            if !self.provider.supports(capability) {
                return Err(format!(
                    "in-process provider does not support capability {capability}"
                ));
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
                data: InvocationData::Json(input.clone()),
                metadata: serde_json::json!({ "source": "agent-gateway" }),
                snapshot: RuntimeSnapshot::default(),
            };
            let invocation = self.provider.invoke(request, cancellation.clone());
            let response = match tokio::time::timeout(timeout, invocation).await {
                Ok(response) => response.map_err(|error| error.public_message())?,
                Err(_) => {
                    cancellation.cancel();
                    return Err("in-process provider request timed out".to_string());
                }
            };
            match response.data {
                InvocationData::Json(value) => Ok(value),
                InvocationData::Binary(_) => {
                    Err("in-process provider returned binary data".to_string())
                }
            }
        })
    }
}

fn binding_text<'a>(binding: &'a serde_json::Value, name: &str) -> Option<&'a str> {
    binding
        .get(name)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
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

    loop {
        match run_connection(
            &websocket_url,
            &runtime,
            session,
            &persistence,
            guest_project_observer.as_ref(),
            authorization_observer.as_ref(),
            pairing_observer.as_ref(),
        )
        .await
        {
            Ok(ConnectionOutcome::Shutdown) => return Ok(()),
            Ok(ConnectionOutcome::Disconnected) => {
                eprintln!(
                    "Agent Gateway disconnected; reconnecting in {}s.",
                    reconnect_delay.as_secs()
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
                    println!();
                    println!("Authorization required");
                    println!("  Dashboard: {}", terminal_link(&auth_url));
                    println!("  Waiting for approval; this Gateway will reconnect automatically.");
                }
                reconnect_delay = retry_after;
            }
            Err(AgentGatewayConnectionError::Failed(error)) => {
                eprintln!(
                    "Agent Gateway connection failed: {}. Retrying in {}s.",
                    sanitize_error(&error),
                    reconnect_delay.as_secs()
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
    print_guest_project(runtime.server_url, guest);
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

fn print_guest_project(server_url: &str, guest: &GuestProjectBootstrap) {
    let endpoint = guest_endpoint_url(server_url, &guest.endpoint_path)
        .unwrap_or_else(|_| guest.endpoint_path.clone());
    println!();
    println!("Project registered");
    println!("  Project:  {}", guest.project.slug);
    println!("  Endpoint: {endpoint}");
    println!("  API key:  {}", guest.api_key);
    println!("  Expires:  {}", guest.expires_at);
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

async fn run_connection(
    websocket_url: &str,
    runtime: &AgentGatewayRuntime<'_>,
    session: &mut SessionSummary,
    persistence: &SessionPersistence,
    guest_project_observer: Option<&GuestProjectObserver>,
    authorization_observer: Option<&GatewayAuthorizationObserver>,
    pairing_observer: Option<&GatewayPairingObserver>,
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
                "features": ["config-sync-v1", "trace-upload-v1", "embedded-runtime-v1"],
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
    if let Some(observer) = pairing_observer.as_ref() {
        observer(None);
    }
    session.resume_session_id = Some(session_id);
    persist_session(runtime, session, persistence)?;
    if let Some(observer) = authorization_observer {
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
            guest_project_observer,
        )?;
    }
    if let Err(error) = sync_runtime_state(runtime, session).await {
        eprintln!(
            "Runtime configuration sync is unavailable: {}",
            sanitize_error(&error)
        );
    }
    println!();
    println!("Status: connected");

    let (outbound_sender, mut outbound_receiver) =
        mpsc::channel::<AgentGatewayCommand>(OUTBOUND_QUEUE_CAPACITY);
    let semaphore = std::sync::Arc::new(Semaphore::new(MAX_CONCURRENT_CALLS));
    let mut calls = HashMap::<Uuid, JoinHandle<()>>::new();
    let mut configuration_sync = tokio::time::interval(Duration::from_secs(30));
    configuration_sync.tick().await;

    let outcome = loop {
        reap_finished(&mut calls);
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break ConnectionOutcome::Shutdown,
            _ = configuration_sync.tick() => {
                if let Err(error) = sync_runtime_state(runtime, session).await {
                    eprintln!(
                        "Runtime configuration sync is unavailable: {}",
                        sanitize_error(&error)
                    );
                }
            }
            outbound = outbound_receiver.recv() => {
                let Some(outbound) = outbound else {
                    return Err("agent gateway output queue closed".to_string().into());
                };
                send_command(&mut socket, &outbound).await?;
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
                        endpoint_id: _,
                        profile_id: _,
                        binding_id: _,
                        agent_id,
                        binding,
                        input,
                        timeout_ms,
                    } => {
                        if calls.contains_key(&request_id) {
                            queue_error(
                                &outbound_sender,
                                Some(request_id),
                                Some(channel_id),
                                "DUPLICATE_REQUEST",
                                "The request id is already running.",
                            ).await?;
                            continue;
                        }
                        let permit = match semaphore.clone().try_acquire_owned() {
                            Ok(permit) => permit,
                            Err(_) => {
                                queue_error(
                                    &outbound_sender,
                                    Some(request_id),
                                    Some(channel_id),
                                    "BACKPRESSURE",
                                    "The agent gateway has reached its concurrent call limit.",
                                ).await?;
                                continue;
                            }
                        };
                        let Some(provider) = resolve_provider(runtime.providers, &binding)
                        else {
                            queue_error(
                                &outbound_sender,
                                Some(request_id),
                                Some(channel_id),
                                "PROVIDER_NOT_AVAILABLE",
                                "The requested provider is not connected to this Agent Gateway.",
                            )
                            .await?;
                            continue;
                        };
                        let provider = Arc::clone(provider);
                        let sender = outbound_sender.clone();
                        let handle = tokio::spawn(async move {
                            let result = provider
                                .invoke(
                                    &agent_id,
                                    &binding,
                                    &input,
                                    Duration::from_millis(timeout_ms),
                                )
                                .await;
                            let message = match result {
                                Ok(output) => AgentGatewayCommand::Result {
                                    request_id,
                                    channel_id,
                                    output,
                                },
                                Err(error) => agent_gateway_error(
                                    request_id,
                                    channel_id,
                                    "PROVIDER_ERROR",
                                    &error,
                                ),
                            };
                            let _permit = permit;
                            let _ = sender.send(message).await;
                        });
                        calls.insert(request_id, handle);
                    }
                    AgentGatewayCommand::Cancel { request_id, .. } => {
                        if let Some(call) = calls.remove(&request_id) {
                            call.abort();
                        }
                    }
                    AgentGatewayCommand::Heartbeat { session_id: received } => {
                        if received != session_id {
                            return Err("server heartbeat session does not match".to_string().into());
                        }
                        outbound_sender
                            .send(AgentGatewayCommand::HeartbeatAck { session_id })
                            .await
                            .map_err(|_| "agent gateway output queue closed".to_string())?;
                    }
                    AgentGatewayCommand::RuntimeConfigChanged { .. } => {
                        if let Err(error) = sync_runtime_state(runtime, session).await {
                            eprintln!(
                                "Runtime configuration sync is unavailable: {}",
                                sanitize_error(&error)
                            );
                        }
                    }
                    AgentGatewayCommand::Error {
                        request_id: None,
                        code,
                        message,
                        ..
                    } if code == "SESSION_REPLACED" => {
                        eprintln!("Agent Gateway session replaced: {}", sanitize_error(&message));
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
) -> Option<&'a Arc<dyn AgentGatewayProvider>> {
    let provider_key = binding
        .get("providerKey")
        .or_else(|| binding.pointer("/source/providerKey"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    match provider_key {
        Some(provider_key) => providers
            .iter()
            .find(|provider| provider.id() == provider_key),
        None if providers.len() == 1 => providers.first(),
        None => None,
    }
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

async fn queue_error(
    sender: &mpsc::Sender<AgentGatewayCommand>,
    request_id: Option<Uuid>,
    channel_id: Option<u64>,
    code: &str,
    message: &str,
) -> Result<(), String> {
    sender
        .send(AgentGatewayCommand::Error {
            request_id,
            channel_id,
            code: code.to_string(),
            message: message.to_string(),
        })
        .await
        .map_err(|_| "agent gateway output queue closed".to_string())
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
    use serde_json::json;
    use std::sync::Arc;
    use std::time::Duration;
    use url::Url;
    use uuid::Uuid;

    use super::{
        agent_gateway_websocket_url, decode_command, encode_command, guest_claim_url,
        resolve_provider, sanitize_error, AgentGatewayProvider, InProcessGatewayProvider,
        OpenClawGatewayProvider,
    };
    use crate::gateway_frame;
    use crate::openclaw::Endpoint;
    use crate::protocol::{
        AgentGatewayCommand, AGENT_GATEWAY_HEARTBEAT_EVENT, AGENT_GATEWAY_HELLO_METHOD,
        AGENT_GATEWAY_HELLO_REQUEST_ID, VERSION,
    };
    use vifu_runtime::{
        AgentProvider, CancellationToken, ProviderFuture, ProviderRequest, ProviderResponse,
    };

    struct PersonaProvider;

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

        let selected = resolve_provider(&providers, &json!({ "providerKey": "story" }))
            .expect("story provider must resolve");
        assert_eq!(selected.id(), "story");
        assert!(resolve_provider(&providers, &json!({})).is_none());
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
