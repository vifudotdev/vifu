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

use crate::auth::{hash_agent_gateway_credential, hash_agent_gateway_enrollment, is_secret_match};
use crate::db;
use crate::error::ApiError;
use crate::models::AgentGatewayAuthorization;
use crate::AppState;

const DEVICE_TOKEN_LIFETIME_DAYS: i64 = 180;
const DEVICE_TOKEN_ROTATION_WINDOW_DAYS: i64 = 30;
const PAIRING_LIFETIME_MINUTES: i64 = 10;

pub async fn upgrade(
    State(state): State<AppState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let audience = gateway_audience(&headers);
    Ok(ws
        .max_message_size(gateway_frame::MAX_GATEWAY_FRAME_BYTES)
        .max_frame_size(gateway_frame::MAX_GATEWAY_FRAME_BYTES)
        .on_upgrade(move |socket| handle_socket(state, socket, audience))
        .into_response())
}

async fn handle_socket(state: AppState, mut socket: WebSocket, audience: String) {
    if let Err(error) = run_socket(&state, &mut socket, &audience).await {
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
    audience: &str,
) -> Result<(), String> {
    let nonce = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let challenge_timestamp = unix_time_ms()?;
    send_command(
        socket,
        &AgentGatewayCommand::Challenge {
            nonce: nonce.clone(),
            timestamp: challenge_timestamp,
            audience: audience.to_string(),
        },
    )
    .await?;
    let hello = tokio::time::timeout(Duration::from_secs(5), receive_command(socket))
        .await
        .map_err(|_| "agent gateway did not send hello in time".to_string())??;
    let AgentGatewayCommand::Hello {
        protocol: _,
        resume_session_id,
        agents,
        metadata,
        machine,
        auth,
        followup,
    } = hello
    else {
        return Err("agent gateway must send hello first".to_string());
    };

    vifu_gateway::identity::validate_signed_at(unix_time_ms()?, machine.signed_at)?;
    let signature_payload = protocol::gateway_signature_payload(
        audience,
        &nonce,
        challenge_timestamp,
        machine.signed_at,
        &machine.id,
        followup.as_deref(),
        auth.device_token.as_deref(),
    );
    vifu_gateway::identity::verify_machine_signature(
        &machine.id,
        &machine.public_key,
        &machine.signature,
        &signature_payload,
    )?;
    db::upsert_agent_gateway_machine(&state.pool, &machine.id, &machine.public_key)
        .await
        .map_err(|error| error.to_string())?;

    let authorization = authorize_gateway_machine(
        state,
        &machine.id,
        auth.device_token.as_deref(),
        followup.as_deref(),
    )
    .await?;
    let (authorization, device_token) = match authorization {
        GatewayAuthorizationOutcome::Authorized {
            authorization,
            device_token,
        } => (authorization, device_token),
        GatewayAuthorizationOutcome::PairingRequired { request_id } => {
            send_command(
                socket,
                &AgentGatewayCommand::PairingRequired {
                    request_id,
                    auth_url: format!("/pair?request={request_id}"),
                    retryable: true,
                    recommended_next_step: "approve-in-dashboard".to_string(),
                    retry_after_ms: 2_000,
                },
            )
            .await?;
            return Ok(());
        }
    };
    let gateway_id = authorization.gateway_id.as_str();
    let application_feedback_supported =
        gateway_supports_feature(&metadata, protocol::APPLICATION_FEEDBACK_FEATURE);

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
        .register(
            gateway_id.to_string(),
            connection_id,
            session_id,
            sender,
            application_feedback_supported,
        )
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
        auth: device_token.map(|token| protocol::GatewayWelcomeAuth {
            device_token: token,
            generation: u64::try_from(authorization.token_generation).unwrap_or(1),
            expires_at: authorization.token_expires_at.to_rfc3339(),
        }),
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

fn gateway_supports_feature(metadata: &serde_json::Value, feature: &str) -> bool {
    metadata
        .get("features")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|features| features.iter().any(|value| value.as_str() == Some(feature)))
}

enum GatewayAuthorizationOutcome {
    Authorized {
        authorization: AgentGatewayAuthorization,
        device_token: Option<String>,
    },
    PairingRequired {
        request_id: Uuid,
    },
}

async fn authorize_gateway_machine(
    state: &AppState,
    machine_id: &str,
    device_token: Option<&str>,
    followup: Option<&str>,
) -> Result<GatewayAuthorizationOutcome, String> {
    let mut authorization =
        db::get_agent_gateway_authorization_for_machine(&state.pool, machine_id)
            .await
            .map_err(|error| error.to_string())?;
    let mut approved_owner = None;
    let mut explicitly_approved = false;
    let mut preferred_gateway_id = None;

    if let Some(enrollment_token) = followup.filter(|value| value.starts_with("vifu_ge_")) {
        let gateway_id = authorization
            .as_ref()
            .map(|value| value.gateway_id.clone())
            .unwrap_or_else(new_gateway_id);
        let enrollment_hash =
            hash_agent_gateway_enrollment(enrollment_token, &state.config.api_key_pepper);
        let assignment = db::consume_agent_gateway_machine_enrollment(
            &state.pool,
            &enrollment_hash,
            &gateway_id,
        )
        .await
        .map_err(|error| error.to_string())?;
        approved_owner = Some(assignment.owner_user_id);
        explicitly_approved = true;
        preferred_gateway_id = Some(gateway_id);
    } else if let Some(pairing_id) = followup.and_then(|value| Uuid::parse_str(value).ok()) {
        if let Ok(pairing) =
            db::consume_agent_gateway_pairing(&state.pool, pairing_id, machine_id).await
        {
            approved_owner = pairing.owner_user_id;
            explicitly_approved = true;
        }
    } else if followup
        .is_some_and(|value| is_secret_match(value, &state.config.agent_gateway_bootstrap_token))
    {
        explicitly_approved = true;
    }

    if !explicitly_approved {
        if let Some(pairing) =
            db::consume_approved_agent_gateway_pairing_for_machine(&state.pool, machine_id)
                .await
                .map_err(|error| error.to_string())?
        {
            approved_owner = pairing.owner_user_id;
            explicitly_approved = true;
        }
    }

    if authorization.is_none() {
        if explicitly_approved || state.config.guest_bootstrap_enabled {
            let issued = issue_device_token(&state.config.api_key_pepper);
            let gateway_id = preferred_gateway_id.unwrap_or_else(new_gateway_id);
            let created = db::create_agent_gateway_authorization(
                &state.pool,
                db::NewAgentGatewayAuthorization {
                    gateway_id: &gateway_id,
                    machine_id,
                    owner_user_id: approved_owner.as_deref(),
                    token_prefix: &issued.prefix,
                    token_hash: &issued.hash,
                    token_expires_at: issued.expires_at,
                },
            )
            .await
            .map_err(|error| error.to_string())?;
            return Ok(GatewayAuthorizationOutcome::Authorized {
                authorization: created,
                device_token: Some(issued.raw),
            });
        }
        return pending_pairing(state, machine_id).await;
    }

    let current = authorization
        .take()
        .expect("authorization was checked above");
    if let Some(owner_user_id) = approved_owner.as_deref() {
        authorization = Some(
            db::claim_agent_gateway_authorization_owner(
                &state.pool,
                &current.gateway_id,
                owner_user_id,
            )
            .await
            .map_err(|error| error.to_string())?,
        );
    } else {
        authorization = Some(current);
    }
    let current = authorization.expect("authorization was assigned above");

    if current.status == "revoked" && !explicitly_approved {
        return pending_pairing(state, machine_id).await;
    }

    if current.status == "active" && !explicitly_approved {
        if let Some(device_token) = device_token {
            let token_hash =
                hash_agent_gateway_credential(device_token, &state.config.api_key_pepper);
            let authenticated =
                db::authenticate_agent_gateway_device_token(&state.pool, &token_hash)
                    .await
                    .is_ok_and(|gateway_id| gateway_id == current.gateway_id);
            if !authenticated {
                return pending_pairing(state, machine_id).await;
            }
            if current.token_expires_at
                > chrono::Utc::now() + chrono::Duration::days(DEVICE_TOKEN_ROTATION_WINDOW_DAYS)
            {
                return Ok(GatewayAuthorizationOutcome::Authorized {
                    authorization: current,
                    device_token: None,
                });
            }
        } else {
            return pending_pairing(state, machine_id).await;
        }
    }

    let issued = issue_device_token(&state.config.api_key_pepper);
    let rotated = db::rotate_agent_gateway_authorization(
        &state.pool,
        db::RotatedAgentGatewayAuthorization {
            gateway_id: &current.gateway_id,
            token_prefix: &issued.prefix,
            token_hash: &issued.hash,
            token_expires_at: issued.expires_at,
        },
    )
    .await
    .map_err(|error| error.to_string())?;
    Ok(GatewayAuthorizationOutcome::Authorized {
        authorization: rotated,
        device_token: Some(issued.raw),
    })
}

async fn pending_pairing(
    state: &AppState,
    machine_id: &str,
) -> Result<GatewayAuthorizationOutcome, String> {
    let pairing = db::create_or_get_agent_gateway_pairing(
        &state.pool,
        machine_id,
        chrono::Utc::now() + chrono::Duration::minutes(PAIRING_LIFETIME_MINUTES),
    )
    .await
    .map_err(|error| error.to_string())?;
    Ok(GatewayAuthorizationOutcome::PairingRequired {
        request_id: pairing.id,
    })
}

struct IssuedDeviceToken {
    raw: String,
    prefix: String,
    hash: Vec<u8>,
    expires_at: chrono::DateTime<chrono::Utc>,
}

fn issue_device_token(pepper: &str) -> IssuedDeviceToken {
    let raw = format!(
        "vifu_gw_{}{}",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    );
    IssuedDeviceToken {
        prefix: raw.chars().take(20).collect(),
        hash: hash_agent_gateway_credential(&raw, pepper),
        raw,
        expires_at: chrono::Utc::now() + chrono::Duration::days(DEVICE_TOKEN_LIFETIME_DAYS),
    }
}

fn new_gateway_id() -> String {
    format!("gateway-{}", Uuid::new_v4().simple())
}

fn gateway_audience(headers: &HeaderMap) -> String {
    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .filter(|value| matches!(*value, "http" | "https"))
        .unwrap_or("http");
    let host = headers
        .get("host")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("vifu.local");
    format!("{scheme}://{host}/v1/agent-gateway/connect")
}

fn unix_time_ms() -> Result<u64, String> {
    use std::time::{SystemTime, UNIX_EPOCH};
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system clock is before the Unix epoch".to_string())?;
    u64::try_from(duration.as_millis()).map_err(|_| "system clock is invalid".to_string())
}

async fn reconcile_project_agents(
    state: &AppState,
    gateway_id: &str,
    agents: &[vifu_gateway::protocol::AgentDescriptor],
) -> Result<(), crate::error::ApiError> {
    let gateway_projects = db::list_projects_for_gateway(&state.pool, gateway_id).await?;
    for agent in agents {
        let Some(provider_key) = agent
            .metadata
            .get("providerKey")
            .and_then(serde_json::Value::as_str)
        else {
            continue;
        };
        let provider_type = agent
            .metadata
            .get("providerType")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("openclaw");
        let mut projects = gateway_projects.clone();
        projects.extend(db::list_projects_for_provider_key(&state.pool, provider_key).await?);
        projects.sort_unstable_by_key(|(project_id, _)| *project_id);
        projects.dedup_by_key(|(project_id, _)| *project_id);
        for (project_id, _) in projects {
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
                        provider_type,
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
    use vifu_gateway::identity::MachineIdentity;
    use vifu_gateway::protocol::{
        self, AgentDescriptor, AgentGatewayCommand, AGENT_GATEWAY_HELLO_METHOD,
        AGENT_GATEWAY_HELLO_REQUEST_ID, AGENT_GATEWAY_INVOKE_METHOD, VERSION,
    };

    use super::{
        authorize_gateway_machine, decode_command, encode_command, reconcile_project_agents,
        GatewayAuthorizationOutcome,
    };
    use crate::auth::hash_api_key;
    use crate::config::Config;
    use crate::db::{self, NewEndpoint, NewProject};
    use crate::error::ApiError;
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
            auth: None,
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
    async fn gateway_owned_project_adds_discovered_agents_without_provider_setup() {
        let Some(pool) = maybe_test_pool().await else {
            return;
        };
        let gateway_id = format!("guest-gateway-{}", Uuid::new_v4().simple());
        let project_id = Uuid::new_v4();
        let project_slug = format!("guest-project-{}", Uuid::new_v4().simple());
        db::create_project(
            &pool,
            NewProject {
                id: project_id,
                owner_user_id: None,
                slug: &project_slug,
                name: "Guest project",
                description: None,
                gateway_id: &gateway_id,
                binding_ids: &[],
            },
        )
        .await
        .unwrap();
        let state = state_with_storage(Config::from_env().unwrap(), pool);

        reconcile_project_agents(
            &state,
            &gateway_id,
            &[AgentDescriptor {
                id: "guide-agent".to_string(),
                name: "Guide".to_string(),
                metadata: json!({
                    "providerKey": "openclaw-local",
                    "providerType": "openclaw"
                }),
            }],
        )
        .await
        .unwrap();

        let profiles = db::list_project_profiles(&state.pool, project_id)
            .await
            .unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].name, "Guide");
        assert!(
            !db::project_provider_is_assigned(&state.pool, project_id, "openclaw-local")
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn unknown_machine_can_be_approved_through_pairing() {
        let Some(pool) = maybe_test_pool().await else {
            return;
        };
        let state = state_with_storage(Config::from_env().unwrap(), pool);
        let machine = MachineIdentity::generate().unwrap();
        db::upsert_agent_gateway_machine(&state.pool, &machine.machine_id, &machine.public_key)
            .await
            .unwrap();

        let request_id = match authorize_gateway_machine(&state, &machine.machine_id, None, None)
            .await
            .unwrap()
        {
            GatewayAuthorizationOutcome::PairingRequired { request_id } => request_id,
            GatewayAuthorizationOutcome::Authorized { .. } => {
                panic!("an unknown machine must require pairing")
            }
        };
        db::resolve_agent_gateway_pairing(
            &state.pool,
            request_id,
            "approved",
            Some("user-pairing-owner"),
        )
        .await
        .unwrap();

        let (authorization, device_token) = match authorize_gateway_machine(
            &state,
            &machine.machine_id,
            None,
            Some(&request_id.to_string()),
        )
        .await
        .unwrap()
        {
            GatewayAuthorizationOutcome::Authorized {
                authorization,
                device_token,
            } => (authorization, device_token.expect("new Device Token")),
            GatewayAuthorizationOutcome::PairingRequired { .. } => {
                panic!("approved pairing must authorize the machine")
            }
        };
        assert_eq!(
            authorization.owner_user_id.as_deref(),
            Some("user-pairing-owner")
        );
        let token_hash =
            crate::auth::hash_agent_gateway_credential(&device_token, &state.config.api_key_pepper);
        assert_eq!(
            db::authenticate_agent_gateway_device_token(&state.pool, &token_hash)
                .await
                .unwrap(),
            authorization.gateway_id
        );
        assert!(matches!(
            db::consume_agent_gateway_pairing(&state.pool, request_id, &machine.machine_id,).await,
            Err(ApiError::Unauthorized)
        ));

        let missing_token_request =
            match authorize_gateway_machine(&state, &machine.machine_id, None, None)
                .await
                .unwrap()
            {
                GatewayAuthorizationOutcome::PairingRequired { request_id } => request_id,
                GatewayAuthorizationOutcome::Authorized { .. } => {
                    panic!("an authorized machine still requires its Device Token")
                }
            };
        let wrong_token = format!("vifu_gw_{}", "b".repeat(64));
        assert!(matches!(
            authorize_gateway_machine(
                &state,
                &machine.machine_id,
                Some(&wrong_token),
                None,
            )
            .await
            .unwrap(),
            GatewayAuthorizationOutcome::PairingRequired { request_id }
                if request_id == missing_token_request
        ));
        assert!(matches!(
            authorize_gateway_machine(&state, &machine.machine_id, Some(&device_token), None,)
                .await
                .unwrap(),
            GatewayAuthorizationOutcome::Authorized {
                device_token: None,
                ..
            }
        ));
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
        let admin_key = config.admin_key.clone();
        let gateway_credential =
            "vifu_gw_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let state = state_with_storage(config, pool);
        let machine = MachineIdentity::generate().unwrap();
        db::upsert_agent_gateway_machine(&state.pool, &machine.machine_id, &machine.public_key)
            .await
            .unwrap();
        let token_hash = crate::auth::hash_agent_gateway_credential(
            gateway_credential,
            &state.config.api_key_pepper,
        );
        db::create_agent_gateway_authorization(
            &state.pool,
            db::NewAgentGatewayAuthorization {
                gateway_id: &gateway_id,
                machine_id: &machine.machine_id,
                owner_user_id: None,
                token_prefix: &gateway_credential.chars().take(20).collect::<String>(),
                token_hash: &token_hash,
                token_expires_at: chrono::Utc::now() + chrono::Duration::days(180),
            },
        )
        .await
        .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server_state = state.clone();
        let server = tokio::spawn(async move {
            axum::serve(listener, app(server_state)).await.unwrap();
        });

        let mut socket = connect_agent_gateway(addr).await;
        let challenge = receive_json_frame(&mut socket).await;
        assert_eq!(challenge["type"], "event");
        assert_eq!(challenge["event"], protocol::AGENT_GATEWAY_CHALLENGE_EVENT);
        let nonce = challenge["payload"]["nonce"].as_str().unwrap();
        let timestamp = challenge["payload"]["timestamp"].as_u64().unwrap();
        let audience = challenge["payload"]["audience"].as_str().unwrap();
        let signed_at = super::unix_time_ms().unwrap();
        let signature = machine
            .sign(&protocol::gateway_signature_payload(
                audience,
                nonce,
                timestamp,
                signed_at,
                &machine.machine_id,
                None,
                Some(gateway_credential),
            ))
            .unwrap();
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
                    },
                    "machine": {
                        "id": machine.machine_id,
                        "publicKey": machine.public_key,
                        "signature": signature,
                        "signedAt": signed_at
                    },
                    "auth": {
                        "deviceToken": gateway_credential
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
        let traces = db::list_traces(
            &state.pool,
            db::TraceListOptions {
                endpoint_id: None,
                project_id: Some(seeded.project_id),
                request_id: None,
                trace_id: None,
                allowed_profile_ids: None,
                created_from: None,
                created_before: None,
                cursor: None,
                limit: 10,
            },
        )
        .await
        .unwrap();
        let trace = traces
            .iter()
            .find(|trace| trace.profile_id == Some(seeded.profile_id))
            .expect("project invocation trace");
        assert_eq!(trace.project_id, Some(seeded.project_id));

        let generic_state = state.clone();
        let generic_model = seeded.endpoint_slug.clone();
        let generic_admin_key = admin_key.clone();
        let generic_task = tokio::spawn(async move {
            app(generic_state)
                .oneshot(
                    Request::post("/v1/chat/completions")
                        .header(CONTENT_TYPE, "application/json")
                        .header(AUTHORIZATION, format!("Bearer {generic_admin_key}"))
                        .body(Body::from(
                            json!({
                                "model": generic_model,
                                "messages": [{"role": "user", "content": "Root trace"}],
                                "stream": false,
                            })
                            .to_string(),
                        ))
                        .unwrap(),
                )
                .await
                .expect("generic chat response")
        });
        let generic_invoke = receive_json_frame(&mut socket).await;
        complete_invocation(&mut socket, &generic_invoke, "Root observation recorded").await;
        let generic_response = generic_task.await.unwrap();
        assert_eq!(generic_response.status(), StatusCode::OK);
        let generic_request_id = generic_response
            .headers()
            .get("x-vifu-invocation-id")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| Uuid::parse_str(value).ok())
            .expect("canonical invocation id header");
        let generic_traces = db::list_traces(
            &state.pool,
            db::TraceListOptions {
                endpoint_id: None,
                project_id: None,
                request_id: Some(generic_request_id),
                trace_id: None,
                allowed_profile_ids: None,
                created_from: None,
                created_before: None,
                cursor: None,
                limit: 1,
            },
        )
        .await
        .unwrap();
        assert_eq!(generic_traces.len(), 1);
        let generic_spans = db::list_trace_spans(&state.pool, generic_traces[0].id)
            .await
            .unwrap();
        let root = generic_spans
            .iter()
            .find(|span| span.id == generic_request_id)
            .expect("generic endpoint trace root");
        assert_eq!(root.parent_span_id, None);
        assert_eq!(root.observation_type, "generation");
        assert_eq!(root.status, "completed");
        assert_eq!(root.input_summary.as_ref().unwrap()["messageCount"], 1);
        assert_eq!(root.output_summary.as_ref().unwrap()["choiceCount"], 1);

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
                    embeddings: EndpointPermission::None,
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
        let authorization = db::get_agent_gateway_authorization(&state.pool, &seeded.gateway_id)
            .await
            .unwrap();
        assert_eq!(authorization.status, "revoked");

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
                owner_user_id: None,
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
        db::create_endpoint(
            pool,
            NewEndpoint {
                id: Uuid::new_v4(),
                slug: &profile_slug,
                name: "Wire Test Legacy Endpoint",
                profile_id,
                binding_id,
                enabled: true,
                request_timeout_ms: 30_000,
            },
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
    ) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>
    {
        let request = format!("ws://{addr}/v1/agent-gateway/connect")
            .into_client_request()
            .unwrap();
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
