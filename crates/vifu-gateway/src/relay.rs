use std::collections::HashMap;
use std::net::IpAddr;
use std::path::Path;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::sync::{mpsc, Semaphore};
use tokio::task::JoinHandle;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{header::AUTHORIZATION, HeaderValue};
use tokio_tungstenite::tungstenite::Message;
use url::Url;
use uuid::Uuid;

use crate::gateway_frame;
use crate::openclaw::{self, Endpoint};
use crate::protocol::{self, AgentDescriptor, AgentGatewayCommand};
use crate::session::{self, SessionSummary};

const MAX_CONCURRENT_CALLS: usize = 64;
const OUTBOUND_QUEUE_CAPACITY: usize = 128;
const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(30);

pub struct AgentGatewayRuntime<'a> {
    pub server_url: &'a str,
    pub agent_gateway_bootstrap_token: &'a str,
    pub providers: &'a [OpenClawRuntimeProvider],
    pub agents: &'a [AgentDescriptor],
    pub session_path: &'a Path,
}

#[derive(Debug, Clone)]
pub struct OpenClawRuntimeProvider {
    pub id: String,
    pub endpoint: Endpoint,
    pub token: Option<String>,
}

pub async fn run_agent_gateway(
    runtime: AgentGatewayRuntime<'_>,
    session: &mut SessionSummary,
) -> Result<(), String> {
    let websocket_url = agent_gateway_websocket_url(runtime.server_url)?;
    let mut reconnect_delay = Duration::from_secs(1);

    loop {
        if let Err(error) = ensure_agent_gateway_registered(&runtime, session).await {
            if error == RegisterAgentGatewayError::Revoked {
                return Err(
                    "agent gateway access was revoked; run `vifu --reset` to enroll a new gateway identity"
                        .to_string(),
                );
            }
            eprintln!(
                "Agent Gateway registration failed: {}. Retrying in {}s.",
                sanitize_error(&error.into_message()),
                reconnect_delay.as_secs()
            );
            tokio::select! {
                _ = tokio::time::sleep(reconnect_delay) => {}
                _ = tokio::signal::ctrl_c() => return Ok(()),
            }
            reconnect_delay = reconnect_delay.saturating_mul(2).min(MAX_RECONNECT_DELAY);
            continue;
        }
        match run_connection(&websocket_url, &runtime, session).await {
            Ok(ConnectionOutcome::Shutdown) => return Ok(()),
            Ok(ConnectionOutcome::Disconnected) => {
                eprintln!(
                    "Agent Gateway disconnected; reconnecting in {}s.",
                    reconnect_delay.as_secs()
                );
            }
            Err(error) => {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnectionOutcome {
    Disconnected,
    Shutdown,
}

async fn run_connection(
    websocket_url: &str,
    runtime: &AgentGatewayRuntime<'_>,
    session: &mut SessionSummary,
) -> Result<ConnectionOutcome, String> {
    let mut request = websocket_url
        .into_client_request()
        .map_err(|error| error.to_string())?;
    request.headers_mut().insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {}", session.gateway_credential)).map_err(|_| {
            "agent gateway credential contains invalid header characters".to_string()
        })?,
    );
    let (mut socket, _) = connect_async(request)
        .await
        .map_err(|error| error.to_string())?;

    send_command(
        &mut socket,
        &AgentGatewayCommand::Hello {
            protocol: protocol::VERSION.to_string(),
            resume_session_id: session.resume_session_id,
            agents: runtime.agents.to_vec(),
            metadata: serde_json::json!({
                "adapter": "openclaw",
                "providerIds": runtime.providers.iter().map(|provider| provider.id.as_str()).collect::<Vec<_>>(),
                "version": env!("CARGO_PKG_VERSION")
            }),
        },
    )
    .await?;

    let welcome = tokio::time::timeout(Duration::from_secs(10), receive_command(&mut socket))
        .await
        .map_err(|_| "server did not accept the agent gateway in time".to_string())??;
    let AgentGatewayCommand::Welcome {
        gateway_id,
        connection_id,
        session_id,
        heartbeat_interval_ms: _,
        resumed,
    } = welcome
    else {
        return Err("server must send welcome after agent gateway hello".to_string());
    };
    if gateway_id != session.gateway_id {
        return Err("server authenticated a different agent gateway identity".to_string());
    }
    session.resume_session_id = Some(session_id);
    session::write_session(runtime.session_path, session)?;
    println!(
        "Agent Gateway: connected as {} (connection {}, session {}, resumed: {})",
        session.gateway_id, connection_id, session_id, resumed
    );

    let (outbound_sender, mut outbound_receiver) =
        mpsc::channel::<AgentGatewayCommand>(OUTBOUND_QUEUE_CAPACITY);
    let semaphore = std::sync::Arc::new(Semaphore::new(MAX_CONCURRENT_CALLS));
    let mut calls = HashMap::<Uuid, JoinHandle<()>>::new();

    let outcome = loop {
        reap_finished(&mut calls);
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break ConnectionOutcome::Shutdown,
            outbound = outbound_receiver.recv() => {
                let Some(outbound) = outbound else {
                    return Err("agent gateway output queue closed".to_string());
                };
                send_command(&mut socket, &outbound).await?;
            }
            incoming = receive_command(&mut socket) => {
                let incoming = match incoming {
                    Ok(message) => message,
                    Err(error) if error == "server disconnected" => break ConnectionOutcome::Disconnected,
                    Err(error) => return Err(error),
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
                        let Some(provider) = resolve_openclaw_provider(runtime.providers, &binding)
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
                        let endpoint = provider.endpoint.clone();
                        let openclaw_token = provider.token.clone();
                        let sender = outbound_sender.clone();
                        let handle = tokio::spawn(async move {
                            let result = openclaw::invoke(
                                &endpoint,
                                openclaw_token.as_deref(),
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
                                    "OPENCLAW_ERROR",
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
                            return Err("server heartbeat session does not match".to_string());
                        }
                        outbound_sender
                            .send(AgentGatewayCommand::HeartbeatAck { session_id })
                            .await
                            .map_err(|_| "agent gateway output queue closed".to_string())?;
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
                        return Err(
                            "agent gateway access was revoked; run `vifu --reset` to enroll a new gateway identity"
                                .to_string(),
                        );
                    }
                    AgentGatewayCommand::Error {
                        request_id: None,
                        message,
                        ..
                    } => return Err(format!("server rejected agent gateway: {}", sanitize_error(&message))),
                    _ => return Err("server sent an unexpected agent gateway message".to_string()),
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

fn resolve_openclaw_provider<'a>(
    providers: &'a [OpenClawRuntimeProvider],
    binding: &serde_json::Value,
) -> Option<&'a OpenClawRuntimeProvider> {
    let provider_key = binding
        .get("providerKey")
        .or_else(|| binding.pointer("/source/providerKey"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    match provider_key {
        Some(provider_key) => providers
            .iter()
            .find(|provider| provider.id == provider_key),
        None if providers.len() == 1 => providers.first(),
        None => None,
    }
}

async fn ensure_agent_gateway_registered(
    runtime: &AgentGatewayRuntime<'_>,
    session: &SessionSummary,
) -> Result<(), RegisterAgentGatewayError> {
    register_agent_gateway(
        runtime.server_url,
        runtime.agent_gateway_bootstrap_token,
        &session.gateway_id,
        &session.gateway_credential,
    )
    .await
}

#[derive(Debug, PartialEq, Eq)]
enum RegisterAgentGatewayError {
    Revoked,
    Failed(String),
}

impl RegisterAgentGatewayError {
    fn into_message(self) -> String {
        match self {
            Self::Revoked => "agent gateway credential was revoked".to_string(),
            Self::Failed(message) => message,
        }
    }
}

async fn register_agent_gateway(
    server_url: &str,
    bootstrap_token: &str,
    gateway_id: &str,
    credential: &str,
) -> Result<(), RegisterAgentGatewayError> {
    let registration_url =
        agent_gateway_registration_url(server_url).map_err(RegisterAgentGatewayError::Failed)?;
    let response = reqwest::Client::new()
        .post(registration_url)
        .bearer_auth(bootstrap_token)
        .json(&serde_json::json!({
            "gatewayId": gateway_id,
            "credential": credential,
        }))
        .send()
        .await
        .map_err(|error| RegisterAgentGatewayError::Failed(error.to_string()))?;
    if response.status().is_success() {
        return Ok(());
    }
    let status = response.status();
    let payload = response.json::<serde_json::Value>().await.ok();
    let code = payload
        .as_ref()
        .and_then(|value| value.pointer("/error/code"))
        .and_then(serde_json::Value::as_str);
    if code == Some("gateway_credential_revoked") {
        return Err(RegisterAgentGatewayError::Revoked);
    }
    Err(RegisterAgentGatewayError::Failed(format!(
        "server rejected agent gateway registration (HTTP {})",
        status.as_u16()
    )))
}

fn agent_gateway_registration_url(server_url: &str) -> Result<Url, String> {
    let _ = agent_gateway_websocket_url(server_url)?;
    let mut url = Url::parse(server_url.trim())
        .map_err(|_| "gateway.serverUrl must be a valid HTTP or HTTPS URL".to_string())?;
    let base_path = url.path().trim_end_matches('/');
    url.set_path(&format!("{base_path}/v1/agent-gateways/register"));
    Ok(url)
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
    use uuid::Uuid;

    use super::{
        agent_gateway_websocket_url, decode_command, encode_command, resolve_openclaw_provider,
        sanitize_error, OpenClawRuntimeProvider,
    };
    use crate::gateway_frame;
    use crate::openclaw::Endpoint;
    use crate::protocol::{
        AgentGatewayCommand, AGENT_GATEWAY_HEARTBEAT_EVENT, AGENT_GATEWAY_HELLO_METHOD,
        AGENT_GATEWAY_HELLO_REQUEST_ID, VERSION,
    };

    #[test]
    fn routes_calls_to_the_provider_named_by_the_binding() {
        let providers = vec![
            OpenClawRuntimeProvider {
                id: "primary".to_string(),
                endpoint: Endpoint {
                    host: "127.0.0.1".to_string(),
                    port: 18789,
                },
                token: None,
            },
            OpenClawRuntimeProvider {
                id: "story".to_string(),
                endpoint: Endpoint {
                    host: "127.0.0.1".to_string(),
                    port: 18790,
                },
                token: None,
            },
        ];

        let selected = resolve_openclaw_provider(&providers, &json!({ "providerKey": "story" }))
            .expect("story provider must resolve");
        assert_eq!(selected.id, "story");
        assert!(resolve_openclaw_provider(&providers, &json!({})).is_none());
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
