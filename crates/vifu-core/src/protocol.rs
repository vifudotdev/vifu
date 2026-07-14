use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

pub const VERSION: &str = "vifu.agent-gateway/1";
pub const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_BODY_BYTES: usize = 512 * 1024;
pub const MAX_PATH_BYTES: usize = 2 * 1024;
pub const MAX_AGENTS: usize = 256;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDescriptor {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum AgentGatewayMessage {
    Hello {
        protocol: String,
        gateway_id: String,
        resume_session_id: Option<Uuid>,
        agents: Vec<AgentDescriptor>,
        #[serde(default)]
        metadata: Value,
    },
    Welcome {
        connection_id: Uuid,
        session_id: Uuid,
        heartbeat_interval_ms: u64,
        resumed: bool,
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
}

pub fn encode(message: &AgentGatewayMessage) -> Result<String, String> {
    validate_message(message)?;
    let encoded = serde_json::to_string(message).map_err(|error| error.to_string())?;
    if encoded.len() > MAX_FRAME_BYTES {
        return Err("protocol frame is too large".to_string());
    }
    Ok(encoded)
}

pub fn decode(frame: &str) -> Result<AgentGatewayMessage, String> {
    if frame.is_empty() {
        return Err("protocol frame is empty".to_string());
    }
    if frame.len() > MAX_FRAME_BYTES {
        return Err("protocol frame is too large".to_string());
    }
    let message = serde_json::from_str(frame).map_err(|_| "invalid protocol frame".to_string())?;
    validate_message(&message)?;
    Ok(message)
}

pub fn validate_message(message: &AgentGatewayMessage) -> Result<(), String> {
    match message {
        AgentGatewayMessage::Hello {
            protocol,
            gateway_id,
            resume_session_id: _,
            agents,
            metadata,
        } => {
            if protocol != VERSION {
                return Err(format!("unsupported agent gateway protocol: {protocol}"));
            }
            validate_identifier("agent gateway id", gateway_id)?;
            if agents.len() > MAX_AGENTS {
                return Err("too many agent gateway agents".to_string());
            }
            for agent in agents {
                validate_identifier("agent id", &agent.id)?;
                validate_text("agent name", &agent.name, 1, 128)?;
                validate_json("agent metadata", &agent.metadata, 64 * 1024)?;
            }
            validate_json("agent gateway metadata", metadata, 64 * 1024)
        }
        AgentGatewayMessage::Welcome {
            heartbeat_interval_ms,
            ..
        } => {
            if !(1_000..=60_000).contains(heartbeat_interval_ms) {
                return Err("invalid heartbeat interval".to_string());
            }
            Ok(())
        }
        AgentGatewayMessage::Invoke {
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
            validate_json("input", input, MAX_BODY_BYTES)?;
            if !(500..=120_000).contains(timeout_ms) {
                return Err("invalid request timeout".to_string());
            }
            Ok(())
        }
        AgentGatewayMessage::Result {
            channel_id, output, ..
        } => {
            validate_channel(*channel_id)?;
            validate_json("output", output, MAX_BODY_BYTES)
        }
        AgentGatewayMessage::Error {
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
        AgentGatewayMessage::Cancel { channel_id, .. } => validate_channel(*channel_id),
        AgentGatewayMessage::Heartbeat { .. } | AgentGatewayMessage::HeartbeatAck { .. } => Ok(()),
    }
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
    use serde_json::json;
    use uuid::Uuid;

    use super::{decode, encode, AgentDescriptor, AgentGatewayMessage, VERSION};

    #[test]
    fn round_trips_multiplexed_invoke() {
        let message = AgentGatewayMessage::Invoke {
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

        let encoded = encode(&message).unwrap();
        assert_eq!(decode(&encoded).unwrap(), message);
    }

    #[test]
    fn round_trips_resume_hello() {
        let message = AgentGatewayMessage::Hello {
            protocol: VERSION.to_string(),
            gateway_id: "local-gateway".to_string(),
            resume_session_id: Some(Uuid::new_v4()),
            agents: vec![AgentDescriptor {
                id: "default-agent".to_string(),
                name: "Default".to_string(),
                metadata: json!({}),
            }],
            metadata: json!({ "adapter": "openclaw" }),
        };
        assert_eq!(decode(&encode(&message).unwrap()).unwrap(), message);
    }

    #[test]
    fn rejects_zero_channel() {
        let message = AgentGatewayMessage::Cancel {
            request_id: Uuid::new_v4(),
            channel_id: 0,
        };
        assert!(encode(&message).unwrap_err().contains("channel id"));
    }

    #[test]
    fn rejects_unknown_protocol() {
        let message = AgentGatewayMessage::Hello {
            protocol: "vifu.agent-gateway/999".to_string(),
            gateway_id: "local-gateway".to_string(),
            resume_session_id: None,
            agents: Vec::new(),
            metadata: json!({}),
        };
        assert!(encode(&message).unwrap_err().contains("unsupported"));
    }
}
