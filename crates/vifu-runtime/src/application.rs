use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
#[cfg(not(target_arch = "wasm32"))]
use std::thread::JoinHandle;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
#[cfg(not(target_arch = "wasm32"))]
use tokio::sync::mpsc;
use tokio::sync::{watch, Notify};
use web_time::Instant;

use crate::{
    EffectRequest, EffectResult, LocalProviderBinding, ProjectSettings, RuntimeManifest,
    RuntimeRelease, RuntimeSnapshot, RuntimeTraceRecord, MAX_ENDPOINT_TIMEOUT_MS,
};

const SNAPSHOT_VERSION: u32 = 1;
const DEFAULT_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_EFFECT_LIMIT: usize = 64;
#[cfg(not(target_arch = "wasm32"))]
const MAX_IN_FLIGHT_INVOCATIONS: usize = 64;
const MAX_RETAINED_INVOCATIONS: usize = 256;
const MAX_RETAINED_INVOCATION_EVENTS: usize = 256;
const MAX_COALESCED_EVENT_BYTES: usize = 64 * 1024;
#[cfg(not(target_arch = "wasm32"))]
const WORKER_QUEUE_CAPACITY: usize = 64;
const MAX_RUNTIME_MONITOR_IO_BYTES: usize = 128 * 1024;

/// A boxed provider future used by [`AgentProvider`].
#[cfg(not(target_arch = "wasm32"))]
pub type ProviderFuture<'a> =
    Pin<Box<dyn Future<Output = Result<ProviderResponse, RuntimeError>> + Send + 'a>>;

#[cfg(target_arch = "wasm32")]
pub type ProviderFuture<'a> =
    Pin<Box<dyn Future<Output = Result<ProviderResponse, RuntimeError>> + 'a>>;

/// JSON or binary data passed through an embedded runtime invocation.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "format", content = "value", rename_all = "camelCase")]
pub enum InvocationData {
    Json(Value),
    Binary(Vec<u8>),
}

impl fmt::Debug for InvocationData {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(_) => formatter.write_str("InvocationData::Json([REDACTED])"),
            Self::Binary(bytes) => formatter
                .debug_tuple("InvocationData::Binary")
                .field(&format_args!("{} bytes", bytes.len()))
                .finish(),
        }
    }
}

impl Default for InvocationData {
    fn default() -> Self {
        Self::Json(Value::Null)
    }
}

/// Input for one application-facing endpoint invocation.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvocationInput {
    pub endpoint: String,
    #[serde(default = "default_session_id")]
    pub session_id: String,
    #[serde(default)]
    pub data: InvocationData,
    #[serde(default)]
    pub metadata: Value,
}

impl InvocationInput {
    pub fn json(endpoint: impl Into<String>, data: Value) -> Self {
        Self {
            endpoint: endpoint.into(),
            session_id: default_session_id(),
            data: InvocationData::Json(data),
            metadata: Value::Object(Default::default()),
        }
    }

    pub fn with_session(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = session_id.into();
        self
    }
}

impl fmt::Debug for InvocationInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InvocationInput")
            .field("endpoint", &self.endpoint)
            .field("session_id", &self.session_id)
            .field("data", &"[REDACTED]")
            .field("metadata", &"[REDACTED]")
            .finish()
    }
}

/// An agent registered inside one application runtime.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDefinition {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub metadata: Value,
}

impl fmt::Debug for AgentDefinition {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AgentDefinition")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("provider", &self.provider)
            .field("capabilities", &self.capabilities)
            .field("metadata", &"[REDACTED]")
            .finish()
    }
}

/// A stable, named application endpoint backed by one registered agent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EndpointDefinition {
    pub name: String,
    pub agent: String,
    pub capability: String,
    /// Maximum time without a provider event before the invocation is cancelled.
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

/// Request delivered to a dynamically registered [`AgentProvider`].
#[derive(Clone)]
pub struct ProviderRequest {
    pub project_id: String,
    pub endpoint: String,
    pub session_id: String,
    pub agent: AgentDefinition,
    pub capability: String,
    pub data: InvocationData,
    pub metadata: Value,
    pub snapshot: RuntimeSnapshot,
}

impl fmt::Debug for ProviderRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderRequest")
            .field("project_id", &self.project_id)
            .field("endpoint", &self.endpoint)
            .field("session_id", &self.session_id)
            .field("agent", &self.agent.id)
            .field("capability", &self.capability)
            .field("data", &"[REDACTED]")
            .field("metadata", &"[REDACTED]")
            .field("snapshot_revision", &self.snapshot.revision)
            .finish()
    }
}

/// Provider result and an optional replacement for the session's durable state.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderResponse {
    #[serde(default)]
    pub data: InvocationData,
    #[serde(default)]
    pub metadata: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<Value>,
}

impl ProviderResponse {
    pub fn json(data: Value) -> Self {
        Self {
            data: InvocationData::Json(data),
            metadata: Value::Object(Default::default()),
            state: None,
        }
    }
}

impl fmt::Debug for ProviderResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderResponse")
            .field("data", &"[REDACTED]")
            .field("metadata", &"[REDACTED]")
            .field("state", &self.state.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
}

/// Cooperative cancellation signal passed to providers.
#[derive(Clone, Default)]
pub struct CancellationToken {
    inner: Arc<CancellationState>,
}

#[derive(Default)]
struct CancellationState {
    cancelled: std::sync::atomic::AtomicBool,
    notify: Notify,
}

impl CancellationToken {
    pub fn cancel(&self) {
        if !self.inner.cancelled.swap(true, Ordering::AcqRel) {
            self.inner.notify.notify_waiters();
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::Acquire)
    }

    pub async fn cancelled(&self) {
        if self.is_cancelled() {
            return;
        }
        self.inner.notify.notified().await;
    }
}

impl fmt::Debug for CancellationToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CancellationToken")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

/// Kind of event emitted while a non-blocking invocation is running.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InvocationEventKind {
    Started,
    OutputDelta,
    Completed,
    Failed,
    Cancelled,
}

/// A provider stage that can be rendered as an observation in a live trace.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProviderStage {
    Queue,
    Load,
    Tokenize,
    Prefill,
    FirstToken,
    Decode,
    Validate,
}

/// A typed provider event emitted while an invocation is running.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ProviderEvent {
    /// Payload-free liveness signal for a provider that is still making
    /// progress inside a long-running stage.
    Activity,
    OutputDelta {
        data: InvocationData,
    },
    StageStarted {
        stage: ProviderStage,
        #[serde(default, skip_serializing_if = "is_null")]
        metadata: Value,
    },
    StageCompleted {
        stage: ProviderStage,
        elapsed_ms: u64,
        #[serde(default, skip_serializing_if = "is_null")]
        metadata: Value,
    },
    StageFailed {
        stage: ProviderStage,
        elapsed_ms: u64,
        error: String,
        #[serde(default, skip_serializing_if = "is_null")]
        metadata: Value,
    },
}

/// Terminal outcome reported to an embedded runtime monitor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeMonitorStatus {
    Completed,
    Cancelled,
    Error,
}

/// State of a provider stage reported to an embedded runtime monitor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeMonitorStageStatus {
    Started,
    Completed,
    Failed,
}

/// Payload-safe lifecycle metadata for one embedded runtime invocation.
///
/// Prompt content and streamed output are intentionally excluded. Hosts may
/// forward these events to a remote monitor without exposing model input or
/// output data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum RuntimeMonitorEvent {
    InvocationStarted {
        trace_id: String,
        invocation_id: String,
        project_id: String,
        endpoint: String,
        agent_id: String,
        provider_id: String,
        capability: String,
        started_at_ms: u64,
    },
    ProviderStage {
        trace_id: String,
        invocation_id: String,
        stage: ProviderStage,
        status: RuntimeMonitorStageStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        elapsed_ms: Option<u64>,
        request_elapsed_ms: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input_tokens: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output_tokens: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resident: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    InvocationFinished {
        trace_id: String,
        invocation_id: String,
        status: RuntimeMonitorStatus,
        duration_ms: u64,
        ended_at_ms: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
}

/// Thread-safe callback installed by an embedding host that wants live,
/// payload-safe runtime lifecycle metadata.
pub type RuntimeMonitorObserver = Arc<dyn Fn(RuntimeMonitorEvent) + Send + Sync>;

/// Bounded process-local I/O summary for an opt-in diagnostic observer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeMonitorIoSummary {
    pub value: Value,
    pub truncated: bool,
}

/// Invocation I/O exposed only to an explicitly installed diagnostic observer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum RuntimeMonitorIoEvent {
    InvocationInput {
        trace_id: String,
        invocation_id: String,
        summary: RuntimeMonitorIoSummary,
    },
    InvocationOutput {
        trace_id: String,
        invocation_id: String,
        summary: RuntimeMonitorIoSummary,
    },
}

/// Thread-safe callback for bounded invocation I/O diagnostics.
pub type RuntimeMonitorIoObserver = Arc<dyn Fn(RuntimeMonitorIoEvent) + Send + Sync>;

/// One ordered event produced by an invocation.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvocationEvent {
    pub sequence: u64,
    pub kind: InvocationEventKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<InvocationData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl fmt::Debug for InvocationEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InvocationEvent")
            .field("sequence", &self.sequence)
            .field("kind", &self.kind)
            .field("data", &self.data.as_ref().map(|_| "[REDACTED]"))
            .field("error", &self.error.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
}

/// Sink supplied to providers that can produce incremental output.
///
/// Providers that only return a final response can keep implementing
/// [`AgentProvider::invoke`]. Streaming providers call [`Self::output_delta`]
/// while their invocation is running.
#[derive(Clone)]
pub struct ProviderEventSink {
    emit: Arc<dyn ProviderEventCallback>,
}

#[cfg(not(target_arch = "wasm32"))]
trait ProviderEventCallback: Fn(ProviderEvent) + Send + Sync {}

#[cfg(not(target_arch = "wasm32"))]
impl<T> ProviderEventCallback for T where T: Fn(ProviderEvent) + Send + Sync {}

#[cfg(target_arch = "wasm32")]
trait ProviderEventCallback: Fn(ProviderEvent) {}

#[cfg(target_arch = "wasm32")]
impl<T> ProviderEventCallback for T where T: Fn(ProviderEvent) {}

impl ProviderEventSink {
    fn new(emit: impl ProviderEventCallback + 'static) -> Self {
        Self {
            emit: Arc::new(emit),
        }
    }

    /// Creates a sink that forwards every typed provider event to `emit`.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn from_fn(emit: impl Fn(ProviderEvent) + Send + Sync + 'static) -> Self {
        Self::new(emit)
    }

    /// Creates a sink that forwards every typed provider event to `emit`.
    #[cfg(target_arch = "wasm32")]
    pub fn from_fn(emit: impl Fn(ProviderEvent) + 'static) -> Self {
        Self::new(emit)
    }

    pub fn discard() -> Self {
        Self::new(|_event| {})
    }

    pub fn output_delta(&self, data: InvocationData) {
        (self.emit)(ProviderEvent::OutputDelta { data });
    }

    pub fn activity(&self) {
        (self.emit)(ProviderEvent::Activity);
    }

    pub fn stage_started(&self, stage: ProviderStage, metadata: Value) {
        (self.emit)(ProviderEvent::StageStarted { stage, metadata });
    }

    pub fn stage_completed(&self, stage: ProviderStage, elapsed_ms: u64, metadata: Value) {
        (self.emit)(ProviderEvent::StageCompleted {
            stage,
            elapsed_ms,
            metadata,
        });
    }

    pub fn stage_failed(
        &self,
        stage: ProviderStage,
        elapsed_ms: u64,
        error: impl Into<String>,
        metadata: Value,
    ) {
        (self.emit)(ProviderEvent::StageFailed {
            stage,
            elapsed_ms,
            error: error.into(),
            metadata,
        });
    }
}

impl fmt::Debug for ProviderEventSink {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ProviderEventSink")
    }
}

/// Runtime-selected provider implementation.
///
/// Providers are registered dynamically by name. A provider may hold credentials
/// internally, but credentials must never be placed in agent definitions,
/// invocation metadata, snapshots, or returned trace attributes.
#[cfg(not(target_arch = "wasm32"))]
pub trait AgentProviderBounds: Send + Sync {}

#[cfg(not(target_arch = "wasm32"))]
impl<T: Send + Sync> AgentProviderBounds for T {}

#[cfg(target_arch = "wasm32")]
pub trait AgentProviderBounds {}

#[cfg(target_arch = "wasm32")]
impl<T> AgentProviderBounds for T {}

pub trait AgentProvider: AgentProviderBounds + 'static {
    fn supports(&self, capability: &str) -> bool;

    fn invoke<'a>(
        &'a self,
        request: ProviderRequest,
        cancellation: CancellationToken,
    ) -> ProviderFuture<'a>;

    fn invoke_with_events<'a>(
        &'a self,
        request: ProviderRequest,
        cancellation: CancellationToken,
        _events: ProviderEventSink,
    ) -> ProviderFuture<'a> {
        self.invoke(request, cancellation)
    }
}

/// Persistence adapter supplied by an embedding host.
///
/// The default [`MemoryRuntimeStore`] keeps session state in memory. Server
/// deployments can implement this trait with their database adapter.
pub trait RuntimeStore: Send + Sync + 'static {
    fn load(
        &self,
        project_id: &str,
        session_id: &str,
    ) -> Result<Option<RuntimeSnapshot>, RuntimeError>;

    fn save(
        &self,
        project_id: &str,
        session_id: &str,
        snapshot: &RuntimeSnapshot,
    ) -> Result<(), RuntimeError>;

    fn save_release(&self, _release: &RuntimeRelease) -> Result<(), RuntimeError> {
        Err(RuntimeError::store(
            "this runtime store does not support releases".to_string(),
        ))
    }

    fn load_release(
        &self,
        _project_id: &str,
        _version: u64,
    ) -> Result<Option<RuntimeRelease>, RuntimeError> {
        Ok(None)
    }

    fn list_releases(&self, _project_id: &str) -> Result<Vec<RuntimeRelease>, RuntimeError> {
        Ok(Vec::new())
    }

    fn active_release(&self, _project_id: &str) -> Result<Option<u64>, RuntimeError> {
        Ok(None)
    }

    fn set_active_release(&self, _project_id: &str, _version: u64) -> Result<(), RuntimeError> {
        Err(RuntimeError::store(
            "this runtime store does not support releases".to_string(),
        ))
    }

    fn save_local_provider_binding(
        &self,
        _project_id: &str,
        _binding: &LocalProviderBinding,
    ) -> Result<(), RuntimeError> {
        Err(RuntimeError::store(
            "this runtime store does not support provider bindings".to_string(),
        ))
    }

    fn local_provider_bindings(
        &self,
        _project_id: &str,
    ) -> Result<Vec<LocalProviderBinding>, RuntimeError> {
        Ok(Vec::new())
    }

    fn enqueue_trace(&self, _trace: &RuntimeTraceRecord) -> Result<(), RuntimeError> {
        Ok(())
    }

    fn pending_traces(&self, _limit: usize) -> Result<Vec<RuntimeTraceRecord>, RuntimeError> {
        Ok(Vec::new())
    }

    fn acknowledge_traces(&self, _trace_ids: &[String]) -> Result<(), RuntimeError> {
        Ok(())
    }
}

/// In-memory persistence used by the standalone embedded runtime.
#[derive(Default)]
pub struct MemoryRuntimeStore {
    snapshots: RwLock<HashMap<(String, String), RuntimeSnapshot>>,
    releases: RwLock<HashMap<(String, u64), RuntimeRelease>>,
    active_releases: RwLock<HashMap<String, u64>>,
    provider_bindings: RwLock<HashMap<(String, String), LocalProviderBinding>>,
    trace_outbox: RwLock<VecDeque<RuntimeTraceRecord>>,
}

impl RuntimeStore for MemoryRuntimeStore {
    fn load(
        &self,
        project_id: &str,
        session_id: &str,
    ) -> Result<Option<RuntimeSnapshot>, RuntimeError> {
        let snapshots = self.snapshots.read().map_err(|_| RuntimeError::Internal)?;
        Ok(snapshots
            .get(&(project_id.to_string(), session_id.to_string()))
            .cloned())
    }

    fn save(
        &self,
        project_id: &str,
        session_id: &str,
        snapshot: &RuntimeSnapshot,
    ) -> Result<(), RuntimeError> {
        let mut snapshots = self.snapshots.write().map_err(|_| RuntimeError::Internal)?;
        snapshots.insert(
            (project_id.to_string(), session_id.to_string()),
            snapshot.clone(),
        );
        Ok(())
    }

    fn save_release(&self, release: &RuntimeRelease) -> Result<(), RuntimeError> {
        release.validate()?;
        let key = (release.manifest.project_id.clone(), release.version);
        let mut releases = self.releases.write().map_err(|_| RuntimeError::Internal)?;
        if let Some(existing) = releases.get(&key) {
            if existing != release {
                return Err(RuntimeError::store(
                    "runtime release versions are immutable".to_string(),
                ));
            }
            return Ok(());
        }
        releases.insert(key, release.clone());
        Ok(())
    }

    fn load_release(
        &self,
        project_id: &str,
        version: u64,
    ) -> Result<Option<RuntimeRelease>, RuntimeError> {
        Ok(self
            .releases
            .read()
            .map_err(|_| RuntimeError::Internal)?
            .get(&(project_id.to_string(), version))
            .cloned())
    }

    fn list_releases(&self, project_id: &str) -> Result<Vec<RuntimeRelease>, RuntimeError> {
        let mut releases = self
            .releases
            .read()
            .map_err(|_| RuntimeError::Internal)?
            .iter()
            .filter(|((stored_project_id, _), _)| stored_project_id == project_id)
            .map(|(_, release)| release.clone())
            .collect::<Vec<_>>();
        releases.sort_by_key(|release| std::cmp::Reverse(release.version));
        Ok(releases)
    }

    fn active_release(&self, project_id: &str) -> Result<Option<u64>, RuntimeError> {
        Ok(self
            .active_releases
            .read()
            .map_err(|_| RuntimeError::Internal)?
            .get(project_id)
            .copied())
    }

    fn set_active_release(&self, project_id: &str, version: u64) -> Result<(), RuntimeError> {
        if !self
            .releases
            .read()
            .map_err(|_| RuntimeError::Internal)?
            .contains_key(&(project_id.to_string(), version))
        {
            return Err(RuntimeError::store("runtime release was not found"));
        }
        self.active_releases
            .write()
            .map_err(|_| RuntimeError::Internal)?
            .insert(project_id.to_string(), version);
        Ok(())
    }

    fn save_local_provider_binding(
        &self,
        project_id: &str,
        binding: &LocalProviderBinding,
    ) -> Result<(), RuntimeError> {
        self.provider_bindings
            .write()
            .map_err(|_| RuntimeError::Internal)?
            .insert(
                (project_id.to_string(), binding.provider_id.clone()),
                binding.clone(),
            );
        Ok(())
    }

    fn local_provider_bindings(
        &self,
        project_id: &str,
    ) -> Result<Vec<LocalProviderBinding>, RuntimeError> {
        let mut bindings = self
            .provider_bindings
            .read()
            .map_err(|_| RuntimeError::Internal)?
            .iter()
            .filter(|((stored_project_id, _), _)| stored_project_id == project_id)
            .map(|(_, binding)| binding.clone())
            .collect::<Vec<_>>();
        bindings.sort_by(|left, right| left.provider_id.cmp(&right.provider_id));
        Ok(bindings)
    }

    fn enqueue_trace(&self, trace: &RuntimeTraceRecord) -> Result<(), RuntimeError> {
        const MAX_MEMORY_TRACES: usize = 1_000;
        let mut traces = self
            .trace_outbox
            .write()
            .map_err(|_| RuntimeError::Internal)?;
        if traces.iter().any(|stored| stored.id == trace.id) {
            return Ok(());
        }
        traces.push_back(trace.clone());
        while traces.len() > MAX_MEMORY_TRACES {
            traces.pop_front();
        }
        Ok(())
    }

    fn pending_traces(&self, limit: usize) -> Result<Vec<RuntimeTraceRecord>, RuntimeError> {
        Ok(self
            .trace_outbox
            .read()
            .map_err(|_| RuntimeError::Internal)?
            .iter()
            .take(limit)
            .cloned()
            .collect())
    }

    fn acknowledge_traces(&self, trace_ids: &[String]) -> Result<(), RuntimeError> {
        self.trace_outbox
            .write()
            .map_err(|_| RuntimeError::Internal)?
            .retain(|trace| !trace_ids.contains(&trace.id));
        Ok(())
    }
}

/// One safe trace event emitted by the application runtime.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvocationTraceEvent {
    pub name: String,
    pub status: String,
    pub duration_ms: u64,
    #[serde(default)]
    pub attributes: Value,
}

/// Result of one endpoint invocation.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvocationOutput {
    pub invocation_id: String,
    pub project_id: String,
    pub endpoint: String,
    pub session_id: String,
    pub agent: String,
    pub provider: String,
    pub capability: String,
    pub data: InvocationData,
    #[serde(default)]
    pub metadata: Value,
    pub snapshot: RuntimeSnapshot,
    pub trace: Vec<InvocationTraceEvent>,
}

impl fmt::Debug for InvocationOutput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InvocationOutput")
            .field("invocation_id", &self.invocation_id)
            .field("project_id", &self.project_id)
            .field("endpoint", &self.endpoint)
            .field("session_id", &self.session_id)
            .field("agent", &self.agent)
            .field("provider", &self.provider)
            .field("capability", &self.capability)
            .field("data", &"[REDACTED]")
            .field("metadata", &"[REDACTED]")
            .field("snapshot_revision", &self.snapshot.revision)
            .field("trace_count", &self.trace.len())
            .finish()
    }
}

/// Opaque handle returned by the game-loop invocation API.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct InvocationHandle(pub String);

/// Current state of an invocation started with [`VifuRuntime::start_invoke`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InvocationStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

/// Non-blocking game-loop poll result.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvocationPoll {
    pub handle: InvocationHandle,
    pub status: InvocationStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<InvocationOutput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl fmt::Debug for InvocationPoll {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InvocationPoll")
            .field("handle", &self.handle)
            .field("status", &self.status)
            .field("output", &self.output.as_ref().map(|_| "[REDACTED]"))
            .field("error", &self.error.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
}

/// Result of running host effects through the runtime.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectExecution {
    pub results: Vec<EffectResult>,
    pub unhandled: Vec<EffectRequest>,
}

/// Errors returned by the embedded application runtime.
pub enum RuntimeError {
    InvalidDefinition(String),
    EndpointNotFound(String),
    AgentNotFound(String),
    ProviderNotFound(String),
    CapabilityUnavailable {
        provider: String,
        capability: String,
    },
    Timeout(u64),
    Cancelled,
    Unavailable(String),
    Backpressure(String),
    Provider {
        provider: String,
        message: String,
    },
    Store(String),
    Snapshot(String),
    EffectLimitExceeded(usize),
    InvocationNotFound(String),
    Internal,
}

impl RuntimeError {
    pub fn provider(provider: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Provider {
            provider: provider.into(),
            message: message.into(),
        }
    }

    pub fn store(message: impl Into<String>) -> Self {
        Self::Store(message.into())
    }

    pub fn public_message(&self) -> String {
        match self {
            Self::Provider { provider, .. } => {
                format!("provider {provider} request failed")
            }
            Self::Store(_) => "runtime state could not be persisted".to_string(),
            Self::Snapshot(_) => "runtime snapshot is invalid".to_string(),
            Self::Unavailable(_) => "provider is not available".to_string(),
            Self::Backpressure(_) => "runtime is busy".to_string(),
            _ => self.to_string(),
        }
    }
}

impl fmt::Debug for RuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDefinition(_) => formatter.write_str("InvalidDefinition([REDACTED])"),
            Self::EndpointNotFound(endpoint) => formatter
                .debug_tuple("EndpointNotFound")
                .field(endpoint)
                .finish(),
            Self::AgentNotFound(agent) => {
                formatter.debug_tuple("AgentNotFound").field(agent).finish()
            }
            Self::ProviderNotFound(provider) => formatter
                .debug_tuple("ProviderNotFound")
                .field(provider)
                .finish(),
            Self::CapabilityUnavailable {
                provider,
                capability,
            } => formatter
                .debug_struct("CapabilityUnavailable")
                .field("provider", provider)
                .field("capability", capability)
                .finish(),
            Self::Timeout(timeout) => formatter.debug_tuple("Timeout").field(timeout).finish(),
            Self::Cancelled => formatter.write_str("Cancelled"),
            Self::Unavailable(_) => formatter.write_str("Unavailable([REDACTED])"),
            Self::Backpressure(_) => formatter.write_str("Backpressure([REDACTED])"),
            Self::Provider { provider, .. } => formatter
                .debug_struct("Provider")
                .field("provider", provider)
                .field("message", &"[REDACTED]")
                .finish(),
            Self::Store(_) => formatter.write_str("Store([REDACTED])"),
            Self::Snapshot(_) => formatter.write_str("Snapshot([REDACTED])"),
            Self::EffectLimitExceeded(limit) => formatter
                .debug_tuple("EffectLimitExceeded")
                .field(limit)
                .finish(),
            Self::InvocationNotFound(handle) => formatter
                .debug_tuple("InvocationNotFound")
                .field(handle)
                .finish(),
            Self::Internal => formatter.write_str("Internal"),
        }
    }
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDefinition(message) => {
                write!(formatter, "invalid runtime definition: {message}")
            }
            Self::EndpointNotFound(endpoint) => {
                write!(formatter, "endpoint {endpoint} is not registered")
            }
            Self::AgentNotFound(agent) => write!(formatter, "agent {agent} is not registered"),
            Self::ProviderNotFound(provider) => {
                write!(formatter, "provider {provider} is not registered")
            }
            Self::CapabilityUnavailable {
                provider,
                capability,
            } => write!(
                formatter,
                "provider {provider} does not support capability {capability}"
            ),
            Self::Timeout(timeout_ms) => {
                write!(formatter, "agent invocation was idle for {timeout_ms} ms")
            }
            Self::Cancelled => formatter.write_str("agent invocation was cancelled"),
            Self::Unavailable(message) => {
                write!(formatter, "provider is not available: {message}")
            }
            Self::Backpressure(message) => write!(formatter, "runtime is busy: {message}"),
            Self::Provider { provider, message } => {
                write!(formatter, "provider {provider} failed: {message}")
            }
            Self::Store(message) => write!(formatter, "runtime store failed: {message}"),
            Self::Snapshot(message) => write!(formatter, "runtime snapshot failed: {message}"),
            Self::EffectLimitExceeded(limit) => {
                write!(formatter, "runtime effect limit {limit} was exceeded")
            }
            Self::InvocationNotFound(handle) => {
                write!(formatter, "invocation {handle} was not found")
            }
            Self::Internal => formatter.write_str("runtime internal error"),
        }
    }
}

impl std::error::Error for RuntimeError {}

#[derive(Default)]
struct RuntimeRegistry {
    providers: HashMap<String, Arc<dyn AgentProvider>>,
    agents: HashMap<String, AgentDefinition>,
    endpoints: HashMap<String, EndpointDefinition>,
}

struct RuntimeCore {
    project_id: String,
    registry: RwLock<RuntimeRegistry>,
    manifest: RwLock<Option<RuntimeManifest>>,
    store: Arc<dyn RuntimeStore>,
    sessions: RwLock<HashMap<String, RuntimeSnapshot>>,
    session_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    invocations: Mutex<InvocationRegistry>,
    next_invocation: AtomicU64,
    monitor_observer: RwLock<Option<RuntimeMonitorObserver>>,
    monitor_io_observer: RwLock<Option<RuntimeMonitorIoObserver>>,
}

struct InvocationEntry {
    poll: InvocationPoll,
    cancellation: CancellationToken,
    events: VecDeque<InvocationEvent>,
    next_event_sequence: u64,
}

#[derive(Default)]
struct InvocationRegistry {
    entries: HashMap<String, InvocationEntry>,
    terminal_order: VecDeque<String>,
    active_count: usize,
}

impl InvocationRegistry {
    #[cfg(not(target_arch = "wasm32"))]
    fn insert(
        &mut self,
        handle: InvocationHandle,
        cancellation: CancellationToken,
    ) -> Result<(), RuntimeError> {
        if self.active_count >= MAX_IN_FLIGHT_INVOCATIONS {
            return Err(RuntimeError::Backpressure(
                "too many invocations are already running".to_string(),
            ));
        }
        self.entries.insert(
            handle.0.clone(),
            InvocationEntry {
                poll: InvocationPoll {
                    handle,
                    status: InvocationStatus::Pending,
                    output: None,
                    error: None,
                },
                cancellation,
                events: VecDeque::new(),
                next_event_sequence: 1,
            },
        );
        self.active_count += 1;
        Ok(())
    }

    fn update(
        &mut self,
        handle: &InvocationHandle,
        status: InvocationStatus,
        output: Option<InvocationOutput>,
        error: Option<String>,
    ) {
        let Some(entry) = self.entries.get_mut(&handle.0) else {
            return;
        };
        if is_terminal_status(entry.poll.status) {
            return;
        }
        entry.poll.status = status;
        entry.poll.output = output;
        entry.poll.error = error;
        let event = match status {
            InvocationStatus::Pending => None,
            InvocationStatus::Running => Some((InvocationEventKind::Started, None, None)),
            InvocationStatus::Completed => Some((
                InvocationEventKind::Completed,
                entry.poll.output.as_ref().map(|value| value.data.clone()),
                None,
            )),
            InvocationStatus::Failed => {
                Some((InvocationEventKind::Failed, None, entry.poll.error.clone()))
            }
            InvocationStatus::Cancelled => Some((InvocationEventKind::Cancelled, None, None)),
        };
        if let Some((kind, data, error)) = event {
            entry.push_event(kind, data, error);
        }
        if is_terminal_status(status) {
            self.active_count = self.active_count.saturating_sub(1);
            self.terminal_order.push_back(handle.0.clone());
            self.evict_old_terminal_entries();
        }
    }

    fn remove(&mut self, handle: &InvocationHandle) {
        if let Some(entry) = self.entries.remove(&handle.0) {
            if !is_terminal_status(entry.poll.status) {
                self.active_count = self.active_count.saturating_sub(1);
            }
        }
        self.terminal_order.retain(|stored| stored != &handle.0);
    }

    fn take(&mut self, handle: &InvocationHandle) -> Result<InvocationPoll, RuntimeError> {
        let poll = self
            .entries
            .get(&handle.0)
            .map(|entry| entry.poll.clone())
            .ok_or_else(|| RuntimeError::InvocationNotFound(handle.0.clone()))?;
        if is_terminal_status(poll.status) {
            self.remove(handle);
        }
        Ok(poll)
    }

    fn push_provider_event(&mut self, handle: &InvocationHandle, event: ProviderEvent) {
        let Some(entry) = self.entries.get_mut(&handle.0) else {
            return;
        };
        if entry.poll.status != InvocationStatus::Running {
            return;
        }
        match event {
            ProviderEvent::Activity => {}
            ProviderEvent::OutputDelta { data } => {
                entry.push_event(InvocationEventKind::OutputDelta, Some(data), None);
            }
            ProviderEvent::StageStarted { .. }
            | ProviderEvent::StageCompleted { .. }
            | ProviderEvent::StageFailed { .. } => {}
        }
    }

    fn drain_events(
        &mut self,
        handle: &InvocationHandle,
    ) -> Result<Vec<InvocationEvent>, RuntimeError> {
        let entry = self
            .entries
            .get_mut(&handle.0)
            .ok_or_else(|| RuntimeError::InvocationNotFound(handle.0.clone()))?;
        Ok(entry.events.drain(..).collect())
    }

    fn evict_old_terminal_entries(&mut self) {
        while self.terminal_order.len() > MAX_RETAINED_INVOCATIONS {
            if let Some(handle) = self.terminal_order.pop_front() {
                self.entries.remove(&handle);
            }
        }
    }
}

impl InvocationEntry {
    fn push_event(
        &mut self,
        kind: InvocationEventKind,
        data: Option<InvocationData>,
        error: Option<String>,
    ) {
        if kind == InvocationEventKind::OutputDelta {
            if let (
                Some(InvocationEvent {
                    kind: InvocationEventKind::OutputDelta,
                    data: Some(previous),
                    ..
                }),
                Some(next),
            ) = (self.events.back_mut(), data.as_ref())
            {
                if merge_invocation_data(previous, next) {
                    return;
                }
            }
        }
        self.events.push_back(InvocationEvent {
            sequence: self.next_event_sequence,
            kind,
            data,
            error,
        });
        self.next_event_sequence = self.next_event_sequence.saturating_add(1);
        while self.events.len() > MAX_RETAINED_INVOCATION_EVENTS {
            self.events.pop_front();
        }
    }
}

impl RuntimeCore {
    async fn invoke(
        self: &Arc<Self>,
        invocation_id: String,
        input: InvocationInput,
        cancellation: CancellationToken,
        forwarded_events: ProviderEventSink,
    ) -> Result<InvocationOutput, RuntimeError> {
        let endpoint = input.endpoint.clone();
        let created_at_ms = crate::unix_time_ms();
        let trace_id = format!("trace-{created_at_ms}-{invocation_id}");
        let started = Instant::now();
        let result = self
            .invoke_provider(
                trace_id.clone(),
                created_at_ms,
                invocation_id.clone(),
                input,
                cancellation,
                forwarded_events,
            )
            .await;
        let elapsed_ms = duration_ms(started.elapsed());
        let trace = match &result {
            Ok(output) => RuntimeTraceRecord {
                id: trace_id.clone(),
                project_id: self.project_id.clone(),
                invocation_id: invocation_id.clone(),
                endpoint: endpoint.clone(),
                agent: Some(output.agent.clone()),
                provider: Some(output.provider.clone()),
                capability: Some(output.capability.clone()),
                status: "completed".to_string(),
                duration_ms: elapsed_ms,
                created_at_ms,
            },
            Err(error) => RuntimeTraceRecord {
                id: trace_id.clone(),
                project_id: self.project_id.clone(),
                invocation_id: invocation_id.clone(),
                endpoint,
                agent: None,
                provider: None,
                capability: None,
                status: match error {
                    RuntimeError::Cancelled => "cancelled",
                    _ => "error",
                }
                .to_string(),
                duration_ms: elapsed_ms,
                created_at_ms,
            },
        };
        let _ = self.store.enqueue_trace(&trace);
        if let Ok(output) = &result {
            self.emit_monitor_io_event(RuntimeMonitorIoEvent::InvocationOutput {
                trace_id: trace_id.clone(),
                invocation_id: invocation_id.clone(),
                summary: runtime_monitor_io_summary(&output.data),
            });
        }
        self.emit_monitor_event(RuntimeMonitorEvent::InvocationFinished {
            trace_id,
            invocation_id,
            status: match &result {
                Ok(_) => RuntimeMonitorStatus::Completed,
                Err(RuntimeError::Cancelled) => RuntimeMonitorStatus::Cancelled,
                Err(_) => RuntimeMonitorStatus::Error,
            },
            duration_ms: elapsed_ms,
            // Derive the terminal timestamp from the invocation start and a
            // monotonic duration. A wall-clock adjustment during the request
            // must not produce an end time before the start time.
            ended_at_ms: created_at_ms.saturating_add(elapsed_ms),
            error: result.as_ref().err().map(RuntimeError::public_message),
        });
        result
    }

    async fn invoke_provider(
        self: &Arc<Self>,
        trace_id: String,
        started_at_ms: u64,
        invocation_id: String,
        input: InvocationInput,
        cancellation: CancellationToken,
        forwarded_events: ProviderEventSink,
    ) -> Result<InvocationOutput, RuntimeError> {
        validate_identifier("endpoint", &input.endpoint)?;
        validate_identifier("session", &input.session_id)?;
        let session_lock = {
            let mut locks = self
                .session_locks
                .lock()
                .map_err(|_| RuntimeError::Internal)?;
            Arc::clone(
                locks
                    .entry(input.session_id.clone())
                    .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
            )
        };
        let _session_guard = session_lock.lock().await;
        let (endpoint, agent, provider) = {
            let registry = self.registry.read().map_err(|_| RuntimeError::Internal)?;
            let endpoint = registry
                .endpoints
                .get(&input.endpoint)
                .cloned()
                .ok_or_else(|| RuntimeError::EndpointNotFound(input.endpoint.clone()))?;
            let agent = registry
                .agents
                .get(&endpoint.agent)
                .cloned()
                .ok_or_else(|| RuntimeError::AgentNotFound(endpoint.agent.clone()))?;
            let provider = registry
                .providers
                .get(&agent.provider)
                .cloned()
                .ok_or_else(|| RuntimeError::ProviderNotFound(agent.provider.clone()))?;
            (endpoint, agent, provider)
        };
        if !agent
            .capabilities
            .iter()
            .any(|capability| capability == &endpoint.capability)
            || !provider.supports(&endpoint.capability)
        {
            return Err(RuntimeError::CapabilityUnavailable {
                provider: agent.provider.clone(),
                capability: endpoint.capability,
            });
        }
        if cancellation.is_cancelled() {
            return Err(RuntimeError::Cancelled);
        }

        self.emit_monitor_event(RuntimeMonitorEvent::InvocationStarted {
            trace_id: trace_id.clone(),
            invocation_id: invocation_id.clone(),
            project_id: self.project_id.clone(),
            endpoint: endpoint.name.clone(),
            agent_id: agent.id.clone(),
            provider_id: agent.provider.clone(),
            capability: endpoint.capability.clone(),
            started_at_ms,
        });
        self.emit_monitor_io_event(RuntimeMonitorIoEvent::InvocationInput {
            trace_id: trace_id.clone(),
            invocation_id: invocation_id.clone(),
            summary: runtime_monitor_io_summary(&input.data),
        });

        let snapshot = self.load_snapshot(&input.session_id)?;
        let request = ProviderRequest {
            project_id: self.project_id.clone(),
            endpoint: endpoint.name.clone(),
            session_id: input.session_id.clone(),
            agent: agent.clone(),
            capability: endpoint.capability.clone(),
            data: input.data,
            metadata: input.metadata,
            snapshot: snapshot.clone(),
        };
        let started = Instant::now();
        let (activity_sender, mut activity_receiver) = watch::channel(0_u64);
        let provider_trace = Arc::new(Mutex::new(Vec::new()));
        let events = self.provider_event_sink(
            &InvocationHandle(invocation_id.clone()),
            trace_id,
            started,
            activity_sender,
            forwarded_events,
            Arc::clone(&provider_trace),
        );
        let provider_call = provider.invoke_with_events(request, cancellation.clone(), events);
        tokio::pin!(provider_call);
        let idle_timeout = Duration::from_millis(endpoint.timeout_ms);
        let mut activity_open = true;
        let response = loop {
            let idle_deadline = runtime_sleep(idle_timeout);
            tokio::pin!(idle_deadline);
            tokio::select! {
                biased;
                _ = cancellation.cancelled() => return Err(RuntimeError::Cancelled),
                response = &mut provider_call => break response?,
                changed = activity_receiver.changed(), if activity_open => {
                    if changed.is_err() {
                        activity_open = false;
                    }
                }
                _ = &mut idle_deadline => {
                    cancellation.cancel();
                    return Err(RuntimeError::Timeout(endpoint.timeout_ms));
                }
            }
        };
        if cancellation.is_cancelled() {
            return Err(RuntimeError::Cancelled);
        }

        let next_snapshot = RuntimeSnapshot {
            revision: snapshot.revision.saturating_add(1),
            state: response.state.unwrap_or(snapshot.state),
        };
        self.store
            .save(&self.project_id, &input.session_id, &next_snapshot)?;
        self.sessions
            .write()
            .map_err(|_| RuntimeError::Internal)?
            .insert(input.session_id.clone(), next_snapshot.clone());
        let mut trace = provider_trace
            .lock()
            .map_err(|_| RuntimeError::Internal)?
            .clone();
        trace.push(InvocationTraceEvent {
            name: "provider.invoke".to_string(),
            status: "completed".to_string(),
            duration_ms: duration_ms(started.elapsed()),
            attributes: json!({
                "endpoint": input.endpoint,
            }),
        });
        Ok(InvocationOutput {
            invocation_id,
            project_id: self.project_id.clone(),
            endpoint: endpoint.name,
            session_id: input.session_id,
            agent: agent.id,
            provider: agent.provider,
            capability: endpoint.capability,
            data: response.data,
            metadata: response.metadata,
            snapshot: next_snapshot,
            trace,
        })
    }

    fn load_snapshot(&self, session_id: &str) -> Result<RuntimeSnapshot, RuntimeError> {
        if let Some(snapshot) = self
            .sessions
            .read()
            .map_err(|_| RuntimeError::Internal)?
            .get(session_id)
            .cloned()
        {
            return Ok(snapshot);
        }
        let snapshot = self
            .store
            .load(&self.project_id, session_id)?
            .unwrap_or_default();
        self.sessions
            .write()
            .map_err(|_| RuntimeError::Internal)?
            .insert(session_id.to_string(), snapshot.clone());
        Ok(snapshot)
    }

    fn next_invocation_id(&self) -> String {
        let id = self.next_invocation.fetch_add(1, Ordering::Relaxed);
        format!("invocation-{id}")
    }

    fn update_poll(
        &self,
        handle: &InvocationHandle,
        status: InvocationStatus,
        output: Option<InvocationOutput>,
        error: Option<String>,
    ) {
        if let Ok(mut invocations) = self.invocations.lock() {
            invocations.update(handle, status, output, error);
        }
    }

    fn provider_event_sink(
        self: &Arc<Self>,
        handle: &InvocationHandle,
        trace_id: String,
        request_started: Instant,
        activity: watch::Sender<u64>,
        forwarded_events: ProviderEventSink,
        provider_trace: Arc<Mutex<Vec<InvocationTraceEvent>>>,
    ) -> ProviderEventSink {
        let core = Arc::clone(self);
        let handle = handle.clone();
        ProviderEventSink::new(move |event| {
            activity.send_modify(|sequence| *sequence = sequence.saturating_add(1));
            (forwarded_events.emit)(event.clone());
            if let Ok(mut invocations) = core.invocations.lock() {
                invocations.push_provider_event(&handle, event.clone());
            }
            let trace_event = match &event {
                ProviderEvent::StageCompleted {
                    stage,
                    elapsed_ms,
                    metadata,
                } => Some(InvocationTraceEvent {
                    name: provider_stage_name(*stage).to_string(),
                    status: "completed".to_string(),
                    duration_ms: *elapsed_ms,
                    attributes: metadata.clone(),
                }),
                ProviderEvent::StageFailed {
                    stage,
                    elapsed_ms,
                    metadata,
                    ..
                } => Some(InvocationTraceEvent {
                    name: provider_stage_name(*stage).to_string(),
                    status: "failed".to_string(),
                    duration_ms: *elapsed_ms,
                    attributes: metadata.clone(),
                }),
                ProviderEvent::Activity
                | ProviderEvent::OutputDelta { .. }
                | ProviderEvent::StageStarted { .. } => None,
            };
            if let Some(trace_event) = trace_event {
                if let Ok(mut trace) = provider_trace.lock() {
                    trace.push(trace_event);
                }
            }
            let (stage, status, elapsed_ms, metadata, error) = match event {
                ProviderEvent::Activity => return,
                ProviderEvent::OutputDelta { .. } => return,
                ProviderEvent::StageStarted { stage, metadata } => (
                    stage,
                    RuntimeMonitorStageStatus::Started,
                    None,
                    metadata,
                    None,
                ),
                ProviderEvent::StageCompleted {
                    stage,
                    elapsed_ms,
                    metadata,
                } => (
                    stage,
                    RuntimeMonitorStageStatus::Completed,
                    Some(elapsed_ms),
                    metadata,
                    None,
                ),
                ProviderEvent::StageFailed {
                    stage,
                    elapsed_ms,
                    error,
                    metadata,
                } => (
                    stage,
                    RuntimeMonitorStageStatus::Failed,
                    Some(elapsed_ms),
                    metadata,
                    Some(error),
                ),
            };
            core.emit_monitor_event(RuntimeMonitorEvent::ProviderStage {
                trace_id: trace_id.clone(),
                invocation_id: handle.0.clone(),
                stage,
                status,
                elapsed_ms,
                request_elapsed_ms: duration_ms(request_started.elapsed()),
                input_tokens: monitor_u64(&metadata, "inputTokens"),
                output_tokens: monitor_u64(&metadata, "outputTokens"),
                resident: metadata.get("resident").and_then(Value::as_bool),
                error,
            });
        })
    }

    fn emit_monitor_event(&self, event: RuntimeMonitorEvent) {
        let observer = self
            .monitor_observer
            .read()
            .ok()
            .and_then(|observer| observer.clone());
        if let Some(observer) = observer {
            observer(event);
        }
    }

    fn emit_monitor_io_event(&self, event: RuntimeMonitorIoEvent) {
        let observer = self
            .monitor_io_observer
            .read()
            .ok()
            .and_then(|observer| observer.clone());
        if let Some(observer) = observer {
            observer(event);
        }
    }
}

fn provider_stage_name(stage: ProviderStage) -> &'static str {
    match stage {
        ProviderStage::Queue => "queue",
        ProviderStage::Load => "load",
        ProviderStage::Tokenize => "tokenize",
        ProviderStage::Prefill => "prefill",
        ProviderStage::FirstToken => "first_token",
        ProviderStage::Decode => "decode",
        ProviderStage::Validate => "validate",
    }
}

#[cfg(not(target_arch = "wasm32"))]
async fn runtime_sleep(duration: Duration) {
    tokio::time::sleep(duration).await;
}

#[cfg(target_arch = "wasm32")]
async fn runtime_sleep(duration: Duration) {
    use wasm_bindgen::JsCast;

    let milliseconds = duration.as_millis().min(i32::MAX as u128) as i32;
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        let global = js_sys::global();
        let set_timeout = js_sys::Reflect::get(&global, &"setTimeout".into())
            .ok()
            .and_then(|value| value.dyn_into::<js_sys::Function>().ok());
        if let Some(set_timeout) = set_timeout {
            if let Ok(handle) = set_timeout.call2(&global, &resolve, &milliseconds.into()) {
                let unref = js_sys::Reflect::get(&handle, &"unref".into())
                    .ok()
                    .and_then(|value| value.dyn_into::<js_sys::Function>().ok());
                if let Some(unref) = unref {
                    let _ = unref.call0(&handle);
                }
            }
        } else {
            let _ = resolve.call0(&wasm_bindgen::JsValue::UNDEFINED);
        }
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}

fn runtime_monitor_io_summary(data: &InvocationData) -> RuntimeMonitorIoSummary {
    match data {
        InvocationData::Binary(bytes) => RuntimeMonitorIoSummary {
            value: json!({
                "_vifuBinary": true,
                "bytes": bytes.len(),
            }),
            truncated: true,
        },
        InvocationData::Json(value)
            if serde_json::to_vec(value)
                .is_ok_and(|encoded| encoded.len() <= MAX_RUNTIME_MONITOR_IO_BYTES) =>
        {
            RuntimeMonitorIoSummary {
                value: value.clone(),
                truncated: false,
            }
        }
        InvocationData::Json(value) => RuntimeMonitorIoSummary {
            value: json!({
                "summary": monitor_value_shape(value),
                "truncated": true,
            }),
            truncated: true,
        },
    }
}

fn monitor_value_shape(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn monitor_u64(metadata: &Value, key: &str) -> Option<u64> {
    metadata.get(key).and_then(Value::as_u64)
}

fn is_null(value: &Value) -> bool {
    value.is_null()
}

fn merge_invocation_data(previous: &mut InvocationData, next: &InvocationData) -> bool {
    match (previous, next) {
        (
            InvocationData::Json(Value::String(previous)),
            InvocationData::Json(Value::String(next)),
        ) if previous.len().saturating_add(next.len()) <= MAX_COALESCED_EVENT_BYTES => {
            previous.push_str(next);
            true
        }
        (InvocationData::Binary(previous), InvocationData::Binary(next))
            if previous.len().saturating_add(next.len()) <= MAX_COALESCED_EVENT_BYTES =>
        {
            previous.extend_from_slice(next);
            true
        }
        _ => false,
    }
}

#[cfg(not(target_arch = "wasm32"))]
enum WorkerCommand {
    Start {
        handle: InvocationHandle,
        input: InvocationInput,
        cancellation: CancellationToken,
    },
}

#[cfg(not(target_arch = "wasm32"))]
struct RuntimeWorker {
    sender: Mutex<Option<mpsc::Sender<WorkerCommand>>>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl RuntimeWorker {
    fn spawn(core: Arc<RuntimeCore>) -> Result<Self, RuntimeError> {
        let (sender, mut receiver) = mpsc::channel(WORKER_QUEUE_CAPACITY);
        let thread = std::thread::Builder::new()
            .name(format!("vifu-runtime-{}", core.project_id))
            .spawn(move || {
                let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                    .enable_time()
                    .build()
                else {
                    return;
                };
                runtime.block_on(async move {
                    while let Some(command) = receiver.recv().await {
                        match command {
                            WorkerCommand::Start {
                                handle,
                                input,
                                cancellation,
                            } => {
                                let invocation_core = Arc::clone(&core);
                                tokio::spawn(async move {
                                    invocation_core.update_poll(
                                        &handle,
                                        InvocationStatus::Running,
                                        None,
                                        None,
                                    );
                                    let result = invocation_core
                                        .invoke(
                                            handle.0.clone(),
                                            input,
                                            cancellation,
                                            ProviderEventSink::discard(),
                                        )
                                        .await;
                                    match result {
                                        Ok(output) => invocation_core.update_poll(
                                            &handle,
                                            InvocationStatus::Completed,
                                            Some(output),
                                            None,
                                        ),
                                        Err(RuntimeError::Cancelled) => invocation_core
                                            .update_poll(
                                                &handle,
                                                InvocationStatus::Cancelled,
                                                None,
                                                None,
                                            ),
                                        Err(error) => invocation_core.update_poll(
                                            &handle,
                                            InvocationStatus::Failed,
                                            None,
                                            Some(error.public_message()),
                                        ),
                                    }
                                });
                            }
                        }
                    }
                });
            })
            .map_err(|_error| RuntimeError::Internal)?;
        Ok(Self {
            sender: Mutex::new(Some(sender)),
            thread: Mutex::new(Some(thread)),
        })
    }

    fn send(&self, command: WorkerCommand) -> Result<(), RuntimeError> {
        self.sender
            .lock()
            .map_err(|_| RuntimeError::Internal)?
            .as_ref()
            .ok_or(RuntimeError::Internal)?
            .try_send(command)
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => {
                    RuntimeError::Backpressure("invocation queue is full".to_string())
                }
                mpsc::error::TrySendError::Closed(_) => RuntimeError::Internal,
            })
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for RuntimeWorker {
    fn drop(&mut self) {
        if let Ok(sender) = self.sender.get_mut() {
            sender.take();
        }
        if let Ok(thread) = self.thread.get_mut() {
            if let Some(thread) = thread.take() {
                let _ = thread.join();
            }
        }
    }
}

/// A self-contained runtime for one application or project.
///
/// One runtime may register multiple providers, agents, and stable named
/// endpoints. It can run directly inside a Rust host; Vifu Server and Agent
/// Gateway are optional deployment components.
#[derive(Clone)]
pub struct VifuRuntime {
    core: Arc<RuntimeCore>,
    #[cfg(not(target_arch = "wasm32"))]
    worker: Arc<Mutex<Option<RuntimeWorker>>>,
}

impl fmt::Debug for VifuRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let counts = self.core.registry.read().ok().map(|registry| {
            (
                registry.providers.len(),
                registry.agents.len(),
                registry.endpoints.len(),
            )
        });
        formatter
            .debug_struct("VifuRuntime")
            .field("project_id", &self.core.project_id)
            .field("resource_counts", &counts)
            .finish()
    }
}

impl VifuRuntime {
    pub fn new(project_id: impl Into<String>) -> Result<Self, RuntimeError> {
        Self::with_store(project_id, Arc::new(MemoryRuntimeStore::default()))
    }

    pub fn with_store(
        project_id: impl Into<String>,
        store: Arc<dyn RuntimeStore>,
    ) -> Result<Self, RuntimeError> {
        let project_id = project_id.into();
        validate_identifier("project", &project_id)?;
        let core = Arc::new(RuntimeCore {
            project_id,
            registry: RwLock::new(RuntimeRegistry::default()),
            manifest: RwLock::new(None),
            store,
            sessions: RwLock::new(HashMap::new()),
            session_locks: Mutex::new(HashMap::new()),
            invocations: Mutex::new(InvocationRegistry::default()),
            next_invocation: AtomicU64::new(1),
            monitor_observer: RwLock::new(None),
            monitor_io_observer: RwLock::new(None),
        });
        #[cfg(not(target_arch = "wasm32"))]
        let worker = Arc::new(Mutex::new(None));
        Ok(Self {
            core,
            #[cfg(not(target_arch = "wasm32"))]
            worker,
        })
    }

    pub fn project_id(&self) -> &str {
        &self.core.project_id
    }

    /// Installs or clears the payload-safe runtime lifecycle observer.
    pub fn set_monitor_observer(
        &self,
        observer: Option<RuntimeMonitorObserver>,
    ) -> Result<(), RuntimeError> {
        *self
            .core
            .monitor_observer
            .write()
            .map_err(|_| RuntimeError::Internal)? = observer;
        Ok(())
    }

    /// Installs or clears the opt-in bounded invocation I/O observer.
    pub fn set_monitor_io_observer(
        &self,
        observer: Option<RuntimeMonitorIoObserver>,
    ) -> Result<(), RuntimeError> {
        *self
            .core
            .monitor_io_observer
            .write()
            .map_err(|_| RuntimeError::Internal)? = observer;
        Ok(())
    }

    pub fn register_provider(
        &self,
        name: impl Into<String>,
        provider: Arc<dyn AgentProvider>,
    ) -> Result<(), RuntimeError> {
        let name = name.into();
        validate_identifier("provider", &name)?;
        self.core
            .registry
            .write()
            .map_err(|_| RuntimeError::Internal)?
            .providers
            .insert(name, provider);
        Ok(())
    }

    /// Removes a provider from the runtime registry.
    ///
    /// An invocation that already acquired the provider keeps its own `Arc`
    /// until that invocation finishes. New invocations fail normally until a
    /// provider with the same name is registered again.
    pub fn unregister_provider(&self, name: &str) -> Result<bool, RuntimeError> {
        validate_identifier("provider", name)?;
        Ok(self
            .core
            .registry
            .write()
            .map_err(|_| RuntimeError::Internal)?
            .providers
            .remove(name)
            .is_some())
    }

    pub fn unregister_agent(&self, id: &str) -> Result<bool, RuntimeError> {
        validate_identifier("agent", id)?;
        Ok(self
            .core
            .registry
            .write()
            .map_err(|_| RuntimeError::Internal)?
            .agents
            .remove(id)
            .is_some())
    }

    pub fn unregister_endpoint(&self, name: &str) -> Result<bool, RuntimeError> {
        validate_identifier("endpoint", name)?;
        Ok(self
            .core
            .registry
            .write()
            .map_err(|_| RuntimeError::Internal)?
            .endpoints
            .remove(name)
            .is_some())
    }

    pub fn register_agent(&self, mut agent: AgentDefinition) -> Result<(), RuntimeError> {
        validate_identifier("agent", &agent.id)?;
        validate_identifier("provider", &agent.provider)?;
        if agent.name.trim().is_empty() || agent.capabilities.is_empty() {
            return Err(RuntimeError::InvalidDefinition(
                "agent name and at least one capability are required".to_string(),
            ));
        }
        for capability in &mut agent.capabilities {
            *capability = capability.trim().to_ascii_lowercase();
            validate_identifier("capability", capability)?;
        }
        agent.capabilities.sort();
        agent.capabilities.dedup();
        let mut registry = self
            .core
            .registry
            .write()
            .map_err(|_| RuntimeError::Internal)?;
        if !registry.providers.contains_key(&agent.provider) {
            return Err(RuntimeError::ProviderNotFound(agent.provider));
        }
        registry.agents.insert(agent.id.clone(), agent);
        Ok(())
    }

    pub fn register_endpoint(&self, mut endpoint: EndpointDefinition) -> Result<(), RuntimeError> {
        validate_identifier("endpoint", &endpoint.name)?;
        validate_identifier("agent", &endpoint.agent)?;
        endpoint.capability = endpoint.capability.trim().to_ascii_lowercase();
        validate_identifier("capability", &endpoint.capability)?;
        if !(1..=MAX_ENDPOINT_TIMEOUT_MS).contains(&endpoint.timeout_ms) {
            return Err(RuntimeError::InvalidDefinition(format!(
                "endpoint timeout must be between 1 and {MAX_ENDPOINT_TIMEOUT_MS} ms"
            )));
        }
        let mut registry = self
            .core
            .registry
            .write()
            .map_err(|_| RuntimeError::Internal)?;
        let agent = registry
            .agents
            .get(&endpoint.agent)
            .ok_or_else(|| RuntimeError::AgentNotFound(endpoint.agent.clone()))?;
        if !agent
            .capabilities
            .iter()
            .any(|capability| capability == &endpoint.capability)
        {
            return Err(RuntimeError::CapabilityUnavailable {
                provider: agent.provider.clone(),
                capability: endpoint.capability,
            });
        }
        registry.endpoints.insert(endpoint.name.clone(), endpoint);
        Ok(())
    }

    pub fn agent_definitions(&self) -> Result<Vec<AgentDefinition>, RuntimeError> {
        let mut agents = self
            .core
            .registry
            .read()
            .map_err(|_| RuntimeError::Internal)?
            .agents
            .values()
            .cloned()
            .collect::<Vec<_>>();
        agents.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(agents)
    }

    pub fn endpoint_definitions(&self) -> Result<Vec<EndpointDefinition>, RuntimeError> {
        let mut endpoints = self
            .core
            .registry
            .read()
            .map_err(|_| RuntimeError::Internal)?
            .endpoints
            .values()
            .cloned()
            .collect::<Vec<_>>();
        endpoints.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(endpoints)
    }

    /// Replaces the portable agent and endpoint graph with a validated manifest.
    /// Provider implementations must be registered locally before activation.
    pub fn apply_manifest(&self, manifest: RuntimeManifest) -> Result<(), RuntimeError> {
        manifest.validate()?;
        if manifest.project_id != self.core.project_id {
            return Err(RuntimeError::InvalidDefinition(
                "project settings belong to another project".to_string(),
            ));
        }
        let mut registry = self
            .core
            .registry
            .write()
            .map_err(|_| RuntimeError::Internal)?;
        for requirement in &manifest.providers {
            let provider = registry
                .providers
                .get(&requirement.id)
                .ok_or_else(|| RuntimeError::ProviderNotFound(requirement.id.clone()))?;
            for capability in &requirement.capabilities {
                if !provider.supports(capability) {
                    return Err(RuntimeError::CapabilityUnavailable {
                        provider: requirement.id.clone(),
                        capability: capability.clone(),
                    });
                }
            }
        }
        registry.agents = manifest
            .agents
            .iter()
            .cloned()
            .map(|agent| (agent.id.clone(), agent))
            .collect();
        registry.endpoints = manifest
            .endpoints
            .iter()
            .cloned()
            .map(|endpoint| (endpoint.name.clone(), endpoint))
            .collect();
        *self
            .core
            .manifest
            .write()
            .map_err(|_| RuntimeError::Internal)? = Some(manifest);
        Ok(())
    }

    pub fn apply_project_settings(&self, settings: ProjectSettings) -> Result<(), RuntimeError> {
        self.apply_manifest(settings)
    }

    pub fn current_manifest(&self) -> Result<Option<RuntimeManifest>, RuntimeError> {
        Ok(self
            .core
            .manifest
            .read()
            .map_err(|_| RuntimeError::Internal)?
            .clone())
    }

    pub fn current_project_settings(&self) -> Result<Option<ProjectSettings>, RuntimeError> {
        self.current_manifest()
    }

    pub fn install_release(&self, release: &RuntimeRelease) -> Result<(), RuntimeError> {
        release.validate()?;
        if release.manifest.project_id != self.core.project_id {
            return Err(RuntimeError::InvalidDefinition(
                "runtime release belongs to another project".to_string(),
            ));
        }
        self.core.store.save_release(release)
    }

    pub fn releases(&self) -> Result<Vec<RuntimeRelease>, RuntimeError> {
        self.core.store.list_releases(&self.core.project_id)
    }

    pub fn active_release_version(&self) -> Result<Option<u64>, RuntimeError> {
        self.core.store.active_release(&self.core.project_id)
    }

    pub fn activate_release(&self, version: u64) -> Result<RuntimeRelease, RuntimeError> {
        let release = self
            .core
            .store
            .load_release(&self.core.project_id, version)?
            .ok_or_else(|| RuntimeError::store("runtime release was not found"))?;
        self.apply_manifest(release.manifest.clone())?;
        self.core
            .store
            .set_active_release(&self.core.project_id, version)?;
        Ok(release)
    }

    pub fn restore_active_release(&self) -> Result<Option<RuntimeRelease>, RuntimeError> {
        self.active_release_version()?
            .map(|version| self.activate_release(version))
            .transpose()
    }

    pub fn bootstrap_release(
        &self,
        manifest: RuntimeManifest,
    ) -> Result<RuntimeRelease, RuntimeError> {
        if let Some(active) = self.restore_active_release()? {
            return Ok(active);
        }
        let release = RuntimeRelease::new(1, manifest)?;
        self.install_release(&release)?;
        self.activate_release(release.version)
    }

    pub fn bootstrap_project_settings(
        &self,
        settings: ProjectSettings,
    ) -> Result<RuntimeRelease, RuntimeError> {
        self.bootstrap_release(settings)
    }

    pub fn save_local_provider_binding(
        &self,
        binding: &LocalProviderBinding,
    ) -> Result<(), RuntimeError> {
        validate_identifier("provider", &binding.provider_id)?;
        self.core
            .store
            .save_local_provider_binding(&self.core.project_id, binding)
    }

    pub fn local_provider_bindings(&self) -> Result<Vec<LocalProviderBinding>, RuntimeError> {
        self.core
            .store
            .local_provider_bindings(&self.core.project_id)
    }

    pub fn pending_traces(&self, limit: usize) -> Result<Vec<RuntimeTraceRecord>, RuntimeError> {
        self.core.store.pending_traces(limit.min(1_000))
    }

    pub fn acknowledge_traces(&self, trace_ids: &[String]) -> Result<(), RuntimeError> {
        self.core.store.acknowledge_traces(trace_ids)
    }

    pub fn session(&self, session_id: impl Into<String>) -> Result<RuntimeSession, RuntimeError> {
        let session_id = session_id.into();
        validate_identifier("session", &session_id)?;
        Ok(RuntimeSession {
            runtime: self.clone(),
            session_id,
        })
    }

    pub async fn invoke(&self, input: InvocationInput) -> Result<InvocationOutput, RuntimeError> {
        self.invoke_with_cancellation(input, CancellationToken::default())
            .await
    }

    /// Invokes an endpoint while honoring a cancellation signal owned by the
    /// embedding host.
    pub async fn invoke_with_cancellation(
        &self,
        input: InvocationInput,
        cancellation: CancellationToken,
    ) -> Result<InvocationOutput, RuntimeError> {
        self.invoke_with_events_and_cancellation(input, cancellation, ProviderEventSink::discard())
            .await
    }

    /// Invokes an endpoint while forwarding real provider progress to an
    /// embedding host and honoring the host's cancellation signal.
    pub async fn invoke_with_events_and_cancellation(
        &self,
        input: InvocationInput,
        cancellation: CancellationToken,
        events: ProviderEventSink,
    ) -> Result<InvocationOutput, RuntimeError> {
        let invocation_id = self.core.next_invocation_id();
        self.core
            .invoke(invocation_id, input, cancellation, events)
            .await
    }

    pub fn start_invoke(&self, input: InvocationInput) -> Result<InvocationHandle, RuntimeError> {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = input;
            return Err(RuntimeError::InvalidDefinition(
                "background invocation polling is unavailable in WASM; use invoke".to_string(),
            ));
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            validate_identifier("endpoint", &input.endpoint)?;
            validate_identifier("session", &input.session_id)?;
            let handle = InvocationHandle(self.core.next_invocation_id());
            let cancellation = CancellationToken::default();
            let mut worker = self.worker.lock().map_err(|_| RuntimeError::Internal)?;
            if worker.is_none() {
                *worker = Some(RuntimeWorker::spawn(Arc::clone(&self.core))?);
            }
            self.core
                .invocations
                .lock()
                .map_err(|_| RuntimeError::Internal)?
                .insert(handle.clone(), cancellation.clone())?;
            let send_result =
                worker
                    .as_ref()
                    .ok_or(RuntimeError::Internal)?
                    .send(WorkerCommand::Start {
                        handle: handle.clone(),
                        input,
                        cancellation,
                    });
            if let Err(error) = send_result {
                self.core
                    .invocations
                    .lock()
                    .map_err(|_| RuntimeError::Internal)?
                    .remove(&handle);
                return Err(error);
            }
            Ok(handle)
        }
    }

    pub fn poll_invocation(
        &self,
        handle: &InvocationHandle,
    ) -> Result<InvocationPoll, RuntimeError> {
        self.core
            .invocations
            .lock()
            .map_err(|_| RuntimeError::Internal)?
            .entries
            .get(&handle.0)
            .map(|entry| entry.poll.clone())
            .ok_or_else(|| RuntimeError::InvocationNotFound(handle.0.clone()))
    }

    /// Drains incremental events produced since the previous call.
    pub fn drain_invocation_events(
        &self,
        handle: &InvocationHandle,
    ) -> Result<Vec<InvocationEvent>, RuntimeError> {
        self.core
            .invocations
            .lock()
            .map_err(|_| RuntimeError::Internal)?
            .drain_events(handle)
    }

    /// Returns the current poll state and removes it once it is terminal.
    ///
    /// Pending and running invocations remain registered so callers can keep
    /// polling the same handle.
    pub fn take_invocation(
        &self,
        handle: &InvocationHandle,
    ) -> Result<InvocationPoll, RuntimeError> {
        self.core
            .invocations
            .lock()
            .map_err(|_| RuntimeError::Internal)?
            .take(handle)
    }

    pub fn cancel_invocation(&self, handle: &InvocationHandle) -> Result<(), RuntimeError> {
        let cancellation = self
            .core
            .invocations
            .lock()
            .map_err(|_| RuntimeError::Internal)?
            .entries
            .get(&handle.0)
            .map(|entry| entry.cancellation.clone())
            .ok_or_else(|| RuntimeError::InvocationNotFound(handle.0.clone()))?;
        cancellation.cancel();
        self.core
            .update_poll(handle, InvocationStatus::Cancelled, None, None);
        Ok(())
    }

    pub async fn execute_effects(
        &self,
        effects: Vec<EffectRequest>,
    ) -> Result<EffectExecution, RuntimeError> {
        self.execute_effects_with_limit(effects, DEFAULT_EFFECT_LIMIT)
            .await
    }

    pub async fn execute_effects_with_limit(
        &self,
        effects: Vec<EffectRequest>,
        limit: usize,
    ) -> Result<EffectExecution, RuntimeError> {
        if effects.len() > limit {
            return Err(RuntimeError::EffectLimitExceeded(limit));
        }
        let mut results = Vec::new();
        let mut unhandled = Vec::new();
        for effect in effects {
            if effect.kind != "agent.invoke" {
                unhandled.push(effect);
                continue;
            }
            let input = serde_json::from_value::<InvocationInput>(effect.payload.clone())
                .map_err(|error| RuntimeError::InvalidDefinition(error.to_string()))?;
            let result = self.invoke(input).await;
            match result {
                Ok(output) => results.push(EffectResult {
                    effect_id: effect.id,
                    succeeded: true,
                    output: serde_json::to_value(output)
                        .map_err(|_error| RuntimeError::Internal)?,
                }),
                Err(error) => results.push(EffectResult {
                    effect_id: effect.id,
                    succeeded: false,
                    output: json!({ "error": error.public_message() }),
                }),
            }
        }
        Ok(EffectExecution { results, unhandled })
    }

    pub fn export_snapshot(&self) -> Result<Vec<u8>, RuntimeError> {
        let snapshot = PortableProjectSnapshot {
            version: SNAPSHOT_VERSION,
            project_id: self.core.project_id.clone(),
            sessions: self
                .core
                .sessions
                .read()
                .map_err(|_| RuntimeError::Internal)?
                .clone(),
        };
        serde_json::to_vec(&snapshot).map_err(|error| RuntimeError::Snapshot(error.to_string()))
    }

    pub fn restore_snapshot(&self, bytes: &[u8]) -> Result<(), RuntimeError> {
        let snapshot = serde_json::from_slice::<PortableProjectSnapshot>(bytes)
            .map_err(|error| RuntimeError::Snapshot(error.to_string()))?;
        if snapshot.version != SNAPSHOT_VERSION || snapshot.project_id != self.core.project_id {
            return Err(RuntimeError::Snapshot(
                "snapshot version or project does not match".to_string(),
            ));
        }
        for (session_id, state) in &snapshot.sessions {
            validate_identifier("session", session_id)?;
            self.core
                .store
                .save(&self.core.project_id, session_id, state)?;
        }
        *self
            .core
            .sessions
            .write()
            .map_err(|_| RuntimeError::Internal)? = snapshot.sessions;
        Ok(())
    }
}

/// A session-scoped view over [`VifuRuntime`].
#[derive(Clone, Debug)]
pub struct RuntimeSession {
    runtime: VifuRuntime,
    session_id: String,
}

impl RuntimeSession {
    pub fn id(&self) -> &str {
        &self.session_id
    }

    pub async fn invoke(
        &self,
        mut input: InvocationInput,
    ) -> Result<InvocationOutput, RuntimeError> {
        input.session_id.clone_from(&self.session_id);
        self.runtime.invoke(input).await
    }

    pub fn start_invoke(
        &self,
        mut input: InvocationInput,
    ) -> Result<InvocationHandle, RuntimeError> {
        input.session_id.clone_from(&self.session_id);
        self.runtime.start_invoke(input)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PortableProjectSnapshot {
    version: u32,
    project_id: String,
    sessions: HashMap<String, RuntimeSnapshot>,
}

fn default_session_id() -> String {
    "default".to_string()
}

const fn default_timeout_ms() -> u64 {
    DEFAULT_TIMEOUT_MS
}

fn validate_identifier(kind: &str, value: &str) -> Result<(), RuntimeError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(RuntimeError::InvalidDefinition(format!(
            "{kind} must be a portable identifier"
        )));
    }
    Ok(())
}

fn duration_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

const fn is_terminal_status(status: InvocationStatus) -> bool {
    matches!(
        status,
        InvocationStatus::Completed | InvocationStatus::Failed | InvocationStatus::Cancelled
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestProvider {
        fail: bool,
        delay: Duration,
    }

    impl TestProvider {
        fn immediate() -> Self {
            Self {
                fail: false,
                delay: Duration::ZERO,
            }
        }
    }

    impl AgentProvider for TestProvider {
        fn supports(&self, capability: &str) -> bool {
            matches!(capability, "chat" | "speech" | "transcription")
        }

        fn invoke<'a>(
            &'a self,
            request: ProviderRequest,
            cancellation: CancellationToken,
        ) -> ProviderFuture<'a> {
            Box::pin(async move {
                if !self.delay.is_zero() {
                    tokio::select! {
                        _ = tokio::time::sleep(self.delay) => {}
                        _ = cancellation.cancelled() => {
                            return Err(RuntimeError::Cancelled);
                        }
                    }
                }
                if self.fail {
                    return Err(RuntimeError::provider(
                        request.agent.provider,
                        "synthetic provider failure",
                    ));
                }
                Ok(ProviderResponse {
                    data: match request.data {
                        InvocationData::Json(data) => InvocationData::Json(json!({
                            "capability": request.capability,
                            "input": data,
                        })),
                        InvocationData::Binary(bytes) => InvocationData::Binary(bytes),
                    },
                    metadata: json!({}),
                    state: Some(json!({
                        "lastEndpoint": request.endpoint,
                        "previousRevision": request.snapshot.revision,
                    })),
                })
            })
        }
    }

    struct StreamingTestProvider;

    impl AgentProvider for StreamingTestProvider {
        fn supports(&self, capability: &str) -> bool {
            capability == "chat"
        }

        fn invoke<'a>(
            &'a self,
            request: ProviderRequest,
            cancellation: CancellationToken,
        ) -> ProviderFuture<'a> {
            self.invoke_with_events(request, cancellation, ProviderEventSink::discard())
        }

        fn invoke_with_events<'a>(
            &'a self,
            _request: ProviderRequest,
            cancellation: CancellationToken,
            events: ProviderEventSink,
        ) -> ProviderFuture<'a> {
            Box::pin(async move {
                if cancellation.is_cancelled() {
                    return Err(RuntimeError::Cancelled);
                }
                events.stage_started(ProviderStage::Tokenize, Value::Null);
                events.stage_completed(ProviderStage::Tokenize, 2, json!({ "inputTokens": 4 }));
                events.output_delta(InvocationData::Json(Value::String("Hello".to_string())));
                events.output_delta(InvocationData::Json(Value::String(", world".to_string())));
                Ok(ProviderResponse::json(json!({ "text": "Hello, world" })))
            })
        }
    }

    struct ActiveSlowProvider;

    impl AgentProvider for ActiveSlowProvider {
        fn supports(&self, capability: &str) -> bool {
            capability == "chat"
        }

        fn invoke<'a>(
            &'a self,
            request: ProviderRequest,
            cancellation: CancellationToken,
        ) -> ProviderFuture<'a> {
            self.invoke_with_events(request, cancellation, ProviderEventSink::discard())
        }

        fn invoke_with_events<'a>(
            &'a self,
            _request: ProviderRequest,
            cancellation: CancellationToken,
            events: ProviderEventSink,
        ) -> ProviderFuture<'a> {
            Box::pin(async move {
                for _ in 0..4 {
                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_millis(8)) => events.activity(),
                        _ = cancellation.cancelled() => return Err(RuntimeError::Cancelled),
                    }
                }
                Ok(ProviderResponse::json(json!({ "ok": true })))
            })
        }
    }

    fn configured_runtime(provider: Arc<dyn AgentProvider>) -> VifuRuntime {
        let runtime = VifuRuntime::new("test-project").expect("runtime should start");
        runtime
            .register_provider("test-provider", provider)
            .expect("provider should register");
        runtime
            .register_agent(AgentDefinition {
                id: "guide".to_string(),
                name: "Guide".to_string(),
                provider: "test-provider".to_string(),
                capabilities: vec![
                    "chat".to_string(),
                    "speech".to_string(),
                    "transcription".to_string(),
                ],
                metadata: json!({ "public": true }),
            })
            .expect("agent should register");
        for capability in ["chat", "speech", "transcription"] {
            runtime
                .register_endpoint(EndpointDefinition {
                    name: capability.to_string(),
                    agent: "guide".to_string(),
                    capability: capability.to_string(),
                    timeout_ms: 500,
                })
                .expect("endpoint should register");
        }
        runtime
    }

    #[test]
    fn dynamic_endpoint_accepts_slow_local_model_inference() {
        let runtime = configured_runtime(Arc::new(TestProvider::immediate()));

        let result = runtime.register_endpoint(EndpointDefinition {
            name: "slow-chat".to_string(),
            agent: "guide".to_string(),
            capability: "chat".to_string(),
            timeout_ms: 300_000,
        });

        assert!(
            result.is_ok(),
            "five-minute endpoint should register: {result:?}"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn embedded_runtime_invokes_chat_speech_and_transcription_without_a_server() {
        let runtime = configured_runtime(Arc::new(TestProvider::immediate()));

        for capability in ["chat", "speech", "transcription"] {
            let output = runtime
                .invoke(InvocationInput::json(
                    capability,
                    json!({ "message": capability }),
                ))
                .await
                .expect("endpoint should invoke");
            assert_eq!(output.capability, capability);
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn invocation_result_includes_completed_provider_stages() {
        let runtime = configured_runtime(Arc::new(StreamingTestProvider));

        let output = runtime
            .invoke(InvocationInput::json("chat", json!({})))
            .await
            .expect("streaming provider should complete");

        assert_eq!(output.trace[0].name, "tokenize");
        assert_eq!(output.trace[0].status, "completed");
        assert_eq!(output.trace[0].duration_ms, 2);
        assert_eq!(output.trace[0].attributes, json!({ "inputTokens": 4 }));
        assert_eq!(output.trace[1].name, "provider.invoke");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn monitor_observer_receives_provider_performance_metadata() {
        let runtime = configured_runtime(Arc::new(StreamingTestProvider));
        let monitor_events = Arc::new(Mutex::new(Vec::new()));
        let captured_events = Arc::clone(&monitor_events);
        runtime
            .set_monitor_observer(Some(Arc::new(move |event| {
                captured_events.lock().unwrap().push(event);
            })))
            .unwrap();

        runtime
            .invoke(InvocationInput::json("chat", json!({ "text": "hello" })))
            .await
            .unwrap();

        assert!(monitor_events.lock().unwrap().iter().any(|event| matches!(
            event,
            RuntimeMonitorEvent::ProviderStage {
                stage: ProviderStage::Tokenize,
                status: RuntimeMonitorStageStatus::Completed,
                input_tokens: Some(4),
                ..
            }
        )));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn monitor_io_observer_receives_chat_input_and_output() {
        let runtime = configured_runtime(Arc::new(TestProvider::immediate()));
        let monitor_events = Arc::new(Mutex::new(Vec::new()));
        let captured_events = Arc::clone(&monitor_events);
        runtime
            .set_monitor_io_observer(Some(Arc::new(move |event| {
                captured_events.lock().unwrap().push(event);
            })))
            .unwrap();

        runtime
            .invoke(InvocationInput::json("chat", json!({ "text": "hello" })))
            .await
            .unwrap();

        let events = monitor_events.lock().unwrap();
        assert!(matches!(
            events.as_slice(),
            [
                RuntimeMonitorIoEvent::InvocationInput { summary: input, .. },
                RuntimeMonitorIoEvent::InvocationOutput { summary: output, .. }
            ] if input.value == json!({ "text": "hello" })
                && output.value["input"] == json!({ "text": "hello" })
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn runtime_sessions_keep_independent_durable_state() {
        let runtime = configured_runtime(Arc::new(TestProvider::immediate()));
        let first = runtime
            .session("player-one")
            .expect("first session should open");
        let second = runtime
            .session("player-two")
            .expect("second session should open");

        let first_output = first
            .invoke(InvocationInput::json("chat", json!({ "text": "one" })))
            .await
            .expect("first session should invoke");
        let second_output = second
            .invoke(InvocationInput::json("chat", json!({ "text": "two" })))
            .await
            .expect("second session should invoke");

        assert_eq!(first_output.snapshot.revision, 1);
        assert_eq!(second_output.snapshot.revision, 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn concurrent_calls_serialize_state_updates_for_one_session() {
        let runtime = configured_runtime(Arc::new(TestProvider {
            fail: false,
            delay: Duration::from_millis(5),
        }));
        let first = runtime.invoke(
            InvocationInput::json("chat", json!({ "text": "one" })).with_session("shared-session"),
        );
        let second = runtime.invoke(
            InvocationInput::json("chat", json!({ "text": "two" })).with_session("shared-session"),
        );

        let (first, second) = tokio::join!(first, second);
        let mut revisions = [
            first
                .expect("first invocation should complete")
                .snapshot
                .revision,
            second
                .expect("second invocation should complete")
                .snapshot
                .revision,
        ];
        revisions.sort_unstable();
        assert_eq!(revisions, [1, 2]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn runtime_round_trips_binary_provider_results() {
        let runtime = configured_runtime(Arc::new(TestProvider::immediate()));
        let output = runtime
            .invoke(InvocationInput {
                endpoint: "speech".to_string(),
                session_id: "audio-session".to_string(),
                data: InvocationData::Binary(vec![1, 2, 3, 4]),
                metadata: json!({}),
            })
            .await
            .expect("binary invocation should complete");

        assert_eq!(output.data, InvocationData::Binary(vec![1, 2, 3, 4]));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn runtime_times_out_slow_providers() {
        let runtime = VifuRuntime::new("timeout-project").expect("runtime should start");
        runtime
            .register_provider(
                "slow",
                Arc::new(TestProvider {
                    fail: false,
                    delay: Duration::from_millis(100),
                }),
            )
            .expect("provider should register");
        runtime
            .register_agent(AgentDefinition {
                id: "slow-agent".to_string(),
                name: "Slow agent".to_string(),
                provider: "slow".to_string(),
                capabilities: vec!["chat".to_string()],
                metadata: json!({}),
            })
            .expect("agent should register");
        runtime
            .register_endpoint(EndpointDefinition {
                name: "slow-chat".to_string(),
                agent: "slow-agent".to_string(),
                capability: "chat".to_string(),
                timeout_ms: 10,
            })
            .expect("endpoint should register");

        let error = runtime
            .invoke(InvocationInput::json("slow-chat", json!({})))
            .await
            .expect_err("slow invocation should time out");
        assert!(matches!(error, RuntimeError::Timeout(10)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn provider_activity_resets_the_runtime_idle_timeout() {
        let runtime = VifuRuntime::new("active-project").expect("runtime should start");
        runtime
            .register_provider("active", Arc::new(ActiveSlowProvider))
            .expect("provider should register");
        runtime
            .register_agent(AgentDefinition {
                id: "active-agent".to_string(),
                name: "Active agent".to_string(),
                provider: "active".to_string(),
                capabilities: vec!["chat".to_string()],
                metadata: json!({}),
            })
            .expect("agent should register");
        runtime
            .register_endpoint(EndpointDefinition {
                name: "active-chat".to_string(),
                agent: "active-agent".to_string(),
                capability: "chat".to_string(),
                timeout_ms: 10,
            })
            .expect("endpoint should register");

        let output = runtime
            .invoke(InvocationInput::json("active-chat", json!({})))
            .await
            .expect("ongoing provider activity should renew the idle timeout");
        assert_eq!(output.data, InvocationData::Json(json!({ "ok": true })));
    }

    #[test]
    fn game_loop_api_starts_polls_and_cancels_invocations() {
        let runtime = configured_runtime(Arc::new(TestProvider {
            fail: false,
            delay: Duration::from_secs(5),
        }));
        let handle = runtime
            .start_invoke(InvocationInput::json("chat", json!({})))
            .expect("invocation should start");
        let running_deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let poll = runtime
                .poll_invocation(&handle)
                .expect("invocation should remain pollable");
            if poll.status == InvocationStatus::Running {
                break;
            }
            assert!(
                Instant::now() < running_deadline,
                "invocation did not start"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        runtime
            .cancel_invocation(&handle)
            .expect("invocation should cancel");

        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let poll = runtime
                .poll_invocation(&handle)
                .expect("invocation should remain pollable");
            if poll.status == InvocationStatus::Cancelled {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "cancelled provider did not observe cancellation"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn game_loop_poll_returns_the_same_provider_result_shape_as_async_invoke() {
        let runtime = configured_runtime(Arc::new(TestProvider::immediate()));
        let handle = runtime
            .start_invoke(
                InvocationInput::json("chat", json!({ "text": "hello" }))
                    .with_session("poll-session"),
            )
            .expect("invocation should start");
        let deadline = Instant::now() + Duration::from_secs(1);
        let output = loop {
            let poll = runtime
                .poll_invocation(&handle)
                .expect("invocation should remain pollable");
            if let Some(output) = poll.output {
                break output;
            }
            assert!(
                !matches!(
                    poll.status,
                    InvocationStatus::Failed | InvocationStatus::Cancelled
                ),
                "invocation unexpectedly failed: {poll:?}"
            );
            assert!(Instant::now() < deadline, "invocation did not complete");
            std::thread::sleep(Duration::from_millis(5));
        };

        assert_eq!(
            output.data,
            InvocationData::Json(json!({
                "capability": "chat",
                "input": { "text": "hello" },
            }))
        );
    }

    #[test]
    fn game_loop_v1_event_stream_ignores_provider_stages() {
        let runtime = configured_runtime(Arc::new(StreamingTestProvider));
        let handle = runtime
            .start_invoke(InvocationInput::json("chat", json!({})))
            .expect("invocation should start");
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let poll = runtime
                .poll_invocation(&handle)
                .expect("invocation should remain pollable");
            if poll.status == InvocationStatus::Completed {
                break;
            }
            assert!(Instant::now() < deadline, "invocation did not complete");
            std::thread::sleep(Duration::from_millis(5));
        }

        let events = runtime
            .drain_invocation_events(&handle)
            .expect("events should be available");
        assert_eq!(
            events.iter().map(|event| event.kind).collect::<Vec<_>>(),
            vec![
                InvocationEventKind::Started,
                InvocationEventKind::OutputDelta,
                InvocationEventKind::Completed,
            ]
        );
        assert_eq!(
            events[1].data,
            Some(InvocationData::Json(Value::String(
                "Hello, world".to_string()
            )))
        );
    }

    #[test]
    fn invocation_registry_ignores_output_after_terminal_event() {
        let handle = InvocationHandle("late-output".to_string());
        let mut registry = InvocationRegistry::default();
        registry
            .insert(handle.clone(), CancellationToken::default())
            .expect("invocation should be registered");
        registry.update(&handle, InvocationStatus::Running, None, None);
        registry.update(
            &handle,
            InvocationStatus::Failed,
            None,
            Some("provider failed".to_string()),
        );

        registry.push_provider_event(
            &handle,
            ProviderEvent::OutputDelta {
                data: InvocationData::Json(Value::String("too late".to_string())),
            },
        );

        let events = registry
            .drain_events(&handle)
            .expect("events should remain available");
        assert_eq!(
            events.iter().map(|event| event.kind).collect::<Vec<_>>(),
            vec![InvocationEventKind::Started, InvocationEventKind::Failed]
        );
    }

    #[test]
    fn taking_a_terminal_invocation_releases_its_result() {
        let runtime = configured_runtime(Arc::new(TestProvider::immediate()));
        let handle = runtime
            .start_invoke(InvocationInput::json("chat", json!({})))
            .expect("invocation should start");
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let poll = runtime
                .take_invocation(&handle)
                .expect("invocation should remain available until terminal");
            if is_terminal_status(poll.status) {
                break;
            }
            assert!(Instant::now() < deadline, "invocation did not complete");
            std::thread::sleep(Duration::from_millis(5));
        }

        assert!(matches!(
            runtime.poll_invocation(&handle),
            Err(RuntimeError::InvocationNotFound(_))
        ));
    }

    #[test]
    fn game_loop_api_applies_backpressure_to_excess_invocations() {
        let runtime = configured_runtime(Arc::new(TestProvider {
            fail: false,
            delay: Duration::from_secs(5),
        }));
        let handles = (0..MAX_IN_FLIGHT_INVOCATIONS)
            .map(|index| {
                runtime
                    .start_invoke(
                        InvocationInput::json("chat", json!({}))
                            .with_session(format!("session-{index}")),
                    )
                    .expect("invocation within the bound should start")
            })
            .collect::<Vec<_>>();

        let error = runtime
            .start_invoke(
                InvocationInput::json("chat", json!({})).with_session("one-session-too-many"),
            )
            .expect_err("invocations above the bound should be rejected");
        assert!(matches!(error, RuntimeError::Backpressure(_)));

        for handle in handles {
            runtime
                .cancel_invocation(&handle)
                .expect("test invocation should cancel");
        }
    }

    #[test]
    fn terminal_invocation_history_is_bounded() {
        let runtime = configured_runtime(Arc::new(TestProvider::immediate()));
        let first = runtime
            .start_invoke(
                InvocationInput::json("chat", json!({})).with_session("retained-session-0"),
            )
            .expect("first invocation should start");
        let mut last = first.clone();
        for index in 0..=MAX_RETAINED_INVOCATIONS {
            let handle = if index == 0 {
                first.clone()
            } else {
                runtime
                    .start_invoke(
                        InvocationInput::json("chat", json!({}))
                            .with_session(format!("retained-session-{index}")),
                    )
                    .expect("invocation should start")
            };
            let deadline = Instant::now() + Duration::from_secs(1);
            loop {
                let poll = runtime
                    .poll_invocation(&handle)
                    .expect("latest invocation should remain available");
                if is_terminal_status(poll.status) {
                    break;
                }
                assert!(Instant::now() < deadline, "invocation did not complete");
                std::thread::sleep(Duration::from_millis(2));
            }
            last = handle;
        }

        assert!(matches!(
            runtime.poll_invocation(&first),
            Err(RuntimeError::InvocationNotFound(_))
        ));
        assert!(runtime.poll_invocation(&last).is_ok());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn runtime_executes_agent_effects_and_returns_custom_effects_to_the_host() {
        let runtime = configured_runtime(Arc::new(TestProvider::immediate()));
        let execution = runtime
            .execute_effects(vec![
                EffectRequest {
                    id: "agent-effect".to_string(),
                    kind: "agent.invoke".to_string(),
                    payload: serde_json::to_value(InvocationInput::json(
                        "chat",
                        json!({ "text": "hello" }),
                    ))
                    .unwrap(),
                },
                EffectRequest {
                    id: "host-effect".to_string(),
                    kind: "game.play_animation".to_string(),
                    payload: json!({ "name": "wave" }),
                },
            ])
            .await
            .expect("effects should execute");

        assert_eq!(execution.results.len(), 1);
        assert_eq!(execution.unhandled[0].kind, "game.play_animation");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn runtime_rejects_effect_batches_above_the_bound() {
        let runtime = configured_runtime(Arc::new(TestProvider::immediate()));
        let effects = (0..3)
            .map(|index| EffectRequest {
                id: format!("effect-{index}"),
                kind: "host.effect".to_string(),
                payload: json!({}),
            })
            .collect();

        let error = runtime
            .execute_effects_with_limit(effects, 2)
            .await
            .expect_err("oversized effect batch should fail");
        assert!(matches!(error, RuntimeError::EffectLimitExceeded(2)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn snapshots_restore_session_state_without_runtime_definitions_or_secrets() {
        let secret = "synthetic-secret-must-not-leak";
        let runtime = configured_runtime(Arc::new(TestProvider::immediate()));
        runtime
            .invoke(
                InvocationInput::json("chat", json!({ "text": "hello" }))
                    .with_session("saved-session"),
            )
            .await
            .expect("invocation should create state");
        let bytes = runtime.export_snapshot().expect("snapshot should export");
        assert!(!String::from_utf8_lossy(&bytes).contains(secret));

        let restored = configured_runtime(Arc::new(TestProvider::immediate()));
        restored
            .restore_snapshot(&bytes)
            .expect("snapshot should restore");
        let output = restored
            .invoke(
                InvocationInput::json("chat", json!({ "text": "again" }))
                    .with_session("saved-session"),
            )
            .await
            .expect("restored session should invoke");

        assert_eq!(output.snapshot.revision, 2);
    }

    #[test]
    fn debug_output_redacts_payloads_provider_errors_and_snapshots() {
        let secret = "synthetic-secret-must-not-leak";
        let input = InvocationInput::json("chat", json!({ "secret": secret }));
        let error = RuntimeError::provider("test-provider", secret);
        let runtime = configured_runtime(Arc::new(TestProvider::immediate()));

        assert!(!format!("{input:?}").contains(secret));
        assert!(!format!("{error:?}").contains(secret));
        assert!(!format!("{runtime:?}").contains(secret));
    }
}
