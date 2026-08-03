use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use uuid::Uuid;
use vifu_gateway::protocol::{canonical_trace_io_summary, AgentDescriptor};

const RUNTIME_EVENT_CAPACITY: usize = 2_048;

struct RuntimeEventMailbox {
    queue: Mutex<VecDeque<RuntimeEvent>>,
    notify: tokio::sync::Notify,
    capacity: usize,
    senders: AtomicUsize,
    receiver_alive: AtomicBool,
    dropped_events: AtomicUsize,
}

pub struct RuntimeEventSender {
    mailbox: Arc<RuntimeEventMailbox>,
}

pub struct RuntimeEventReceiver {
    mailbox: Arc<RuntimeEventMailbox>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeEventTryRecvError {
    Empty,
    Disconnected,
}

pub fn runtime_event_channel() -> (RuntimeEventSender, RuntimeEventReceiver) {
    runtime_event_channel_with_capacity(RUNTIME_EVENT_CAPACITY)
}

fn runtime_event_channel_with_capacity(
    capacity: usize,
) -> (RuntimeEventSender, RuntimeEventReceiver) {
    let mailbox = Arc::new(RuntimeEventMailbox {
        queue: Mutex::new(VecDeque::with_capacity(capacity)),
        notify: tokio::sync::Notify::new(),
        capacity: capacity.max(1),
        senders: AtomicUsize::new(1),
        receiver_alive: AtomicBool::new(true),
        dropped_events: AtomicUsize::new(0),
    });
    (
        RuntimeEventSender {
            mailbox: Arc::clone(&mailbox),
        },
        RuntimeEventReceiver { mailbox },
    )
}

impl Clone for RuntimeEventSender {
    fn clone(&self) -> Self {
        self.mailbox.senders.fetch_add(1, Ordering::Relaxed);
        Self {
            mailbox: Arc::clone(&self.mailbox),
        }
    }
}

impl Drop for RuntimeEventSender {
    fn drop(&mut self) {
        if self.mailbox.senders.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.mailbox.notify.notify_waiters();
        }
    }
}

impl RuntimeEventSender {
    pub fn send(&self, event: RuntimeEvent) -> Result<(), Box<RuntimeEvent>> {
        if !self.mailbox.receiver_alive.load(Ordering::Acquire) {
            return Err(Box::new(event));
        }
        let mut queue = lock(&self.mailbox.queue);
        if queue.len() == self.mailbox.capacity {
            if let Some(index) = queue
                .iter()
                .position(|queued| event_coalescing_key(queued) == event_coalescing_key(&event))
            {
                queue[index] = event;
                drop(queue);
                self.mailbox.notify.notify_one();
                return Ok(());
            }
            if event_requires_delivery(&event) {
                if let Some(index) = queue
                    .iter()
                    .position(|queued| !event_requires_delivery(queued))
                {
                    queue.remove(index);
                    self.mailbox.dropped_events.fetch_add(1, Ordering::Relaxed);
                } else {
                    self.mailbox.dropped_events.fetch_add(1, Ordering::Relaxed);
                    drop(queue);
                    self.mailbox.notify.notify_one();
                    return Err(Box::new(event));
                }
            } else if event_is_priority(&event) {
                if let Some(index) = queue
                    .iter()
                    .position(|queued| !event_requires_delivery(queued))
                {
                    queue.remove(index);
                    self.mailbox.dropped_events.fetch_add(1, Ordering::Relaxed);
                } else {
                    self.mailbox.dropped_events.fetch_add(1, Ordering::Relaxed);
                    drop(queue);
                    self.mailbox.notify.notify_one();
                    return Ok(());
                }
            } else {
                self.mailbox.dropped_events.fetch_add(1, Ordering::Relaxed);
                drop(queue);
                self.mailbox.notify.notify_one();
                return Ok(());
            }
        }
        queue.push_back(event);
        drop(queue);
        self.mailbox.notify.notify_one();
        Ok(())
    }
}

impl RuntimeEventReceiver {
    pub async fn recv(&mut self) -> Option<RuntimeEvent> {
        loop {
            let notified = self.mailbox.notify.notified();
            if let Some(event) = self.take_overflow_event() {
                return Some(event);
            }
            if let Some(event) = lock(&self.mailbox.queue).pop_front() {
                return Some(event);
            }
            if self.mailbox.senders.load(Ordering::Acquire) == 0 {
                return None;
            }
            notified.await;
        }
    }

    pub fn try_recv(&mut self) -> Result<RuntimeEvent, RuntimeEventTryRecvError> {
        if let Some(event) = lock(&self.mailbox.queue).pop_front() {
            Ok(event)
        } else if let Some(event) = self.take_overflow_event() {
            Ok(event)
        } else if self.mailbox.senders.load(Ordering::Acquire) == 0 {
            Err(RuntimeEventTryRecvError::Disconnected)
        } else {
            Err(RuntimeEventTryRecvError::Empty)
        }
    }

    fn take_overflow_event(&self) -> Option<RuntimeEvent> {
        let dropped_events = self.mailbox.dropped_events.swap(0, Ordering::AcqRel);
        (dropped_events > 0).then_some(RuntimeEvent::MonitorEventsDropped { dropped_events })
    }
}

impl Drop for RuntimeEventReceiver {
    fn drop(&mut self) {
        self.mailbox.receiver_alive.store(false, Ordering::Release);
        self.mailbox.notify.notify_waiters();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeHealth {
    Starting,
    Live,
    Reconnecting,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeStage {
    Connect,
    Queue,
    Load,
    Tokenize,
    Prefill,
    FirstToken,
    Decode,
    Validate,
    Deliver,
    AppAccepted,
    Action,
    Frame,
}

impl RuntimeStage {
    pub const ORDERED: [Self; 12] = [
        Self::Connect,
        Self::Queue,
        Self::Load,
        Self::Tokenize,
        Self::Prefill,
        Self::FirstToken,
        Self::Decode,
        Self::Validate,
        Self::Deliver,
        Self::AppAccepted,
        Self::Action,
        Self::Frame,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Connect => "Connect",
            Self::Queue => "Queue",
            Self::Load => "Load",
            Self::Tokenize => "Tokenize",
            Self::Prefill => "Prefill",
            Self::FirstToken => "First token",
            Self::Decode => "Decode",
            Self::Validate => "Validate",
            Self::Deliver => "Deliver",
            Self::AppAccepted => "App accepted",
            Self::Action => "Action",
            Self::Frame => "Frame",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StageStatus {
    Active,
    Passed,
    Failed,
    Skipped,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeTerminal {
    Delivered,
    ProviderFailed,
    TimedOut,
    DeliveryFailed,
    PreflightFailed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FeedbackEvent {
    OutputAccepted,
    ActionApplied,
    FramePresented,
}

impl FeedbackEvent {
    pub(crate) fn wire_name(self) -> &'static str {
        match self {
            Self::OutputAccepted => "OUTPUT_ACCEPTED",
            Self::ActionApplied => "ACTION_APPLIED",
            Self::FramePresented => "FRAME_PRESENTED",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FeedbackOutcome {
    Pass,
    Fail,
    Unknown,
    NotApplicable,
}

impl FeedbackOutcome {
    pub(crate) fn wire_name(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "fail",
            Self::Unknown => "unknown",
            Self::NotApplicable => "notApplicable",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegisteredAgent {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub capability: String,
    pub model: String,
    pub local_model_loaded: bool,
}

impl RegisteredAgent {
    pub fn from_descriptor(descriptor: &AgentDescriptor) -> Vec<Self> {
        let metadata = &descriptor.metadata;
        let provider = metadata
            .get("providerKey")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(&descriptor.id)
            .to_string();
        let capabilities = metadata
            .get("capabilities")
            .and_then(serde_json::Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .filter(|values| !values.is_empty())
            .unwrap_or_else(|| vec!["unknown".to_string()]);
        let local_model_loaded = metadata
            .get("modelLoaded")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);

        capabilities
            .into_iter()
            .map(|capability| {
                let model = metadata
                    .pointer(&format!("/models/{capability}"))
                    .or_else(|| metadata.get("model"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(&provider)
                    .to_string();
                Self {
                    id: descriptor.id.clone(),
                    name: descriptor.name.clone(),
                    provider: provider.clone(),
                    capability,
                    model,
                    local_model_loaded,
                }
            })
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectProfileRegistration {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub capabilities: Vec<String>,
    pub model: String,
}

#[derive(Clone, Debug)]
pub enum RuntimeEvent {
    HealthChanged {
        health: RuntimeHealth,
        message: Option<String>,
    },
    AgentsRegistered(Vec<RegisteredAgent>),
    ProjectProfilesRegistered(Vec<ProjectProfileRegistration>),
    BackendsChanged(Vec<String>),
    LoadedModelsChanged(usize),
    IdentityChanged {
        project: Option<String>,
        deployment: Option<String>,
    },
    MonitorEventsDropped {
        dropped_events: usize,
    },
    InvocationStarted {
        invocation_id: Uuid,
        agent_id: String,
        agent_name: String,
        source_agent_id: String,
        capability: String,
        provider: String,
        model: String,
        started_unix_ms: u64,
    },
    InvocationMetadata {
        invocation_id: Uuid,
        model_parameters: Value,
    },
    StageChanged {
        invocation_id: Uuid,
        observation_id: Uuid,
        stage: RuntimeStage,
        status: StageStatus,
        start_offset: Duration,
        end_offset: Option<Duration>,
        elapsed: Duration,
        request_elapsed: Option<Duration>,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        resident: Option<bool>,
        error: Option<String>,
    },
    IoCaptured {
        invocation_id: Uuid,
        input: Option<Value>,
        output: Option<Value>,
        truncated: bool,
    },
    IoDropped {
        invocation_id: Uuid,
    },
    ApplicationFeedback {
        invocation_id: Uuid,
        observation_id: Uuid,
        start_offset: Duration,
        end_offset: Duration,
        event: FeedbackEvent,
        outcome: FeedbackOutcome,
        message: Option<String>,
        path: Option<String>,
    },
    InvocationFinished {
        invocation_id: Uuid,
        elapsed: Duration,
        terminal: RuntimeTerminal,
        error: Option<String>,
    },
    InvocationCancelled {
        invocation_id: Uuid,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RuntimeEventKey {
    Global(u8),
    Invocation(Uuid, u8),
    Observation(Uuid, Uuid),
}

fn event_coalescing_key(event: &RuntimeEvent) -> RuntimeEventKey {
    match event {
        RuntimeEvent::HealthChanged { .. } => RuntimeEventKey::Global(0),
        RuntimeEvent::AgentsRegistered(_) => RuntimeEventKey::Global(1),
        RuntimeEvent::BackendsChanged(_) => RuntimeEventKey::Global(2),
        RuntimeEvent::LoadedModelsChanged(_) => RuntimeEventKey::Global(3),
        RuntimeEvent::IdentityChanged { .. } => RuntimeEventKey::Global(4),
        RuntimeEvent::MonitorEventsDropped { .. } => RuntimeEventKey::Global(5),
        RuntimeEvent::ProjectProfilesRegistered(_) => RuntimeEventKey::Global(6),
        RuntimeEvent::InvocationStarted { invocation_id, .. } => {
            RuntimeEventKey::Invocation(*invocation_id, 0)
        }
        RuntimeEvent::InvocationMetadata { invocation_id, .. } => {
            RuntimeEventKey::Invocation(*invocation_id, 1)
        }
        RuntimeEvent::StageChanged {
            invocation_id,
            observation_id,
            ..
        } => RuntimeEventKey::Observation(*invocation_id, *observation_id),
        RuntimeEvent::IoCaptured {
            invocation_id,
            input,
            ..
        } => RuntimeEventKey::Invocation(*invocation_id, if input.is_some() { 2 } else { 3 }),
        RuntimeEvent::IoDropped { invocation_id } => RuntimeEventKey::Invocation(*invocation_id, 4),
        RuntimeEvent::ApplicationFeedback {
            invocation_id,
            observation_id,
            ..
        } => RuntimeEventKey::Observation(*invocation_id, *observation_id),
        RuntimeEvent::InvocationFinished { invocation_id, .. } => {
            RuntimeEventKey::Invocation(*invocation_id, 8)
        }
        RuntimeEvent::InvocationCancelled { invocation_id } => {
            RuntimeEventKey::Invocation(*invocation_id, 9)
        }
    }
}

fn event_is_priority(event: &RuntimeEvent) -> bool {
    !matches!(
        event,
        RuntimeEvent::StageChanged { .. } | RuntimeEvent::IoCaptured { .. }
    )
}

fn event_requires_delivery(event: &RuntimeEvent) -> bool {
    matches!(
        event,
        RuntimeEvent::InvocationStarted { .. }
            | RuntimeEvent::IoCaptured { .. }
            | RuntimeEvent::IoDropped { .. }
            | RuntimeEvent::InvocationFinished { .. }
            | RuntimeEvent::InvocationCancelled { .. }
    )
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn redacted_io_summary(value: &Value) -> (Value, bool) {
    let summary = canonical_trace_io_summary(value);
    (summary.value, summary.truncated)
}

pub fn safe_error_message(error: &str) -> String {
    let lower = error.to_ascii_lowercase();
    if [
        "authorization",
        "bearer ",
        "basic ",
        "api key",
        "api_key",
        "apikey",
        "access token",
        "access_token",
        "secret",
        "token=",
        "token:",
        "password",
        "credential",
        "cookie",
        "session=",
        "session:",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
    {
        return "Provider failed; sensitive details were redacted".to_string();
    }

    error.chars().take(240).collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use vifu_gateway::protocol::AgentDescriptor;

    use super::{
        redacted_io_summary, runtime_event_channel_with_capacity, safe_error_message,
        RegisteredAgent, RuntimeEvent, RuntimeStage, RuntimeTerminal, StageStatus,
    };

    #[test]
    fn saturated_monitor_prioritizes_terminal_state_over_stage_churn() {
        let (sender, mut receiver) = runtime_event_channel_with_capacity(2);
        let terminal_id = uuid::Uuid::new_v4();
        for invocation_id in [terminal_id, uuid::Uuid::new_v4()] {
            sender
                .send(RuntimeEvent::StageChanged {
                    invocation_id,
                    observation_id: uuid::Uuid::new_v4(),
                    stage: RuntimeStage::Decode,
                    status: StageStatus::Active,
                    start_offset: std::time::Duration::ZERO,
                    end_offset: None,
                    elapsed: std::time::Duration::from_millis(1),
                    request_elapsed: None,
                    input_tokens: None,
                    output_tokens: None,
                    resident: None,
                    error: None,
                })
                .unwrap();
        }
        sender
            .send(RuntimeEvent::InvocationFinished {
                invocation_id: terminal_id,
                elapsed: std::time::Duration::from_millis(2),
                terminal: RuntimeTerminal::Delivered,
                error: None,
            })
            .unwrap();

        let drained = [receiver.try_recv().unwrap(), receiver.try_recv().unwrap()];
        assert!(drained.iter().any(|event| matches!(
            event,
            RuntimeEvent::InvocationFinished { invocation_id, .. } if *invocation_id == terminal_id
        )));
    }

    #[test]
    fn saturated_monitor_never_silently_drops_canonical_io() {
        let (sender, mut receiver) = runtime_event_channel_with_capacity(1);
        let invocation_id = uuid::Uuid::new_v4();
        sender
            .send(RuntimeEvent::StageChanged {
                invocation_id,
                observation_id: uuid::Uuid::new_v4(),
                stage: RuntimeStage::Decode,
                status: StageStatus::Active,
                start_offset: std::time::Duration::ZERO,
                end_offset: None,
                elapsed: std::time::Duration::ZERO,
                request_elapsed: None,
                input_tokens: None,
                output_tokens: None,
                resident: None,
                error: None,
            })
            .unwrap();

        sender
            .send(RuntimeEvent::IoCaptured {
                invocation_id,
                input: Some(json!({"message": "hello"})),
                output: None,
                truncated: false,
            })
            .unwrap();

        assert!(matches!(
            receiver.try_recv().unwrap(),
            RuntimeEvent::IoCaptured {
                invocation_id: received,
                ..
            } if received == invocation_id
        ));
    }

    #[test]
    fn saturated_critical_events_return_without_blocking_and_report_the_gap() {
        let (sender, mut receiver) = runtime_event_channel_with_capacity(1);
        let invocation_id = uuid::Uuid::new_v4();
        sender
            .send(RuntimeEvent::InvocationStarted {
                invocation_id,
                agent_id: "agent".to_string(),
                agent_name: "Agent".to_string(),
                source_agent_id: "provider-agent".to_string(),
                capability: "chat".to_string(),
                provider: "local".to_string(),
                model: "qwen".to_string(),
                started_unix_ms: 1,
            })
            .unwrap();
        let rejected = sender.send(RuntimeEvent::InvocationFinished {
            invocation_id,
            elapsed: std::time::Duration::from_millis(1),
            terminal: RuntimeTerminal::Delivered,
            error: None,
        });

        assert!(rejected.is_err());
        assert!(matches!(
            receiver.try_recv().unwrap(),
            RuntimeEvent::InvocationStarted { .. }
        ));
        assert!(matches!(
            receiver.try_recv().unwrap(),
            RuntimeEvent::MonitorEventsDropped { dropped_events: 1 }
        ));
    }

    #[test]
    fn descriptor_should_expand_into_one_lane_per_capability() {
        let descriptor = AgentDescriptor {
            id: "local-qwen".to_string(),
            name: "Qwen".to_string(),
            metadata: json!({
                "providerKey": "llama-local",
                "localProviderType": "llama",
                "modelLoaded": true,
                "capabilities": ["chat", "embedding"],
                "models": {"chat": "qwen-chat", "embedding": "qwen-embed"}
            }),
        };

        let registrations = RegisteredAgent::from_descriptor(&descriptor);

        assert_eq!(registrations.len(), 2);
        assert_eq!(registrations[0].model, "qwen-chat");
        assert!(registrations[0].local_model_loaded);
    }

    #[test]
    fn sensitive_provider_errors_should_be_redacted() {
        let message = safe_error_message("Authorization: Bearer private-token");

        assert!(!message.contains("private-token"));
    }

    #[test]
    fn provider_secret_markers_should_be_redacted() {
        for error in [
            "Basic cHJpdmF0ZS11c2VyOnByaXZhdGUtcGFzcw==",
            "token=private-token",
            "password: hunter2",
            "credential rejected",
            "cookie=session-secret",
            "session=private-session",
        ] {
            assert_eq!(
                safe_error_message(error),
                "Provider failed; sensitive details were redacted"
            );
        }
    }

    #[test]
    fn io_summary_omits_media_binary_and_data_uris_without_a_prefix() {
        let encoded = "A".repeat(512);
        let (summary, truncated) = redacted_io_summary(&json!({
            "image": encoded,
            "audioData": "data:audio/wav;base64,private-audio",
            "blob": {"_vifuBinary": true, "data": "private-binary"}
        }));

        assert!(truncated);
        let serialized = summary.to_string();
        assert!(serialized.contains("media/binary omitted"));
        assert!(serialized.contains("_vifuBinary object omitted"));
        assert!(!serialized.contains("private-audio"));
        assert!(!serialized.contains("private-binary"));
        assert!(!serialized.contains(&"A".repeat(64)));
    }

    #[test]
    fn io_summary_redacts_nested_sensitive_keys_and_credential_like_strings() {
        let (summary, _) = redacted_io_summary(&json!({
            "nested": {
                "apiKey": "sk-private",
                "message": "request failed with Authorization: Bearer private-token"
            },
            "items": [{"note": "password=hunter2"}]
        }));

        let serialized = summary.to_string();
        assert!(!serialized.contains("sk-private"));
        assert!(!serialized.contains("private-token"));
        assert!(!serialized.contains("hunter2"));
        assert!(serialized.contains("REDACTED"));
    }
}
