//! Transport-neutral frames shared by embedded hosts and Vifu Gateway.

use serde::de::{self, DeserializeOwned};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

pub const MAX_PROTOCOL_FRAME_BYTES: usize = 16 * 1024 * 1024;

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
pub enum ProtocolFrame {
    Request(RequestFrame),
    Response(ResponseFrame),
    Event(EventFrame),
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

pub fn encode_protocol_frame(frame: &ProtocolFrame) -> Result<String, String> {
    validate_protocol_frame(frame)?;
    let encoded = serde_json::to_string(frame).map_err(|error| error.to_string())?;
    if encoded.len() > MAX_PROTOCOL_FRAME_BYTES {
        return Err("protocol frame is too large".to_string());
    }
    Ok(encoded)
}

pub fn decode_protocol_frame(source: &str) -> Result<ProtocolFrame, String> {
    if source.is_empty() {
        return Err("protocol frame is empty".to_string());
    }
    if source.len() > MAX_PROTOCOL_FRAME_BYTES {
        return Err("protocol frame is too large".to_string());
    }
    let frame = serde_json::from_str(source).map_err(|_| "invalid protocol frame".to_string())?;
    validate_protocol_frame(&frame)?;
    Ok(frame)
}

pub fn validate_protocol_frame(frame: &ProtocolFrame) -> Result<(), String> {
    match frame {
        ProtocolFrame::Request(request) => {
            validate_non_empty("request id", &request.id)?;
            validate_non_empty("request method", &request.method)
        }
        ProtocolFrame::Response(response) => {
            validate_non_empty("response id", &response.id)?;
            if let Some(error) = &response.error {
                validate_error_shape(error)?;
            }
            Ok(())
        }
        ProtocolFrame::Event(event) => validate_non_empty("event name", &event.event),
    }
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
    use serde_json::json;

    use super::*;

    #[test]
    fn protocol_frames_round_trip() {
        let frame = ProtocolFrame::Request(RequestFrame {
            frame_type: RequestFrameType::Req,
            id: "request-1".to_string(),
            method: "runtime.invoke".to_string(),
            params: Some(json!({"endpoint": "guide"})),
        });
        let encoded = encode_protocol_frame(&frame).unwrap();
        assert_eq!(decode_protocol_frame(&encoded).unwrap(), frame);
    }

    #[test]
    fn protocol_frames_reject_extra_fields_and_null_typed_options() {
        assert!(decode_protocol_frame(
            r#"{"type":"req","id":"request-1","method":"runtime.invoke","extra":true}"#
        )
        .is_err());
        assert!(decode_protocol_frame(r#"{"type":"event","event":"tick","seq":null}"#).is_err());
    }
}
