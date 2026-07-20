use std::collections::HashSet;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

use crate::openclaw::Endpoint;

const OPENCLAW_PROTOCOL_VERSION: u8 = 4;
const MAX_FRAME_BYTES: usize = 1024 * 1024;
const RPC_TIMEOUT: Duration = Duration::from_secs(10);
const CLIENT_ID: &str = "gateway-client";
const CLIENT_MODE: &str = "backend";
const CLIENT_ROLE: &str = "operator";
const CLIENT_SCOPES: [&str; 3] = ["operator.read", "operator.write", "operator.admin"];

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub trait OpenClawDeviceSigner: Send + Sync {
    fn device_id(&self) -> &str;
    fn public_key(&self) -> &str;
    fn sign(&self, payload: &str) -> Result<String, String>;
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenClawAgent {
    pub id: String,
    pub name: Option<String>,
    pub workspace: Option<String>,
    #[serde(default)]
    pub model: Option<Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenClawAgentFile {
    pub name: String,
    pub missing: bool,
    pub size: Option<u64>,
    pub updated_at_ms: Option<u64>,
    pub content: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentListResult {
    agents: Vec<OpenClawAgent>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentFilesResult {
    files: Vec<OpenClawAgentFile>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentFileResult {
    file: OpenClawAgentFile,
}

/// A short-lived client for OpenClaw's documented Gateway request protocol.
///
/// Vifu uses this control connection only for provider-owned agent metadata,
/// persona files, and tools. Agent traffic continues through Vifu's own Agent
/// Gateway connection so games never depend on the provider protocol.
pub struct OpenClawGatewayClient {
    socket: Socket,
    methods: HashSet<String>,
    next_request_id: u64,
}

impl OpenClawGatewayClient {
    pub async fn connect(
        endpoint: &Endpoint,
        token: Option<&str>,
        device: Option<&dyn OpenClawDeviceSigner>,
    ) -> Result<Self, String> {
        let url = websocket_url(endpoint);
        let (socket, _) = tokio::time::timeout(RPC_TIMEOUT, connect_async(&url))
            .await
            .map_err(|_| "OpenClaw Gateway connection timed out".to_string())?
            .map_err(|error| format!("OpenClaw Gateway connection failed: {error}"))?;
        let mut client = Self {
            socket,
            methods: HashSet::new(),
            next_request_id: 1,
        };
        let nonce = client.wait_for_challenge().await?;
        let token = token.map(str::trim).filter(|value| !value.is_empty());
        let mut params = json!({
            "minProtocol": OPENCLAW_PROTOCOL_VERSION,
            "maxProtocol": OPENCLAW_PROTOCOL_VERSION,
            "client": {
                "id": CLIENT_ID,
                "displayName": "Vifu",
                "version": env!("CARGO_PKG_VERSION"),
                "platform": std::env::consts::OS,
                "mode": CLIENT_MODE
            },
            "caps": [],
            "role": CLIENT_ROLE,
            "scopes": CLIENT_SCOPES
        });
        if let Some(token) = token {
            params
                .as_object_mut()
                .ok_or_else(|| "OpenClaw connect payload is invalid".to_string())?
                .insert("auth".to_string(), json!({ "token": token }));
        }
        if let Some(device) = device {
            let signed_at_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| "system clock is before the Unix epoch".to_string())?
                .as_millis();
            let signed_at_ms = u64::try_from(signed_at_ms)
                .map_err(|_| "system clock value is too large".to_string())?;
            let payload = device_auth_payload(
                device.device_id(),
                signed_at_ms,
                token,
                &nonce,
                std::env::consts::OS,
            );
            let signature = device.sign(&payload)?;
            params
                .as_object_mut()
                .ok_or_else(|| "OpenClaw connect payload is invalid".to_string())?
                .insert(
                    "device".to_string(),
                    json!({
                        "id": device.device_id(),
                        "publicKey": device.public_key(),
                        "signature": signature,
                        "signedAt": signed_at_ms,
                        "nonce": nonce,
                    }),
                );
        }
        let hello = client.call_unchecked("connect", params).await?;
        let protocol = hello
            .get("protocol")
            .and_then(Value::as_u64)
            .ok_or_else(|| "OpenClaw Gateway returned an invalid hello response".to_string())?;
        if protocol != u64::from(OPENCLAW_PROTOCOL_VERSION) {
            return Err(format!(
                "OpenClaw Gateway negotiated unsupported protocol {protocol}"
            ));
        }
        client.methods = hello
            .pointer("/features/methods")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();
        Ok(client)
    }

    pub async fn agents(&mut self) -> Result<Vec<OpenClawAgent>, String> {
        let payload = self.call("agents.list", json!({})).await?;
        serde_json::from_value::<AgentListResult>(payload)
            .map(|result| result.agents)
            .map_err(|error| format!("OpenClaw returned an invalid agent list: {error}"))
    }

    pub async fn list_agent_files(
        &mut self,
        agent_id: &str,
    ) -> Result<Vec<OpenClawAgentFile>, String> {
        let payload = self
            .call("agents.files.list", json!({ "agentId": agent_id }))
            .await?;
        serde_json::from_value::<AgentFilesResult>(payload)
            .map(|result| result.files)
            .map_err(|error| format!("OpenClaw returned an invalid agent file list: {error}"))
    }

    pub async fn get_agent_file(
        &mut self,
        agent_id: &str,
        name: &str,
    ) -> Result<OpenClawAgentFile, String> {
        let payload = self
            .call(
                "agents.files.get",
                json!({ "agentId": agent_id, "name": name }),
            )
            .await?;
        serde_json::from_value::<AgentFileResult>(payload)
            .map(|result| result.file)
            .map_err(|error| format!("OpenClaw returned an invalid agent file: {error}"))
    }

    pub async fn set_agent_file(
        &mut self,
        agent_id: &str,
        name: &str,
        content: &str,
    ) -> Result<OpenClawAgentFile, String> {
        let payload = self
            .call(
                "agents.files.set",
                json!({ "agentId": agent_id, "name": name, "content": content }),
            )
            .await?;
        serde_json::from_value::<AgentFileResult>(payload)
            .map(|result| result.file)
            .map_err(|error| format!("OpenClaw returned an invalid saved agent file: {error}"))
    }

    pub async fn create_agent(
        &mut self,
        name: &str,
        workspace: &str,
        model: Option<&str>,
    ) -> Result<String, String> {
        let mut params = json!({ "name": name, "workspace": workspace });
        if let Some(model) = model.map(str::trim).filter(|value| !value.is_empty()) {
            params
                .as_object_mut()
                .ok_or_else(|| "OpenClaw agent payload is invalid".to_string())?
                .insert("model".to_string(), Value::String(model.to_string()));
        }
        let payload = self.call("agents.create", params).await?;
        payload
            .get("agentId")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| "OpenClaw did not return the created agent ID".to_string())
    }

    pub async fn delete_agent(&mut self, agent_id: &str) -> Result<(), String> {
        self.call(
            "agents.delete",
            json!({ "agentId": agent_id, "deleteFiles": true }),
        )
        .await?;
        Ok(())
    }

    pub async fn tools_catalog(&mut self, agent_id: &str) -> Result<Value, String> {
        self.call("tools.catalog", json!({ "agentId": agent_id }))
            .await
    }

    pub async fn close(mut self) -> Result<(), String> {
        self.socket
            .close(None)
            .await
            .map_err(|error| format!("OpenClaw Gateway close failed: {error}"))
    }

    async fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        if !self.methods.contains(method) {
            return Err(format!(
                "OpenClaw Gateway does not advertise required method {method}"
            ));
        }
        self.call_unchecked(method, params).await
    }

    async fn call_unchecked(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let request_id = format!("vifu-{}", self.next_request_id);
        self.next_request_id = self.next_request_id.saturating_add(1);
        let request = json!({
            "type": "req",
            "id": request_id,
            "method": method,
            "params": params,
        });
        self.socket
            .send(Message::Text(request.to_string().into()))
            .await
            .map_err(|error| format!("OpenClaw Gateway request failed: {error}"))?;

        tokio::time::timeout(RPC_TIMEOUT, async {
            loop {
                let frame = self.receive_json().await?;
                if frame.get("type").and_then(Value::as_str) != Some("res")
                    || frame.get("id").and_then(Value::as_str) != Some(request_id.as_str())
                {
                    continue;
                }
                if frame.get("ok").and_then(Value::as_bool) == Some(true) {
                    return Ok(frame.get("payload").cloned().unwrap_or(Value::Null));
                }
                let message = frame
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .map(sanitize_error)
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| "request was rejected".to_string());
                if method == "connect"
                    && frame.pointer("/error/details/code").and_then(Value::as_str)
                        == Some("PAIRING_REQUIRED")
                {
                    return Err(pairing_required_error(&frame, &message));
                }
                return Err(format!("OpenClaw {method} failed: {message}"));
            }
        })
        .await
        .map_err(|_| format!("OpenClaw {method} timed out"))?
    }

    async fn wait_for_challenge(&mut self) -> Result<String, String> {
        tokio::time::timeout(RPC_TIMEOUT, async {
            loop {
                let frame = self.receive_json().await?;
                if frame.get("type").and_then(Value::as_str) == Some("event")
                    && frame.get("event").and_then(Value::as_str) == Some("connect.challenge")
                {
                    if let Some(nonce) = frame
                        .pointer("/payload/nonce")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|nonce| !nonce.is_empty())
                    {
                        return Ok(nonce.to_string());
                    }
                }
            }
        })
        .await
        .map_err(|_| "OpenClaw Gateway did not send a connect challenge".to_string())?
    }

    async fn receive_json(&mut self) -> Result<Value, String> {
        loop {
            let message = self
                .socket
                .next()
                .await
                .ok_or_else(|| "OpenClaw Gateway closed the connection".to_string())?
                .map_err(|error| format!("OpenClaw Gateway receive failed: {error}"))?;
            match message {
                Message::Text(text) => {
                    if text.len() > MAX_FRAME_BYTES {
                        return Err("OpenClaw Gateway frame is too large".to_string());
                    }
                    return serde_json::from_str(text.as_str()).map_err(|error| {
                        format!("OpenClaw Gateway returned invalid JSON: {error}")
                    });
                }
                Message::Binary(bytes) => {
                    if bytes.len() > MAX_FRAME_BYTES {
                        return Err("OpenClaw Gateway frame is too large".to_string());
                    }
                    return serde_json::from_slice(&bytes).map_err(|error| {
                        format!("OpenClaw Gateway returned invalid JSON: {error}")
                    });
                }
                Message::Ping(payload) => {
                    self.socket
                        .send(Message::Pong(payload))
                        .await
                        .map_err(|error| format!("OpenClaw Gateway pong failed: {error}"))?;
                }
                Message::Close(_) => {
                    return Err("OpenClaw Gateway closed the connection".to_string());
                }
                Message::Pong(_) | Message::Frame(_) => {}
            }
        }
    }
}

fn device_auth_payload(
    device_id: &str,
    signed_at_ms: u64,
    token: Option<&str>,
    nonce: &str,
    platform: &str,
) -> String {
    [
        "v3".to_string(),
        device_id.to_string(),
        CLIENT_ID.to_string(),
        CLIENT_MODE.to_string(),
        CLIENT_ROLE.to_string(),
        CLIENT_SCOPES.join(","),
        signed_at_ms.to_string(),
        token.unwrap_or_default().to_string(),
        nonce.to_string(),
        normalize_device_metadata(platform),
        String::new(),
    ]
    .join("|")
}

fn normalize_device_metadata(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn pairing_required_error(frame: &Value, fallback: &str) -> String {
    let request_id = frame
        .pointer("/error/details/requestId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| valid_pairing_request_id(value));
    match request_id {
        Some(request_id) => format!(
            "OpenClaw connect failed: device pairing required. Run `openclaw devices approve {request_id}` on the OpenClaw host, then retry"
        ),
        None => format!("OpenClaw connect failed: {fallback}"),
    }
}

fn valid_pairing_request_id(value: &str) -> bool {
    value.len() <= 128
        && value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
}

fn websocket_url(endpoint: &Endpoint) -> String {
    let host = if endpoint.host.contains(':') {
        format!("[{}]", endpoint.host)
    } else {
        endpoint.host.clone()
    };
    format!("ws://{host}:{}/", endpoint.port)
}

fn sanitize_error(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(512)
        .collect::<String>()
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use futures_util::{SinkExt, StreamExt};
    use serde_json::{json, Value};
    use tokio::net::TcpListener;
    use tokio_tungstenite::accept_async;
    use tokio_tungstenite::tungstenite::Message;

    use super::{pairing_required_error, Endpoint, OpenClawDeviceSigner, OpenClawGatewayClient};

    struct TestDeviceSigner;

    impl OpenClawDeviceSigner for TestDeviceSigner {
        fn device_id(&self) -> &str {
            "test-device-id"
        }

        fn public_key(&self) -> &str {
            "test-public-key"
        }

        fn sign(&self, payload: &str) -> Result<String, String> {
            Ok(format!("signed:{payload}"))
        }
    }

    #[tokio::test]
    async fn client_should_complete_gateway_handshake_before_calling_agents() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            socket
                .send(Message::Text(
                    json!({
                        "type": "event",
                        "event": "connect.challenge",
                        "payload": { "nonce": "unit-test-nonce" }
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .unwrap();

            let connect = receive_value(&mut socket).await;
            assert_eq!(
                connect.get("method").and_then(Value::as_str),
                Some("connect")
            );
            assert_eq!(
                connect
                    .pointer("/params/auth/token")
                    .and_then(Value::as_str),
                Some("test-token")
            );
            assert_eq!(
                connect.pointer("/params/device/id").and_then(Value::as_str),
                Some("test-device-id")
            );
            assert_eq!(
                connect
                    .pointer("/params/device/publicKey")
                    .and_then(Value::as_str),
                Some("test-public-key")
            );
            let signed_at = connect
                .pointer("/params/device/signedAt")
                .and_then(Value::as_u64)
                .unwrap();
            let expected_payload = format!(
                "v3|test-device-id|gateway-client|backend|operator|operator.read,operator.write,operator.admin|{signed_at}|test-token|unit-test-nonce|{}|",
                std::env::consts::OS.to_ascii_lowercase()
            );
            assert_eq!(
                connect
                    .pointer("/params/device/signature")
                    .and_then(Value::as_str),
                Some(format!("signed:{expected_payload}").as_str())
            );
            let connect_id = connect.get("id").and_then(Value::as_str).unwrap();
            socket
                .send(Message::Text(
                    json!({
                        "type": "res",
                        "id": connect_id,
                        "ok": true,
                        "payload": {
                            "type": "hello-ok",
                            "protocol": 4,
                            "features": { "methods": ["agents.list"], "events": [] }
                        }
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .unwrap();

            let request = receive_value(&mut socket).await;
            assert_eq!(
                request.get("method").and_then(Value::as_str),
                Some("agents.list")
            );
            let request_id = request.get("id").and_then(Value::as_str).unwrap();
            socket
                .send(Message::Text(
                    json!({
                        "type": "res",
                        "id": request_id,
                        "ok": true,
                        "payload": {
                            "agents": [{ "id": "steward", "name": "Steward" }]
                        }
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .unwrap();
        });

        let endpoint = Endpoint {
            host: "127.0.0.1".to_string(),
            port,
        };
        let signer = TestDeviceSigner;
        let mut client =
            OpenClawGatewayClient::connect(&endpoint, Some("test-token"), Some(&signer))
                .await
                .unwrap();
        let agents = client.agents().await.unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].id, "steward");
        let _ = client.close().await;
        server.await.unwrap();
    }

    #[test]
    fn pairing_errors_include_only_safe_approval_request_ids() {
        let frame = json!({
            "error": {
                "details": {
                    "code": "PAIRING_REQUIRED",
                    "requestId": "request_01:pending"
                }
            }
        });
        assert_eq!(
            pairing_required_error(&frame, "device pairing required"),
            "OpenClaw connect failed: device pairing required. Run `openclaw devices approve request_01:pending` on the OpenClaw host, then retry"
        );

        let unsafe_frame = json!({
            "error": {
                "details": {
                    "code": "PAIRING_REQUIRED",
                    "requestId": "request; echo unsafe"
                }
            }
        });
        assert_eq!(
            pairing_required_error(&unsafe_frame, "device pairing required"),
            "OpenClaw connect failed: device pairing required"
        );
    }

    async fn receive_value<S>(socket: &mut tokio_tungstenite::WebSocketStream<S>) -> Value
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        let message = socket.next().await.unwrap().unwrap();
        let Message::Text(text) = message else {
            panic!("expected a text frame");
        };
        serde_json::from_str(text.as_str()).unwrap()
    }
}
