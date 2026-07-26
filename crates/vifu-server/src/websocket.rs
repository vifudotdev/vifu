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
use vifu_gateway::gateway_frame;
use vifu_gateway::protocol::{self, AgentGatewayCommand};

use crate::auth::{bearer_token, hash_agent_gateway_credential};
use crate::db;
use crate::error::ApiError;
use crate::AppState;

pub async fn upgrade(
    State(state): State<AppState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let credential = bearer_token(&headers).ok_or(ApiError::Unauthorized)?;
    let credential_hash = hash_agent_gateway_credential(credential, &state.config.api_key_pepper);
    let gateway_id =
        db::authenticate_agent_gateway_credential(&state.pool, &credential_hash).await?;
    Ok(ws
        .max_message_size(gateway_frame::MAX_GATEWAY_FRAME_BYTES)
        .max_frame_size(gateway_frame::MAX_GATEWAY_FRAME_BYTES)
        .on_upgrade(move |socket| handle_socket(state, socket, gateway_id))
        .into_response())
}

async fn handle_socket(state: AppState, mut socket: WebSocket, gateway_id: String) {
    if let Err(error) = run_socket(&state, &mut socket, &gateway_id).await {
        warn!(error = %error, "agent gateway websocket closed with an error");
        let protocol_error = AgentGatewayCommand::Error {
            request_id: None,
            channel_id: None,
            code: "PROTOCOL_ERROR".to_string(),
            message: public_error(&error),
        };
        if let Ok(encoded) = encode_command(&protocol_error) {
            let _ = socket.send(Message::Text(encoded.into())).await;
        }
    }
    let _ = socket.close().await;
}

async fn run_socket(
    state: &AppState,
    socket: &mut WebSocket,
    gateway_id: &str,
) -> Result<(), String> {
    let hello = tokio::time::timeout(Duration::from_secs(5), receive_command(socket))
        .await
        .map_err(|_| "agent gateway did not send hello in time".to_string())??;
    let AgentGatewayCommand::Hello {
        protocol: _,
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
        gateway_id,
        resume_session_id,
        &agents_json,
        &metadata,
    )
    .await
    .map_err(|error| error.to_string())?;
    reconcile_project_agents(state, gateway_id, &agents)
        .await
        .map_err(|error| error.to_string())?;
    let connection_id = Uuid::new_v4();
    let (sender, mut receiver) = state.relay.channel();
    state
        .relay
        .register(gateway_id.to_string(), connection_id, session_id, sender)
        .await;

    let welcome = AgentGatewayCommand::Welcome {
        gateway_id: gateway_id.to_string(),
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
    send_command(socket, &welcome).await?;
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
                let disconnect = matches!(
                    &outbound,
                    AgentGatewayCommand::Error { code, .. }
                        if code == "SESSION_REPLACED" || code == "CREDENTIAL_REVOKED"
                );
                if let Err(error) = send_command(socket, &outbound).await {
                    break Err(error);
                }
                if disconnect {
                    break Ok(());
                }
            }
            incoming = receive_command(socket) => {
                let incoming = match incoming {
                    Ok(message) => message,
                    Err(error) if error == "agent gateway disconnected" => break Ok(()),
                    Err(error) => break Err(error),
                };
                last_seen = Instant::now();
                match incoming {
                    AgentGatewayCommand::Result { request_id, channel_id, output } => {
                        state.relay.complete_result(connection_id, request_id, channel_id, output).await;
                    }
                    AgentGatewayCommand::Error {
                        request_id: Some(request_id),
                        channel_id: Some(channel_id),
                        message,
                        ..
                    } => {
                        state.relay.complete_error(connection_id, request_id, channel_id, message).await;
                    }
                    AgentGatewayCommand::Heartbeat { session_id: received }
                    | AgentGatewayCommand::HeartbeatAck { session_id: received } => {
                        if received != session_id {
                            break Err("heartbeat session does not match this connection".to_string());
                        }
                        if let Err(error) = db::touch_agent_gateway_session(&state.pool, session_id).await {
                            warn!(error = %error, %session_id, "could not persist agent gateway heartbeat");
                        }
                        if matches!(incoming, AgentGatewayCommand::Heartbeat { .. }) {
                            send_command(socket, &AgentGatewayCommand::HeartbeatAck { session_id }).await?;
                        }
                    }
                    AgentGatewayCommand::Error { request_id: None, .. } => {}
                    _ => break Err("agent gateway sent an unexpected message".to_string()),
                }
            }
            _ = heartbeat.tick() => {
                if last_seen.elapsed() > state.config.heartbeat_interval.saturating_mul(3) {
                    break Err("agent gateway heartbeat timed out".to_string());
                }
                if let Err(error) = send_command(socket, &AgentGatewayCommand::Heartbeat { session_id }).await {
                    break Err(error);
                }
            }
        }
    };

    let removed_current = state.relay.unregister(gateway_id, connection_id).await;
    if removed_current {
        if let Err(error) = db::close_agent_gateway_session(&state.pool, session_id).await {
            warn!(error = %error, %session_id, "could not persist agent gateway disconnect");
        }
    }
    info!(%gateway_id, %connection_id, %session_id, "agent gateway disconnected");
    result
}

async fn reconcile_project_agents(
    state: &AppState,
    gateway_id: &str,
    agents: &[vifu_gateway::protocol::AgentDescriptor],
) -> Result<(), crate::error::ApiError> {
    for agent in agents {
        let Some(provider_key) = agent
            .metadata
            .get("providerKey")
            .and_then(serde_json::Value::as_str)
        else {
            continue;
        };
        for (project_id, _) in db::list_projects_for_provider_key(&state.pool, provider_key).await?
        {
            match db::find_project_profile_by_provider_resource(
                &state.pool,
                project_id,
                provider_key,
                &agent.id,
            )
            .await?
            {
                Some((_profile_id, _archived, binding_id)) => {
                    db::refresh_discovered_binding(
                        &state.pool,
                        binding_id,
                        gateway_id,
                        &agent.name,
                    )
                    .await?;
                }
                None => {
                    db::ensure_discovered_binding(
                        &state.pool,
                        project_id,
                        gateway_id,
                        &agent.id,
                        &agent.name,
                        provider_key,
                    )
                    .await?;
                }
            }
        }
    }
    Ok(())
}

async fn receive_command(socket: &mut WebSocket) -> Result<AgentGatewayCommand, String> {
    loop {
        match socket.next().await {
            Some(Ok(Message::Text(frame))) => return decode_command(frame.as_str()),
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

async fn send_command(socket: &mut WebSocket, message: &AgentGatewayCommand) -> Result<(), String> {
    let encoded = encode_command(message)?;
    socket
        .send(Message::Text(encoded.into()))
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
    use tokio::net::TcpListener;
    use tokio_tungstenite::connect_async;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::Message;
    use tower::ServiceExt;
    use uuid::Uuid;
    use vifu_gateway::gateway_frame::{
        self, GatewayFrame, RequestFrame, RequestFrameType, ResponseFrame, ResponseFrameType,
    };
    use vifu_gateway::protocol::{
        AgentGatewayCommand, AGENT_GATEWAY_HELLO_METHOD, AGENT_GATEWAY_HELLO_REQUEST_ID,
        AGENT_GATEWAY_INVOKE_METHOD, VERSION,
    };

    use super::{decode_command, encode_command};
    use crate::auth::hash_api_key;
    use crate::config::Config;
    use crate::db::{self, NewProject};
    use crate::models::{
        ApiKeyAgentScope, ApiKeyPermissions, EndpointPermission, ProfileCapabilityDraft,
        ResourcePermission,
    };
    use crate::{app, state_with_storage};

    #[test]
    fn server_transport_codec_round_trips_gateway_frames() {
        let command = AgentGatewayCommand::Welcome {
            gateway_id: "local-gateway".to_string(),
            connection_id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            heartbeat_interval_ms: 30_000,
            resumed: false,
        };
        let encoded = encode_command(&command).unwrap();
        let frame = gateway_frame::decode(&encoded).unwrap();
        let GatewayFrame::Response(response) = frame else {
            panic!("welcome must encode as a response frame");
        };

        assert_eq!(response.id, AGENT_GATEWAY_HELLO_REQUEST_ID);
        assert!(response.ok);
        assert_eq!(decode_command(&encoded).unwrap(), command);
    }

    #[test]
    fn server_transport_codec_rejects_invalid_frames() {
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
            "type": "res",
            "id": AGENT_GATEWAY_HELLO_REQUEST_ID,
            "ok": false,
            "error": null
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

    #[tokio::test]
    async fn agent_gateway_websocket_uses_frame_transport_for_invocations() {
        let Some(pool) = maybe_test_pool().await else {
            return;
        };
        let raw_api_key = "synthetic-project-api-key";
        let gateway_id = format!("wire-gateway-{}", Uuid::new_v4().simple());
        let seeded = seed_endpoint(&pool, raw_api_key, &gateway_id).await;
        let mut config = Config::from_env().unwrap();
        config.heartbeat_interval = std::time::Duration::from_secs(30);
        let bootstrap_token = config.agent_gateway_bootstrap_token.clone();
        let admin_key = config.admin_key.clone();
        let gateway_credential =
            "vifu_gw_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let state = state_with_storage(config, pool);
        let registration = app(state.clone())
            .oneshot(
                Request::post("/v1/agent-gateways/register")
                    .header(AUTHORIZATION, format!("Bearer {bootstrap_token}"))
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "gatewayId": gateway_id,
                            "credential": gateway_credential,
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(registration.status(), StatusCode::CREATED);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server_state = state.clone();
        let server = tokio::spawn(async move {
            axum::serve(listener, app(server_state)).await.unwrap();
        });

        let mut socket = connect_agent_gateway(addr, gateway_credential).await;
        send_gateway_frame(
            &mut socket,
            GatewayFrame::Request(RequestFrame {
                frame_type: RequestFrameType::Req,
                id: AGENT_GATEWAY_HELLO_REQUEST_ID.to_string(),
                method: AGENT_GATEWAY_HELLO_METHOD.to_string(),
                params: Some(json!({
                    "protocol": VERSION,
                    "agents": [
                        {
                            "id": "guide-agent",
                            "name": "Guide",
                            "metadata": {}
                        },
                        {
                            "id": "other-agent",
                            "name": "Other",
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
        assert_eq!(welcome["payload"]["gatewayId"], seeded.gateway_id);
        assert!(welcome["payload"]["sessionId"].is_string());
        assert!(welcome.get("type").is_some());
        assert!(welcome.get("method").is_none());

        let request = chat_completion_request(
            &seeded.project_slug,
            Some(&seeded.endpoint_slug),
            raw_api_key,
        );
        let invoke_state = state.clone();
        let invoke_task = tokio::spawn(async move {
            app(invoke_state)
                .oneshot(request)
                .await
                .expect("chat response")
        });

        let invoke = receive_json_frame(&mut socket).await;
        assert_eq!(invoke["type"], "req");
        assert_eq!(invoke["method"], AGENT_GATEWAY_INVOKE_METHOD);
        assert!(invoke["id"]
            .as_str()
            .is_some_and(|value| Uuid::parse_str(value).is_ok()));
        assert!(invoke.get("requestId").is_none());
        assert_eq!(invoke["params"]["channelId"], 1);
        assert_eq!(
            invoke["params"]["endpointId"],
            seeded.endpoint_id.to_string()
        );
        assert_eq!(invoke["params"]["agentId"], "guide-agent");
        assert_eq!(invoke["params"]["input"]["model"], seeded.endpoint_slug);
        assert_eq!(invoke["params"]["input"]["messages"][0]["role"], "user");
        assert_eq!(invoke["params"]["input"]["messages"][0]["content"], "Hello");

        complete_invocation(&mut socket, &invoke, "Hi from frame transport").await;

        let response = invoke_task.await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        let payload: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["object"], "chat.completion");
        assert_eq!(payload["model"], seeded.endpoint_slug);
        assert_eq!(
            payload["choices"][0]["message"]["content"],
            "Hi from frame transport"
        );
        let traces = db::list_traces(&state.pool, None, Some(seeded.project_id), 10)
            .await
            .unwrap();
        let trace = traces
            .iter()
            .find(|trace| trace.profile_id == Some(seeded.profile_id))
            .expect("project invocation trace");
        assert_eq!(trace.project_id, Some(seeded.project_id));

        let missing_model = app(state.clone())
            .oneshot(chat_completion_request(
                &seeded.project_slug,
                None,
                raw_api_key,
            ))
            .await
            .unwrap();
        assert_api_error(missing_model, StatusCode::BAD_REQUEST, "model_required").await;

        let outside_project = app(state.clone())
            .oneshot(chat_completion_request(
                &seeded.project_slug,
                Some("not-in-this-project"),
                raw_api_key,
            ))
            .await
            .unwrap();
        assert_api_error(
            outside_project,
            StatusCode::FORBIDDEN,
            "agent_access_denied",
        )
        .await;

        let denied_api_key = "synthetic-endpoint-denied-api-key";
        let denied_key_hash = hash_api_key(denied_api_key, &state.config.api_key_pepper);
        db::create_api_key(
            &state.pool,
            db::NewApiKey {
                id: Uuid::new_v4(),
                project_id: seeded.project_id,
                name: "Endpoint Denied Wire Test Key",
                agent_scope: &ApiKeyAgentScope::All,
                permissions: &ApiKeyPermissions {
                    chat_completions: EndpointPermission::None,
                    speech: EndpointPermission::None,
                    transcriptions: EndpointPermission::None,
                    realtime: EndpointPermission::None,
                    runtime: EndpointPermission::None,
                    agents: ResourcePermission::Read,
                    project: ResourcePermission::Read,
                },
                key_prefix: "denied-test",
                key_hash: &denied_key_hash,
            },
        )
        .await
        .unwrap();
        let denied_models = app(state.clone())
            .oneshot(project_models_request(&seeded.project_slug, denied_api_key))
            .await
            .unwrap();
        assert_api_error(
            denied_models,
            StatusCode::FORBIDDEN,
            "endpoint_access_denied",
        )
        .await;
        let denied_chat = app(state.clone())
            .oneshot(chat_completion_request(
                &seeded.project_slug,
                Some(&seeded.endpoint_slug),
                denied_api_key,
            ))
            .await
            .unwrap();
        assert_api_error(denied_chat, StatusCode::FORBIDDEN, "endpoint_access_denied").await;

        let selected_api_key = "synthetic-selected-project-api-key";
        let selected_key_hash = hash_api_key(selected_api_key, &state.config.api_key_pepper);
        db::create_api_key(
            &state.pool,
            db::NewApiKey {
                id: Uuid::new_v4(),
                project_id: seeded.project_id,
                name: "Selected Wire Test Key",
                agent_scope: &ApiKeyAgentScope::Selected {
                    profile_ids: vec![seeded.profile_id],
                },
                permissions: &ApiKeyPermissions::default(),
                key_prefix: "selected-test",
                key_hash: &selected_key_hash,
            },
        )
        .await
        .unwrap();
        let models = app(state.clone())
            .oneshot(project_models_request(
                &seeded.project_slug,
                selected_api_key,
            ))
            .await
            .unwrap();
        assert_eq!(models.status(), StatusCode::OK);
        let models_body = to_bytes(models.into_body(), 64 * 1024).await.unwrap();
        let models_payload: Value = serde_json::from_slice(&models_body).unwrap();
        assert_eq!(models_payload["data"].as_array().unwrap().len(), 1);
        assert_eq!(models_payload["data"][0]["id"], seeded.endpoint_slug);

        let selected_outside_scope = app(state.clone())
            .oneshot(chat_completion_request(
                &seeded.project_slug,
                Some(&seeded.other_endpoint_slug),
                selected_api_key,
            ))
            .await
            .unwrap();
        assert_api_error(
            selected_outside_scope,
            StatusCode::FORBIDDEN,
            "agent_access_denied",
        )
        .await;

        let selected_request = chat_completion_request(
            &seeded.project_slug,
            Some(&seeded.endpoint_slug),
            selected_api_key,
        );
        let selected_state = state.clone();
        let selected_task = tokio::spawn(async move {
            app(selected_state)
                .oneshot(selected_request)
                .await
                .expect("selected key chat response")
        });
        let selected_invoke = receive_json_frame(&mut socket).await;
        assert_eq!(selected_invoke["params"]["agentId"], "guide-agent");
        complete_invocation(&mut socket, &selected_invoke, "Hi from selected key").await;
        assert_eq!(selected_task.await.unwrap().status(), StatusCode::OK);

        let revocation = app(state.clone())
            .oneshot(
                Request::post(format!("/v1/agent-gateways/{}/revoke", seeded.gateway_id))
                    .header(AUTHORIZATION, format!("Bearer {admin_key}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(revocation.status(), StatusCode::OK);
        let revoked = receive_json_frame(&mut socket).await;
        assert_eq!(revoked["type"], "event");
        assert_eq!(revoked["event"], "gateway.error");
        assert_eq!(revoked["payload"]["code"], "CREDENTIAL_REVOKED");
        let reenrollment = app(state.clone())
            .oneshot(
                Request::post("/v1/agent-gateways/register")
                    .header(AUTHORIZATION, format!("Bearer {bootstrap_token}"))
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "gatewayId": seeded.gateway_id,
                            "credential": "vifu_gw_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_api_error(
            reenrollment,
            StatusCode::CONFLICT,
            "gateway_credential_revoked",
        )
        .await;

        let _ = socket.close(None).await;
        server.abort();
    }

    async fn maybe_test_pool() -> Option<db::Storage> {
        let database_url = if std::env::var("VIFU_TEST_DATABASE_REQUIRED").as_deref() == Ok("1") {
            "postgres://vifu@127.0.0.1:5432/vifu"
        } else {
            "sqlite::memory:"
        };
        let pool = match db::connect(database_url, 5).await {
            Ok(pool) => pool,
            Err(error) => {
                if std::env::var("VIFU_TEST_DATABASE_REQUIRED").as_deref() == Ok("1") {
                    panic!("agent gateway wire integration database unavailable: {error}");
                }
                eprintln!(
                    "skipping agent gateway wire integration test: database unavailable ({error})"
                );
                return None;
            }
        };
        if let Err(error) = db::migrate(&pool).await {
            if std::env::var("VIFU_TEST_DATABASE_REQUIRED").as_deref() == Ok("1") {
                panic!("agent gateway wire integration migration failed: {error}");
            }
            eprintln!("skipping agent gateway wire integration test: migration failed ({error})");
            return None;
        }
        Some(pool)
    }

    struct SeededProject {
        endpoint_id: Uuid,
        endpoint_slug: String,
        other_endpoint_slug: String,
        profile_id: Uuid,
        project_id: Uuid,
        project_slug: String,
        gateway_id: String,
    }

    async fn seed_endpoint(
        pool: &db::Storage,
        raw_api_key: &str,
        gateway_id: &str,
    ) -> SeededProject {
        let config = Config::from_env().unwrap();
        let profile_id = Uuid::new_v4();
        let binding_id = Uuid::new_v4();
        let other_profile_id = Uuid::new_v4();
        let other_binding_id = Uuid::new_v4();
        let project_id = Uuid::new_v4();
        let suffix = Uuid::new_v4().simple().to_string();
        let profile_slug = format!("wire-test-profile-{suffix}");
        let other_profile_slug = format!("wire-test-other-profile-{suffix}");
        let project_slug = format!("wire-test-project-{suffix}");
        db::create_project(
            pool,
            NewProject {
                id: project_id,
                slug: &project_slug,
                name: "Wire Test Project",
                description: None,
                gateway_id,
                binding_ids: &[],
            },
        )
        .await
        .unwrap();
        db::create_profile(
            pool,
            profile_id,
            project_id,
            &profile_slug,
            "Wire Test",
            None,
        )
        .await
        .unwrap();
        db::create_binding(
            pool,
            binding_id,
            profile_id,
            "openclaw",
            gateway_id,
            "guide-agent",
            &json!({}),
        )
        .await
        .unwrap();
        db::create_profile(
            pool,
            other_profile_id,
            project_id,
            &other_profile_slug,
            "Other Wire Test",
            None,
        )
        .await
        .unwrap();
        db::create_binding(
            pool,
            other_binding_id,
            other_profile_id,
            "openclaw",
            gateway_id,
            "other-agent",
            &json!({}),
        )
        .await
        .unwrap();
        let capability = ProfileCapabilityDraft {
            kind: "chat".to_string(),
            provider_type: "openclaw".to_string(),
            provider_key: "openclaw".to_string(),
            resource_id: Some("guide-agent".to_string()),
            config: json!({ "gatewayId": gateway_id }),
            input_schema: json!({}),
            output_schema: json!({}),
        };
        let version = db::create_profile_version(
            pool,
            profile_id,
            db::NewProfileVersion {
                persona: &json!({ "files": {} }),
                runtime: &json!({}),
                presentation: &json!({}),
                source: &json!({
                    "type": "openclaw",
                    "providerKey": "openclaw",
                    "gatewayId": gateway_id,
                    "resourceId": "guide-agent",
                    "managed": false,
                }),
                capabilities: &[capability],
                change_summary: Some("Wire test"),
            },
        )
        .await
        .unwrap();
        let endpoint_id = db::list_profile_capabilities(pool, version.id)
            .await
            .unwrap()[0]
            .id;
        let other_capability = ProfileCapabilityDraft {
            kind: "chat".to_string(),
            provider_type: "openclaw".to_string(),
            provider_key: "openclaw".to_string(),
            resource_id: Some("other-agent".to_string()),
            config: json!({ "gatewayId": gateway_id }),
            input_schema: json!({}),
            output_schema: json!({}),
        };
        db::create_profile_version(
            pool,
            other_profile_id,
            db::NewProfileVersion {
                persona: &json!({ "files": {} }),
                runtime: &json!({}),
                presentation: &json!({}),
                source: &json!({
                    "type": "openclaw",
                    "providerKey": "openclaw",
                    "gatewayId": gateway_id,
                    "resourceId": "other-agent",
                    "managed": false,
                }),
                capabilities: &[other_capability],
                change_summary: Some("Wire test"),
            },
        )
        .await
        .unwrap();
        db::attach_project_binding(pool, project_id, binding_id)
            .await
            .unwrap();
        db::attach_project_binding(pool, project_id, other_binding_id)
            .await
            .unwrap();
        let key_hash = hash_api_key(raw_api_key, &config.api_key_pepper);
        db::create_api_key(
            pool,
            db::NewApiKey {
                id: Uuid::new_v4(),
                project_id,
                name: "Wire Test Key",
                agent_scope: &ApiKeyAgentScope::All,
                permissions: &ApiKeyPermissions::default(),
                key_prefix: "test",
                key_hash: &key_hash,
            },
        )
        .await
        .unwrap();
        SeededProject {
            endpoint_id,
            endpoint_slug: profile_slug,
            other_endpoint_slug: other_profile_slug,
            profile_id,
            project_id,
            project_slug,
            gateway_id: gateway_id.to_string(),
        }
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

    fn chat_completion_request(
        project_slug: &str,
        model: Option<&str>,
        raw_api_key: &str,
    ) -> Request<Body> {
        let mut body = json!({
            "messages": [{ "role": "user", "content": "Hello" }],
            "stream": false
        });
        if let Some(model) = model {
            body.as_object_mut()
                .unwrap()
                .insert("model".to_string(), Value::String(model.to_string()));
        }
        Request::post(format!("/{project_slug}/v1/chat/completions"))
            .header(AUTHORIZATION, format!("Bearer {raw_api_key}"))
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    fn project_models_request(project_slug: &str, raw_api_key: &str) -> Request<Body> {
        Request::get(format!("/{project_slug}/v1/models"))
            .header(AUTHORIZATION, format!("Bearer {raw_api_key}"))
            .body(Body::empty())
            .unwrap()
    }

    async fn assert_api_error(response: axum::response::Response, status: StatusCode, code: &str) {
        assert_eq!(response.status(), status);
        let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        let payload: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["error"]["code"], code);
    }

    async fn complete_invocation<S>(
        socket: &mut tokio_tungstenite::WebSocketStream<S>,
        invoke: &Value,
        content: &str,
    ) where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        let request_id = invoke["id"].as_str().unwrap();
        let channel_id = invoke["params"]["channelId"].as_u64().unwrap();
        send_gateway_frame(
            socket,
            GatewayFrame::Response(ResponseFrame {
                frame_type: ResponseFrameType::Res,
                id: request_id.to_string(),
                ok: true,
                payload: Some(json!({
                    "channelId": channel_id,
                    "output": {
                        "id": "upstream-chatcmpl",
                        "object": "chat.completion",
                        "model": "openclaw/guide-agent",
                        "choices": [{
                            "index": 0,
                            "message": {
                                "role": "assistant",
                                "content": content
                            },
                            "finish_reason": "stop"
                        }]
                    }
                })),
                error: None,
            }),
        )
        .await;
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
