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
    pub agent_gateway_token: &'a str,
    pub endpoint: &'a Endpoint,
    pub openclaw_token: Option<&'a str>,
    pub agents: &'a [AgentDescriptor],
    pub session_path: &'a Path,
}

pub async fn run_agent_gateway(
    runtime: AgentGatewayRuntime<'_>,
    session: &mut SessionSummary,
) -> Result<(), String> {
    let websocket_url = agent_gateway_websocket_url(runtime.server_url)?;
    let mut reconnect_delay = Duration::from_secs(1);

    loop {
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
        HeaderValue::from_str(&format!("Bearer {}", runtime.agent_gateway_token))
            .map_err(|_| "agent gateway token contains invalid header characters".to_string())?,
    );
    let (mut socket, _) = connect_async(request)
        .await
        .map_err(|error| error.to_string())?;

    send_command(
        &mut socket,
        &AgentGatewayCommand::Hello {
            protocol: protocol::VERSION.to_string(),
            gateway_id: session.gateway_id.clone(),
            resume_session_id: session.resume_session_id,
            agents: runtime.agents.to_vec(),
            metadata: serde_json::json!({
                "adapter": "openclaw",
                "version": env!("CARGO_PKG_VERSION")
            }),
        },
    )
    .await?;

    let welcome = tokio::time::timeout(Duration::from_secs(10), receive_command(&mut socket))
        .await
        .map_err(|_| "server did not accept the agent gateway in time".to_string())??;
    let AgentGatewayCommand::Welcome {
        connection_id,
        session_id,
        heartbeat_interval_ms: _,
        resumed,
    } = welcome
    else {
        return Err("server must send welcome after agent gateway hello".to_string());
    };
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
                        let endpoint = runtime.endpoint.clone();
                        let openclaw_token = runtime.openclaw_token.map(str::to_string);
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

pub fn agent_gateway_websocket_url(server_url: &str) -> Result<String, String> {
    let mut url = Url::parse(server_url.trim())
        .map_err(|_| "VIFU_SERVER_URL must be a valid HTTP or HTTPS URL".to_string())?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(
            "VIFU_SERVER_URL must not include credentials, a query, or a fragment".to_string(),
        );
    }
    let websocket_scheme = match url.scheme() {
        "http" if is_loopback_server(&url) => "ws",
        "http" => {
            return Err(
                "Remote VIFU_SERVER_URL values must use https so agent gateway credentials are encrypted"
                    .to_string(),
            );
        }
        "https" => "wss",
        _ => return Err("VIFU_SERVER_URL must use http or https".to_string()),
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

fn is_loopback_server(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
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
    use super::{agent_gateway_websocket_url, sanitize_error};

    #[test]
    fn builds_agent_gateway_websocket_url_from_http_base() {
        assert_eq!(
            agent_gateway_websocket_url("http://127.0.0.1:6790").unwrap(),
            "ws://127.0.0.1:6790/v1/agent-gateway/connect"
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
}
