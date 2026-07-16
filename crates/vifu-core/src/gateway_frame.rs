use serde::de::{self, DeserializeOwned};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

pub const MAX_GATEWAY_FRAME_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequestFrameType {
    #[serde(rename = "req")]
    Req,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResponseFrameType {
    #[serde(rename = "res")]
    Res,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventFrameType {
    #[serde(rename = "event")]
    Event,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestFrame {
    #[serde(rename = "type")]
    pub frame_type: RequestFrameType,
    pub id: String,
    pub method: String,
    #[serde(
        default,
        deserialize_with = "optional_json_value",
        skip_serializing_if = "Option::is_none"
    )]
    pub params: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResponseFrame {
    #[serde(rename = "type")]
    pub frame_type: ResponseFrameType,
    pub id: String,
    pub ok: bool,
    #[serde(
        default,
        deserialize_with = "optional_json_value",
        skip_serializing_if = "Option::is_none"
    )]
    pub payload: Option<Value>,
    #[serde(
        default,
        deserialize_with = "optional_non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub error: Option<ErrorShape>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EventFrame {
    #[serde(rename = "type")]
    pub frame_type: EventFrameType,
    pub event: String,
    #[serde(
        default,
        deserialize_with = "optional_json_value",
        skip_serializing_if = "Option::is_none"
    )]
    pub payload: Option<Value>,
    #[serde(
        default,
        deserialize_with = "optional_non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub seq: Option<u64>,
    #[serde(
        default,
        deserialize_with = "optional_non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub state_version: Option<StateVersion>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum GatewayFrame {
    Request(RequestFrame),
    Response(ResponseFrame),
    Event(EventFrame),
}

pub fn encode(frame: &GatewayFrame) -> Result<String, String> {
    validate_gateway_frame(frame)?;
    let encoded = serde_json::to_string(frame).map_err(|error| error.to_string())?;
    if encoded.len() > MAX_GATEWAY_FRAME_BYTES {
        return Err("gateway frame is too large".to_string());
    }
    Ok(encoded)
}

pub fn decode(source: &str) -> Result<GatewayFrame, String> {
    if source.is_empty() {
        return Err("gateway frame is empty".to_string());
    }
    if source.len() > MAX_GATEWAY_FRAME_BYTES {
        return Err("gateway frame is too large".to_string());
    }
    let frame = serde_json::from_str(source).map_err(|_| "invalid gateway frame".to_string())?;
    validate_gateway_frame(&frame)?;
    Ok(frame)
}

pub fn validate_gateway_frame(frame: &GatewayFrame) -> Result<(), String> {
    match frame {
        GatewayFrame::Request(request) => {
            validate_non_empty("request id", &request.id)?;
            validate_non_empty("request method", &request.method)
        }
        GatewayFrame::Response(response) => {
            validate_non_empty("response id", &response.id)?;
            if let Some(error) = &response.error {
                validate_error_shape(error)?;
            }
            Ok(())
        }
        GatewayFrame::Event(event) => {
            validate_non_empty("event name", &event.event)?;
            Ok(())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ErrorShape {
    pub code: String,
    pub message: String,
    #[serde(
        default,
        deserialize_with = "optional_json_value",
        skip_serializing_if = "Option::is_none"
    )]
    pub details: Option<Value>,
    #[serde(
        default,
        deserialize_with = "optional_non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub retryable: Option<bool>,
    #[serde(
        default,
        deserialize_with = "optional_non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub retry_after_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StateVersion {
    pub presence: u64,
    pub health: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NodeInvokeRequestPayload {
    pub id: String,
    pub node_id: String,
    pub command: String,
    #[serde(
        rename = "paramsJSON",
        default,
        deserialize_with = "optional_non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub params_json: Option<String>,
    #[serde(
        default,
        deserialize_with = "optional_non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub timeout_ms: Option<u64>,
    #[serde(
        default,
        deserialize_with = "optional_non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NodeInvokeResultError {
    #[serde(
        default,
        deserialize_with = "optional_non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub code: Option<String>,
    #[serde(
        default,
        deserialize_with = "optional_non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NodeInvokeResultParams {
    pub id: String,
    pub node_id: String,
    pub ok: bool,
    #[serde(
        default,
        deserialize_with = "optional_json_value",
        skip_serializing_if = "Option::is_none"
    )]
    pub payload: Option<Value>,
    #[serde(
        rename = "payloadJSON",
        default,
        deserialize_with = "optional_non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub payload_json: Option<String>,
    #[serde(
        default,
        deserialize_with = "optional_non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub error: Option<NodeInvokeResultError>,
}

fn optional_json_value<'de, D>(deserializer: D) -> Result<Option<Value>, D::Error>
where
    D: Deserializer<'de>,
{
    Value::deserialize(deserializer).map(Some)
}

fn optional_non_null<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    let value = Value::deserialize(deserializer)?;
    if value.is_null() {
        return Err(de::Error::custom("null is not allowed for this field"));
    }
    T::deserialize(value).map(Some).map_err(de::Error::custom)
}

fn validate_error_shape(error: &ErrorShape) -> Result<(), String> {
    validate_non_empty("error code", &error.code)?;
    validate_non_empty("error message", &error.message)
}

fn validate_non_empty(name: &str, value: &str) -> Result<(), String> {
    if value.is_empty() {
        Err(format!("{name} must not be empty"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde::de::DeserializeOwned;
    use serde_json::{json, Value};

    use super::{
        decode, encode, EventFrame, NodeInvokeRequestPayload, NodeInvokeResultParams, RequestFrame,
    };

    const REQUEST: &str =
        include_str!("../../../packages/protocol/fixtures/gateway-frame/request.json");
    const RESPONSE_OK: &str =
        include_str!("../../../packages/protocol/fixtures/gateway-frame/response-ok.json");
    const RESPONSE_ERROR: &str =
        include_str!("../../../packages/protocol/fixtures/gateway-frame/response-error.json");
    const EVENT: &str =
        include_str!("../../../packages/protocol/fixtures/gateway-frame/event.json");
    const EVENT_STATE_VERSION: &str =
        include_str!("../../../packages/protocol/fixtures/gateway-frame/event-state-version.json");
    const NODE_INVOKE_REQUEST: &str =
        include_str!("../../../packages/protocol/fixtures/gateway-frame/node-invoke-request.json");
    const NODE_INVOKE_RESULT: &str =
        include_str!("../../../packages/protocol/fixtures/gateway-frame/node-invoke-result.json");

    #[test]
    fn round_trips_shared_gateway_frame_fixtures() {
        for source in [
            REQUEST,
            RESPONSE_OK,
            RESPONSE_ERROR,
            EVENT,
            EVENT_STATE_VERSION,
            NODE_INVOKE_REQUEST,
            NODE_INVOKE_RESULT,
        ] {
            assert_gateway_frame_round_trip(source);
        }
    }

    #[test]
    fn parses_node_invoke_fixture_payloads() {
        let event = decode_json::<EventFrame>(NODE_INVOKE_REQUEST);
        let payload = event.payload.expect("node invoke event payload");
        let request = serde_json::from_value::<NodeInvokeRequestPayload>(payload).unwrap();
        assert_eq!(request.node_id, "ios-node-1");
        assert_eq!(request.command, "camera.snap");
        assert_eq!(
            request.params_json.as_deref(),
            Some("{\"quality\":\"medium\"}")
        );

        let frame = decode_json::<RequestFrame>(NODE_INVOKE_RESULT);
        let params = frame.params.expect("node invoke result params");
        let result = serde_json::from_value::<NodeInvokeResultParams>(params).unwrap();
        assert_eq!(result.node_id, "ios-node-1");
        assert!(result.ok);
        assert_eq!(
            result.payload_json.as_deref(),
            Some("{\"imageId\":\"img_123\"}")
        );
    }

    #[test]
    fn rejects_extra_frame_fields() {
        assert_gateway_frame_rejected(json!({
            "type": "req",
            "id": "req-1",
            "method": "runtime.ping",
            "extra": true
        }));
        assert_gateway_frame_rejected(json!({
            "type": "res",
            "id": "req-1",
            "ok": false,
            "error": {
                "code": "BAD_REQUEST",
                "message": "invalid request",
                "extra": true
            }
        }));
        assert_gateway_frame_rejected(json!({
            "type": "event",
            "event": "runtime.ready",
            "stateVersion": {
                "presence": 1,
                "health": 1,
                "extra": true
            }
        }));
    }

    #[test]
    fn rejects_null_for_typed_optional_fields() {
        assert_gateway_frame_rejected(json!({
            "type": "event",
            "event": "tick",
            "seq": null
        }));
        assert_gateway_frame_rejected(json!({
            "type": "res",
            "id": "req-1",
            "ok": false,
            "error": null
        }));
        assert!(serde_json::from_value::<NodeInvokeRequestPayload>(json!({
            "id": "invoke-1",
            "nodeId": "ios-node-1",
            "command": "camera.snap",
            "paramsJSON": null
        }))
        .is_err());
        assert!(serde_json::from_value::<NodeInvokeResultParams>(json!({
            "id": "invoke-1",
            "nodeId": "ios-node-1",
            "ok": true,
            "payloadJSON": null
        }))
        .is_err());
        assert!(serde_json::from_value::<NodeInvokeResultParams>(json!({
            "id": "invoke-1",
            "nodeId": "ios-node-1",
            "ok": false,
            "error": null
        }))
        .is_err());
    }

    #[test]
    fn codec_rejects_empty_required_strings() {
        assert_gateway_frame_rejected(json!({
            "type": "req",
            "id": "",
            "method": "runtime.ping"
        }));
        assert_gateway_frame_rejected(json!({
            "type": "req",
            "id": "req-1",
            "method": ""
        }));
        assert_gateway_frame_rejected(json!({
            "type": "event",
            "event": ""
        }));
        assert_gateway_frame_rejected(json!({
            "type": "res",
            "id": "req-1",
            "ok": false,
            "error": {
                "code": "",
                "message": "invalid request"
            }
        }));
    }

    fn assert_gateway_frame_round_trip(source: &str) {
        let value = serde_json::from_str::<Value>(source).unwrap();
        let frame = decode(source).unwrap();
        let encoded = encode(&frame).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&encoded).unwrap(), value);
    }

    fn assert_gateway_frame_rejected(value: Value) {
        assert!(decode(&value.to_string()).is_err());
    }

    fn decode_json<T>(source: &str) -> T
    where
        T: DeserializeOwned,
    {
        serde_json::from_str(source).unwrap()
    }
}
