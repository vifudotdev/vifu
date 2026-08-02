use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::gateway_frame::{
    self, ErrorShape, EventFrame, EventFrameType, GatewayFrame, RequestFrame, RequestFrameType,
    ResponseFrame, ResponseFrameType,
};

pub const VERSION: &str = "vifu.agent-gateway/1";
pub const MAX_FRAME_BYTES: usize = gateway_frame::MAX_GATEWAY_FRAME_BYTES;
pub const MAX_BODY_BYTES: usize = 512 * 1024;
pub const MAX_INVOCATION_BODY_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_PATH_BYTES: usize = 2 * 1024;
pub const MAX_AGENTS: usize = 256;

pub const AGENT_GATEWAY_HELLO_METHOD: &str = "gateway.hello";
pub const AGENT_GATEWAY_HELLO_REQUEST_ID: &str = "gateway.hello";
pub const AGENT_GATEWAY_CHALLENGE_EVENT: &str = "gateway.challenge";
pub const AGENT_GATEWAY_PAIRING_REQUIRED_EVENT: &str = "gateway.pairingRequired";
pub const AGENT_GATEWAY_INVOKE_METHOD: &str = "agent.invoke";
pub const AGENT_GATEWAY_CANCEL_EVENT: &str = "agent.cancel";
pub const AGENT_GATEWAY_HEARTBEAT_EVENT: &str = "gateway.heartbeat";
pub const AGENT_GATEWAY_HEARTBEAT_ACK_EVENT: &str = "gateway.heartbeatAck";
pub const AGENT_GATEWAY_ERROR_EVENT: &str = "gateway.error";
pub const RUNTIME_CONFIG_CHANGED_EVENT: &str = "runtime.config.changed";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDescriptor {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GatewayMachineProof {
    pub id: String,
    pub public_key: String,
    pub signature: String,
    pub signed_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GatewayHelloAuth {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GatewayWelcomeAuth {
    pub device_token: String,
    pub generation: u64,
    pub expires_at: String,
}

/// Internal semantic command for relay state machines.
///
/// The WebSocket wire contract is `GatewayFrame`; do not serialize this enum directly.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentGatewayCommand {
    Challenge {
        nonce: String,
        timestamp: u64,
        audience: String,
    },
    Hello {
        protocol: String,
        resume_session_id: Option<Uuid>,
        agents: Vec<AgentDescriptor>,
        metadata: Value,
        machine: GatewayMachineProof,
        auth: GatewayHelloAuth,
        followup: Option<String>,
    },
    Welcome {
        gateway_id: String,
        connection_id: Uuid,
        session_id: Uuid,
        heartbeat_interval_ms: u64,
        resumed: bool,
        auth: Option<GatewayWelcomeAuth>,
    },
    PairingRequired {
        request_id: Uuid,
        auth_url: String,
        retryable: bool,
        recommended_next_step: String,
        retry_after_ms: u64,
    },
    Invoke {
        request_id: Uuid,
        channel_id: u64,
        endpoint_id: Uuid,
        profile_id: Uuid,
        binding_id: Uuid,
        agent_id: String,
        binding: Value,
        input: Value,
        timeout_ms: u64,
    },
    Result {
        request_id: Uuid,
        channel_id: u64,
        output: Value,
    },
    Error {
        request_id: Option<Uuid>,
        channel_id: Option<u64>,
        code: String,
        message: String,
    },
    Cancel {
        request_id: Uuid,
        channel_id: u64,
    },
    Heartbeat {
        session_id: Uuid,
    },
    HeartbeatAck {
        session_id: Uuid,
    },
    RuntimeConfigChanged {
        deployment_ids: Vec<Uuid>,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HelloParams {
    protocol: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    resume_session_id: Option<Uuid>,
    agents: Vec<AgentDescriptor>,
    #[serde(default)]
    metadata: Value,
    machine: GatewayMachineProof,
    #[serde(default)]
    auth: GatewayHelloAuth,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    followup: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WelcomePayload {
    gateway_id: String,
    connection_id: Uuid,
    session_id: Uuid,
    heartbeat_interval_ms: u64,
    resumed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    auth: Option<GatewayWelcomeAuth>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ChallengePayload {
    nonce: String,
    timestamp: u64,
    audience: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PairingRequiredPayload {
    request_id: Uuid,
    auth_url: String,
    retryable: bool,
    recommended_next_step: String,
    retry_after_ms: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InvokeParams {
    channel_id: u64,
    endpoint_id: Uuid,
    profile_id: Uuid,
    binding_id: Uuid,
    agent_id: String,
    binding: Value,
    input: Value,
    timeout_ms: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InvokeResultPayload {
    channel_id: u64,
    output: Value,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ResponseErrorDetails {
    channel_id: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CancelPayload {
    request_id: Uuid,
    channel_id: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HeartbeatPayload {
    session_id: Uuid,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ErrorEventPayload {
    code: String,
    message: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RuntimeConfigChangedPayload {
    deployment_ids: Vec<Uuid>,
}

pub fn to_gateway_frame(command: &AgentGatewayCommand) -> Result<GatewayFrame, String> {
    validate_command(command)?;
    match command {
        AgentGatewayCommand::Challenge {
            nonce,
            timestamp,
            audience,
        } => event_frame(
            AGENT_GATEWAY_CHALLENGE_EVENT,
            &ChallengePayload {
                nonce: nonce.clone(),
                timestamp: *timestamp,
                audience: audience.clone(),
            },
        ),
        AgentGatewayCommand::Hello {
            protocol,
            resume_session_id,
            agents,
            metadata,
            machine,
            auth,
            followup,
        } => request_frame(
            AGENT_GATEWAY_HELLO_REQUEST_ID,
            AGENT_GATEWAY_HELLO_METHOD,
            &HelloParams {
                protocol: protocol.clone(),
                resume_session_id: *resume_session_id,
                agents: agents.clone(),
                metadata: metadata.clone(),
                machine: machine.clone(),
                auth: auth.clone(),
                followup: followup.clone(),
            },
        ),
        AgentGatewayCommand::Welcome {
            gateway_id,
            connection_id,
            session_id,
            heartbeat_interval_ms,
            resumed,
            auth,
        } => response_frame(
            AGENT_GATEWAY_HELLO_REQUEST_ID,
            &WelcomePayload {
                gateway_id: gateway_id.clone(),
                connection_id: *connection_id,
                session_id: *session_id,
                heartbeat_interval_ms: *heartbeat_interval_ms,
                resumed: *resumed,
                auth: auth.clone(),
            },
        ),
        AgentGatewayCommand::PairingRequired {
            request_id,
            auth_url,
            retryable,
            recommended_next_step,
            retry_after_ms,
        } => event_frame(
            AGENT_GATEWAY_PAIRING_REQUIRED_EVENT,
            &PairingRequiredPayload {
                request_id: *request_id,
                auth_url: auth_url.clone(),
                retryable: *retryable,
                recommended_next_step: recommended_next_step.clone(),
                retry_after_ms: *retry_after_ms,
            },
        ),
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
        } => request_frame(
            &request_id.to_string(),
            AGENT_GATEWAY_INVOKE_METHOD,
            &InvokeParams {
                channel_id: *channel_id,
                endpoint_id: *endpoint_id,
                profile_id: *profile_id,
                binding_id: *binding_id,
                agent_id: agent_id.clone(),
                binding: binding.clone(),
                input: input.clone(),
                timeout_ms: *timeout_ms,
            },
        ),
        AgentGatewayCommand::Result {
            request_id,
            channel_id,
            output,
        } => response_frame(
            &request_id.to_string(),
            &InvokeResultPayload {
                channel_id: *channel_id,
                output: output.clone(),
            },
        ),
        AgentGatewayCommand::Error {
            request_id: Some(request_id),
            channel_id: Some(channel_id),
            code,
            message,
        } => Ok(GatewayFrame::Response(ResponseFrame {
            frame_type: ResponseFrameType::Res,
            id: request_id.to_string(),
            ok: false,
            payload: None,
            error: Some(ErrorShape {
                code: code.clone(),
                message: message.clone(),
                details: Some(json!({ "channelId": channel_id })),
                retryable: None,
                retry_after_ms: None,
            }),
        })),
        AgentGatewayCommand::Error {
            request_id: None,
            channel_id: None,
            code,
            message,
        } => event_frame(
            AGENT_GATEWAY_ERROR_EVENT,
            &ErrorEventPayload {
                code: code.clone(),
                message: message.clone(),
            },
        ),
        AgentGatewayCommand::Error { .. } => {
            Err("request and channel ids must be provided together".to_string())
        }
        AgentGatewayCommand::Cancel {
            request_id,
            channel_id,
        } => event_frame(
            AGENT_GATEWAY_CANCEL_EVENT,
            &CancelPayload {
                request_id: *request_id,
                channel_id: *channel_id,
            },
        ),
        AgentGatewayCommand::Heartbeat { session_id } => event_frame(
            AGENT_GATEWAY_HEARTBEAT_EVENT,
            &HeartbeatPayload {
                session_id: *session_id,
            },
        ),
        AgentGatewayCommand::HeartbeatAck { session_id } => event_frame(
            AGENT_GATEWAY_HEARTBEAT_ACK_EVENT,
            &HeartbeatPayload {
                session_id: *session_id,
            },
        ),
        AgentGatewayCommand::RuntimeConfigChanged { deployment_ids } => event_frame(
            RUNTIME_CONFIG_CHANGED_EVENT,
            &RuntimeConfigChangedPayload {
                deployment_ids: deployment_ids.clone(),
            },
        ),
    }
}

pub fn from_gateway_frame(frame: GatewayFrame) -> Result<AgentGatewayCommand, String> {
    let command = match frame {
        GatewayFrame::Request(request) => from_request_frame(request)?,
        GatewayFrame::Response(response) => from_response_frame(response)?,
        GatewayFrame::Event(event) => from_event_frame(event)?,
    };
    validate_command(&command)?;
    Ok(command)
}

fn from_request_frame(request: RequestFrame) -> Result<AgentGatewayCommand, String> {
    match request.method.as_str() {
        AGENT_GATEWAY_HELLO_METHOD => {
            let params = decode_required::<HelloParams>(request.params, "gateway.hello params")?;
            Ok(AgentGatewayCommand::Hello {
                protocol: params.protocol,
                resume_session_id: params.resume_session_id,
                agents: params.agents,
                metadata: params.metadata,
                machine: params.machine,
                auth: params.auth,
                followup: params.followup,
            })
        }
        AGENT_GATEWAY_INVOKE_METHOD => {
            let request_id = parse_uuid("request id", &request.id)?;
            let params = decode_required::<InvokeParams>(request.params, "agent.invoke params")?;
            Ok(AgentGatewayCommand::Invoke {
                request_id,
                channel_id: params.channel_id,
                endpoint_id: params.endpoint_id,
                profile_id: params.profile_id,
                binding_id: params.binding_id,
                agent_id: params.agent_id,
                binding: params.binding,
                input: params.input,
                timeout_ms: params.timeout_ms,
            })
        }
        _ => Err(format!(
            "unsupported agent gateway request method: {}",
            request.method
        )),
    }
}

fn from_response_frame(response: ResponseFrame) -> Result<AgentGatewayCommand, String> {
    if response.id == AGENT_GATEWAY_HELLO_REQUEST_ID {
        if !response.ok {
            let error = response
                .error
                .ok_or_else(|| "gateway.hello error is required".to_string())?;
            return Ok(AgentGatewayCommand::Error {
                request_id: None,
                channel_id: None,
                code: error.code,
                message: error.message,
            });
        }
        let payload = decode_required::<WelcomePayload>(response.payload, "gateway.hello payload")?;
        return Ok(AgentGatewayCommand::Welcome {
            gateway_id: payload.gateway_id,
            connection_id: payload.connection_id,
            session_id: payload.session_id,
            heartbeat_interval_ms: payload.heartbeat_interval_ms,
            resumed: payload.resumed,
            auth: payload.auth,
        });
    }

    let request_id = parse_uuid("request id", &response.id)?;
    if response.ok {
        let payload =
            decode_required::<InvokeResultPayload>(response.payload, "agent.invoke payload")?;
        return Ok(AgentGatewayCommand::Result {
            request_id,
            channel_id: payload.channel_id,
            output: payload.output,
        });
    }

    let error = response
        .error
        .ok_or_else(|| "response error is required".to_string())?;
    let details =
        decode_required::<ResponseErrorDetails>(error.details, "agent.invoke error details")?;
    Ok(AgentGatewayCommand::Error {
        request_id: Some(request_id),
        channel_id: Some(details.channel_id),
        code: error.code,
        message: error.message,
    })
}

fn from_event_frame(event: EventFrame) -> Result<AgentGatewayCommand, String> {
    match event.event.as_str() {
        AGENT_GATEWAY_CHALLENGE_EVENT => {
            let payload =
                decode_required::<ChallengePayload>(event.payload, "gateway.challenge payload")?;
            Ok(AgentGatewayCommand::Challenge {
                nonce: payload.nonce,
                timestamp: payload.timestamp,
                audience: payload.audience,
            })
        }
        AGENT_GATEWAY_PAIRING_REQUIRED_EVENT => {
            let payload = decode_required::<PairingRequiredPayload>(
                event.payload,
                "gateway.pairingRequired payload",
            )?;
            Ok(AgentGatewayCommand::PairingRequired {
                request_id: payload.request_id,
                auth_url: payload.auth_url,
                retryable: payload.retryable,
                recommended_next_step: payload.recommended_next_step,
                retry_after_ms: payload.retry_after_ms,
            })
        }
        AGENT_GATEWAY_CANCEL_EVENT => {
            let payload = decode_required::<CancelPayload>(event.payload, "agent.cancel payload")?;
            Ok(AgentGatewayCommand::Cancel {
                request_id: payload.request_id,
                channel_id: payload.channel_id,
            })
        }
        AGENT_GATEWAY_HEARTBEAT_EVENT => {
            let payload =
                decode_required::<HeartbeatPayload>(event.payload, "gateway.heartbeat payload")?;
            Ok(AgentGatewayCommand::Heartbeat {
                session_id: payload.session_id,
            })
        }
        AGENT_GATEWAY_HEARTBEAT_ACK_EVENT => {
            let payload =
                decode_required::<HeartbeatPayload>(event.payload, "gateway.heartbeatAck payload")?;
            Ok(AgentGatewayCommand::HeartbeatAck {
                session_id: payload.session_id,
            })
        }
        AGENT_GATEWAY_ERROR_EVENT => {
            let payload =
                decode_required::<ErrorEventPayload>(event.payload, "gateway.error payload")?;
            Ok(AgentGatewayCommand::Error {
                request_id: None,
                channel_id: None,
                code: payload.code,
                message: payload.message,
            })
        }
        RUNTIME_CONFIG_CHANGED_EVENT => {
            let payload = decode_required::<RuntimeConfigChangedPayload>(
                event.payload,
                "runtime.config.changed payload",
            )?;
            Ok(AgentGatewayCommand::RuntimeConfigChanged {
                deployment_ids: payload.deployment_ids,
            })
        }
        _ => Err(format!("unsupported agent gateway event: {}", event.event)),
    }
}

fn request_frame<P>(id: &str, method: &str, params: &P) -> Result<GatewayFrame, String>
where
    P: Serialize,
{
    Ok(GatewayFrame::Request(RequestFrame {
        frame_type: RequestFrameType::Req,
        id: id.to_string(),
        method: method.to_string(),
        params: Some(serde_json::to_value(params).map_err(|error| error.to_string())?),
    }))
}

fn response_frame<P>(id: &str, payload: &P) -> Result<GatewayFrame, String>
where
    P: Serialize,
{
    Ok(GatewayFrame::Response(ResponseFrame {
        frame_type: ResponseFrameType::Res,
        id: id.to_string(),
        ok: true,
        payload: Some(serde_json::to_value(payload).map_err(|error| error.to_string())?),
        error: None,
    }))
}

fn event_frame<P>(event: &str, payload: &P) -> Result<GatewayFrame, String>
where
    P: Serialize,
{
    Ok(GatewayFrame::Event(EventFrame {
        frame_type: EventFrameType::Event,
        event: event.to_string(),
        payload: Some(serde_json::to_value(payload).map_err(|error| error.to_string())?),
        seq: None,
        state_version: None,
    }))
}

fn decode_required<T>(value: Option<Value>, name: &str) -> Result<T, String>
where
    T: DeserializeOwned,
{
    let value = value.ok_or_else(|| format!("{name} is required"))?;
    serde_json::from_value(value).map_err(|_| format!("invalid {name}"))
}

fn parse_uuid(name: &str, value: &str) -> Result<Uuid, String> {
    Uuid::parse_str(value).map_err(|_| format!("invalid {name}"))
}

pub fn validate_command(command: &AgentGatewayCommand) -> Result<(), String> {
    match command {
        AgentGatewayCommand::Challenge {
            nonce,
            timestamp,
            audience,
        } => {
            validate_token("challenge nonce", nonce, 32, 256)?;
            if *timestamp == 0 {
                return Err("challenge timestamp is required".to_string());
            }
            validate_text("challenge audience", audience, 1, 512)?;
            Ok(())
        }
        AgentGatewayCommand::Hello {
            protocol,
            resume_session_id: _,
            agents,
            metadata,
            machine,
            auth,
            followup,
        } => {
            if protocol != VERSION {
                return Err(format!("unsupported agent gateway protocol: {protocol}"));
            }
            if agents.len() > MAX_AGENTS {
                return Err("too many agent gateway agents".to_string());
            }
            for agent in agents {
                validate_identifier("agent id", &agent.id)?;
                validate_text("agent name", &agent.name, 1, 128)?;
                validate_json("agent metadata", &agent.metadata, 64 * 1024)?;
            }
            validate_json("agent gateway metadata", metadata, 64 * 1024)?;
            validate_identifier("Gateway machine id", &machine.id)?;
            validate_token("Gateway machine public key", &machine.public_key, 32, 256)?;
            validate_token("Gateway machine signature", &machine.signature, 32, 256)?;
            if machine.signed_at == 0 {
                return Err("Gateway machine signature timestamp is required".to_string());
            }
            if let Some(device_token) = auth.device_token.as_deref() {
                validate_token("Gateway device token", device_token, 48, 256)?;
            }
            if let Some(followup) = followup.as_deref() {
                validate_token("Gateway follow-up", followup, 16, 512)?;
            }
            Ok(())
        }
        AgentGatewayCommand::Welcome {
            gateway_id,
            heartbeat_interval_ms,
            auth,
            ..
        } => {
            validate_identifier("agent gateway id", gateway_id)?;
            if !(1_000..=60_000).contains(heartbeat_interval_ms) {
                return Err("invalid heartbeat interval".to_string());
            }
            if let Some(auth) = auth {
                validate_token("Gateway device token", &auth.device_token, 48, 256)?;
                if auth.generation == 0 || auth.expires_at.is_empty() || auth.expires_at.len() > 64
                {
                    return Err("invalid Gateway authorization update".to_string());
                }
            }
            Ok(())
        }
        AgentGatewayCommand::PairingRequired {
            auth_url,
            recommended_next_step,
            retry_after_ms,
            ..
        } => {
            validate_text("Gateway authorization URL", auth_url, 1, 2048)?;
            validate_token("Gateway pairing next step", recommended_next_step, 3, 64)?;
            if !(250..=60_000).contains(retry_after_ms) {
                return Err("invalid Gateway pairing retry interval".to_string());
            }
            Ok(())
        }
        AgentGatewayCommand::Invoke {
            channel_id,
            agent_id,
            binding,
            input,
            timeout_ms,
            ..
        } => {
            validate_channel(*channel_id)?;
            validate_identifier("agent id", agent_id)?;
            validate_json("binding", binding, 64 * 1024)?;
            validate_json("input", input, MAX_INVOCATION_BODY_BYTES)?;
            if !(500..=120_000).contains(timeout_ms) {
                return Err("invalid request timeout".to_string());
            }
            Ok(())
        }
        AgentGatewayCommand::Result {
            channel_id, output, ..
        } => {
            validate_channel(*channel_id)?;
            validate_json("output", output, MAX_INVOCATION_BODY_BYTES)
        }
        AgentGatewayCommand::Error {
            request_id,
            channel_id,
            code,
            message,
        } => {
            if request_id.is_some() != channel_id.is_some() {
                return Err("request and channel ids must be provided together".to_string());
            }
            if let Some(channel_id) = channel_id {
                validate_channel(*channel_id)?;
            }
            validate_code(code)?;
            validate_text("error message", message, 1, 2048)
        }
        AgentGatewayCommand::Cancel { channel_id, .. } => validate_channel(*channel_id),
        AgentGatewayCommand::Heartbeat { .. } | AgentGatewayCommand::HeartbeatAck { .. } => Ok(()),
        AgentGatewayCommand::RuntimeConfigChanged { deployment_ids } => {
            if deployment_ids.is_empty() || deployment_ids.len() > 256 {
                return Err("runtime configuration notification must name deployments".to_string());
            }
            let mut unique = std::collections::HashSet::with_capacity(deployment_ids.len());
            if deployment_ids.iter().all(|id| unique.insert(*id)) {
                Ok(())
            } else {
                Err("runtime configuration notification contains duplicates".to_string())
            }
        }
    }
}

pub fn gateway_signature_payload(
    server_origin: &str,
    nonce: &str,
    challenge_timestamp: u64,
    signed_at: u64,
    machine_id: &str,
    followup: Option<&str>,
    device_token: Option<&str>,
) -> Vec<u8> {
    use sha2::{Digest, Sha256};
    let token_hash = Sha256::digest(device_token.unwrap_or_default().as_bytes());
    format!(
        "{VERSION}\n{server_origin}\n{nonce}\n{challenge_timestamp}\n{signed_at}\n{machine_id}\ngateway\n{}\n{}",
        followup.unwrap_or_default(),
        hex(&token_hash),
    )
    .into_bytes()
}

fn validate_token(name: &str, value: &str, min: usize, max: usize) -> Result<(), String> {
    if !(min..=max).contains(&value.len())
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(format!("invalid {name}"));
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write;
        let _ = write!(value, "{byte:02x}");
    }
    value
}

pub fn validate_path(path: &str) -> Result<(), String> {
    if path.is_empty() || path.len() > MAX_PATH_BYTES {
        return Err("invalid request path length".to_string());
    }
    if !path.starts_with('/') || path.starts_with("//") {
        return Err("request path must be absolute-path only".to_string());
    }
    if path.contains("..") {
        return Err("request path must not contain parent traversal".to_string());
    }
    if path
        .bytes()
        .any(|byte| byte < 0x20 || byte == 0x7f || byte == b'\t')
    {
        return Err("request path contains control characters".to_string());
    }
    Ok(())
}

pub fn validate_method(method: &str) -> Result<(), String> {
    match method {
        "GET" | "POST" => Ok(()),
        _ => Err("only GET and POST requests are supported".to_string()),
    }
}

pub fn validate_body(body: &[u8]) -> Result<(), String> {
    if body.len() > MAX_BODY_BYTES {
        return Err("protocol body is too large".to_string());
    }
    Ok(())
}

pub fn validate_identifier(name: &str, value: &str) -> Result<(), String> {
    if value.len() < 3 || value.len() > 128 {
        return Err(format!("invalid {name} length"));
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(format!("invalid {name} characters"));
    }
    Ok(())
}

fn validate_channel(channel_id: u64) -> Result<(), String> {
    if channel_id == 0 {
        return Err("channel id must be greater than zero".to_string());
    }
    Ok(())
}

fn validate_code(value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 64 {
        return Err("invalid error code length".to_string());
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Err("invalid error code characters".to_string());
    }
    Ok(())
}

fn validate_text(name: &str, value: &str, min: usize, max: usize) -> Result<(), String> {
    if value.len() < min || value.len() > max || value.chars().any(|character| character == '\0') {
        return Err(format!("invalid {name}"));
    }
    Ok(())
}

fn validate_json(name: &str, value: &Value, max: usize) -> Result<(), String> {
    let size = serde_json::to_vec(value)
        .map_err(|_| format!("invalid {name}"))?
        .len();
    if size > max {
        return Err(format!("{name} is too large"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};
    use uuid::Uuid;

    use super::{
        from_gateway_frame, to_gateway_frame, AgentDescriptor, AgentGatewayCommand,
        GatewayHelloAuth, GatewayMachineProof, VERSION,
    };
    use crate::gateway_frame;

    #[test]
    fn round_trips_multiplexed_invoke_command_over_gateway_frame() {
        let command = AgentGatewayCommand::Invoke {
            request_id: Uuid::new_v4(),
            channel_id: 7,
            endpoint_id: Uuid::new_v4(),
            profile_id: Uuid::new_v4(),
            binding_id: Uuid::new_v4(),
            agent_id: "guide-agent".to_string(),
            binding: json!({}),
            input: json!({ "message": "Hello" }),
            timeout_ms: 30_000,
        };

        let value = round_trip_command_over_gateway_frame(&command);
        assert_eq!(value["type"], "req");
        assert_eq!(value["method"], "agent.invoke");
        assert_eq!(value["id"], command_request_id(&command).to_string());
        assert_eq!(value["params"]["channelId"], 7);
        assert!(value.get("requestId").is_none());
    }

    #[test]
    fn round_trips_multimodal_invoke_larger_than_the_http_proxy_body_limit() {
        let image = "A".repeat(super::MAX_BODY_BYTES + 1);
        let command = AgentGatewayCommand::Invoke {
            request_id: Uuid::new_v4(),
            channel_id: 7,
            endpoint_id: Uuid::new_v4(),
            profile_id: Uuid::new_v4(),
            binding_id: Uuid::new_v4(),
            agent_id: "vision-agent".to_string(),
            binding: json!({}),
            input: json!({
                "messages": [{
                    "role": "user",
                    "content": [{
                        "type": "image_url",
                        "image_url": {
                            "url": format!("data:image/png;base64,{image}")
                        }
                    }]
                }]
            }),
            timeout_ms: 30_000,
        };

        let value = round_trip_command_over_gateway_frame(&command);

        assert_eq!(value["method"], "agent.invoke");
    }

    #[test]
    fn round_trips_resume_hello_command_over_gateway_frame() {
        let command = AgentGatewayCommand::Hello {
            protocol: VERSION.to_string(),
            resume_session_id: Some(Uuid::new_v4()),
            agents: vec![AgentDescriptor {
                id: "default-agent".to_string(),
                name: "Default".to_string(),
                metadata: json!({}),
            }],
            metadata: json!({ "adapter": "openclaw" }),
            machine: machine_proof(),
            auth: GatewayHelloAuth::default(),
            followup: None,
        };
        let value = round_trip_command_over_gateway_frame(&command);
        assert_eq!(value["type"], "req");
        assert_eq!(value["id"], "gateway.hello");
        assert_eq!(value["method"], "gateway.hello");
        assert_eq!(value["params"]["protocol"], VERSION);
        assert!(value["params"].get("gatewayId").is_none());
    }

    #[test]
    fn encodes_results_and_errors_as_responses() {
        let request_id = Uuid::new_v4();
        let result = AgentGatewayCommand::Result {
            request_id,
            channel_id: 7,
            output: json!({ "text": "Hi" }),
        };
        let value = round_trip_command_over_gateway_frame(&result);
        assert_eq!(value["type"], "res");
        assert_eq!(value["id"], request_id.to_string());
        assert_eq!(value["ok"], true);
        assert_eq!(value["payload"]["channelId"], 7);

        let error = AgentGatewayCommand::Error {
            request_id: Some(request_id),
            channel_id: Some(7),
            code: "OPENCLAW_ERROR".to_string(),
            message: "failed".to_string(),
        };
        let value = round_trip_command_over_gateway_frame(&error);
        assert_eq!(value["type"], "res");
        assert_eq!(value["id"], request_id.to_string());
        assert_eq!(value["ok"], false);
        assert_eq!(value["error"]["details"]["channelId"], 7);
    }

    #[test]
    fn encodes_control_messages_as_events() {
        let request_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();

        let cancel = AgentGatewayCommand::Cancel {
            request_id,
            channel_id: 7,
        };
        let value = round_trip_command_over_gateway_frame(&cancel);
        assert_eq!(value["type"], "event");
        assert_eq!(value["event"], "agent.cancel");
        assert_eq!(value["payload"]["requestId"], request_id.to_string());

        let heartbeat = AgentGatewayCommand::Heartbeat { session_id };
        let value = round_trip_command_over_gateway_frame(&heartbeat);
        assert_eq!(value["type"], "event");
        assert_eq!(value["event"], "gateway.heartbeat");
        assert_eq!(value["payload"]["sessionId"], session_id.to_string());

        let deployment_id = Uuid::new_v4();
        let changed = AgentGatewayCommand::RuntimeConfigChanged {
            deployment_ids: vec![deployment_id],
        };
        let value = round_trip_command_over_gateway_frame(&changed);
        assert_eq!(value["type"], "event");
        assert_eq!(value["event"], "runtime.config.changed");
        assert_eq!(
            value["payload"]["deploymentIds"][0],
            deployment_id.to_string()
        );
    }

    #[test]
    fn rejects_zero_channel() {
        let command = AgentGatewayCommand::Cancel {
            request_id: Uuid::new_v4(),
            channel_id: 0,
        };
        assert!(to_gateway_frame(&command)
            .unwrap_err()
            .contains("channel id"));
    }

    #[test]
    fn rejects_unknown_protocol() {
        let command = AgentGatewayCommand::Hello {
            protocol: "vifu.agent-gateway/999".to_string(),
            resume_session_id: None,
            agents: Vec::new(),
            metadata: json!({}),
            machine: machine_proof(),
            auth: GatewayHelloAuth::default(),
            followup: None,
        };
        assert!(to_gateway_frame(&command)
            .unwrap_err()
            .contains("unsupported"));
    }

    fn round_trip_command_over_gateway_frame(command: &AgentGatewayCommand) -> Value {
        let frame = to_gateway_frame(command).unwrap();
        let encoded = gateway_frame::encode(&frame).unwrap();
        let value = serde_json::from_str::<Value>(&encoded).unwrap();
        let decoded_frame = gateway_frame::decode(&encoded).unwrap();
        assert_eq!(from_gateway_frame(decoded_frame).unwrap(), command.clone());
        value
    }

    fn machine_proof() -> GatewayMachineProof {
        GatewayMachineProof {
            id: format!("machine-{}", "a".repeat(64)),
            public_key: "a".repeat(43),
            signature: "b".repeat(86),
            signed_at: 42,
        }
    }

    fn command_request_id(command: &AgentGatewayCommand) -> Uuid {
        let AgentGatewayCommand::Invoke { request_id, .. } = command else {
            panic!("expected invoke command");
        };
        *request_id
    }
}
