use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;
use vifu_runtime::ProviderStage;

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
pub const MAX_TRACE_DURATION_MS: u64 = 24 * 60 * 60 * 1_000;
pub const MAX_TRACE_TOKEN_COUNT: u64 = 1_000_000_000;
pub const MAX_TRACE_TELEMETRY_EVENTS: usize = 32;
pub const MAX_TRACE_DROPPED_EVENTS: u32 = 10_000;
pub const MAX_TRACE_IO_SUMMARY_BYTES: usize = 8 * 1024;

const MAX_TRACE_IO_DEPTH: usize = 5;
const MAX_TRACE_IO_ITEMS: usize = 16;
const MAX_TRACE_IO_STRING_CHARS: usize = 512;

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
pub const APPLICATION_FEEDBACK_EVENT: &str = "trace.applicationFeedback";
pub const APPLICATION_FEEDBACK_FEATURE: &str = "trace.application-feedback.v1";

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TraceStageStatus {
    Started,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TraceDeliveryStatus {
    Delivered,
    Failed,
}

/// Payload-safe telemetry emitted by an Agent Gateway. It intentionally carries
/// only resolved identity, timings, counters, and a bounded public error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum TraceTelemetry {
    InvocationStarted {
        provider_key: String,
        capability: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
    },
    ProviderStage {
        observation_id: Uuid,
        stage: ProviderStage,
        status: TraceStageStatus,
        start_offset_ms: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        end_offset_ms: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        elapsed_ms: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_elapsed_ms: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input_tokens: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output_tokens: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resident: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    Delivery {
        observation_id: Uuid,
        status: TraceDeliveryStatus,
        start_offset_ms: u64,
        end_offset_ms: u64,
        elapsed_ms: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ApplicationFeedbackEvent {
    OutputAccepted,
    ActionApplied,
    FramePresented,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ApplicationFeedbackOutcome {
    Pass,
    Fail,
    Unknown,
    NotApplicable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApplicationFeedback {
    pub event: ApplicationFeedbackEvent,
    pub outcome: ApplicationFeedbackOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TraceIoSummary {
    pub value: Value,
    pub truncated: bool,
}

impl TraceIoSummary {
    pub fn effective_truncated(&self) -> bool {
        self.truncated || trace_io_summary_requires_truncation(&self.value)
    }
}

/// Produces the single bounded, redacted I/O representation used by both the
/// live runtime monitor and persisted root Generation observations.
pub fn canonical_trace_io_summary(value: &Value) -> TraceIoSummary {
    let mut truncated = false;
    let mut summary = redact_trace_io_value(value, 0, None, &mut truncated);
    if serde_json::to_vec(&summary)
        .map_or(true, |encoded| encoded.len() > MAX_TRACE_IO_SUMMARY_BYTES)
    {
        truncated = true;
        summary = json!({
            "summary": trace_value_shape(value),
            "truncated": true,
        });
    }
    TraceIoSummary {
        value: summary,
        truncated,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TraceTelemetryBatch {
    pub events: Vec<TraceTelemetry>,
    #[serde(default)]
    pub dropped_events: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_input_summary: Option<TraceIoSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_output_summary: Option<TraceIoSummary>,
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
    ApplicationFeedback {
        request_id: Uuid,
        observation_id: Uuid,
        start_offset_ms: u64,
        end_offset_ms: u64,
        feedback: ApplicationFeedback,
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ApplicationFeedbackPayload {
    request_id: Uuid,
    observation_id: Uuid,
    start_offset_ms: u64,
    end_offset_ms: u64,
    feedback: ApplicationFeedback,
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
        AgentGatewayCommand::ApplicationFeedback {
            request_id,
            observation_id,
            start_offset_ms,
            end_offset_ms,
            feedback,
        } => event_frame(
            APPLICATION_FEEDBACK_EVENT,
            &ApplicationFeedbackPayload {
                request_id: *request_id,
                observation_id: *observation_id,
                start_offset_ms: *start_offset_ms,
                end_offset_ms: *end_offset_ms,
                feedback: feedback.clone(),
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
        APPLICATION_FEEDBACK_EVENT => {
            let payload = decode_required::<ApplicationFeedbackPayload>(
                event.payload,
                "trace.applicationFeedback payload",
            )?;
            Ok(AgentGatewayCommand::ApplicationFeedback {
                request_id: payload.request_id,
                observation_id: payload.observation_id,
                start_offset_ms: payload.start_offset_ms,
                end_offset_ms: payload.end_offset_ms,
                feedback: payload.feedback,
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
        AgentGatewayCommand::ApplicationFeedback {
            start_offset_ms,
            end_offset_ms,
            feedback,
            ..
        } => {
            if start_offset_ms > end_offset_ms || *end_offset_ms > MAX_TRACE_DURATION_MS {
                return Err("application feedback offsets are out of range".to_string());
            }
            if let Some(message) = feedback.message.as_deref() {
                validate_text("application feedback message", message, 1, 512)?;
            }
            if let Some(path) = feedback.path.as_deref() {
                validate_text("application feedback path", path, 1, 512)?;
            }
            Ok(())
        }
    }
}

pub fn validate_trace_telemetry_batch(batch: &TraceTelemetryBatch) -> Result<(), String> {
    if batch.events.is_empty() || batch.events.len() > MAX_TRACE_TELEMETRY_EVENTS {
        return Err("trace telemetry batch size is out of range".to_string());
    }
    if batch.dropped_events > MAX_TRACE_DROPPED_EVENTS {
        return Err("trace telemetry dropped event count is out of range".to_string());
    }
    for telemetry in &batch.events {
        validate_trace_telemetry(telemetry)?;
    }
    for (name, summary) in [
        ("root input summary", batch.root_input_summary.as_ref()),
        ("root output summary", batch.root_output_summary.as_ref()),
    ] {
        let Some(summary) = summary else {
            continue;
        };
        validate_json(name, &summary.value, MAX_TRACE_IO_SUMMARY_BYTES)?;
        if canonical_trace_io_summary(&summary.value).value != summary.value {
            return Err(format!("trace {name} is not canonical"));
        }
        if summary.effective_truncated() != summary.truncated {
            return Err(format!("trace {name} under-reports truncation"));
        }
    }
    Ok(())
}

fn redact_trace_io_value(
    value: &Value,
    depth: usize,
    key_hint: Option<&str>,
    truncated: &mut bool,
) -> Value {
    if depth >= MAX_TRACE_IO_DEPTH
        && matches!(value, Value::Object(_) | Value::Array(_))
        && !trace_tool_call_container(depth, key_hint)
    {
        *truncated = true;
        return Value::String(format!("<{} omitted>", trace_value_shape(value)));
    }
    match value {
        Value::Object(object) => {
            if object
                .get("_vifuBinary")
                .is_some_and(|marker| marker.as_bool() == Some(true) || !marker.is_null())
            {
                *truncated = true;
                return Value::String("<_vifuBinary object omitted>".to_string());
            }
            if object.len() >= MAX_TRACE_IO_ITEMS {
                *truncated = true;
            }
            let mut output = serde_json::Map::new();
            for (index, (key, value)) in object.iter().enumerate() {
                if index >= MAX_TRACE_IO_ITEMS {
                    *truncated = true;
                    output.insert("…".to_string(), Value::String("<more fields>".to_string()));
                    break;
                }
                output.insert(
                    key.clone(),
                    if trace_sensitive_key(key) {
                        *truncated = true;
                        Value::String("[REDACTED]".to_string())
                    } else {
                        redact_trace_io_value(value, depth + 1, Some(key), truncated)
                    },
                );
            }
            Value::Object(output)
        }
        Value::Array(values) => {
            if values.len() >= MAX_TRACE_IO_ITEMS {
                *truncated = true;
            }
            Value::Array(
                values
                    .iter()
                    .take(MAX_TRACE_IO_ITEMS)
                    .map(|value| redact_trace_io_value(value, depth + 1, key_hint, truncated))
                    .collect(),
            )
        }
        Value::String(text) => {
            let character_count = text.chars().count();
            if key_hint.is_some_and(trace_tool_call_arguments_key) {
                if let Ok(decoded) = serde_json::from_str::<Value>(text) {
                    if matches!(decoded, Value::Object(_) | Value::Array(_)) {
                        return redact_trace_io_value(&decoded, depth, key_hint, truncated);
                    }
                }
            }
            if canonical_trace_placeholder(text) {
                *truncated = true;
                value.clone()
            } else if trace_sensitive_value_string(text) {
                *truncated = true;
                Value::String("[REDACTED sensitive value]".to_string())
            } else if key_hint.is_some_and(trace_media_or_binary_key)
                || text.trim_start().to_ascii_lowercase().starts_with("data:")
                || trace_looks_like_base64(text)
            {
                *truncated = true;
                Value::String(format!("<media/binary omitted: {character_count} chars>"))
            } else if character_count >= MAX_TRACE_IO_STRING_CHARS {
                *truncated = true;
                if character_count == MAX_TRACE_IO_STRING_CHARS {
                    return value.clone();
                }
                let suffix = format!("… <{character_count} chars total>");
                let prefix_chars = MAX_TRACE_IO_STRING_CHARS.saturating_sub(suffix.chars().count());
                Value::String(format!(
                    "{}{}",
                    text.chars().take(prefix_chars).collect::<String>(),
                    suffix
                ))
            } else {
                value.clone()
            }
        }
        _ => value.clone(),
    }
}

fn trace_tool_call_container(depth: usize, key_hint: Option<&str>) -> bool {
    let Some(key) = key_hint else {
        return false;
    };
    match key.to_ascii_lowercase().as_str() {
        "tool_calls" | "toolcalls" => depth == 5,
        "function" => depth <= 6,
        "arguments" | "args" => depth <= 7,
        _ => false,
    }
}

fn trace_tool_call_arguments_key(key: &str) -> bool {
    matches!(key.to_ascii_lowercase().as_str(), "arguments" | "args")
}

fn canonical_trace_placeholder(value: &str) -> bool {
    value == "[REDACTED]"
        || value == "[REDACTED sensitive value]"
        || value == "<_vifuBinary object omitted>"
        || value == "<more fields>"
        || (value.starts_with('<') && value.ends_with(" omitted>"))
        || (value.starts_with("<media/binary omitted: ") && value.ends_with(" chars>"))
}

fn trace_io_summary_requires_truncation(value: &Value) -> bool {
    match value {
        Value::Object(object) => {
            object.len() >= MAX_TRACE_IO_ITEMS
                || object.get("truncated").and_then(Value::as_bool) == Some(true)
                || object.values().any(trace_io_summary_requires_truncation)
        }
        Value::Array(values) => {
            values.len() >= MAX_TRACE_IO_ITEMS
                || values.iter().any(trace_io_summary_requires_truncation)
        }
        Value::String(text) => {
            text.chars().count() >= MAX_TRACE_IO_STRING_CHARS
                || canonical_trace_placeholder(text)
                || text.contains(" chars total>")
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => false,
    }
}

fn trace_sensitive_value_string(value: &str) -> bool {
    let normalized = value.trim().to_ascii_lowercase();
    [
        "authorization:",
        "authorization=",
        "bearer ",
        "basic ",
        "api_key=",
        "api-key=",
        "apikey=",
        "access_token=",
        "refresh_token=",
        "token=",
        "password=",
        "password:",
        "secret=",
        "secret:",
        "credential=",
        "cookie:",
        "session=",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
}

fn trace_sensitive_key(key: &str) -> bool {
    let normalized = key
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    [
        "authorization",
        "apikey",
        "accesstoken",
        "refreshtoken",
        "token",
        "secret",
        "password",
        "credential",
        "cookie",
        "session",
        "sessionid",
    ]
    .iter()
    .any(|candidate| normalized == *candidate || normalized.ends_with(candidate))
}

fn trace_media_or_binary_key(key: &str) -> bool {
    let normalized = key
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    normalized == "data"
        || ["image", "audio", "media", "binary", "base64"]
            .iter()
            .any(|marker| normalized.contains(marker))
}

fn trace_looks_like_base64(value: &str) -> bool {
    let compact = value.trim();
    compact.len() >= 128
        && compact.len().is_multiple_of(4)
        && compact
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'='))
}

fn trace_value_shape(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn validate_trace_telemetry(telemetry: &TraceTelemetry) -> Result<(), String> {
    match telemetry {
        TraceTelemetry::InvocationStarted {
            provider_key,
            capability,
            model,
        } => {
            validate_identifier("trace provider key", provider_key)?;
            validate_identifier("trace capability", capability)?;
            if let Some(model) = model {
                validate_text("trace model", model, 1, 512)?;
            }
        }
        TraceTelemetry::ProviderStage {
            status,
            start_offset_ms,
            end_offset_ms,
            elapsed_ms,
            request_elapsed_ms,
            input_tokens,
            output_tokens,
            error,
            ..
        } => {
            if *status == TraceStageStatus::Started && elapsed_ms.is_some() {
                return Err("started trace stage cannot have elapsed time".to_string());
            }
            if *status != TraceStageStatus::Started && elapsed_ms.is_none() {
                return Err("completed trace stage requires elapsed time".to_string());
            }
            if *status == TraceStageStatus::Failed && error.is_none() {
                return Err("failed trace stage requires an error".to_string());
            }
            if let Some(error) = error {
                validate_text("trace stage error", error, 1, 512)?;
            }
            if *start_offset_ms > MAX_TRACE_DURATION_MS {
                return Err("trace stage startOffsetMs is out of range".to_string());
            }
            match (status, end_offset_ms) {
                (TraceStageStatus::Started, None) => {}
                (TraceStageStatus::Started, Some(_)) => {
                    return Err("started trace stage cannot have an end offset".to_string())
                }
                (_, Some(end_offset_ms))
                    if *end_offset_ms >= *start_offset_ms
                        && *end_offset_ms <= MAX_TRACE_DURATION_MS => {}
                (_, Some(_)) => return Err("trace stage endOffsetMs is out of range".to_string()),
                (_, None) => return Err("completed trace stage requires an end offset".to_string()),
            }
            for (name, value) in [
                ("trace stage elapsedMs", *elapsed_ms),
                ("trace stage requestElapsedMs", *request_elapsed_ms),
            ] {
                if value.is_some_and(|value| value > MAX_TRACE_DURATION_MS) {
                    return Err(format!("{name} is out of range"));
                }
            }
            for (name, value) in [
                ("trace stage inputTokens", *input_tokens),
                ("trace stage outputTokens", *output_tokens),
            ] {
                if value.is_some_and(|value| value > MAX_TRACE_TOKEN_COUNT) {
                    return Err(format!("{name} is out of range"));
                }
            }
        }
        TraceTelemetry::Delivery {
            status,
            start_offset_ms,
            end_offset_ms,
            elapsed_ms,
            error,
            ..
        } => {
            if *start_offset_ms > *end_offset_ms || *end_offset_ms > MAX_TRACE_DURATION_MS {
                return Err("trace delivery offsets are out of range".to_string());
            }
            if *elapsed_ms > MAX_TRACE_DURATION_MS {
                return Err("trace delivery elapsedMs is out of range".to_string());
            }
            if *status == TraceDeliveryStatus::Failed && error.is_none() {
                return Err("failed trace delivery requires an error".to_string());
            }
            if let Some(error) = error {
                validate_text("trace delivery error", error, 1, 512)?;
            }
        }
    }
    Ok(())
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
        canonical_trace_io_summary, from_gateway_frame, to_gateway_frame,
        validate_trace_telemetry_batch, AgentDescriptor, AgentGatewayCommand, ApplicationFeedback,
        ApplicationFeedbackEvent, ApplicationFeedbackOutcome, GatewayHelloAuth,
        GatewayMachineProof, TraceIoSummary, TraceStageStatus, TraceTelemetry, TraceTelemetryBatch,
        MAX_TRACE_IO_SUMMARY_BYTES, VERSION,
    };
    use crate::gateway_frame;
    use vifu_runtime::ProviderStage;

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

        let feedback = AgentGatewayCommand::ApplicationFeedback {
            request_id,
            observation_id: Uuid::new_v4(),
            start_offset_ms: 23,
            end_offset_ms: 23,
            feedback: ApplicationFeedback {
                event: ApplicationFeedbackEvent::OutputAccepted,
                outcome: ApplicationFeedbackOutcome::Pass,
                message: None,
                path: Some("$.action".to_string()),
            },
        };
        let value = round_trip_command_over_gateway_frame(&feedback);
        assert_eq!(value["payload"]["startOffsetMs"], 23);
        assert_eq!(value["payload"]["endOffsetMs"], 23);
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

    #[test]
    fn rejects_hostile_trace_telemetry_before_it_reaches_runtime_or_storage() {
        let batch = TraceTelemetryBatch {
            events: vec![TraceTelemetry::ProviderStage {
                observation_id: Uuid::new_v4(),
                stage: ProviderStage::Decode,
                status: TraceStageStatus::Completed,
                start_offset_ms: 0,
                end_offset_ms: Some(u64::MAX),
                elapsed_ms: Some(u64::MAX),
                request_elapsed_ms: Some(u64::MAX),
                input_tokens: Some(u64::MAX),
                output_tokens: Some(u64::MAX),
                resident: Some(true),
                error: Some("x".repeat(513)),
            }],
            dropped_events: 0,
            root_input_summary: None,
            root_output_summary: None,
        };

        let error = validate_trace_telemetry_batch(&batch).unwrap_err();
        assert!(error.contains("trace stage"));
    }

    #[test]
    fn canonical_trace_io_is_bounded_redacted_and_idempotent() {
        let summary = canonical_trace_io_summary(&json!({
            "authorization": "Bearer private-token",
            "providerError": "Basic cHJpdmF0ZS11c2VyOnByaXZhdGUtcGFzcw==",
            "image": format!("data:image/png;base64,{}", "A".repeat(12_000)),
            "nested": {"message": "password=hunter2"},
        }));

        assert!(summary.truncated);
        assert!(serde_json::to_vec(&summary.value).unwrap().len() <= MAX_TRACE_IO_SUMMARY_BYTES);
        let serialized = summary.value.to_string();
        assert!(!serialized.contains("private-token"));
        assert!(!serialized.contains("cHJpdmF0ZS11c2Vy"));
        assert!(!serialized.contains("hunter2"));
        assert!(!serialized.contains(&"A".repeat(128)));
        assert_eq!(
            canonical_trace_io_summary(&summary.value).value,
            summary.value
        );
    }

    #[test]
    fn canonical_trace_io_keeps_bounded_chat_output_content_and_tool_arguments() {
        let summary = canonical_trace_io_summary(&json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "I will move east.",
                    "tool_calls": [{
                        "function": {
                            "name": "move",
                            "arguments": "{\"direction\":\"east\",\"api_key\":\"private-value\"}"
                        }
                    }]
                }
            }]
        }));

        assert!(summary.value.to_string().contains("I will move east."));
        assert!(summary.value.to_string().contains("direction"));
        assert!(!summary.value.to_string().contains("private-value"));
        assert!(serde_json::to_vec(&summary.value).unwrap().len() <= MAX_TRACE_IO_SUMMARY_BYTES);
    }

    #[test]
    fn trace_telemetry_rejects_noncanonical_root_io() {
        let batch = TraceTelemetryBatch {
            events: vec![TraceTelemetry::InvocationStarted {
                provider_key: "local-llama".to_string(),
                capability: "chat".to_string(),
                model: None,
            }],
            dropped_events: 0,
            root_input_summary: Some(TraceIoSummary {
                value: json!({"apiKey": "private"}),
                truncated: false,
            }),
            root_output_summary: None,
        };

        assert!(validate_trace_telemetry_batch(&batch)
            .unwrap_err()
            .contains("not canonical"));
    }

    #[test]
    fn trace_telemetry_rejects_underreported_root_io_truncation() {
        let mut summary = canonical_trace_io_summary(&json!({"token": "private"}));
        summary.truncated = false;
        let batch = TraceTelemetryBatch {
            events: vec![TraceTelemetry::InvocationStarted {
                provider_key: "local-llama".to_string(),
                capability: "chat".to_string(),
                model: None,
            }],
            dropped_events: 0,
            root_input_summary: Some(summary),
            root_output_summary: None,
        };

        assert!(validate_trace_telemetry_batch(&batch)
            .unwrap_err()
            .contains("under-reports truncation"));
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
