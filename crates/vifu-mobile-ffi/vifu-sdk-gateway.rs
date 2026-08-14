//! Native Gateway host used by the TypeScript SDK.
//!
//! Agent handlers remain in the parent TypeScript process. This process owns
//! the existing Rust Gateway so TLS pinning, device identity, reconnect, and
//! trace delivery use the same implementation as the mobile SDKs.

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use vifu_mobile_ffi::{
    generate_vifu_gateway_identity, VifuEmbeddedGateway, VifuEmbeddedGatewayConfig,
    VifuEmbeddedGatewayState, VifuEmbeddedRuntime, VifuInvocationData, VifuProviderInvocation,
    VifuProviderRequest, VifuProviderResponse, VifuProviderStage, VifuRuntimeError,
    VifuStreamingAgentProvider,
};

const SDK_VERSION: &str = env!("CARGO_PKG_VERSION");
const PROVIDER_RESPONSE_TIMEOUT: Duration = Duration::from_secs(125);

fn main() {
    if let Err(error) = run() {
        let _ = writeln!(io::stderr(), "Vifu TypeScript Gateway stopped: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let stdin = io::stdin();
    let mut lines = BufReader::new(stdin.lock()).lines();
    let start = lines
        .next()
        .ok_or_else(|| "the Gateway host did not receive a start message".to_string())?
        .map_err(|error| error.to_string())?;
    let start = serde_json::from_str::<StartMessage>(&start)
        .map_err(|error| format!("the Gateway start message is invalid: {error}"))?;
    if start.message_type != "start" {
        return Err("the first Gateway host message must be start".to_string());
    }

    let writer = Arc::new(Mutex::new(io::stdout()));
    let pending = Arc::new(Mutex::new(HashMap::new()));
    let sequence = Arc::new(AtomicU64::new(1));
    let bridge = Bridge {
        writer: Arc::clone(&writer),
        pending: Arc::clone(&pending),
        sequence,
    };
    let host = start_gateway(start, bridge, Arc::clone(&writer))?;

    for line in lines {
        let line = line.map_err(|error| error.to_string())?;
        let value = serde_json::from_str::<Value>(&line)
            .map_err(|error| format!("the Gateway host message is invalid: {error}"))?;
        match value.get("type").and_then(Value::as_str) {
            Some("trace" | "result") => deliver_result(&pending, value)?,
            Some("stop") => break,
            Some(other) => return Err(format!("unsupported Gateway host message: {other}")),
            None => return Err("Gateway host message type is required".to_string()),
        }
    }

    host.shutdown.store(true, Ordering::Release);
    host.gateway.stop().map_err(|error| error.to_string())?;
    let _ = host.watcher.join();
    Ok(())
}

struct Host {
    gateway: Arc<VifuEmbeddedGateway>,
    shutdown: Arc<AtomicBool>,
    watcher: thread::JoinHandle<()>,
}

fn start_gateway(
    start: StartMessage,
    bridge: Bridge,
    writer: Arc<Mutex<io::Stdout>>,
) -> Result<Host, String> {
    validate_identifier("app ID", &start.app_id)?;
    if start.agents.is_empty() {
        return Err("at least one agent is required before connecting a Gateway".to_string());
    }
    fs::create_dir_all(&start.data_dir).map_err(|error| error.to_string())?;
    let credentials_path = start.data_dir.join("gateway.json");
    let mut credentials = load_credentials(&credentials_path)?;
    let server_url = start
        .server_url
        .clone()
        .or_else(|| credentials.as_ref().map(|stored| stored.server_url.clone()))
        .ok_or_else(|| "a pairing code is required for the first Gateway connection".to_string())?;
    if credentials
        .as_ref()
        .is_some_and(|stored| stored.server_url != server_url)
    {
        credentials = None;
    }
    let mut credentials = match credentials {
        Some(credentials) => credentials,
        None => {
            let identity = generate_vifu_gateway_identity().map_err(|error| error.to_string())?;
            GatewayCredentials {
                server_url: server_url.clone(),
                machine_private_key: identity.private_key,
                device_token: None,
                certificate_der: None,
            }
        }
    };
    if let Some(certificate) = start.server_certificate_der {
        credentials.certificate_der = Some(certificate);
    }
    save_credentials(&credentials_path, &credentials)?;

    let runtime_path = start.data_dir.join("runtime.sqlite");
    let runtime = VifuEmbeddedRuntime::open(
        start.app_id.clone(),
        runtime_path.to_string_lossy().into_owned(),
    )
    .map_err(|error| error.to_string())?;
    let provider_ids = start
        .agents
        .iter()
        .map(|agent| agent.provider_id.clone())
        .collect::<BTreeSet<_>>();
    for provider_id in provider_ids {
        runtime
            .register_streaming_provider(
                provider_id,
                "typescript".to_string(),
                Box::new(bridge.clone()),
            )
            .map_err(|error| error.to_string())?;
    }
    for agent in start.agents {
        validate_identifier("agent ID", &agent.id)?;
        validate_identifier("provider ID", &agent.provider_id)?;
        validate_identifier("endpoint", &agent.endpoint)?;
        runtime
            .register_agent(
                agent.id.clone(),
                agent.name,
                agent.provider_id,
                vec![agent.capability.clone()],
                serde_json::to_string(&agent.metadata).map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
        runtime
            .register_endpoint(agent.endpoint, agent.id, agent.capability, agent.timeout_ms)
            .map_err(|error| error.to_string())?;
    }

    let metadata = merge_metadata(start.metadata, &start.name);
    let gateway = VifuEmbeddedGateway::new(
        Arc::clone(&runtime),
        VifuEmbeddedGatewayConfig {
            server_url,
            runtime_database_path: runtime_path.to_string_lossy().into_owned(),
            server_certificate_der: credentials.certificate_der.clone(),
            gateway_metadata_json: serde_json::to_string(&metadata)
                .map_err(|error| error.to_string())?,
        },
    )
    .map_err(|error| error.to_string())?;
    if start.capture_trace_content {
        gateway
            .start_with_monitor_io(
                credentials.machine_private_key.clone(),
                credentials.device_token.clone(),
                start.enrollment_token,
                true,
            )
            .map_err(|error| error.to_string())?;
    } else {
        gateway
            .start(
                credentials.machine_private_key.clone(),
                credentials.device_token.clone(),
                start.enrollment_token,
            )
            .map_err(|error| error.to_string())?;
    }

    send(&writer, &json!({ "type": "started" }))?;
    let shutdown = Arc::new(AtomicBool::new(false));
    let watcher = spawn_status_watcher(
        Arc::clone(&gateway),
        credentials_path,
        credentials,
        Arc::clone(&writer),
        Arc::clone(&shutdown),
    );
    Ok(Host {
        gateway,
        shutdown,
        watcher,
    })
}

fn merge_metadata(metadata: Value, name: &Option<String>) -> Value {
    let mut metadata = metadata.as_object().cloned().unwrap_or_default();
    metadata.insert(
        "runtime".to_string(),
        Value::String("typescript".to_string()),
    );
    metadata.insert(
        "sdkVersion".to_string(),
        Value::String(SDK_VERSION.to_string()),
    );
    if let Some(name) = name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        metadata.insert("name".to_string(), Value::String(name.to_string()));
    }
    Value::Object(metadata)
}

fn spawn_status_watcher(
    gateway: Arc<VifuEmbeddedGateway>,
    credentials_path: PathBuf,
    mut credentials: GatewayCredentials,
    writer: Arc<Mutex<io::Stdout>>,
    shutdown: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut last_status = None;
        while !shutdown.load(Ordering::Acquire) {
            match gateway.status() {
                Ok(status) => {
                    if let Some(authorization) = status.authorization.as_ref() {
                        if credentials.device_token.as_deref()
                            != Some(authorization.device_token.as_str())
                        {
                            credentials.device_token = Some(authorization.device_token.clone());
                            if let Err(error) = save_credentials(&credentials_path, &credentials) {
                                let _ = send(
                                    &writer,
                                    &json!({ "type": "status", "state": "failed", "lastError": error }),
                                );
                            }
                        }
                    }
                    let state = gateway_state(status.state);
                    let signature = format!(
                        "{}|{}|{}|{}",
                        state,
                        status.last_error.as_deref().unwrap_or_default(),
                        status
                            .authorization
                            .as_ref()
                            .map(|value| value.gateway_id.as_str())
                            .unwrap_or_default(),
                        status
                            .pairing
                            .as_ref()
                            .map(|value| value.auth_url.as_str())
                            .unwrap_or_default(),
                    );
                    if last_status.as_deref() != Some(signature.as_str()) {
                        let _ = send(
                            &writer,
                            &json!({
                                "type": "status",
                                "state": state,
                                "lastError": status.last_error,
                                "gatewayId": status.authorization.map(|value| value.gateway_id),
                                "pairingUrl": status.pairing.map(|value| value.auth_url),
                            }),
                        );
                        last_status = Some(signature);
                    }
                }
                Err(error) => {
                    let _ = send(
                        &writer,
                        &json!({ "type": "status", "state": "failed", "lastError": error.to_string() }),
                    );
                }
            }
            thread::sleep(Duration::from_millis(100));
        }
    })
}

fn gateway_state(state: VifuEmbeddedGatewayState) -> &'static str {
    match state {
        VifuEmbeddedGatewayState::Stopped => "stopped",
        VifuEmbeddedGatewayState::Connecting => "connecting",
        VifuEmbeddedGatewayState::Connected => "connected",
        VifuEmbeddedGatewayState::Reconnecting => "reconnecting",
        VifuEmbeddedGatewayState::AuthorizationRequired => "authorizationRequired",
        VifuEmbeddedGatewayState::Degraded => "degraded",
        VifuEmbeddedGatewayState::Failed => "failed",
    }
}

#[derive(Clone)]
struct Bridge {
    writer: Arc<Mutex<io::Stdout>>,
    pending: Arc<Mutex<HashMap<String, mpsc::Sender<BridgeMessage>>>>,
    sequence: Arc<AtomicU64>,
}

impl VifuStreamingAgentProvider for Bridge {
    fn invoke(
        &self,
        request: VifuProviderRequest,
        invocation: Arc<VifuProviderInvocation>,
    ) -> Result<VifuProviderResponse, VifuRuntimeError> {
        let id = format!("bridge-{}", self.sequence.fetch_add(1, Ordering::Relaxed));
        let (sender, receiver) = mpsc::channel();
        self.pending
            .lock()
            .map_err(|_| runtime_error("the TypeScript provider bridge is unavailable"))?
            .insert(id.clone(), sender);
        let message = json!({
            "type": "invoke",
            "id": id,
            "request": provider_request_json(request),
        });
        if let Err(error) = send(&self.writer, &message) {
            if let Ok(mut pending) = self.pending.lock() {
                pending.remove(&id);
            }
            return Err(runtime_error(error));
        }
        let deadline = Instant::now() + PROVIDER_RESPONSE_TIMEOUT;
        loop {
            if invocation.is_cancelled() {
                return Err(runtime_error(
                    "the TypeScript provider invocation was cancelled",
                ));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            let message = receiver.recv_timeout(remaining).map_err(|_| {
                if let Ok(mut pending) = self.pending.lock() {
                    pending.remove(&id);
                }
                runtime_error("the TypeScript provider did not return a result")
            })?;
            match message {
                BridgeMessage::Trace { event, .. } => apply_trace_event(&invocation, event)?,
                BridgeMessage::Result {
                    ok,
                    output,
                    metadata,
                    state,
                    error,
                    ..
                } => {
                    if !ok {
                        return Err(runtime_error(
                            error.unwrap_or_else(|| "the TypeScript provider failed".to_string()),
                        ));
                    }
                    return Ok(VifuProviderResponse {
                        data: VifuInvocationData::Json {
                            json: serde_json::to_string(&output.unwrap_or(Value::Null))
                                .map_err(|error| runtime_error(error.to_string()))?,
                        },
                        metadata_json: serde_json::to_string(
                            &metadata.unwrap_or_else(|| json!({})),
                        )
                        .map_err(|error| runtime_error(error.to_string()))?,
                        state_json: state
                            .map(|state| serde_json::to_string(&state))
                            .transpose()
                            .map_err(|error| runtime_error(error.to_string()))?,
                    });
                }
            }
        }
    }
}

fn apply_trace_event(
    invocation: &VifuProviderInvocation,
    event: BridgeTraceEvent,
) -> Result<(), VifuRuntimeError> {
    match event {
        BridgeTraceEvent::Activity => invocation.activity(),
        BridgeTraceEvent::OutputDelta { value } => {
            invocation.output_delta(VifuInvocationData::Json {
                json: serde_json::to_string(&value)
                    .map_err(|error| runtime_error(error.to_string()))?,
            })?
        }
        BridgeTraceEvent::StageStarted { stage, metadata } => invocation.stage_started(
            provider_stage(&stage)?,
            serde_json::to_string(&metadata).map_err(|error| runtime_error(error.to_string()))?,
        )?,
        BridgeTraceEvent::StageCompleted {
            stage,
            elapsed_ms,
            metadata,
        } => invocation.stage_completed(
            provider_stage(&stage)?,
            elapsed_ms,
            serde_json::to_string(&metadata).map_err(|error| runtime_error(error.to_string()))?,
        )?,
        BridgeTraceEvent::StageFailed {
            stage,
            elapsed_ms,
            error,
            metadata,
        } => invocation.stage_failed(
            provider_stage(&stage)?,
            elapsed_ms,
            error,
            serde_json::to_string(&metadata).map_err(|error| runtime_error(error.to_string()))?,
        )?,
    }
    Ok(())
}

fn provider_stage(stage: &str) -> Result<VifuProviderStage, VifuRuntimeError> {
    match stage.trim().to_ascii_lowercase().replace('-', "_").as_str() {
        "queue" => Ok(VifuProviderStage::Queue),
        "load" => Ok(VifuProviderStage::Load),
        "tokenize" => Ok(VifuProviderStage::Tokenize),
        "prefill" => Ok(VifuProviderStage::Prefill),
        "first_token" => Ok(VifuProviderStage::FirstToken),
        "decode" => Ok(VifuProviderStage::Decode),
        "validate" => Ok(VifuProviderStage::Validate),
        _ => Err(runtime_error(
            "the TypeScript provider reported an invalid stage",
        )),
    }
}

fn provider_request_json(request: VifuProviderRequest) -> Value {
    json!({
        "appId": request.project_id,
        "endpoint": request.endpoint,
        "sessionId": request.session_id,
        "agent": {
            "id": request.agent_id,
            "name": request.agent_name,
            "provider": request.provider_id,
            "capabilities": request.agent_capabilities,
            "metadata": parse_json_or_null(&request.agent_metadata_json),
        },
        "capability": request.capability,
        "input": invocation_data_json(request.data),
        "metadata": parse_json_or_null(&request.metadata_json),
        "state": parse_json_or_null(&request.state_json),
        "stateRevision": request.state_revision,
    })
}

fn invocation_data_json(data: VifuInvocationData) -> Value {
    match data {
        VifuInvocationData::Json { json } => parse_json_or_null(&json),
        VifuInvocationData::Binary { bytes } => json!({
            "_vifuBinary": true,
            "bytes": bytes,
        }),
    }
}

fn parse_json_or_null(source: &str) -> Value {
    serde_json::from_str(source).unwrap_or(Value::Null)
}

fn deliver_result(
    pending: &Arc<Mutex<HashMap<String, mpsc::Sender<BridgeMessage>>>>,
    value: Value,
) -> Result<(), String> {
    let message = serde_json::from_value::<BridgeMessage>(value)
        .map_err(|error| format!("the TypeScript provider message is invalid: {error}"))?;
    let id = message.id().to_string();
    let sender = pending
        .lock()
        .map_err(|_| "the TypeScript provider bridge is unavailable".to_string())?
        .get(&id)
        .cloned()
        .ok_or_else(|| "the TypeScript provider result is no longer pending".to_string())?;
    let terminal = matches!(message, BridgeMessage::Result { .. });
    sender
        .send(message)
        .map_err(|_| "the TypeScript provider invocation already stopped".to_string())?;
    if terminal {
        pending
            .lock()
            .map_err(|_| "the TypeScript provider bridge is unavailable".to_string())?
            .remove(&id);
    }
    Ok(())
}

fn send(writer: &Arc<Mutex<io::Stdout>>, value: &Value) -> Result<(), String> {
    let encoded = serde_json::to_string(value).map_err(|error| error.to_string())?;
    let mut writer = writer
        .lock()
        .map_err(|_| "the Gateway host output is unavailable".to_string())?;
    writer
        .write_all(encoded.as_bytes())
        .and_then(|_| writer.write_all(b"\n"))
        .and_then(|_| writer.flush())
        .map_err(|error| error.to_string())
}

fn runtime_error(message: impl Into<String>) -> VifuRuntimeError {
    VifuRuntimeError::Runtime {
        reason: message.into(),
    }
}

fn validate_identifier(label: &str, value: &str) -> Result<(), String> {
    if value.len() < 3
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(format!("{label} is invalid"));
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StartMessage {
    #[serde(rename = "type")]
    message_type: String,
    app_id: String,
    data_dir: PathBuf,
    #[serde(default)]
    server_url: Option<String>,
    #[serde(default)]
    enrollment_token: Option<String>,
    #[serde(default)]
    server_certificate_der: Option<Vec<u8>>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    metadata: Value,
    #[serde(default)]
    capture_trace_content: bool,
    agents: Vec<AgentInput>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AgentInput {
    id: String,
    name: String,
    endpoint: String,
    provider_id: String,
    capability: String,
    timeout_ms: u64,
    #[serde(default)]
    metadata: Value,
}

#[derive(Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum BridgeMessage {
    Trace {
        id: String,
        event: BridgeTraceEvent,
    },
    Result {
        id: String,
        ok: bool,
        #[serde(default)]
        output: Option<Value>,
        #[serde(default)]
        metadata: Option<Value>,
        #[serde(default)]
        state: Option<Value>,
        #[serde(default)]
        error: Option<String>,
    },
}

impl BridgeMessage {
    fn id(&self) -> &str {
        match self {
            Self::Trace { id, .. } | Self::Result { id, .. } => id,
        }
    }
}

#[derive(Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum BridgeTraceEvent {
    Activity,
    OutputDelta {
        value: Value,
    },
    StageStarted {
        stage: String,
        metadata: Value,
    },
    StageCompleted {
        stage: String,
        elapsed_ms: u64,
        metadata: Value,
    },
    StageFailed {
        stage: String,
        elapsed_ms: u64,
        error: String,
        metadata: Value,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GatewayCredentials {
    server_url: String,
    machine_private_key: String,
    #[serde(default)]
    device_token: Option<String>,
    #[serde(default)]
    certificate_der: Option<Vec<u8>>,
}

fn load_credentials(path: &Path) -> Result<Option<GatewayCredentials>, String> {
    let source = match fs::read(path) {
        Ok(source) => source,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    serde_json::from_slice(&source)
        .map(Some)
        .map_err(|_| "stored Gateway credentials are invalid".to_string())
}

fn save_credentials(path: &Path, credentials: &GatewayCredentials) -> Result<(), String> {
    let temporary = path.with_extension("tmp");
    let encoded = serde_json::to_vec_pretty(credentials).map_err(|error| error.to_string())?;
    fs::write(&temporary, encoded).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))
            .map_err(|error| error.to_string())?;
    }
    fs::rename(temporary, path).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gateway_metadata_identifies_the_typescript_host() {
        assert_eq!(
            merge_metadata(json!({ "platform": "test" }), &Some("Desk".to_string())),
            json!({
                "name": "Desk",
                "platform": "test",
                "runtime": "typescript",
                "sdkVersion": SDK_VERSION,
            })
        );
    }

    #[test]
    fn identifiers_reject_spaces() {
        assert!(validate_identifier("agent ID", "local-guide").is_ok());
        assert!(validate_identifier("agent ID", "local guide").is_err());
    }
}
