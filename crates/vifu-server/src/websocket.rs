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
use vifu_core::gateway_frame;
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
        .max_message_size(gateway_frame::MAX_GATEWAY_FRAME_BYTES)
        .max_frame_size(gateway_frame::MAX_GATEWAY_FRAME_BYTES)
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
        if let Ok(encoded) = encode_message(&protocol_error) {
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
            Some(Ok(Message::Text(frame))) => return decode_message(frame.as_str()),
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
    let encoded = encode_message(message)?;
    socket
        .send(Message::Text(encoded.into()))
        .await
        .map_err(|error| error.to_string())
}

fn decode_message(source: &str) -> Result<AgentGatewayMessage, String> {
    let frame = gateway_frame::decode(source)?;
    protocol::from_gateway_frame(frame)
}

fn encode_message(message: &AgentGatewayMessage) -> Result<String, String> {
    let frame = protocol::to_gateway_frame(message)?;
    gateway_frame::encode(&frame)
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

#[cfg(test)]
mod tests {
    use axum::body::{to_bytes, Body};
    use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
    use axum::http::{Request, StatusCode};
    use futures_util::{SinkExt, StreamExt};
    use serde_json::{json, Value};
    use sqlx::postgres::PgPoolOptions;
    use sqlx::PgPool;
    use tokio::net::TcpListener;
    use tokio_tungstenite::connect_async;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::Message;
    use tower::ServiceExt;
    use uuid::Uuid;
    use vifu_core::gateway_frame::{
        self, GatewayFrame, RequestFrame, RequestFrameType, ResponseFrame, ResponseFrameType,
    };
    use vifu_core::protocol::{
        AGENT_GATEWAY_HELLO_METHOD, AGENT_GATEWAY_HELLO_REQUEST_ID, AGENT_GATEWAY_INVOKE_METHOD,
        VERSION,
    };

    use crate::auth::hash_api_key;
    use crate::config::Config;
    use crate::db::{self, NewEndpoint};
    use crate::{app, state};

    #[tokio::test]
    async fn agent_gateway_websocket_uses_frame_transport_for_invocations() {
        let Some(pool) = maybe_test_pool().await else {
            return;
        };
        let raw_api_key = "test-endpoint-api-key";
        let endpoint_id = seed_endpoint(&pool, raw_api_key).await;
        let mut config = Config::from_env().unwrap();
        config.heartbeat_interval = std::time::Duration::from_secs(30);
        let agent_gateway_token = config.agent_gateway_token.clone();
        let state = state(config, pool);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server_state = state.clone();
        let server = tokio::spawn(async move {
            axum::serve(listener, app(server_state)).await.unwrap();
        });

        let mut socket = connect_agent_gateway(addr, &agent_gateway_token).await;
        send_gateway_frame(
            &mut socket,
            GatewayFrame::Request(RequestFrame {
                frame_type: RequestFrameType::Req,
                id: AGENT_GATEWAY_HELLO_REQUEST_ID.to_string(),
                method: AGENT_GATEWAY_HELLO_METHOD.to_string(),
                params: Some(json!({
                    "protocol": VERSION,
                    "gatewayId": "openclaw-local",
                    "agents": [
                        {
                            "id": "guide-agent",
                            "name": "Guide",
                            "metadata": {}
                        }
                    ],
                    "metadata": {
                        "adapter": "test"
                    }
                })),
            }),
        )
        .await;

        let welcome = receive_json_frame(&mut socket).await;
        assert_eq!(welcome["type"], "res");
        assert_eq!(welcome["id"], AGENT_GATEWAY_HELLO_REQUEST_ID);
        assert_eq!(welcome["ok"], true);
        assert!(welcome["payload"]["sessionId"].is_string());
        assert!(welcome.get("type").is_some());
        assert!(welcome.get("method").is_none());

        let request = invoke_endpoint_request(endpoint_id, raw_api_key);
        let invoke_task =
            tokio::spawn(
                async move { app(state).oneshot(request).await.expect("invoke response") },
            );

        let invoke = receive_json_frame(&mut socket).await;
        assert_eq!(invoke["type"], "req");
        assert_eq!(invoke["method"], AGENT_GATEWAY_INVOKE_METHOD);
        assert!(invoke["id"]
            .as_str()
            .is_some_and(|value| Uuid::parse_str(value).is_ok()));
        assert!(invoke.get("requestId").is_none());
        assert_eq!(invoke["params"]["channelId"], 1);
        assert_eq!(invoke["params"]["endpointId"], endpoint_id.to_string());
        assert_eq!(invoke["params"]["agentId"], "guide-agent");
        assert_eq!(invoke["params"]["input"]["message"], "Hello");

        let request_id = invoke["id"].as_str().unwrap();
        let channel_id = invoke["params"]["channelId"].as_u64().unwrap();
        send_gateway_frame(
            &mut socket,
            GatewayFrame::Response(ResponseFrame {
                frame_type: ResponseFrameType::Res,
                id: request_id.to_string(),
                ok: true,
                payload: Some(json!({
                    "channelId": channel_id,
                    "output": {
                        "text": "Hi from frame transport"
                    }
                })),
                error: None,
            }),
        )
        .await;

        let response = invoke_task.await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        let payload: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["endpointId"], endpoint_id.to_string());
        assert_eq!(payload["output"]["text"], "Hi from frame transport");

        let _ = socket.close(None).await;
        server.abort();
    }

    async fn maybe_test_pool() -> Option<PgPool> {
        let config = Config::from_env().unwrap();
        let pool = match PgPoolOptions::new()
            .max_connections(5)
            .connect(&config.database_url)
            .await
        {
            Ok(pool) => pool,
            Err(error) => {
                eprintln!(
                    "skipping agent gateway wire integration test: database unavailable ({error})"
                );
                return None;
            }
        };
        if let Err(error) = db::migrate(&pool).await {
            eprintln!("skipping agent gateway wire integration test: migration failed ({error})");
            return None;
        }
        Some(pool)
    }

    async fn seed_endpoint(pool: &PgPool, raw_api_key: &str) -> Uuid {
        let config = Config::from_env().unwrap();
        let profile_id = Uuid::new_v4();
        let binding_id = Uuid::new_v4();
        let endpoint_id = Uuid::new_v4();
        let suffix = Uuid::new_v4().simple().to_string();
        let profile_slug = format!("wire-test-profile-{suffix}");
        let endpoint_slug = format!("wire-test-endpoint-{suffix}");
        db::create_profile(pool, profile_id, &profile_slug, "Wire Test", None)
            .await
            .unwrap();
        db::create_binding(
            pool,
            binding_id,
            profile_id,
            "openclaw",
            "openclaw-local",
            "guide-agent",
            &json!({}),
        )
        .await
        .unwrap();
        db::create_endpoint(
            pool,
            NewEndpoint {
                id: endpoint_id,
                slug: &endpoint_slug,
                name: "Wire Test",
                profile_id,
                binding_id,
                enabled: true,
                request_timeout_ms: 30_000,
            },
        )
        .await
        .unwrap();
        let key_hash = hash_api_key(raw_api_key, &config.api_key_pepper);
        db::create_api_key(
            pool,
            Uuid::new_v4(),
            endpoint_id,
            "Wire Test Key",
            "test",
            &key_hash,
        )
        .await
        .unwrap();
        endpoint_id
    }

    async fn connect_agent_gateway(
        addr: std::net::SocketAddr,
        token: &str,
    ) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>
    {
        let mut request = format!("ws://{addr}/v1/agent-gateway/connect")
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert(AUTHORIZATION, format!("Bearer {token}").parse().unwrap());
        connect_async(request).await.unwrap().0
    }

    fn invoke_endpoint_request(endpoint_id: Uuid, raw_api_key: &str) -> Request<Body> {
        Request::post(format!("/v1/endpoints/{endpoint_id}/invoke"))
            .header(AUTHORIZATION, format!("Bearer {raw_api_key}"))
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from(json!({ "message": "Hello" }).to_string()))
            .unwrap()
    }

    async fn send_gateway_frame<S>(
        socket: &mut tokio_tungstenite::WebSocketStream<S>,
        frame: GatewayFrame,
    ) where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        let encoded = gateway_frame::encode(&frame).unwrap();
        socket.send(Message::Text(encoded.into())).await.unwrap();
    }

    async fn receive_json_frame<S>(socket: &mut tokio_tungstenite::WebSocketStream<S>) -> Value
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        loop {
            match socket.next().await.unwrap().unwrap() {
                Message::Text(value) => return serde_json::from_str(value.as_str()).unwrap(),
                Message::Ping(payload) => socket.send(Message::Pong(payload)).await.unwrap(),
                Message::Pong(_) => {}
                other => panic!("unexpected websocket message: {other:?}"),
            }
        }
    }
}
