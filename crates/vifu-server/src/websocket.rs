use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::time::{Instant, MissedTickBehavior};
use tracing::{info, warn};
use uuid::Uuid;
use vifu_core::protocol::{self, AgentGatewayMessage};

use crate::auth::require_agent_gateway;
use crate::db;
use crate::error::ApiError;
use crate::AppState;

pub async fn upgrade(
    State(state): State<AppState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    require_agent_gateway(&headers, &state.config.agent_gateway_token)?;
    Ok(ws
        .max_message_size(protocol::MAX_FRAME_BYTES)
        .max_frame_size(protocol::MAX_FRAME_BYTES)
        .on_upgrade(move |socket| handle_socket(state, socket))
        .into_response())
}

async fn handle_socket(state: AppState, mut socket: WebSocket) {
    if let Err(error) = run_socket(&state, &mut socket).await {
        warn!(error = %error, "agent gateway websocket closed with an error");
        let protocol_error = AgentGatewayMessage::Error {
            request_id: None,
            channel_id: None,
            code: "PROTOCOL_ERROR".to_string(),
            message: public_error(&error),
        };
        if let Ok(encoded) = protocol::encode(&protocol_error) {
            let _ = socket.send(Message::Text(encoded.into())).await;
        }
    }
    let _ = socket.close().await;
}

async fn run_socket(state: &AppState, socket: &mut WebSocket) -> Result<(), String> {
    let hello = tokio::time::timeout(Duration::from_secs(5), receive_message(socket))
        .await
        .map_err(|_| "agent gateway did not send hello in time".to_string())??;
    let AgentGatewayMessage::Hello {
        protocol: _,
        gateway_id,
        resume_session_id,
        agents,
        metadata,
    } = hello
    else {
        return Err("agent gateway must send hello first".to_string());
    };

    let agents_json = serde_json::to_value(&agents).map_err(|error| error.to_string())?;
    let (session_id, resumed) = db::open_agent_gateway_session(
        &state.pool,
        &gateway_id,
        resume_session_id,
        &agents_json,
        &metadata,
    )
    .await
    .map_err(|error| error.to_string())?;
    let connection_id = Uuid::new_v4();
    let (sender, mut receiver) = state.relay.channel();
    state
        .relay
        .register(gateway_id.clone(), connection_id, session_id, sender)
        .await;

    let welcome = AgentGatewayMessage::Welcome {
        connection_id,
        session_id,
        heartbeat_interval_ms: state
            .config
            .heartbeat_interval
            .as_millis()
            .try_into()
            .unwrap_or(60_000),
        resumed,
    };
    send_message(socket, &welcome).await?;
    info!(%gateway_id, %connection_id, %session_id, resumed, "agent gateway connected");

    let mut heartbeat = tokio::time::interval_at(
        Instant::now() + state.config.heartbeat_interval,
        state.config.heartbeat_interval,
    );
    heartbeat.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut last_seen = Instant::now();

    let result = loop {
        tokio::select! {
            outbound = receiver.recv() => {
                let Some(outbound) = outbound else {
                    break Ok(());
                };
                let replaced = matches!(
                    &outbound,
                    AgentGatewayMessage::Error { code, .. } if code == "SESSION_REPLACED"
                );
                if let Err(error) = send_message(socket, &outbound).await {
                    break Err(error);
                }
                if replaced {
                    break Ok(());
                }
            }
            incoming = receive_message(socket) => {
                let incoming = match incoming {
                    Ok(message) => message,
                    Err(error) if error == "agent gateway disconnected" => break Ok(()),
                    Err(error) => break Err(error),
                };
                last_seen = Instant::now();
                match incoming {
                    AgentGatewayMessage::Result { request_id, channel_id, output } => {
                        state.relay.complete_result(connection_id, request_id, channel_id, output).await;
                    }
                    AgentGatewayMessage::Error {
                        request_id: Some(request_id),
                        channel_id: Some(channel_id),
                        message,
                        ..
                    } => {
                        state.relay.complete_error(connection_id, request_id, channel_id, message).await;
                    }
                    AgentGatewayMessage::Heartbeat { session_id: received }
                    | AgentGatewayMessage::HeartbeatAck { session_id: received } => {
                        if received != session_id {
                            break Err("heartbeat session does not match this connection".to_string());
                        }
                        if let Err(error) = db::touch_agent_gateway_session(&state.pool, session_id).await {
                            warn!(error = %error, %session_id, "could not persist agent gateway heartbeat");
                        }
                        if matches!(incoming, AgentGatewayMessage::Heartbeat { .. }) {
                            send_message(socket, &AgentGatewayMessage::HeartbeatAck { session_id }).await?;
                        }
                    }
                    AgentGatewayMessage::Error { request_id: None, .. } => {}
                    _ => break Err("agent gateway sent an unexpected message".to_string()),
                }
            }
            _ = heartbeat.tick() => {
                if last_seen.elapsed() > state.config.heartbeat_interval.saturating_mul(3) {
                    break Err("agent gateway heartbeat timed out".to_string());
                }
                if let Err(error) = send_message(socket, &AgentGatewayMessage::Heartbeat { session_id }).await {
                    break Err(error);
                }
            }
        }
    };

    let removed_current = state.relay.unregister(&gateway_id, connection_id).await;
    if removed_current {
        if let Err(error) = db::close_agent_gateway_session(&state.pool, session_id).await {
            warn!(error = %error, %session_id, "could not persist agent gateway disconnect");
        }
    }
    info!(%gateway_id, %connection_id, %session_id, "agent gateway disconnected");
    result
}

async fn receive_message(socket: &mut WebSocket) -> Result<AgentGatewayMessage, String> {
    loop {
        match socket.next().await {
            Some(Ok(Message::Text(frame))) => return protocol::decode(frame.as_str()),
            Some(Ok(Message::Ping(payload))) => {
                socket
                    .send(Message::Pong(payload))
                    .await
                    .map_err(|error| error.to_string())?;
            }
            Some(Ok(Message::Pong(_))) => continue,
            Some(Ok(Message::Close(_))) | None => {
                return Err("agent gateway disconnected".to_string());
            }
            Some(Ok(Message::Binary(_))) => {
                return Err("binary agent gateway messages are not supported".to_string());
            }
            Some(Err(error)) => return Err(error.to_string()),
        }
    }
}

async fn send_message(socket: &mut WebSocket, message: &AgentGatewayMessage) -> Result<(), String> {
    let encoded = protocol::encode(message)?;
    socket
        .send(Message::Text(encoded.into()))
        .await
        .map_err(|error| error.to_string())
}

fn public_error(error: &str) -> String {
    let sanitized = error
        .chars()
        .take(256)
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    if sanitized.trim().is_empty() {
        json!({ "error": "invalid agent gateway message" }).to_string()
    } else {
        sanitized
    }
}
