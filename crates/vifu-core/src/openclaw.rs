use std::collections::HashSet;
use std::time::Duration;

use serde_json::{json, Value};

use crate::protocol::{self, AgentDescriptor};

const MAX_HTTP_RESPONSE_BYTES: usize = protocol::MAX_BODY_BYTES;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeStatus {
    Online,
    Offline(String),
    Unsupported(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeReport {
    pub endpoint: Endpoint,
    pub status: ProbeStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

pub async fn probe(url: &str) -> ProbeReport {
    match parse_endpoint(url) {
        Ok(endpoint) => {
            let status = probe_endpoint(&endpoint).await;
            ProbeReport { endpoint, status }
        }
        Err(error) => ProbeReport {
            endpoint: Endpoint {
                host: "invalid".to_string(),
                port: 0,
            },
            status: ProbeStatus::Unsupported(error),
        },
    }
}

pub fn parse_endpoint(url: &str) -> Result<Endpoint, String> {
    let raw = url.trim();
    let rest = raw
        .strip_prefix("http://")
        .ok_or_else(|| "only local http:// OpenClaw Gateway URLs are supported".to_string())?;
    let authority = rest.split('/').next().unwrap_or(rest);
    let (host, port) = parse_authority(authority)?;

    if !is_local_openclaw_host(&host) {
        return Err("only local OpenClaw Gateway hosts are supported".to_string());
    }

    Ok(Endpoint { host, port })
}

fn parse_authority(authority: &str) -> Result<(String, u16), String> {
    if authority.is_empty() {
        return Err("OpenClaw Gateway URL is missing a host".to_string());
    }

    if let Some(rest) = authority.strip_prefix('[') {
        let end = rest
            .find(']')
            .ok_or_else(|| "invalid IPv6 loopback URL".to_string())?;
        let host = rest[..end].to_string();
        let port = rest[end + 1..]
            .strip_prefix(':')
            .map(parse_port)
            .transpose()?
            .unwrap_or(18789);
        return Ok((host, port));
    }

    let mut parts = authority.split(':');
    let host = parts.next().unwrap_or_default().to_string();
    let port = match (parts.next(), parts.next()) {
        (Some(value), None) => parse_port(value)?,
        (None, None) => 18789,
        _ => return Err("invalid OpenClaw Gateway authority".to_string()),
    };

    Ok((host, port))
}

fn parse_port(value: &str) -> Result<u16, String> {
    value
        .parse::<u16>()
        .map_err(|_| format!("invalid OpenClaw Gateway port: {value}"))
}

fn is_local_openclaw_host(host: &str) -> bool {
    matches!(
        host,
        "127.0.0.1" | "localhost" | "::1" | "host.docker.internal"
    )
}

async fn probe_endpoint(endpoint: &Endpoint) -> ProbeStatus {
    match request_with_auth(
        endpoint,
        "GET",
        "/health",
        &[],
        Duration::from_secs(2),
        None,
    )
    .await
    {
        Ok(response) if (200..=299).contains(&response.status) => ProbeStatus::Online,
        Ok(_) => ProbeStatus::Offline("Gateway responded without healthy status".to_string()),
        Err(error) => ProbeStatus::Offline(error),
    }
}

async fn request_with_auth(
    endpoint: &Endpoint,
    method: &str,
    path: &str,
    body: &[u8],
    timeout: Duration,
    token: Option<&str>,
) -> Result<GatewayResponse, String> {
    protocol::validate_method(method)?;
    protocol::validate_path(path)?;
    protocol::validate_body(body)?;

    let host = if endpoint.host.contains(':') {
        format!("[{}]", endpoint.host)
    } else {
        endpoint.host.clone()
    };
    let url = format!("http://{host}:{}{path}", endpoint.port);
    let method = reqwest::Method::from_bytes(method.as_bytes())
        .map_err(|_| "invalid OpenClaw HTTP method".to_string())?;
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| error.to_string())?;
    let mut request = client
        .request(method, url)
        .header("Accept", "application/json")
        .header("User-Agent", concat!("vifu/", env!("CARGO_PKG_VERSION")));
    if !body.is_empty() {
        request = request
            .header("Content-Type", "application/json")
            .body(body.to_vec());
    }
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    let mut response = request.send().await.map_err(|error| error.to_string())?;
    let status = response.status().as_u16();
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|error| error.to_string())? {
        if body.len().saturating_add(chunk.len()) > MAX_HTTP_RESPONSE_BYTES {
            return Err("OpenClaw response is too large".to_string());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(GatewayResponse { status, body })
}

pub async fn discover_agents(
    endpoint: &Endpoint,
    token: Option<&str>,
) -> Result<Vec<AgentDescriptor>, String> {
    let response = request_with_auth(
        endpoint,
        "GET",
        "/v1/models",
        &[],
        Duration::from_secs(5),
        token,
    )
    .await?;
    ensure_openclaw_status(&response, "agent discovery")?;
    let payload = serde_json::from_slice::<Value>(&response.body).map_err(|_| {
        "OpenClaw agent discovery is unavailable; enable gateway.http.endpoints.chatCompletions"
            .to_string()
    })?;
    let agents = read_models(payload);
    if agents.is_empty() {
        Err("OpenClaw returned no agent models".to_string())
    } else {
        Ok(agents)
    }
}

pub async fn invoke(
    endpoint: &Endpoint,
    token: Option<&str>,
    agent_id: &str,
    _binding: &Value,
    input: &Value,
    timeout: Duration,
) -> Result<Value, String> {
    protocol::validate_identifier("agent id", agent_id)?;
    let request = openclaw_chat_request(agent_id, input)?;
    let body = serde_json::to_vec(&request).map_err(|error| error.to_string())?;
    let response = request_with_auth(
        endpoint,
        "POST",
        "/v1/chat/completions",
        &body,
        timeout,
        token,
    )
    .await?;
    ensure_openclaw_status(&response, "agent invocation")?;
    let payload = serde_json::from_slice::<Value>(&response.body)
        .map_err(|_| "OpenClaw returned an invalid chat completion".to_string())?;
    if input.get("messages").is_some() {
        return Ok(payload);
    }
    let reply = payload
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "OpenClaw chat completion did not contain a reply".to_string())?;
    Ok(json!({
        "agentId": agent_id,
        "reply": reply,
        "finishReason": payload.pointer("/choices/0/finish_reason").cloned(),
        "usage": payload.get("usage").cloned(),
    }))
}

fn openclaw_chat_request(agent_id: &str, input: &Value) -> Result<Value, String> {
    if input.get("messages").is_some() {
        let mut request = input
            .as_object()
            .ok_or_else(|| "chat completion request must be an object".to_string())?
            .clone();
        request.insert("model".to_string(), Value::String(openclaw_model(agent_id)));
        request.insert("stream".to_string(), Value::Bool(false));
        return Ok(Value::Object(request));
    }

    let mut request = json!({
        "model": openclaw_model(agent_id),
        "stream": false,
        "messages": [{ "role": "user", "content": input_text(input)? }],
    });
    if let Some(user) = conversation_user(input) {
        request
            .as_object_mut()
            .expect("chat completion request is an object")
            .insert("user".to_string(), Value::String(user));
    }
    Ok(request)
}

fn read_models(value: Value) -> Vec<AgentDescriptor> {
    let items = match value {
        Value::Object(mut object) => object
            .remove("data")
            .and_then(|value| value.as_array().cloned())
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    let mut seen = HashSet::new();
    let mut has_default = false;
    let mut agents = items
        .into_iter()
        .filter_map(|item| {
            let object = item.as_object()?;
            let model_id = object.get("id")?.as_str()?.trim();
            let id = if matches!(model_id, "openclaw" | "openclaw/default") {
                has_default = true;
                return None;
            } else if let Some(id) = model_id.strip_prefix("openclaw/") {
                id
            } else {
                model_id.strip_prefix("openclaw:")?
            };
            if protocol::validate_identifier("agent id", id).is_err()
                || !seen.insert(id.to_string())
            {
                return None;
            }
            let name = object
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .unwrap_or(id);
            Some(AgentDescriptor {
                id: id.to_string(),
                name: name.chars().take(128).collect(),
                metadata: json!({}),
            })
        })
        .take(protocol::MAX_AGENTS)
        .collect::<Vec<_>>();
    if agents.is_empty() && has_default {
        agents.push(AgentDescriptor {
            id: "default".to_string(),
            name: "Default agent".to_string(),
            metadata: json!({ "model": "openclaw/default" }),
        });
    }
    agents
}

fn openclaw_model(agent_id: &str) -> String {
    if agent_id == "default" {
        "openclaw/default".to_string()
    } else {
        format!("openclaw/{agent_id}")
    }
}

fn input_text(input: &Value) -> Result<String, String> {
    if let Some(message) = input
        .get("message")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|message| !message.is_empty())
    {
        return Ok(message.to_string());
    }
    if let Some(value) = input.get("input").filter(|value| !value.is_null()) {
        return serde_json::to_string(value).map_err(|error| error.to_string());
    }
    Err("OpenClaw invocation requires message or input".to_string())
}

fn conversation_user(input: &Value) -> Option<String> {
    let value = input
        .pointer("/context/conversationId")
        .or_else(|| input.pointer("/context/sessionId"))?
        .as_str()?
        .trim();
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return None;
    }
    Some(value.to_string())
}

fn ensure_openclaw_status(response: &GatewayResponse, operation: &str) -> Result<(), String> {
    if (200..=299).contains(&response.status) {
        return Ok(());
    }
    if matches!(response.status, 401 | 403) {
        return Err(format!(
            "OpenClaw {operation} requires an agent provider token"
        ));
    }
    let message = serde_json::from_slice::<Value>(&response.body)
        .ok()
        .and_then(|payload| {
            payload
                .pointer("/error/message")
                .or_else(|| payload.get("error"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|message| !message.is_empty())
                .map(|message| message.chars().take(256).collect::<String>())
        });
    Err(message.unwrap_or_else(|| {
        format!(
            "OpenClaw {operation} failed with status {}",
            response.status
        )
    }))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        conversation_user, input_text, openclaw_model, parse_endpoint, read_models, ProbeStatus,
    };

    #[test]
    fn parses_default_loopback_endpoint() {
        let endpoint = parse_endpoint("http://127.0.0.1:18789").unwrap();
        assert_eq!(endpoint.host, "127.0.0.1");
        assert_eq!(endpoint.port, 18789);
    }

    #[test]
    fn parses_docker_host_endpoint() {
        let endpoint = parse_endpoint("http://host.docker.internal:18789").unwrap();
        assert_eq!(endpoint.host, "host.docker.internal");
        assert_eq!(endpoint.port, 18789);
    }

    #[test]
    fn rejects_remote_hosts() {
        let error = parse_endpoint("http://example.com:18789").unwrap_err();
        assert!(error.contains("local"));
    }

    #[test]
    fn unsupported_status_is_debuggable() {
        let status = ProbeStatus::Unsupported("bad url".to_string());
        assert_eq!(format!("{status:?}"), "Unsupported(\"bad url\")");
    }

    #[test]
    fn reads_openclaw_agent_models() {
        let agents = read_models(json!({
            "data": [
                { "id": "openclaw", "name": "OpenClaw" },
                { "id": "openclaw/default", "name": "Default" },
                { "id": "openclaw/guide-agent", "name": "Guide" }
            ]
        }));
        assert_eq!(agents[0].id, "guide-agent");
        assert_eq!(agents[0].name, "Guide");
    }

    #[test]
    fn maps_default_and_named_agents_to_models() {
        assert_eq!(openclaw_model("default"), "openclaw/default");
        assert_eq!(openclaw_model("writer"), "openclaw/writer");
    }

    #[test]
    fn reads_message_or_structured_input() {
        assert_eq!(
            input_text(&json!({ "message": " hello " })).unwrap(),
            "hello"
        );
        assert_eq!(
            input_text(&json!({ "input": { "task": 1 } })).unwrap(),
            "{\"task\":1}"
        );
    }

    #[test]
    fn maps_safe_conversation_ids_to_openclaw_users() {
        assert_eq!(
            conversation_user(&json!({ "context": { "conversationId": "town-session_1" } })),
            Some("town-session_1".to_string())
        );
        assert_eq!(
            conversation_user(&json!({ "context": { "conversationId": "not safe/value" } })),
            None
        );
    }
}
