pub const VERSION: &str = "vifu/1";
pub const OPENCLAW_GATEWAY_CAPABILITY: &str = "openclaw.gateway";
pub const MAX_FRAME_BYTES: usize = 128 * 1024;
pub const MAX_BODY_BYTES: usize = 64 * 1024;
pub const MAX_PATH_BYTES: usize = 2 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Role {
    Client,
    Server,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    Hello {
        role: Role,
        device_id: String,
    },
    Capability {
        name: String,
        target: String,
    },
    Request {
        id: u64,
        capability: String,
        method: String,
        path: String,
        body: Vec<u8>,
    },
    Response {
        id: u64,
        status: u16,
        body: Vec<u8>,
    },
    Error {
        id: Option<u64>,
        message: String,
    },
    Ping,
    Pong,
}

pub fn encode(message: &Message) -> Result<String, String> {
    let line = match message {
        Message::Hello { role, device_id } => {
            validate_device_id(device_id)?;
            format!("HELLO\t{}\t{device_id}", encode_role(role))
        }
        Message::Capability { name, target } => {
            validate_capability(name)?;
            validate_target(target)?;
            format!("CAP\t{name}\t{target}")
        }
        Message::Request {
            id,
            capability,
            method,
            path,
            body,
        } => {
            validate_id(*id)?;
            validate_capability(capability)?;
            validate_method(method)?;
            validate_path(path)?;
            validate_body(body)?;
            format!(
                "REQ\t{id}\t{capability}\t{method}\t{path}\t{}",
                encode_hex(body)
            )
        }
        Message::Response { id, status, body } => {
            validate_id(*id)?;
            validate_status(*status)?;
            validate_body(body)?;
            format!("RES\t{id}\t{status}\t{}", encode_hex(body))
        }
        Message::Error { id, message } => {
            if let Some(value) = id {
                validate_id(*value)?;
            }
            validate_error_message(message)?;
            let id_part = id
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_string());
            format!("ERR\t{id_part}\t{}", encode_hex(message.as_bytes()))
        }
        Message::Ping => "PING".to_string(),
        Message::Pong => "PONG".to_string(),
    };

    if line.len() + 1 > MAX_FRAME_BYTES {
        return Err("protocol frame is too large".to_string());
    }

    Ok(format!("{line}\n"))
}

pub fn decode(line: &str) -> Result<Message, String> {
    if line.len() > MAX_FRAME_BYTES {
        return Err("protocol frame is too large".to_string());
    }

    let line = line.trim_end_matches(['\r', '\n']);
    if line.is_empty() {
        return Err("protocol frame is empty".to_string());
    }

    let parts: Vec<&str> = line.split('\t').collect();
    match parts.as_slice() {
        ["HELLO", role, device_id] => {
            let role = decode_role(role)?;
            validate_device_id(device_id)?;
            Ok(Message::Hello {
                role,
                device_id: (*device_id).to_string(),
            })
        }
        ["CAP", name, target] => {
            validate_capability(name)?;
            validate_target(target)?;
            Ok(Message::Capability {
                name: (*name).to_string(),
                target: (*target).to_string(),
            })
        }
        ["REQ", id, capability, method, path, body_hex] => {
            let id = parse_id(id)?;
            validate_capability(capability)?;
            validate_method(method)?;
            validate_path(path)?;
            let body = decode_hex(body_hex)?;
            validate_body(&body)?;
            Ok(Message::Request {
                id,
                capability: (*capability).to_string(),
                method: (*method).to_string(),
                path: (*path).to_string(),
                body,
            })
        }
        ["RES", id, status, body_hex] => {
            let id = parse_id(id)?;
            let status = status
                .parse::<u16>()
                .map_err(|_| "invalid response status".to_string())?;
            validate_status(status)?;
            let body = decode_hex(body_hex)?;
            validate_body(&body)?;
            Ok(Message::Response { id, status, body })
        }
        ["ERR", id, message_hex] => {
            let id = if *id == "-" {
                None
            } else {
                Some(parse_id(id)?)
            };
            let message_bytes = decode_hex(message_hex)?;
            let message = String::from_utf8(message_bytes)
                .map_err(|_| "error message must be utf-8".to_string())?;
            validate_error_message(&message)?;
            Ok(Message::Error { id, message })
        }
        ["PING"] => Ok(Message::Ping),
        ["PONG"] => Ok(Message::Pong),
        _ => Err("unknown protocol frame".to_string()),
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

pub fn validate_device_id(value: &str) -> Result<(), String> {
    if value.len() < 8 || value.len() > 128 {
        return Err("invalid device id length".to_string());
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err("invalid device id characters".to_string());
    }
    Ok(())
}

fn validate_capability(value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 128 {
        return Err("invalid capability length".to_string());
    }
    if !value.bytes().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-' | b'_')
    }) {
        return Err("invalid capability characters".to_string());
    }
    Ok(())
}

fn validate_target(value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 256 {
        return Err("invalid capability target length".to_string());
    }
    if value
        .bytes()
        .any(|byte| byte < 0x21 || byte == 0x7f || byte == b'\t')
    {
        return Err("invalid capability target characters".to_string());
    }
    Ok(())
}

fn validate_id(value: u64) -> Result<(), String> {
    if value == 0 {
        return Err("protocol id must be greater than zero".to_string());
    }
    Ok(())
}

fn validate_status(value: u16) -> Result<(), String> {
    if !(100..=599).contains(&value) {
        return Err("invalid response status".to_string());
    }
    Ok(())
}

fn validate_error_message(value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 2048 {
        return Err("invalid error message length".to_string());
    }
    if value.bytes().any(|byte| byte < 0x20 && byte != b'\n') {
        return Err("invalid error message characters".to_string());
    }
    Ok(())
}

fn encode_role(role: &Role) -> &'static str {
    match role {
        Role::Client => "client",
        Role::Server => "server",
    }
}

fn decode_role(value: &str) -> Result<Role, String> {
    match value {
        "client" => Ok(Role::Client),
        "server" => Ok(Role::Server),
        _ => Err("invalid protocol role".to_string()),
    }
}

fn parse_id(value: &str) -> Result<u64, String> {
    let id = value
        .parse::<u64>()
        .map_err(|_| "invalid protocol id".to_string())?;
    validate_id(id)?;
    Ok(id)
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn decode_hex(value: &str) -> Result<Vec<u8>, String> {
    if value.len() % 2 != 0 {
        return Err("invalid hex body length".to_string());
    }
    let mut bytes = Vec::with_capacity(value.len() / 2);
    let raw = value.as_bytes();
    for chunk in raw.chunks_exact(2) {
        let high = decode_hex_digit(chunk[0])?;
        let low = decode_hex_digit(chunk[1])?;
        bytes.push((high << 4) | low);
    }
    Ok(bytes)
}

fn decode_hex_digit(value: u8) -> Result<u8, String> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        b'A'..=b'F' => Ok(value - b'A' + 10),
        _ => Err("invalid hex body".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::{decode, encode, Message, Role, OPENCLAW_GATEWAY_CAPABILITY};

    #[test]
    fn round_trips_request_with_body() {
        let message = Message::Request {
            id: 7,
            capability: OPENCLAW_GATEWAY_CAPABILITY.to_string(),
            method: "POST".to_string(),
            path: "/v1/turn".to_string(),
            body: br#"{"hello":"world"}"#.to_vec(),
        };

        let encoded = encode(&message).unwrap();
        assert_eq!(decode(&encoded).unwrap(), message);
    }

    #[test]
    fn round_trips_hello() {
        let message = Message::Hello {
            role: Role::Client,
            device_id: "device_123".to_string(),
        };

        let encoded = encode(&message).unwrap();
        assert_eq!(decode(&encoded).unwrap(), message);
    }

    #[test]
    fn rejects_parent_path_traversal() {
        let message = Message::Request {
            id: 1,
            capability: OPENCLAW_GATEWAY_CAPABILITY.to_string(),
            method: "GET".to_string(),
            path: "/../secret".to_string(),
            body: Vec::new(),
        };

        assert!(encode(&message).unwrap_err().contains("traversal"));
    }

    #[test]
    fn rejects_oversized_body() {
        let message = Message::Response {
            id: 1,
            status: 200,
            body: vec![0; super::MAX_BODY_BYTES + 1],
        };

        assert!(encode(&message).unwrap_err().contains("too large"));
    }
}
