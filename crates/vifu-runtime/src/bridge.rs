//! Engine-neutral bridge between a host application and [`VifuRuntime`].
//!
//! The same frames can cross an in-process FFI boundary or a WebSocket
//! transport. Godot, Unity, Unreal, and native hosts only need an adapter that
//! moves encoded frames to and from this bridge.

use std::collections::BTreeSet;
use std::fmt;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::protocol::{
    decode_protocol_frame, encode_protocol_frame, ErrorShape, EventFrame, EventFrameType,
    ProtocolFrame, RequestFrame, ResponseFrame, ResponseFrameType,
};
use crate::{
    InvocationEvent, InvocationEventKind, InvocationHandle, InvocationInput, InvocationOutput,
    InvocationStatus, RuntimeError, VifuRuntime,
};

pub const VIFU_RUNTIME_BRIDGE_PROTOCOL_VERSION: &str = "vifu.runtime-bridge/1";

pub const RUNTIME_BRIDGE_HELLO_METHOD: &str = "runtime.hello";
pub const RUNTIME_BRIDGE_INVOKE_METHOD: &str = "runtime.invoke";
pub const RUNTIME_BRIDGE_CANCEL_METHOD: &str = "runtime.cancel";

pub const RUNTIME_BRIDGE_STARTED_EVENT: &str = "runtime.invocation.started";
pub const RUNTIME_BRIDGE_OUTPUT_DELTA_EVENT: &str = "runtime.invocation.outputDelta";
pub const RUNTIME_BRIDGE_COMPLETED_EVENT: &str = "runtime.invocation.completed";
pub const RUNTIME_BRIDGE_FAILED_EVENT: &str = "runtime.invocation.failed";
pub const RUNTIME_BRIDGE_CANCELLED_EVENT: &str = "runtime.invocation.cancelled";

#[derive(Debug)]
pub enum RuntimeBridgeError {
    Protocol(String),
    Runtime(RuntimeError),
    StateUnavailable,
}

impl fmt::Display for RuntimeBridgeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Protocol(message) => formatter.write_str(message),
            Self::Runtime(error) => error.fmt(formatter),
            Self::StateUnavailable => formatter.write_str("runtime bridge state is unavailable"),
        }
    }
}

impl std::error::Error for RuntimeBridgeError {}

impl From<RuntimeError> for RuntimeBridgeError {
    fn from(error: RuntimeError) -> Self {
        Self::Runtime(error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeBridgeHelloParams {
    pub protocol: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeBridgeHelloPayload {
    pub protocol: String,
    pub project_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeBridgeInvokePayload {
    pub handle: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeBridgeCancelParams {
    pub handle: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeBridgeInvocationEvent {
    pub handle: String,
    pub event: InvocationEvent,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<InvocationOutput>,
}

/// Routes protocol frames into one embedded application runtime.
pub struct RuntimeBridge {
    runtime: VifuRuntime,
    active_invocations: Mutex<BTreeSet<String>>,
}

impl RuntimeBridge {
    pub fn new(runtime: VifuRuntime) -> Self {
        Self {
            runtime,
            active_invocations: Mutex::new(BTreeSet::new()),
        }
    }

    pub fn runtime(&self) -> &VifuRuntime {
        &self.runtime
    }

    pub fn handle_encoded(&self, source: &str) -> Result<Vec<String>, RuntimeBridgeError> {
        let frame = decode_protocol_frame(source).map_err(RuntimeBridgeError::Protocol)?;
        self.handle_frame(frame)?
            .iter()
            .map(|frame| encode_protocol_frame(frame).map_err(RuntimeBridgeError::Protocol))
            .collect()
    }

    pub fn handle_frame(
        &self,
        frame: ProtocolFrame,
    ) -> Result<Vec<ProtocolFrame>, RuntimeBridgeError> {
        let ProtocolFrame::Request(request) = frame else {
            return Err(RuntimeBridgeError::Protocol(
                "runtime bridge accepts request frames from the host".to_string(),
            ));
        };
        Ok(vec![self.handle_request(request)])
    }

    pub fn drain_encoded(&self) -> Result<Vec<String>, RuntimeBridgeError> {
        self.drain_events()?
            .iter()
            .map(|frame| encode_protocol_frame(frame).map_err(RuntimeBridgeError::Protocol))
            .collect()
    }

    pub fn drain_events(&self) -> Result<Vec<ProtocolFrame>, RuntimeBridgeError> {
        let handles = self
            .active_invocations
            .lock()
            .map_err(|_| RuntimeBridgeError::StateUnavailable)?
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        let mut frames = Vec::new();
        let mut completed = Vec::new();

        for handle in handles {
            let invocation_handle = InvocationHandle(handle.clone());
            let events = match self.runtime.drain_invocation_events(&invocation_handle) {
                Ok(events) => events,
                Err(RuntimeError::InvocationNotFound(_)) => {
                    completed.push(handle);
                    continue;
                }
                Err(error) => return Err(error.into()),
            };
            let poll = self.runtime.poll_invocation(&invocation_handle)?;
            let output = poll.output.clone();
            for event in events {
                let event_name = invocation_event_name(event.kind);
                let payload = RuntimeBridgeInvocationEvent {
                    handle: handle.clone(),
                    output: if event.kind == InvocationEventKind::Completed {
                        output.clone()
                    } else {
                        None
                    },
                    event,
                };
                frames.push(ProtocolFrame::Event(EventFrame {
                    frame_type: EventFrameType::Event,
                    event: event_name.to_string(),
                    payload: Some(
                        serde_json::to_value(payload)
                            .map_err(|error| RuntimeBridgeError::Protocol(error.to_string()))?,
                    ),
                    seq: None,
                    state_version: None,
                }));
            }
            if is_terminal(poll.status) {
                let _ = self.runtime.take_invocation(&invocation_handle)?;
                completed.push(handle);
            }
        }

        if !completed.is_empty() {
            let mut active = self
                .active_invocations
                .lock()
                .map_err(|_| RuntimeBridgeError::StateUnavailable)?;
            for handle in completed {
                active.remove(&handle);
            }
        }
        Ok(frames)
    }

    fn handle_request(&self, request: RequestFrame) -> ProtocolFrame {
        match request.method.as_str() {
            RUNTIME_BRIDGE_HELLO_METHOD => self.handle_hello(request),
            RUNTIME_BRIDGE_INVOKE_METHOD => self.handle_invoke(request),
            RUNTIME_BRIDGE_CANCEL_METHOD => self.handle_cancel(request),
            _ => error_response(
                request.id,
                "method_not_found",
                "runtime bridge method is not supported",
                None,
            ),
        }
    }

    fn handle_hello(&self, request: RequestFrame) -> ProtocolFrame {
        let params = match decode_params::<RuntimeBridgeHelloParams>(&request) {
            Ok(params) => params,
            Err(error) => return invalid_params_response(request.id, error),
        };
        if params.protocol != VIFU_RUNTIME_BRIDGE_PROTOCOL_VERSION {
            return error_response(
                request.id,
                "protocol_mismatch",
                "runtime bridge protocol is not supported",
                Some(json!({
                    "supported": VIFU_RUNTIME_BRIDGE_PROTOCOL_VERSION,
                })),
            );
        }
        success_response(
            request.id,
            RuntimeBridgeHelloPayload {
                protocol: VIFU_RUNTIME_BRIDGE_PROTOCOL_VERSION.to_string(),
                project_id: self.runtime.project_id().to_string(),
            },
        )
    }

    fn handle_invoke(&self, request: RequestFrame) -> ProtocolFrame {
        let input = match decode_params::<InvocationInput>(&request) {
            Ok(input) => input,
            Err(error) => return invalid_params_response(request.id, error),
        };
        match self.runtime.start_invoke(input) {
            Ok(handle) => {
                let mut active = match self.active_invocations.lock() {
                    Ok(active) => active,
                    Err(_) => {
                        let _ = self.runtime.cancel_invocation(&handle);
                        return internal_error_response(request.id);
                    }
                };
                active.insert(handle.0.clone());
                success_response(request.id, RuntimeBridgeInvokePayload { handle: handle.0 })
            }
            Err(error) => runtime_error_response(request.id, error),
        }
    }

    fn handle_cancel(&self, request: RequestFrame) -> ProtocolFrame {
        let params = match decode_params::<RuntimeBridgeCancelParams>(&request) {
            Ok(params) => params,
            Err(error) => return invalid_params_response(request.id, error),
        };
        match self
            .runtime
            .cancel_invocation(&InvocationHandle(params.handle.clone()))
        {
            Ok(()) => success_response(request.id, json!({"handle": params.handle})),
            Err(error) => runtime_error_response(request.id, error),
        }
    }
}

fn decode_params<T>(request: &RequestFrame) -> Result<T, String>
where
    T: for<'de> Deserialize<'de>,
{
    let params = request
        .params
        .clone()
        .ok_or_else(|| "request params are required".to_string())?;
    serde_json::from_value(params).map_err(|error| error.to_string())
}

fn success_response(payload_id: String, payload: impl Serialize) -> ProtocolFrame {
    match serde_json::to_value(payload) {
        Ok(payload) => ProtocolFrame::Response(ResponseFrame {
            frame_type: ResponseFrameType::Res,
            id: payload_id,
            ok: true,
            payload: Some(payload),
            error: None,
        }),
        Err(_) => internal_error_response(payload_id),
    }
}

fn invalid_params_response(id: String, error: String) -> ProtocolFrame {
    error_response(
        id,
        "invalid_params",
        "runtime bridge request parameters are invalid",
        Some(json!({"reason": error})),
    )
}

fn runtime_error_response(id: String, error: RuntimeError) -> ProtocolFrame {
    error_response(id, "runtime_error", &error.public_message(), None)
}

fn internal_error_response(id: String) -> ProtocolFrame {
    error_response(
        id,
        "internal_error",
        "runtime bridge could not complete the request",
        None,
    )
}

fn error_response(id: String, code: &str, message: &str, details: Option<Value>) -> ProtocolFrame {
    ProtocolFrame::Response(ResponseFrame {
        frame_type: ResponseFrameType::Res,
        id,
        ok: false,
        payload: None,
        error: Some(ErrorShape {
            code: code.to_string(),
            message: message.to_string(),
            details,
            retryable: None,
            retry_after_ms: None,
        }),
    })
}

fn invocation_event_name(kind: InvocationEventKind) -> &'static str {
    match kind {
        InvocationEventKind::Started => RUNTIME_BRIDGE_STARTED_EVENT,
        InvocationEventKind::OutputDelta => RUNTIME_BRIDGE_OUTPUT_DELTA_EVENT,
        InvocationEventKind::Completed => RUNTIME_BRIDGE_COMPLETED_EVENT,
        InvocationEventKind::Failed => RUNTIME_BRIDGE_FAILED_EVENT,
        InvocationEventKind::Cancelled => RUNTIME_BRIDGE_CANCELLED_EVENT,
    }
}

fn is_terminal(status: InvocationStatus) -> bool {
    matches!(
        status,
        InvocationStatus::Completed | InvocationStatus::Failed | InvocationStatus::Cancelled
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use crate::{
        AgentDefinition, AgentProvider, CancellationToken, EndpointDefinition, InvocationData,
        ProviderFuture, ProviderRequest, ProviderResponse,
    };

    use super::*;

    struct EchoProvider;

    impl AgentProvider for EchoProvider {
        fn supports(&self, capability: &str) -> bool {
            capability == "chat"
        }

        fn invoke<'a>(
            &'a self,
            request: ProviderRequest,
            _cancellation: CancellationToken,
        ) -> ProviderFuture<'a> {
            Box::pin(async move {
                Ok(ProviderResponse {
                    data: request.data,
                    metadata: json!({"contentType": "application/json"}),
                    state: None,
                })
            })
        }
    }

    fn configured_bridge() -> RuntimeBridge {
        let runtime = VifuRuntime::new("bridge-project").unwrap();
        runtime
            .register_provider("echo", Arc::new(EchoProvider))
            .unwrap();
        runtime
            .register_agent(AgentDefinition {
                id: "guide".to_string(),
                name: "Guide".to_string(),
                provider: "echo".to_string(),
                capabilities: vec!["chat".to_string()],
                metadata: json!({}),
            })
            .unwrap();
        runtime
            .register_endpoint(EndpointDefinition {
                name: "guide".to_string(),
                agent: "guide".to_string(),
                capability: "chat".to_string(),
                timeout_ms: 1_000,
            })
            .unwrap();
        RuntimeBridge::new(runtime)
    }

    #[test]
    fn bridge_invokes_runtime_and_streams_terminal_event() {
        let bridge = configured_bridge();
        let response = bridge
            .handle_frame(ProtocolFrame::Request(RequestFrame {
                frame_type: crate::protocol::RequestFrameType::Req,
                id: "invoke-1".to_string(),
                method: RUNTIME_BRIDGE_INVOKE_METHOD.to_string(),
                params: Some(json!({
                    "endpoint": "guide",
                    "sessionId": "player-one",
                    "data": {
                        "format": "json",
                        "value": {"message": "hello"}
                    },
                    "metadata": {}
                })),
            }))
            .unwrap();
        assert!(matches!(
            response.as_slice(),
            [ProtocolFrame::Response(ResponseFrame { ok: true, .. })]
        ));

        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let events = bridge.drain_events().unwrap();
            if events.iter().any(|frame| {
                matches!(
                    frame,
                    ProtocolFrame::Event(EventFrame { event, .. })
                        if event == RUNTIME_BRIDGE_COMPLETED_EVENT
                )
            }) {
                let completed = events
                    .into_iter()
                    .find(|frame| {
                        matches!(
                            frame,
                            ProtocolFrame::Event(EventFrame { event, .. })
                                if event == RUNTIME_BRIDGE_COMPLETED_EVENT
                        )
                    })
                    .unwrap();
                let ProtocolFrame::Event(event) = completed else {
                    unreachable!()
                };
                let payload: RuntimeBridgeInvocationEvent =
                    serde_json::from_value(event.payload.unwrap()).unwrap();
                assert_eq!(
                    payload.output.unwrap().data,
                    InvocationData::Json(json!({"message": "hello"}))
                );
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn bridge_rejects_unknown_methods_with_protocol_error() {
        let bridge = configured_bridge();
        let frames = bridge
            .handle_frame(ProtocolFrame::Request(RequestFrame {
                frame_type: crate::protocol::RequestFrameType::Req,
                id: "unknown-1".to_string(),
                method: "runtime.unknown".to_string(),
                params: None,
            }))
            .unwrap();
        assert!(matches!(
            frames.as_slice(),
            [ProtocolFrame::Response(ResponseFrame {
                ok: false,
                error: Some(ErrorShape { code, .. }),
                ..
            })] if code == "method_not_found"
        ));
    }
}
