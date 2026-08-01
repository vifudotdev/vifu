use serde::de::{self, DeserializeOwned};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

pub use vifu_runtime::protocol::{
    ErrorShape, EventFrame, EventFrameType, ProtocolFrame as GatewayFrame, RequestFrame,
    RequestFrameType, ResponseFrame, ResponseFrameType, StateVersion,
    MAX_PROTOCOL_FRAME_BYTES as MAX_GATEWAY_FRAME_BYTES,
};

pub fn encode(frame: &GatewayFrame) -> Result<String, String> {
    vifu_runtime::protocol::encode_protocol_frame(frame)
        .map_err(|error| error.replace("protocol frame", "gateway frame"))
}

pub fn decode(source: &str) -> Result<GatewayFrame, String> {
    vifu_runtime::protocol::decode_protocol_frame(source)
        .map_err(|error| error.replace("protocol frame", "gateway frame"))
}

pub fn validate_gateway_frame(frame: &GatewayFrame) -> Result<(), String> {
    vifu_runtime::protocol::validate_protocol_frame(frame)
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

#[cfg(test)]
mod tests {
    use serde::de::DeserializeOwned;
    use serde_json::{json, Value};
    use std::fs;
    use std::path::PathBuf;

    use super::{
        decode, encode, EventFrame, NodeInvokeRequestPayload, NodeInvokeResultParams, RequestFrame,
    };

    #[test]
    fn round_trips_shared_gateway_frame_fixtures() {
        let fixtures = gateway_frame_fixtures();
        assert!(!fixtures.is_empty(), "gateway frame fixtures must exist");
        for (name, source) in fixtures {
            assert_gateway_frame_round_trip(&name, &source);
        }
    }

    #[test]
    fn parses_node_invoke_fixture_payloads() {
        let request_source = read_gateway_frame_fixture("node-invoke-request.json");
        let event = decode_json::<EventFrame>(&request_source);
        let payload = event.payload.expect("node invoke event payload");
        let request = serde_json::from_value::<NodeInvokeRequestPayload>(payload).unwrap();
        assert_eq!(request.node_id, "ios-node-1");
        assert_eq!(request.command, "camera.snap");
        assert_eq!(
            request.params_json.as_deref(),
            Some("{\"quality\":\"medium\"}")
        );

        let result_source = read_gateway_frame_fixture("node-invoke-result.json");
        let frame = decode_json::<RequestFrame>(&result_source);
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

    fn assert_gateway_frame_round_trip(name: &str, source: &str) {
        let value = serde_json::from_str::<Value>(source).unwrap();
        let frame = decode(source).unwrap_or_else(|error| panic!("{name}: {error}"));
        let encoded = encode(&frame).unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(
            serde_json::from_str::<Value>(&encoded).unwrap(),
            value,
            "{name}"
        );
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

    fn gateway_frame_fixtures() -> Vec<(String, String)> {
        let mut fixtures = fs::read_dir(gateway_frame_fixture_dir())
            .unwrap()
            .filter_map(|entry| {
                let entry = entry.unwrap();
                let path = entry.path();
                if path.extension().and_then(|value| value.to_str()) != Some("json") {
                    return None;
                }
                let name = path.file_name().unwrap().to_string_lossy().into_owned();
                let source = fs::read_to_string(path).unwrap();
                Some((name, source))
            })
            .collect::<Vec<_>>();
        fixtures.sort_by(|left, right| left.0.cmp(&right.0));
        fixtures
    }

    fn read_gateway_frame_fixture(name: &str) -> String {
        fs::read_to_string(gateway_frame_fixture_dir().join(name)).unwrap()
    }

    fn gateway_frame_fixture_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("packages/protocol/fixtures/gateway-frame")
    }
}
