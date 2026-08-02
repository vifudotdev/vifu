use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use serde_json::Value;
use uuid::Uuid;
use vifu_gateway::optimization::RouteCombination;

use crate::monitor::{
    safe_error_message, FeedbackEvent, FeedbackOutcome, RegisteredAgent, RuntimeEvent,
    RuntimeHealth, RuntimeStage, RuntimeTerminal, StageStatus,
};

const TRACE_HISTORY_LIMIT: usize = 8;
const GLOBAL_TRACE_HISTORY_LIMIT: usize = 512;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum LaneFilter {
    #[default]
    All,
    Running,
    Problems,
    Passed,
}

impl LaneFilter {
    pub(crate) fn next(self) -> Self {
        match self {
            Self::All => Self::Running,
            Self::Running => Self::Problems,
            Self::Problems => Self::Passed,
            Self::Passed => Self::All,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::All => "ALL",
            Self::Running => "RUNNING",
            Self::Problems => "PROBLEMS",
            Self::Passed => "PASSED",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum LaneSort {
    #[default]
    Attention,
    Recent,
    Agent,
}

impl LaneSort {
    pub(crate) fn next(self) -> Self {
        match self {
            Self::Attention => Self::Recent,
            Self::Recent => Self::Agent,
            Self::Agent => Self::Attention,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Attention => "ATTENTION",
            Self::Recent => "RECENT",
            Self::Agent => "AGENT",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum LaneOutcome {
    Running,
    Passed,
    Failed,
    Timeout,
    Unknown,
    Skipped,
}

impl LaneOutcome {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Running => "RUNNING",
            Self::Passed => "PASS",
            Self::Failed => "FAILED",
            Self::Timeout => "TIMEOUT",
            Self::Unknown => "UNKNOWN",
            Self::Skipped => "SKIPPED",
        }
    }

    pub(crate) fn symbol(self) -> &'static str {
        match self {
            Self::Running => "●",
            Self::Passed => "✓",
            Self::Failed | Self::Timeout => "✕",
            Self::Unknown => "?",
            Self::Skipped => "–",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum TraceTab {
    #[default]
    Summary,
    Io,
    Metadata,
    Scores,
    Events,
}

impl TraceTab {
    pub(crate) fn next(self) -> Self {
        match self {
            Self::Summary => Self::Io,
            Self::Io => Self::Metadata,
            Self::Metadata => Self::Scores,
            Self::Scores => Self::Events,
            Self::Events => Self::Summary,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Summary => "Summary",
            Self::Io => "I/O",
            Self::Metadata => "Metadata",
            Self::Scores => "Scores",
            Self::Events => "Events",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MetricSummary {
    pub(crate) median: Duration,
    pub(crate) min: Duration,
    pub(crate) max: Duration,
    pub(crate) samples: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ComparisonRow {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) plan: RouteCombination,
    pub(crate) first_total: Option<Duration>,
    pub(crate) first_run_cold: Option<bool>,
    pub(crate) repeat_runs_resident: Option<bool>,
    pub(crate) total: Option<Duration>,
    pub(crate) total_range: Option<MetricSummary>,
    pub(crate) ttft: Option<Duration>,
    pub(crate) ttft_range: Option<MetricSummary>,
    pub(crate) tokens_per_second: Option<f64>,
    pub(crate) first_process_cpu_percent: Option<f64>,
    pub(crate) process_cpu_percent: Option<f64>,
    pub(crate) peak_rss_bytes: Option<u64>,
    pub(crate) route_labels: BTreeMap<String, String>,
    pub(crate) result: LaneOutcome,
    pub(crate) detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OptimizationExclusion {
    pub(crate) route: String,
    pub(crate) capability: String,
    pub(crate) provider: String,
    pub(crate) reason: String,
    pub(crate) message: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OptimizationSummary {
    pub(crate) comparison_id: Uuid,
    pub(crate) started_at_ms: u64,
    pub(crate) completed_at_ms: u64,
    pub(crate) monotonic_duration_ms: u64,
    pub(crate) corpus_agents: usize,
    pub(crate) configured_local_models: usize,
    pub(crate) tested_models: usize,
    pub(crate) passed_models: usize,
    pub(crate) expected_pairs: usize,
    pub(crate) evaluated_pairs: usize,
    pub(crate) passed_pairs: usize,
    pub(crate) recommendation: Option<String>,
    pub(crate) not_exhaustive: bool,
    pub(crate) sequential_replay: bool,
    pub(crate) device_architecture: String,
    pub(crate) device_backend: Option<String>,
    pub(crate) remote_fallbacks: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum View {
    Main,
    Traces {
        agent_key: String,
    },
    Trace {
        agent_key: String,
        trace_id: Uuid,
        tab: TraceTab,
        timeline: bool,
        observation_cursor: Option<Uuid>,
        selected_observation: Option<Uuid>,
    },
    Optimize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ObservationType {
    Generation,
    Span,
    Event,
}

impl ObservationType {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Generation => "generation",
            Self::Span => "span",
            Self::Event => "event",
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct TraceObservation {
    pub(crate) id: Uuid,
    pub(crate) parent_observation_id: Option<Uuid>,
    pub(crate) observation_type: ObservationType,
    pub(crate) name: String,
    pub(crate) stage: Option<RuntimeStage>,
    pub(crate) status: StageStatus,
    pub(crate) start_offset: Option<Duration>,
    pub(crate) end_offset: Option<Duration>,
    pub(crate) elapsed: Duration,
    pub(crate) request_elapsed: Option<Duration>,
    pub(crate) input_tokens: Option<u64>,
    pub(crate) output_tokens: Option<u64>,
    pub(crate) resident: Option<bool>,
    pub(crate) provider: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) model_parameters: Option<Value>,
    pub(crate) capability: Option<String>,
    pub(crate) input: Option<Value>,
    pub(crate) output: Option<Value>,
    pub(crate) attributes: Value,
    pub(crate) error: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct FeedbackRecord {
    pub(crate) observation_id: Uuid,
    pub(crate) event: FeedbackEvent,
    pub(crate) outcome: FeedbackOutcome,
    pub(crate) message: Option<String>,
    pub(crate) path: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct TraceRecord {
    pub(crate) id: Uuid,
    pub(crate) agent_id: String,
    pub(crate) source_agent_id: String,
    pub(crate) capability: String,
    pub(crate) provider: String,
    pub(crate) model: String,
    pub(crate) model_parameters: Option<Value>,
    pub(crate) started_unix_ms: u64,
    started: Instant,
    elapsed: Option<Duration>,
    pub(crate) outcome: LaneOutcome,
    pub(crate) terminal: Option<RuntimeTerminal>,
    pub(crate) error: Option<String>,
    pub(crate) observations: Vec<TraceObservation>,
    pub(crate) ttft: Option<Duration>,
    pub(crate) tokens_per_second: Option<f64>,
    pub(crate) input: Option<Value>,
    pub(crate) output: Option<Value>,
    pub(crate) io_truncated: bool,
    pub(crate) io_dropped: bool,
    pub(crate) feedback: Vec<FeedbackRecord>,
}

impl TraceRecord {
    pub(crate) fn elapsed(&self, now: Instant) -> Duration {
        self.elapsed
            .unwrap_or_else(|| now.duration_since(self.started))
    }

    pub(crate) fn current_stage(&self) -> RuntimeStage {
        RuntimeStage::ORDERED
            .iter()
            .copied()
            .find(|stage| self.observation_status(*stage) == StageStatus::Failed)
            .or_else(|| {
                RuntimeStage::ORDERED
                    .iter()
                    .rev()
                    .copied()
                    .find(|stage| self.observation_status(*stage) == StageStatus::Active)
            })
            .or_else(|| {
                RuntimeStage::ORDERED
                    .iter()
                    .rev()
                    .copied()
                    .find(|stage| self.observation_status(*stage) == StageStatus::Passed)
            })
            .or_else(|| {
                RuntimeStage::ORDERED
                    .iter()
                    .rev()
                    .copied()
                    .find(|stage| self.observation_for_stage(*stage).is_some())
            })
            .unwrap_or(RuntimeStage::Connect)
    }

    pub(crate) fn observation_status(&self, stage: RuntimeStage) -> StageStatus {
        self.observation_for_stage(stage)
            .map(|observation| observation.status)
            .unwrap_or(StageStatus::Unknown)
    }

    pub(crate) fn observation(&self, observation_id: Uuid) -> Option<&TraceObservation> {
        self.observations
            .iter()
            .find(|observation| observation.id == observation_id)
    }

    pub(crate) fn observation_for_stage(&self, stage: RuntimeStage) -> Option<&TraceObservation> {
        self.observations
            .iter()
            .rev()
            .find(|observation| observation.stage == Some(stage))
    }

    fn upsert_observation(&mut self, observation: TraceObservation) {
        if let Some(existing) = self
            .observations
            .iter_mut()
            .find(|existing| existing.id == observation.id)
        {
            *existing = observation;
        } else {
            self.observations.push(observation);
        }
    }

    pub(crate) fn matches_search(&self, query: &str) -> bool {
        !self.matching_observation_choices(query).is_empty()
    }

    pub(crate) fn root_matches_search(&self, query: &str) -> bool {
        let query = query.trim().to_ascii_lowercase();
        query.is_empty() || self.root_matches_normalized_search(&query)
    }

    pub(crate) fn observation_matches_search(
        &self,
        observation: &TraceObservation,
        query: &str,
    ) -> bool {
        let query = query.trim().to_ascii_lowercase();
        query.is_empty() || self.observation_matches_normalized_search(observation, &query)
    }

    pub(crate) fn matching_observation_choices(&self, query: &str) -> Vec<Option<Uuid>> {
        let query = query.trim().to_ascii_lowercase();
        let mut matches = Vec::with_capacity(self.observations.len().saturating_add(1));
        if query.is_empty() || self.root_matches_normalized_search(&query) {
            matches.push(None);
        }
        matches.extend(
            self.observations
                .iter()
                .filter(|observation| {
                    query.is_empty()
                        || self.observation_matches_normalized_search(observation, &query)
                })
                .map(|observation| Some(observation.id)),
        );
        matches
    }

    fn root_matches_normalized_search(&self, query: &str) -> bool {
        [
            self.agent_id.as_str(),
            self.source_agent_id.as_str(),
            self.capability.as_str(),
            self.provider.as_str(),
            self.model.as_str(),
            self.error.as_deref().unwrap_or_default(),
        ]
        .iter()
        .any(|value| value.to_ascii_lowercase().contains(query))
            || optional_json_matches(self.input.as_ref(), query)
            || optional_json_matches(self.output.as_ref(), query)
    }

    fn observation_matches_normalized_search(
        &self,
        observation: &TraceObservation,
        query: &str,
    ) -> bool {
        [
            observation.name.as_str(),
            observation.provider.as_deref().unwrap_or_default(),
            observation.model.as_deref().unwrap_or_default(),
            observation.capability.as_deref().unwrap_or_default(),
            observation.error.as_deref().unwrap_or_default(),
        ]
        .iter()
        .any(|value| value.to_ascii_lowercase().contains(query))
            || optional_json_matches(observation.input.as_ref(), query)
            || optional_json_matches(observation.output.as_ref(), query)
            || self.feedback.iter().any(|feedback| {
                feedback.observation_id == observation.id
                    && [
                        feedback.message.as_deref().unwrap_or_default(),
                        feedback.path.as_deref().unwrap_or_default(),
                    ]
                    .iter()
                    .any(|value| value.to_ascii_lowercase().contains(query))
            })
    }
}

fn optional_json_matches(value: Option<&Value>, query: &str) -> bool {
    value.is_some_and(|value| value.to_string().to_ascii_lowercase().contains(query))
}

#[derive(Clone, Debug)]
pub(crate) struct AgentLane {
    pub(crate) key: String,
    pub(crate) agent_id: String,
    source_agent_id: String,
    pub(crate) name: String,
    pub(crate) provider: String,
    pub(crate) capability: String,
    pub(crate) model: String,
    configured: bool,
    active: HashMap<Uuid, TraceRecord>,
    history: VecDeque<TraceRecord>,
    last_updated_unix_ms: u64,
}

impl AgentLane {
    fn from_registration(registration: RegisteredAgent) -> Self {
        let key = lane_key(&registration.id, &registration.capability);
        let source_agent_id = registration.id.clone();
        Self {
            key,
            agent_id: registration.id,
            source_agent_id,
            name: registration.name,
            provider: registration.provider,
            capability: registration.capability,
            model: registration.model,
            configured: true,
            active: HashMap::new(),
            history: VecDeque::new(),
            last_updated_unix_ms: 0,
        }
    }

    pub(crate) fn concurrency(&self) -> usize {
        self.active.len()
    }

    pub(crate) fn representative(&self) -> Option<&TraceRecord> {
        self.active
            .values()
            .min_by_key(|trace| trace.started)
            .or_else(|| self.history.front())
    }

    pub(crate) fn outcome(&self) -> LaneOutcome {
        self.representative()
            .map(|trace| trace.outcome)
            .unwrap_or(LaneOutcome::Unknown)
    }

    pub(crate) fn recent_traces(&self) -> Vec<&TraceRecord> {
        let mut traces = self.active.values().collect::<Vec<_>>();
        traces.sort_by_key(|trace| std::cmp::Reverse(trace.started_unix_ms));
        traces.extend(self.history.iter());
        traces
    }

    fn trace(&self, trace_id: Uuid) -> Option<&TraceRecord> {
        self.active
            .get(&trace_id)
            .or_else(|| self.history.iter().find(|trace| trace.id == trace_id))
    }

    fn trace_mut(&mut self, trace_id: Uuid) -> Option<&mut TraceRecord> {
        self.active
            .get_mut(&trace_id)
            .or_else(|| self.history.iter_mut().find(|trace| trace.id == trace_id))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct SystemMetrics {
    pub(crate) cpu_percent: Option<f64>,
    pub(crate) rss_bytes: Option<u64>,
    pub(crate) total_memory_bytes: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct LaneCounts {
    pub(crate) total: usize,
    pub(crate) running: usize,
    pub(crate) passed: usize,
    pub(crate) failed: usize,
    pub(crate) timeout: usize,
    pub(crate) unknown: usize,
}

pub(crate) struct App {
    pub(crate) view: View,
    pub(crate) health: RuntimeHealth,
    pub(crate) health_message: Option<String>,
    pub(crate) project: Option<String>,
    pub(crate) deployment: Option<String>,
    pub(crate) loaded_models: usize,
    pub(crate) runtime_backends: Vec<String>,
    pub(crate) metrics: SystemMetrics,
    pub(crate) filter: LaneFilter,
    pub(crate) sort: LaneSort,
    pub(crate) search: String,
    pub(crate) search_active: bool,
    pub(crate) selected_lane: Option<String>,
    pub(crate) selected_trace: Option<Uuid>,
    pub(crate) scroll_offset: usize,
    pub(crate) trace_scroll_offset: usize,
    pub(crate) lanes: HashMap<String, AgentLane>,
    configured_sources: HashSet<String>,
    invocation_lanes: HashMap<Uuid, String>,
    pub(crate) comparison_rows: Vec<ComparisonRow>,
    pub(crate) selected_comparison: usize,
    pub(crate) optimization_summary: Option<OptimizationSummary>,
    pub(crate) optimization_exclusions: Vec<OptimizationExclusion>,
    pub(crate) optimization_excluded_total: usize,
    pub(crate) selected_exclusion: usize,
    pub(crate) optimization_running: bool,
    pub(crate) inventory_generation: u64,
    pub(crate) override_active: bool,
    pub(crate) override_generation: Option<u64>,
    pub(crate) override_route_count: usize,
    pub(crate) quit_confirmation: bool,
    pub(crate) notice: Option<String>,
}

impl Default for App {
    fn default() -> Self {
        Self {
            view: View::Main,
            health: RuntimeHealth::Starting,
            health_message: None,
            project: None,
            deployment: None,
            loaded_models: 0,
            runtime_backends: Vec::new(),
            metrics: SystemMetrics::default(),
            filter: LaneFilter::All,
            sort: LaneSort::Attention,
            search: String::new(),
            search_active: false,
            selected_lane: None,
            selected_trace: None,
            scroll_offset: 0,
            trace_scroll_offset: 0,
            lanes: HashMap::new(),
            configured_sources: HashSet::new(),
            invocation_lanes: HashMap::new(),
            comparison_rows: Vec::new(),
            selected_comparison: 0,
            optimization_summary: None,
            optimization_exclusions: Vec::new(),
            optimization_excluded_total: 0,
            selected_exclusion: 0,
            optimization_running: false,
            inventory_generation: 0,
            override_active: false,
            override_generation: None,
            override_route_count: 0,
            quit_confirmation: false,
            notice: None,
        }
    }
}

impl App {
    pub(crate) fn apply(&mut self, event: RuntimeEvent, now: Instant) {
        match event {
            RuntimeEvent::HealthChanged { health, message } => {
                self.health = health;
                self.health_message = message;
            }
            RuntimeEvent::AgentsRegistered(registrations) => {
                self.inventory_generation = self.inventory_generation.saturating_add(1);
                self.comparison_rows.clear();
                self.selected_comparison = 0;
                self.optimization_summary = None;
                self.optimization_exclusions.clear();
                self.optimization_excluded_total = 0;
                self.selected_exclusion = 0;
                self.loaded_models = registrations
                    .iter()
                    .filter(|registration| registration.local_model_loaded)
                    .map(|registration| registration.provider.as_str())
                    .collect::<std::collections::HashSet<_>>()
                    .len();
                self.configured_sources = registrations
                    .iter()
                    .map(|registration| lane_key(&registration.id, &registration.capability))
                    .collect();
                for lane in self.lanes.values_mut() {
                    lane.configured = self
                        .configured_sources
                        .contains(&lane_key(&lane.source_agent_id, &lane.capability));
                }
                for registration in registrations {
                    let key = lane_key(&registration.id, &registration.capability);
                    let profile_key = self
                        .lanes
                        .iter()
                        .filter(|(lane_key, lane)| {
                            *lane_key != &key
                                && lane.source_agent_id == registration.id
                                && lane.capability == registration.capability
                        })
                        .max_by_key(|(_, lane)| lane.last_updated_unix_ms)
                        .map(|(lane_key, _)| lane_key.clone());
                    if let Some(profile_key) = profile_key {
                        if self
                            .lanes
                            .get(&key)
                            .is_some_and(|lane| lane.representative().is_none())
                        {
                            self.lanes.remove(&key);
                            self.replace_lane_key(&key, &profile_key);
                        }
                        continue;
                    }
                    match self.lanes.get_mut(&key) {
                        Some(lane) => {
                            lane.name = registration.name;
                            lane.provider = registration.provider;
                            lane.model = registration.model;
                            lane.source_agent_id = registration.id;
                            lane.configured = true;
                        }
                        None => {
                            self.lanes
                                .insert(key, AgentLane::from_registration(registration));
                        }
                    }
                }
                let stale_keys = self
                    .lanes
                    .iter()
                    .filter(|(_, lane)| !lane.configured && lane.active.is_empty())
                    .map(|(key, _)| key.clone())
                    .collect::<Vec<_>>();
                for key in stale_keys {
                    if let Some(lane) = self.lanes.remove(&key) {
                        for trace in lane.history {
                            self.invocation_lanes.remove(&trace.id);
                        }
                    }
                }
                self.normalize_lane_selection(now);
            }
            RuntimeEvent::BackendsChanged(backends) => {
                self.runtime_backends = backends;
            }
            RuntimeEvent::LoadedModelsChanged(count) => {
                self.loaded_models = count;
            }
            RuntimeEvent::IdentityChanged {
                project,
                deployment,
            } => {
                self.project = project;
                self.deployment = deployment;
            }
            RuntimeEvent::MonitorEventsDropped { dropped_events } => {
                self.notice = Some(format!(
                    "Live TUI buffer dropped {dropped_events} event(s); persistent Dashboard traces remain authoritative"
                ));
            }
            RuntimeEvent::InvocationStarted {
                invocation_id,
                agent_id,
                agent_name,
                source_agent_id,
                capability,
                provider,
                model,
                started_unix_ms,
            } => {
                let inventory_key = lane_key(&source_agent_id, &capability);
                let key = lane_key(&agent_id, &capability);
                let inventory_configured = self.configured_sources.contains(&inventory_key);
                if inventory_key != key
                    && self
                        .lanes
                        .get(&inventory_key)
                        .is_some_and(|lane| lane.representative().is_none())
                {
                    self.lanes.remove(&inventory_key);
                    self.replace_lane_key(&inventory_key, &key);
                }
                let lane = self.lanes.entry(key.clone()).or_insert_with(|| {
                    let mut lane = AgentLane::from_registration(RegisteredAgent {
                        id: agent_id.clone(),
                        name: agent_name.clone(),
                        provider: provider.clone(),
                        capability: capability.clone(),
                        model: model.clone(),
                        local_model_loaded: false,
                    });
                    lane.source_agent_id = source_agent_id.clone();
                    lane.configured = inventory_configured;
                    lane
                });
                lane.source_agent_id = source_agent_id.clone();
                lane.name = agent_name;
                lane.provider = provider.clone();
                lane.model = model.clone();
                lane.last_updated_unix_ms = started_unix_ms;
                lane.active.insert(
                    invocation_id,
                    TraceRecord {
                        id: invocation_id,
                        agent_id,
                        source_agent_id,
                        capability,
                        provider,
                        model,
                        model_parameters: None,
                        started_unix_ms,
                        started: now,
                        elapsed: None,
                        outcome: LaneOutcome::Running,
                        terminal: None,
                        error: None,
                        observations: Vec::new(),
                        ttft: None,
                        tokens_per_second: None,
                        input: None,
                        output: None,
                        io_truncated: false,
                        io_dropped: false,
                        feedback: Vec::new(),
                    },
                );
                self.invocation_lanes.insert(invocation_id, key);
                self.normalize_lane_selection(now);
            }
            RuntimeEvent::InvocationMetadata {
                invocation_id,
                model_parameters,
            } => {
                let Some(lane_key) = self.invocation_lanes.get(&invocation_id) else {
                    return;
                };
                if let Some(trace) = self
                    .lanes
                    .get_mut(lane_key)
                    .and_then(|lane| lane.trace_mut(invocation_id))
                {
                    trace.model_parameters = Some(model_parameters);
                }
            }
            RuntimeEvent::StageChanged {
                invocation_id,
                observation_id,
                stage,
                status,
                start_offset,
                end_offset,
                elapsed,
                request_elapsed,
                input_tokens,
                output_tokens,
                resident,
                error,
            } => {
                let Some(lane_key) = self.invocation_lanes.get(&invocation_id) else {
                    return;
                };
                let Some(trace) = self
                    .lanes
                    .get_mut(lane_key)
                    .and_then(|lane| lane.trace_mut(invocation_id))
                else {
                    return;
                };
                if stage == RuntimeStage::FirstToken && status == StageStatus::Passed {
                    trace.ttft = request_elapsed;
                }
                if stage == RuntimeStage::Decode && status == StageStatus::Passed {
                    trace.tokens_per_second = output_tokens.and_then(|tokens| {
                        (elapsed > Duration::ZERO).then_some(tokens as f64 / elapsed.as_secs_f64())
                    });
                }
                let provider = trace.provider.clone();
                let model = trace.model.clone();
                let model_parameters = trace.model_parameters.clone();
                let capability = trace.capability.clone();
                trace.upsert_observation(TraceObservation {
                    id: observation_id,
                    parent_observation_id: Some(invocation_id),
                    observation_type: ObservationType::Span,
                    name: stage.label().to_string(),
                    stage: Some(stage),
                    status,
                    start_offset: Some(start_offset),
                    end_offset,
                    elapsed,
                    request_elapsed,
                    input_tokens,
                    output_tokens,
                    resident,
                    provider: Some(provider),
                    model: Some(model),
                    model_parameters,
                    capability: Some(capability),
                    input: None,
                    output: None,
                    attributes: serde_json::json!({
                        "source": "agentGateway",
                        "stage": stage.label(),
                        "startOffsetMs": start_offset.as_millis(),
                        "endOffsetMs": end_offset.map(|value| value.as_millis()),
                        "requestElapsedMs": request_elapsed.map(|value| value.as_millis()),
                        "inputTokens": input_tokens,
                        "outputTokens": output_tokens,
                        "resident": resident,
                    }),
                    error: error.as_deref().map(safe_error_message),
                });
            }
            RuntimeEvent::IoCaptured {
                invocation_id,
                input,
                output,
                truncated,
            } => {
                let Some(lane_key) = self.invocation_lanes.get(&invocation_id) else {
                    return;
                };
                let Some(trace) = self
                    .lanes
                    .get_mut(lane_key)
                    .and_then(|lane| lane.trace_mut(invocation_id))
                else {
                    return;
                };
                if input.is_some() {
                    trace.input = input;
                }
                if output.is_some() {
                    trace.output = output;
                }
                trace.io_truncated |= truncated;
            }
            RuntimeEvent::IoDropped { invocation_id } => {
                let Some(lane_key) = self.invocation_lanes.get(&invocation_id) else {
                    return;
                };
                if let Some(trace) = self
                    .lanes
                    .get_mut(lane_key)
                    .and_then(|lane| lane.trace_mut(invocation_id))
                {
                    trace.io_dropped = true;
                }
            }
            RuntimeEvent::ApplicationFeedback {
                invocation_id,
                observation_id,
                start_offset,
                end_offset,
                event,
                outcome,
                message,
                path,
            } => {
                let Some(lane_key) = self.invocation_lanes.get(&invocation_id) else {
                    return;
                };
                let Some(trace) = self
                    .lanes
                    .get_mut(lane_key)
                    .and_then(|lane| lane.trace_mut(invocation_id))
                else {
                    return;
                };
                let stage = match event {
                    FeedbackEvent::OutputAccepted => RuntimeStage::AppAccepted,
                    FeedbackEvent::ActionApplied => RuntimeStage::Action,
                    FeedbackEvent::FramePresented => RuntimeStage::Frame,
                };
                let status = match outcome {
                    FeedbackOutcome::Pass => StageStatus::Passed,
                    FeedbackOutcome::Fail => StageStatus::Failed,
                    FeedbackOutcome::Unknown => StageStatus::Unknown,
                    FeedbackOutcome::NotApplicable => StageStatus::Skipped,
                };
                let message = message.as_deref().map(safe_error_message);
                let provider = trace.provider.clone();
                let model = trace.model.clone();
                let model_parameters = trace.model_parameters.clone();
                let capability = trace.capability.clone();
                let feedback_output = serde_json::json!({
                    "event": event.wire_name(),
                    "outcome": outcome.wire_name(),
                    "message": message.clone(),
                    "path": path.clone(),
                });
                trace.upsert_observation(TraceObservation {
                    id: observation_id,
                    parent_observation_id: Some(invocation_id),
                    observation_type: ObservationType::Event,
                    name: event.wire_name().to_string(),
                    stage: Some(stage),
                    status,
                    start_offset: Some(start_offset),
                    end_offset: Some(end_offset),
                    elapsed: end_offset.saturating_sub(start_offset),
                    request_elapsed: Some(end_offset),
                    input_tokens: None,
                    output_tokens: None,
                    resident: None,
                    provider: Some(provider),
                    model: Some(model),
                    model_parameters,
                    capability: Some(capability),
                    input: None,
                    output: Some(feedback_output),
                    attributes: serde_json::json!({
                        "source": "application",
                        "path": path.clone(),
                        "startOffsetMs": start_offset.as_millis(),
                        "endOffsetMs": end_offset.as_millis(),
                    }),
                    error: message.clone(),
                });
                if outcome == FeedbackOutcome::Fail {
                    trace.outcome = LaneOutcome::Failed;
                    if trace.error.is_none() {
                        trace.error = Some(
                            message
                                .clone()
                                .unwrap_or_else(|| format!("{} failed", stage.label())),
                        );
                    }
                }
                trace.feedback.push(FeedbackRecord {
                    observation_id,
                    event,
                    outcome,
                    message,
                    path,
                });
            }
            RuntimeEvent::InvocationFinished {
                invocation_id,
                elapsed,
                error,
                terminal,
            } => {
                let Some(lane_key) = self.invocation_lanes.get(&invocation_id).cloned() else {
                    return;
                };
                let Some(lane) = self.lanes.get_mut(&lane_key) else {
                    return;
                };
                let Some(mut trace) = lane.active.remove(&invocation_id) else {
                    return;
                };
                trace.elapsed = Some(elapsed);
                trace.terminal = Some(terminal);
                if terminal == RuntimeTerminal::TimedOut {
                    let timed_out_stage = trace
                        .observations
                        .iter()
                        .filter(|observation| observation.status == StageStatus::Active)
                        .max_by_key(|observation| {
                            (
                                observation.start_offset.unwrap_or(Duration::ZERO),
                                RuntimeStage::ORDERED
                                    .iter()
                                    .position(|stage| Some(*stage) == observation.stage)
                                    .unwrap_or(0),
                            )
                        })
                        .map(|observation| observation.id);
                    if let Some(observation_id) = timed_out_stage {
                        if let Some(observation) = trace
                            .observations
                            .iter_mut()
                            .find(|observation| observation.id == observation_id)
                        {
                            observation.status = StageStatus::Failed;
                            observation.end_offset = Some(elapsed);
                            observation.elapsed = observation
                                .start_offset
                                .map_or(observation.elapsed, |start| elapsed.saturating_sub(start));
                            observation.request_elapsed = Some(elapsed);
                            observation
                                .error
                                .get_or_insert_with(|| "request timed out".to_string());
                        }
                    }
                }
                let application_failed = terminal == RuntimeTerminal::Delivered
                    && trace
                        .feedback
                        .iter()
                        .any(|feedback| feedback.outcome == FeedbackOutcome::Fail);
                trace.outcome = match terminal {
                    RuntimeTerminal::Delivered if application_failed => LaneOutcome::Failed,
                    RuntimeTerminal::Delivered => LaneOutcome::Passed,
                    RuntimeTerminal::TimedOut => LaneOutcome::Timeout,
                    RuntimeTerminal::ProviderFailed
                    | RuntimeTerminal::DeliveryFailed
                    | RuntimeTerminal::PreflightFailed => LaneOutcome::Failed,
                };
                let terminal_error = error.as_deref().map(safe_error_message).or_else(|| {
                    (terminal == RuntimeTerminal::TimedOut).then(|| "request timed out".to_string())
                });
                if terminal_error.is_some() || !application_failed {
                    trace.error = terminal_error;
                }
                lane.last_updated_unix_ms = trace
                    .started_unix_ms
                    .saturating_add(elapsed.as_millis().try_into().unwrap_or(u64::MAX));
                lane.history.push_front(trace);
                let dropped = (lane.history.len() > TRACE_HISTORY_LIMIT)
                    .then(|| lane.history.pop_back())
                    .flatten()
                    .map(|trace| trace.id);
                if let Some(dropped) = dropped {
                    self.invocation_lanes.remove(&dropped);
                }
                self.prune_trace_history();
            }
            RuntimeEvent::InvocationCancelled { invocation_id } => {
                let Some(lane_key) = self.invocation_lanes.remove(&invocation_id) else {
                    return;
                };
                let Some(lane) = self.lanes.get_mut(&lane_key) else {
                    return;
                };
                let Some(mut trace) = lane.active.remove(&invocation_id) else {
                    return;
                };
                let elapsed = now.duration_since(trace.started);
                trace.elapsed = Some(elapsed);
                trace.outcome = LaneOutcome::Skipped;
                trace.error = Some("Invocation cancelled".to_string());
                lane.history.push_front(trace);
                lane.history.truncate(TRACE_HISTORY_LIMIT);
                self.prune_trace_history();
            }
        }
    }

    pub(crate) fn counts(&self) -> LaneCounts {
        let mut counts = LaneCounts {
            total: self.lanes.len(),
            ..LaneCounts::default()
        };
        for lane in self.lanes.values() {
            match lane.outcome() {
                LaneOutcome::Running => counts.running += 1,
                LaneOutcome::Passed => counts.passed += 1,
                LaneOutcome::Failed => counts.failed += 1,
                LaneOutcome::Timeout => counts.timeout += 1,
                LaneOutcome::Unknown | LaneOutcome::Skipped => counts.unknown += 1,
            }
        }
        counts
    }

    pub(crate) fn visible_lane_keys(&self, now: Instant) -> Vec<String> {
        let search = self.search.to_ascii_lowercase();
        let mut lanes = self
            .lanes
            .values()
            .filter(|lane| self.matches_filter(lane))
            .filter(|lane| {
                search.is_empty()
                    || lane.name.to_ascii_lowercase().contains(&search)
                    || lane.agent_id.to_ascii_lowercase().contains(&search)
                    || lane.model.to_ascii_lowercase().contains(&search)
                    || lane.provider.to_ascii_lowercase().contains(&search)
            })
            .collect::<Vec<_>>();
        match self.sort {
            LaneSort::Attention => lanes.sort_by(|left, right| attention_cmp(left, right, now)),
            LaneSort::Recent => lanes.sort_by(|left, right| {
                right
                    .last_updated_unix_ms
                    .cmp(&left.last_updated_unix_ms)
                    .then_with(|| agent_cmp(left, right))
            }),
            LaneSort::Agent => lanes.sort_by(|left, right| agent_cmp(left, right)),
        }
        lanes.into_iter().map(|lane| lane.key.clone()).collect()
    }

    pub(crate) fn lane(&self, key: &str) -> Option<&AgentLane> {
        self.lanes.get(key)
    }

    pub(crate) fn selected_lane(&self) -> Option<&AgentLane> {
        self.selected_lane
            .as_deref()
            .and_then(|key| self.lanes.get(key))
    }

    pub(crate) fn trace(&self, agent_key: &str, trace_id: Uuid) -> Option<&TraceRecord> {
        self.lanes.get(agent_key)?.trace(trace_id)
    }

    pub(crate) fn selected_observation_id(&self) -> Option<Uuid> {
        let View::Trace {
            selected_observation: Some(observation_id),
            ..
        } = &self.view
        else {
            return None;
        };
        Some(*observation_id)
    }

    pub(crate) fn active_invocations(&self) -> usize {
        self.lanes.values().map(AgentLane::concurrency).sum()
    }

    fn prune_trace_history(&mut self) {
        while self
            .lanes
            .values()
            .map(|lane| lane.history.len())
            .sum::<usize>()
            > GLOBAL_TRACE_HISTORY_LIMIT
        {
            let Some(oldest_lane) = self
                .lanes
                .iter()
                .filter_map(|(key, lane)| {
                    lane.history
                        .back()
                        .map(|trace| (key.clone(), trace.started_unix_ms))
                })
                .min_by_key(|(_, started)| *started)
                .map(|(key, _)| key)
            else {
                break;
            };
            if let Some(trace) = self
                .lanes
                .get_mut(&oldest_lane)
                .and_then(|lane| lane.history.pop_back())
            {
                self.invocation_lanes.remove(&trace.id);
            }
        }
        let empty_profile_keys = self
            .lanes
            .iter()
            .filter(|(_, lane)| {
                lane.active.is_empty()
                    && lane.history.is_empty()
                    && lane.key != lane_key(&lane.source_agent_id, &lane.capability)
            })
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        for key in empty_profile_keys {
            self.lanes.remove(&key);
        }
    }

    fn replace_lane_key(&mut self, old_key: &str, new_key: &str) {
        if self.selected_lane.as_deref() == Some(old_key) {
            self.selected_lane = Some(new_key.to_string());
        }
        match &mut self.view {
            View::Traces { agent_key } | View::Trace { agent_key, .. } if agent_key == old_key => {
                *agent_key = new_key.to_string();
            }
            _ => {}
        }
    }

    pub(crate) fn move_lane_selection(&mut self, amount: isize, now: Instant) {
        let keys = self.visible_lane_keys(now);
        if keys.is_empty() {
            self.selected_lane = None;
            return;
        }
        let current = self
            .selected_lane
            .as_ref()
            .and_then(|selected| keys.iter().position(|key| key == selected))
            .unwrap_or_default();
        let next = current
            .saturating_add_signed(amount)
            .min(keys.len().saturating_sub(1));
        self.selected_lane = Some(keys[next].clone());
    }

    pub(crate) fn normalize_lane_selection(&mut self, now: Instant) {
        let keys = self.visible_lane_keys(now);
        if self
            .selected_lane
            .as_ref()
            .is_some_and(|selected| keys.contains(selected))
        {
            return;
        }
        self.selected_lane = keys.first().cloned();
        self.scroll_offset = 0;
    }

    pub(crate) fn normalize_search_selection(&mut self, now: Instant) {
        let View::Trace {
            agent_key,
            trace_id,
            ..
        } = &self.view
        else {
            self.normalize_lane_selection(now);
            return;
        };
        if self.search.trim().is_empty() {
            return;
        }
        let first_match = self.trace(agent_key, *trace_id).and_then(|trace| {
            trace
                .matching_observation_choices(&self.search)
                .into_iter()
                .next()
        });
        let Some(first_match) = first_match else {
            return;
        };
        if let View::Trace {
            observation_cursor,
            selected_observation,
            ..
        } = &mut self.view
        {
            *observation_cursor = first_match;
            *selected_observation = first_match;
        }
    }

    pub(crate) fn ensure_lane_visible(&mut self, now: Instant, visible_rows: usize) {
        let keys = self.visible_lane_keys(now);
        let Some(selected) = self.selected_lane.as_ref() else {
            self.scroll_offset = 0;
            return;
        };
        let Some(index) = keys.iter().position(|key| key == selected) else {
            self.scroll_offset = 0;
            return;
        };
        if index < self.scroll_offset {
            self.scroll_offset = index;
        } else if visible_rows > 0 && index >= self.scroll_offset.saturating_add(visible_rows) {
            self.scroll_offset = index.saturating_add(1).saturating_sub(visible_rows);
        }
        self.scroll_offset = self
            .scroll_offset
            .min(keys.len().saturating_sub(visible_rows.max(1)));
    }

    pub(crate) fn open_selected_agent(&mut self) {
        let Some(agent_key) = self.selected_lane.clone() else {
            self.notice = Some("No Agent is available yet".to_string());
            return;
        };
        self.selected_trace = self
            .lanes
            .get(&agent_key)
            .and_then(|lane| lane.recent_traces().first().map(|trace| trace.id));
        self.trace_scroll_offset = 0;
        self.view = View::Traces { agent_key };
    }

    pub(crate) fn move_trace_selection(&mut self, amount: isize) {
        let View::Traces { agent_key } = &self.view else {
            return;
        };
        let Some(lane) = self.lanes.get(agent_key) else {
            return;
        };
        let traces = lane.recent_traces();
        if traces.is_empty() {
            self.selected_trace = None;
            return;
        }
        let current = self
            .selected_trace
            .and_then(|selected| traces.iter().position(|trace| trace.id == selected))
            .unwrap_or_default();
        let next = current
            .saturating_add_signed(amount)
            .min(traces.len().saturating_sub(1));
        self.selected_trace = Some(traces[next].id);
    }

    pub(crate) fn open_selected_trace(&mut self) {
        let View::Traces { agent_key } = &self.view else {
            return;
        };
        let Some(trace_id) = self.selected_trace else {
            self.notice = Some("This Agent has no invocations yet".to_string());
            return;
        };
        self.view = View::Trace {
            agent_key: agent_key.clone(),
            trace_id,
            tab: TraceTab::Summary,
            timeline: false,
            observation_cursor: None,
            selected_observation: None,
        };
    }

    pub(crate) fn go_back(&mut self) {
        self.notice = None;
        self.view = match &self.view {
            View::Main => View::Main,
            View::Traces { .. } | View::Optimize => View::Main,
            View::Trace { agent_key, .. } => View::Traces {
                agent_key: agent_key.clone(),
            },
        };
    }

    pub(crate) fn cycle_trace_tab(&mut self) {
        if let View::Trace { tab, .. } = &mut self.view {
            *tab = tab.next();
        }
    }

    pub(crate) fn toggle_timeline(&mut self) {
        if let View::Trace { timeline, .. } = &mut self.view {
            *timeline = !*timeline;
        }
    }

    pub(crate) fn move_observation_cursor(&mut self, amount: isize) {
        let View::Trace {
            agent_key,
            trace_id,
            observation_cursor,
            ..
        } = &self.view
        else {
            return;
        };
        let choices = self
            .trace(agent_key, *trace_id)
            .map_or_else(Vec::new, |trace| {
                trace.matching_observation_choices(&self.search)
            });
        if choices.is_empty() {
            self.notice = Some(format!(
                "No Trace observations match {:?}",
                self.search.trim()
            ));
            return;
        }
        let current = choices
            .iter()
            .position(|choice| choice == observation_cursor)
            .unwrap_or_default();
        let next = current
            .saturating_add_signed(amount)
            .min(choices.len().saturating_sub(1));
        if let View::Trace {
            observation_cursor, ..
        } = &mut self.view
        {
            *observation_cursor = choices[next];
        }
    }

    pub(crate) fn inspect_observation_cursor(&mut self) {
        let observation_name = match &self.view {
            View::Trace {
                agent_key,
                trace_id,
                observation_cursor: Some(observation_id),
                ..
            } => self
                .trace(agent_key, *trace_id)
                .and_then(|trace| trace.observation(*observation_id))
                .map(|observation| observation.name.clone()),
            _ => None,
        };
        let View::Trace {
            observation_cursor,
            selected_observation,
            ..
        } = &mut self.view
        else {
            return;
        };
        *selected_observation = *observation_cursor;
        self.notice = Some(observation_cursor.map_or_else(
            || "Inspecting the complete Trace".to_string(),
            |_| {
                format!(
                    "Inspecting the {} observation",
                    observation_name.as_deref().unwrap_or("selected")
                )
            },
        ));
    }

    pub(crate) fn open_optimize(&mut self) {
        self.view = View::Optimize;
        self.selected_comparison = self
            .selected_comparison
            .min(self.comparison_rows.len().saturating_sub(1));
    }

    pub(crate) fn move_comparison_selection(&mut self, amount: isize) {
        if self.comparison_rows.is_empty() {
            self.selected_comparison = 0;
            return;
        }
        self.selected_comparison = self
            .selected_comparison
            .saturating_add_signed(amount)
            .min(self.comparison_rows.len().saturating_sub(1));
    }

    pub(crate) fn selected_comparison(&self) -> Option<&ComparisonRow> {
        self.comparison_rows.get(self.selected_comparison)
    }

    pub(crate) fn move_exclusion_selection(&mut self, amount: isize) {
        if self.optimization_exclusions.is_empty() {
            self.selected_exclusion = 0;
            return;
        }
        self.selected_exclusion = self
            .selected_exclusion
            .saturating_add_signed(amount)
            .min(self.optimization_exclusions.len().saturating_sub(1));
    }

    fn matches_filter(&self, lane: &AgentLane) -> bool {
        match self.filter {
            LaneFilter::All => true,
            LaneFilter::Running => lane.outcome() == LaneOutcome::Running,
            LaneFilter::Problems => {
                matches!(lane.outcome(), LaneOutcome::Failed | LaneOutcome::Timeout)
            }
            LaneFilter::Passed => lane.outcome() == LaneOutcome::Passed,
        }
    }
}

fn lane_key(agent_id: &str, capability: &str) -> String {
    format!("{agent_id}\0{capability}")
}

fn attention_cmp(left: &AgentLane, right: &AgentLane, now: Instant) -> Ordering {
    let left_outcome = left.outcome();
    let right_outcome = right.outcome();
    attention_rank(left_outcome)
        .cmp(&attention_rank(right_outcome))
        .then_with(|| match (left_outcome, right_outcome) {
            (LaneOutcome::Running, LaneOutcome::Running) => right
                .representative()
                .map(|trace| trace.elapsed(now))
                .cmp(&left.representative().map(|trace| trace.elapsed(now))),
            _ => right.last_updated_unix_ms.cmp(&left.last_updated_unix_ms),
        })
        .then_with(|| agent_cmp(left, right))
}

fn attention_rank(outcome: LaneOutcome) -> u8 {
    match outcome {
        LaneOutcome::Failed | LaneOutcome::Timeout => 0,
        LaneOutcome::Running => 1,
        LaneOutcome::Unknown => 2,
        LaneOutcome::Passed => 3,
        LaneOutcome::Skipped => 4,
    }
}

fn agent_cmp(left: &AgentLane, right: &AgentLane) -> Ordering {
    left.name
        .to_ascii_lowercase()
        .cmp(&right.name.to_ascii_lowercase())
        .then_with(|| left.capability.cmp(&right.capability))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::time::{Duration, Instant};

    use uuid::Uuid;
    use vifu_gateway::optimization::{CombinationKind, RouteCombination};

    use crate::monitor::{
        FeedbackEvent, FeedbackOutcome, RegisteredAgent, RuntimeEvent, RuntimeStage,
        RuntimeTerminal, StageStatus,
    };

    use super::{
        App, ComparisonRow, LaneFilter, LaneOutcome, LaneSort, OptimizationExclusion,
        OptimizationSummary, View,
    };

    fn registration(index: usize) -> RegisteredAgent {
        RegisteredAgent {
            id: format!("agent-{index:03}"),
            name: format!("Agent {index:03}"),
            provider: "local-qwen".to_string(),
            capability: "chat".to_string(),
            model: "qwen2.5:2b".to_string(),
            local_model_loaded: true,
        }
    }

    #[test]
    fn reducer_should_keep_selection_stable_across_one_hundred_agent_updates() {
        let now = Instant::now();
        let mut app = App::default();
        app.apply(
            RuntimeEvent::AgentsRegistered((0..100).map(registration).collect()),
            now,
        );
        app.sort = LaneSort::Agent;
        app.selected_lane = Some("agent-050\0chat".to_string());
        let invocation_id = Uuid::new_v4();

        app.apply(
            RuntimeEvent::InvocationStarted {
                invocation_id,
                agent_id: "agent-099".to_string(),
                agent_name: "Agent 099".to_string(),
                source_agent_id: "local-qwen".to_string(),
                capability: "chat".to_string(),
                provider: "local-qwen".to_string(),
                model: "qwen2.5:2b".to_string(),
                started_unix_ms: 100,
            },
            now,
        );

        assert_eq!(app.selected_lane.as_deref(), Some("agent-050\0chat"));
        assert_eq!(app.counts().total, 100);
    }

    #[test]
    fn attention_sort_should_put_failures_before_running_and_passed() {
        let now = Instant::now();
        let mut app = App::default();
        app.apply(
            RuntimeEvent::AgentsRegistered((0..3).map(registration).collect()),
            now,
        );
        let failed = Uuid::new_v4();
        let running = Uuid::new_v4();
        for (id, agent) in [(failed, "agent-000"), (running, "agent-001")] {
            app.apply(
                RuntimeEvent::InvocationStarted {
                    invocation_id: id,
                    agent_id: agent.to_string(),
                    agent_name: agent.to_string(),
                    source_agent_id: "local-qwen".to_string(),
                    capability: "chat".to_string(),
                    provider: "local-qwen".to_string(),
                    model: "qwen2.5:2b".to_string(),
                    started_unix_ms: 100,
                },
                now,
            );
        }
        app.apply(
            RuntimeEvent::InvocationFinished {
                invocation_id: failed,
                elapsed: Duration::from_millis(500),
                error: Some("invalid response".to_string()),
                terminal: RuntimeTerminal::ProviderFailed,
            },
            now,
        );

        let keys = app.visible_lane_keys(now);

        assert_eq!(keys[0], "agent-000\0chat");
        assert_eq!(keys[1], "agent-001\0chat");
    }

    #[test]
    fn problem_filter_should_exclude_unknown_and_passed_lanes() {
        let now = Instant::now();
        let mut app = App::default();
        app.apply(
            RuntimeEvent::AgentsRegistered((0..2).map(registration).collect()),
            now,
        );
        let invocation_id = Uuid::new_v4();
        app.apply(
            RuntimeEvent::InvocationStarted {
                invocation_id,
                agent_id: "agent-000".to_string(),
                agent_name: "Agent 000".to_string(),
                source_agent_id: "local-qwen".to_string(),
                capability: "chat".to_string(),
                provider: "local-qwen".to_string(),
                model: "qwen2.5:2b".to_string(),
                started_unix_ms: 100,
            },
            now,
        );
        app.apply(
            RuntimeEvent::InvocationFinished {
                invocation_id,
                elapsed: Duration::from_secs(1),
                error: None,
                terminal: RuntimeTerminal::Delivered,
            },
            now,
        );
        app.filter = LaneFilter::Problems;

        assert!(app.visible_lane_keys(now).is_empty());
        assert_eq!(app.lanes["agent-000\0chat"].outcome(), LaneOutcome::Passed);
    }

    #[test]
    fn provider_failure_keeps_failed_stage_as_the_diagnosis_boundary() {
        let now = Instant::now();
        let mut app = App::default();
        app.apply(RuntimeEvent::AgentsRegistered(vec![registration(0)]), now);
        let invocation_id = Uuid::new_v4();
        app.apply(
            RuntimeEvent::InvocationStarted {
                invocation_id,
                agent_id: "agent-000".to_string(),
                agent_name: "Agent 000".to_string(),
                source_agent_id: "agent-000".to_string(),
                capability: "chat".to_string(),
                provider: "local-qwen".to_string(),
                model: "qwen2.5:2b".to_string(),
                started_unix_ms: 100,
            },
            now,
        );
        app.apply(
            RuntimeEvent::StageChanged {
                invocation_id,
                observation_id: Uuid::new_v4(),
                stage: RuntimeStage::Decode,
                status: StageStatus::Failed,
                start_offset: Duration::from_millis(8),
                end_offset: Some(Duration::from_millis(20)),
                elapsed: Duration::from_millis(12),
                request_elapsed: None,
                input_tokens: None,
                output_tokens: None,
                resident: None,
                error: Some("decode failed".to_string()),
            },
            now,
        );
        app.apply(
            RuntimeEvent::InvocationFinished {
                invocation_id,
                elapsed: Duration::from_millis(20),
                terminal: RuntimeTerminal::ProviderFailed,
                error: Some("provider failed".to_string()),
            },
            now,
        );

        let trace = app.lanes["agent-000\0chat"].representative().unwrap();
        assert_eq!(trace.current_stage(), RuntimeStage::Decode);
        assert_eq!(
            trace.observation_status(RuntimeStage::Deliver),
            StageStatus::Unknown
        );
    }

    #[test]
    fn timeout_marks_the_latest_active_observation_as_the_failure_boundary() {
        let now = Instant::now();
        let mut app = App::default();
        app.apply(RuntimeEvent::AgentsRegistered(vec![registration(0)]), now);
        let invocation_id = Uuid::new_v4();
        app.apply(
            RuntimeEvent::InvocationStarted {
                invocation_id,
                agent_id: "agent-000".to_string(),
                agent_name: "Agent 000".to_string(),
                source_agent_id: "agent-000".to_string(),
                capability: "chat".to_string(),
                provider: "local-qwen".to_string(),
                model: "qwen2.5:2b".to_string(),
                started_unix_ms: 100,
            },
            now,
        );
        app.apply(
            RuntimeEvent::StageChanged {
                invocation_id,
                observation_id: Uuid::new_v4(),
                stage: RuntimeStage::Prefill,
                status: StageStatus::Passed,
                start_offset: Duration::from_millis(4),
                end_offset: Some(Duration::from_millis(8)),
                elapsed: Duration::from_millis(4),
                request_elapsed: Some(Duration::from_millis(8)),
                input_tokens: Some(8),
                output_tokens: None,
                resident: None,
                error: None,
            },
            now,
        );
        app.apply(
            RuntimeEvent::StageChanged {
                invocation_id,
                observation_id: Uuid::new_v4(),
                stage: RuntimeStage::Decode,
                status: StageStatus::Active,
                start_offset: Duration::from_millis(10),
                end_offset: None,
                elapsed: Duration::ZERO,
                request_elapsed: Some(Duration::from_millis(10)),
                input_tokens: Some(8),
                output_tokens: None,
                resident: Some(true),
                error: None,
            },
            now,
        );
        app.apply(
            RuntimeEvent::InvocationFinished {
                invocation_id,
                elapsed: Duration::from_millis(50),
                terminal: RuntimeTerminal::TimedOut,
                error: None,
            },
            now,
        );

        let trace = app.lanes["agent-000\0chat"].representative().unwrap();
        let decode = trace
            .observation_for_stage(RuntimeStage::Decode)
            .expect("decode observation");
        assert_eq!(decode.status, StageStatus::Failed);
        assert_eq!(decode.end_offset, Some(Duration::from_millis(50)));
        assert_eq!(decode.elapsed, Duration::from_millis(40));
        assert_eq!(decode.error.as_deref(), Some("request timed out"));
        assert_eq!(trace.current_stage(), RuntimeStage::Decode);
        assert_eq!(
            trace.observation_status(RuntimeStage::Deliver),
            StageStatus::Unknown
        );
    }

    #[test]
    fn application_failure_remains_visible_regardless_of_finish_order() {
        let now = Instant::now();
        let mut app = App::default();
        app.apply(
            RuntimeEvent::AgentsRegistered(vec![registration(0), registration(1)]),
            now,
        );
        let feedback_first = Uuid::new_v4();
        let finish_first = Uuid::new_v4();
        for (invocation_id, agent_id) in
            [(feedback_first, "agent-000"), (finish_first, "agent-001")]
        {
            app.apply(
                RuntimeEvent::InvocationStarted {
                    invocation_id,
                    agent_id: agent_id.to_string(),
                    agent_name: agent_id.to_string(),
                    source_agent_id: agent_id.to_string(),
                    capability: "chat".to_string(),
                    provider: "local-qwen".to_string(),
                    model: "qwen2.5:2b".to_string(),
                    started_unix_ms: 100,
                },
                now,
            );
        }
        let feedback = |invocation_id| RuntimeEvent::ApplicationFeedback {
            invocation_id,
            observation_id: Uuid::new_v4(),
            start_offset: Duration::from_millis(18),
            end_offset: Duration::from_millis(18),
            event: FeedbackEvent::OutputAccepted,
            outcome: FeedbackOutcome::Fail,
            message: Some("application could not parse the response".to_string()),
            path: Some("$.action".to_string()),
        };
        let finish = |invocation_id| RuntimeEvent::InvocationFinished {
            invocation_id,
            elapsed: Duration::from_millis(20),
            terminal: RuntimeTerminal::Delivered,
            error: None,
        };

        app.apply(feedback(feedback_first), now);
        app.apply(finish(feedback_first), now);
        app.apply(finish(finish_first), now);
        app.apply(feedback(finish_first), now);

        for agent_id in ["agent-000", "agent-001"] {
            let trace = app.lanes[&format!("{agent_id}\0chat")]
                .representative()
                .unwrap();
            assert_eq!(trace.outcome, LaneOutcome::Failed);
            assert!(trace
                .error
                .as_deref()
                .is_some_and(|error| error.contains("could not parse")));
            let observation = trace.observations.last().unwrap();
            assert_eq!(observation.name, "OUTPUT_ACCEPTED");
            assert_eq!(
                observation.output,
                Some(serde_json::json!({
                    "event": "OUTPUT_ACCEPTED",
                    "outcome": "fail",
                    "message": "application could not parse the response",
                    "path": "$.action",
                }))
            );
            assert_eq!(observation.start_offset, Some(Duration::from_millis(18)));
            assert_eq!(observation.end_offset, Some(Duration::from_millis(18)));
            assert_eq!(observation.attributes["startOffsetMs"], 18);
            assert_eq!(observation.attributes["endOffsetMs"], 18);
        }
        app.filter = LaneFilter::Problems;
        assert_eq!(app.visible_lane_keys(now).len(), 2);
    }

    #[test]
    fn inventory_refresh_clears_stale_optimization_but_preserves_override_snapshot() {
        let now = Instant::now();
        let mut app = App::default();
        let plan = RouteCombination {
            id: "old-plan".to_string(),
            label: "old-plan".to_string(),
            kind: CombinationKind::Current,
            explanation: "old inventory".to_string(),
            routes: BTreeMap::from([("old-route".to_string(), "old-provider".to_string())]),
        };
        app.comparison_rows.push(ComparisonRow {
            id: plan.id.clone(),
            name: plan.label.clone(),
            plan,
            first_total: None,
            first_run_cold: None,
            repeat_runs_resident: None,
            total: None,
            total_range: None,
            ttft: None,
            ttft_range: None,
            tokens_per_second: None,
            first_process_cpu_percent: None,
            process_cpu_percent: None,
            peak_rss_bytes: None,
            route_labels: BTreeMap::new(),
            result: LaneOutcome::Passed,
            detail: "old result".to_string(),
        });
        app.optimization_summary = Some(OptimizationSummary {
            comparison_id: Uuid::from_u128(1),
            started_at_ms: 1_000,
            completed_at_ms: 1_900,
            monotonic_duration_ms: 900,
            corpus_agents: 1,
            configured_local_models: 1,
            tested_models: 1,
            passed_models: 1,
            expected_pairs: 1,
            evaluated_pairs: 1,
            passed_pairs: 1,
            recommendation: Some("old-plan".to_string()),
            not_exhaustive: true,
            sequential_replay: true,
            device_architecture: "aarch64".to_string(),
            device_backend: Some("llama.cpp".to_string()),
            remote_fallbacks: Vec::new(),
        });
        app.optimization_exclusions.push(OptimizationExclusion {
            route: "old-route".to_string(),
            capability: "chat".to_string(),
            provider: "old-provider".to_string(),
            reason: "unavailable".to_string(),
            message: None,
        });
        app.optimization_excluded_total = 1;
        app.override_active = true;
        app.override_generation = Some(4);
        app.override_route_count = 1;

        app.apply(RuntimeEvent::AgentsRegistered(vec![registration(0)]), now);

        assert!(app.comparison_rows.is_empty());
        assert!(app.optimization_summary.is_none());
        assert!(app.optimization_exclusions.is_empty());
        assert_eq!(app.optimization_excluded_total, 0);
        assert!(app.override_active);
        assert_eq!(app.override_generation, Some(4));
        assert_eq!(app.override_route_count, 1);
        assert_eq!(app.inventory_generation, 1);
    }

    #[test]
    fn observation_selection_is_committed_and_stable_as_trace_events_arrive() {
        let now = Instant::now();
        let mut app = App::default();
        app.apply(RuntimeEvent::AgentsRegistered(vec![registration(0)]), now);
        let invocation_id = Uuid::new_v4();
        app.apply(
            RuntimeEvent::InvocationStarted {
                invocation_id,
                agent_id: "agent-000".to_string(),
                agent_name: "Agent 000".to_string(),
                source_agent_id: "agent-000".to_string(),
                capability: "chat".to_string(),
                provider: "local-qwen".to_string(),
                model: "qwen2.5:2b".to_string(),
                started_unix_ms: 100,
            },
            now,
        );
        let decode_observation_id = Uuid::new_v4();
        app.apply(
            RuntimeEvent::StageChanged {
                invocation_id,
                observation_id: decode_observation_id,
                stage: RuntimeStage::Decode,
                status: StageStatus::Active,
                start_offset: Duration::from_millis(8),
                end_offset: None,
                elapsed: Duration::from_millis(12),
                request_elapsed: Some(Duration::from_millis(20)),
                input_tokens: Some(8),
                output_tokens: Some(2),
                resident: Some(true),
                error: None,
            },
            now,
        );
        app.selected_lane = Some("agent-000\0chat".to_string());
        app.open_selected_agent();
        app.open_selected_trace();
        app.move_observation_cursor(2);
        app.inspect_observation_cursor();

        assert!(matches!(
            app.view,
            View::Trace {
                observation_cursor: Some(id),
                selected_observation: Some(selected),
                ..
            } if id == decode_observation_id && selected == decode_observation_id
        ));

        app.apply(
            RuntimeEvent::StageChanged {
                invocation_id,
                observation_id: Uuid::new_v4(),
                stage: RuntimeStage::Decode,
                status: StageStatus::Passed,
                start_offset: Duration::from_millis(4),
                end_offset: Some(Duration::from_millis(8)),
                elapsed: Duration::from_millis(4),
                request_elapsed: Some(Duration::from_millis(8)),
                input_tokens: Some(8),
                output_tokens: None,
                resident: None,
                error: None,
            },
            now,
        );
        assert!(matches!(
            app.view,
            View::Trace {
                selected_observation: Some(selected),
                ..
            } if selected == decode_observation_id
        ));
        let trace = app.lanes["agent-000\0chat"].representative().unwrap();
        assert_eq!(trace.observations.len(), 2);
        assert!(trace
            .observations
            .iter()
            .all(|observation| observation.parent_observation_id == Some(trace.id)));
        assert_eq!(
            trace
                .observations
                .iter()
                .filter(|observation| observation.stage == Some(RuntimeStage::Decode))
                .count(),
            2
        );
    }

    #[test]
    fn trace_search_locates_observation_error_and_io_and_selects_the_first_match() {
        let now = Instant::now();
        let mut app = App::default();
        app.apply(RuntimeEvent::AgentsRegistered(vec![registration(0)]), now);
        let invocation_id = Uuid::new_v4();
        app.apply(
            RuntimeEvent::InvocationStarted {
                invocation_id,
                agent_id: "agent-000".to_string(),
                agent_name: "Agent 000".to_string(),
                source_agent_id: "agent-000".to_string(),
                capability: "chat".to_string(),
                provider: "local-qwen".to_string(),
                model: "qwen2.5:2b".to_string(),
                started_unix_ms: 100,
            },
            now,
        );
        let validate_id = Uuid::new_v4();
        app.apply(
            RuntimeEvent::StageChanged {
                invocation_id,
                observation_id: validate_id,
                stage: RuntimeStage::Validate,
                status: StageStatus::Failed,
                start_offset: Duration::from_millis(8),
                end_offset: Some(Duration::from_millis(9)),
                elapsed: Duration::from_millis(1),
                request_elapsed: Some(Duration::from_millis(9)),
                input_tokens: None,
                output_tokens: None,
                resident: None,
                error: Some("missing action contract".to_string()),
            },
            now,
        );
        let feedback_id = Uuid::new_v4();
        app.apply(
            RuntimeEvent::ApplicationFeedback {
                invocation_id,
                observation_id: feedback_id,
                start_offset: Duration::from_millis(10),
                end_offset: Duration::from_millis(10),
                event: FeedbackEvent::OutputAccepted,
                outcome: FeedbackOutcome::Fail,
                message: Some("parser rejected required field".to_string()),
                path: Some("$.actions[0]".to_string()),
            },
            now,
        );
        app.view = View::Trace {
            agent_key: "agent-000\0chat".to_string(),
            trace_id: invocation_id,
            tab: super::TraceTab::Summary,
            timeline: false,
            observation_cursor: None,
            selected_observation: None,
        };

        let trace = app.trace("agent-000\0chat", invocation_id).unwrap();
        assert_eq!(
            trace.matching_observation_choices("missing action"),
            vec![Some(validate_id)]
        );
        assert_eq!(
            trace.matching_observation_choices("$.actions[0]"),
            vec![Some(feedback_id)]
        );

        app.search = "$.actions[0]".to_string();
        app.normalize_search_selection(now);
        assert!(matches!(
            app.view,
            View::Trace {
                observation_cursor: Some(cursor),
                selected_observation: Some(selected),
                ..
            } if cursor == feedback_id && selected == feedback_id
        ));

        app.search = "no such observation".to_string();
        app.normalize_search_selection(now);
        assert!(app
            .trace("agent-000\0chat", invocation_id)
            .unwrap()
            .matching_observation_choices(&app.search)
            .is_empty());
        assert!(matches!(
            app.view,
            View::Trace {
                observation_cursor: Some(cursor),
                selected_observation: Some(selected),
                ..
            } if cursor == feedback_id && selected == feedback_id
        ));
    }

    #[test]
    fn completed_trace_history_has_one_global_bound() {
        let now = Instant::now();
        let mut app = App::default();
        app.apply(
            RuntimeEvent::AgentsRegistered((0..65).map(registration).collect()),
            now,
        );
        for agent in 0..65 {
            for run in 0..8 {
                let invocation_id = Uuid::new_v4();
                let agent_id = format!("agent-{agent:03}");
                app.apply(
                    RuntimeEvent::InvocationStarted {
                        invocation_id,
                        agent_id: agent_id.clone(),
                        agent_name: format!("Agent {agent:03}"),
                        source_agent_id: agent_id,
                        capability: "chat".to_string(),
                        provider: "local-qwen".to_string(),
                        model: "qwen2.5:2b".to_string(),
                        started_unix_ms: (agent * 8 + run) as u64,
                    },
                    now,
                );
                app.apply(
                    RuntimeEvent::InvocationFinished {
                        invocation_id,
                        elapsed: Duration::from_millis(1),
                        terminal: RuntimeTerminal::Delivered,
                        error: None,
                    },
                    now,
                );
            }
        }

        assert_eq!(
            app.lanes
                .values()
                .map(|lane| lane.history.len())
                .sum::<usize>(),
            512
        );
    }

    #[test]
    fn profiles_sharing_one_provider_keep_history_and_post_finish_feedback() {
        let now = Instant::now();
        let mut app = App::default();
        let provider = RegisteredAgent {
            id: "local-qwen".to_string(),
            name: "Local Qwen".to_string(),
            provider: "local-qwen".to_string(),
            capability: "chat".to_string(),
            model: "qwen2.5:2b".to_string(),
            local_model_loaded: true,
        };
        app.apply(RuntimeEvent::AgentsRegistered(vec![provider.clone()]), now);

        let invocations = [
            (Uuid::new_v4(), "planner", "Planner"),
            (Uuid::new_v4(), "narrator", "Narrator"),
        ];
        for (invocation_id, agent_id, agent_name) in invocations {
            app.apply(
                RuntimeEvent::InvocationStarted {
                    invocation_id,
                    agent_id: agent_id.to_string(),
                    agent_name: agent_name.to_string(),
                    source_agent_id: provider.id.clone(),
                    capability: "chat".to_string(),
                    provider: provider.provider.clone(),
                    model: provider.model.clone(),
                    started_unix_ms: 100,
                },
                now,
            );
            app.apply(
                RuntimeEvent::InvocationFinished {
                    invocation_id,
                    elapsed: Duration::from_millis(20),
                    terminal: RuntimeTerminal::Delivered,
                    error: None,
                },
                now,
            );
            app.apply(
                RuntimeEvent::ApplicationFeedback {
                    invocation_id,
                    observation_id: Uuid::new_v4(),
                    start_offset: Duration::from_millis(21),
                    end_offset: Duration::from_millis(21),
                    event: FeedbackEvent::OutputAccepted,
                    outcome: FeedbackOutcome::Fail,
                    message: Some("application could not parse response".to_string()),
                    path: Some("$.action".to_string()),
                },
                now,
            );
        }

        // A transient reconnect reports the provider inventory again. Both
        // application Agent lanes and their late feedback must remain visible.
        app.apply(RuntimeEvent::AgentsRegistered(vec![provider]), now);

        assert!(!app.lanes.contains_key("local-qwen\0chat"));
        for (invocation_id, agent_id, _) in invocations {
            let lane = &app.lanes[&format!("{agent_id}\0chat")];
            assert!(lane.configured);
            let trace = lane.trace(invocation_id).expect("completed trace");
            assert_eq!(trace.outcome, LaneOutcome::Failed);
            assert_eq!(trace.feedback.len(), 1);
            assert_eq!(trace.feedback[0].path.as_deref(), Some("$.action"));
        }
    }

    #[test]
    fn inventory_refresh_removes_inactive_agents_that_are_no_longer_configured() {
        let now = Instant::now();
        let mut app = App::default();
        app.apply(
            RuntimeEvent::AgentsRegistered(vec![registration(0), registration(1)]),
            now,
        );
        app.apply(RuntimeEvent::AgentsRegistered(vec![registration(1)]), now);

        assert!(!app.lanes.contains_key("agent-000\0chat"));
        assert!(app.lanes.contains_key("agent-001\0chat"));
    }
}
