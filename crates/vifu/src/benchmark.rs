use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;
use uuid::Uuid;
use vifu_gateway::control::{GatewayRuntimeConfiguration, RuntimeControlClient};
use vifu_gateway::optimization::{
    coverage, generate_combinations, AgentCase, CandidateEvaluation, CandidateOutcome,
    CombinationKind, ExclusionReason, MetricRange, OptimizationCoverage, RouteCombination,
    RuntimeComparisonDevice, RuntimeComparisonOutcome, RuntimeComparisonRunUpload,
    RuntimeComparisonStatus, RuntimeComparisonUpload, SessionRouteOverrides,
};
use vifu_gateway::protocol::AgentDescriptor;
use vifu_gateway::relay::{
    AgentGatewayProvider, GatewayCaptureEvent, GatewayInvocationTerminal, ProviderEvent,
    ProviderEventSink, ProviderStage,
};

#[cfg(feature = "local-llama")]
use crate::local_models::LocalModelPool;
use crate::monitor::safe_error_message;
use crate::tui::system::{current_process_cpu_time, current_rss_bytes};

const MAX_CAPTURED_CASES: usize = 128;
const MAX_CAPTURED_BYTES: usize = 24 * 1024 * 1024;
const MAX_SINGLE_CAPTURE_BYTES: usize = 6 * 1024 * 1024;
const MAX_IN_FLIGHT_CAPTURES: usize = 16;
const MAX_IN_FLIGHT_CAPTURE_BYTES: usize = 8 * 1024 * 1024;
const REPEAT_RUNS: usize = 3;
const HISTORY_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub(crate) struct OptimizationController {
    state: Arc<Mutex<OptimizationState>>,
    route_overrides: Arc<SessionRouteOverrides>,
    history_target: Arc<Mutex<Option<ComparisonHistoryTarget>>>,
    #[cfg(feature = "local-llama")]
    local_model_pool: LocalModelPool,
}

#[derive(Clone)]
struct ComparisonHistoryTarget {
    gateway_id: String,
    client: RuntimeControlClient,
}

#[derive(Default)]
struct OptimizationState {
    config_epoch: u64,
    providers: BTreeMap<String, ConfiguredProvider>,
    in_flight: HashMap<Uuid, CapturedStart>,
    corpus: BTreeMap<String, CapturedCase>,
    capture_issues: BTreeMap<String, OptimizationCaptureIssue>,
    next_capture_sequence: u64,
}

#[derive(Clone)]
struct ConfiguredProvider {
    provider: Arc<dyn AgentGatewayProvider>,
    display_name: String,
    capabilities: BTreeSet<String>,
    models: BTreeSet<String>,
    replay_safe: bool,
    replay_capabilities: BTreeSet<String>,
    local_kind: Option<String>,
}

struct CapturedStart {
    route_key: String,
    agent_id: String,
    provider_key: String,
    capability: String,
    binding: Arc<Value>,
    input: Arc<Value>,
    timeout_ms: u64,
    captured_bytes: usize,
}

#[derive(Clone)]
struct CapturedCase {
    sequence: u64,
    route_key: String,
    agent_id: String,
    provider_key: String,
    capability: String,
    binding: Arc<Value>,
    input: Arc<Value>,
    baseline_output: Arc<Value>,
    timeout: Duration,
    captured_bytes: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OptimizationReport {
    pub(crate) comparison_id: Uuid,
    pub(crate) started_at_ms: u64,
    pub(crate) completed_at_ms: u64,
    pub(crate) monotonic_duration_ms: u64,
    pub(crate) coverage: OptimizationCoverage,
    pub(crate) candidate_evaluations: Vec<CandidateEvaluation>,
    pub(crate) combinations: Vec<MeasuredCombination>,
    pub(crate) recommendation: Option<String>,
    pub(crate) corpus_agents: usize,
    pub(crate) configured_local_models: usize,
    pub(crate) remote_fallbacks: Vec<RemoteFallback>,
    pub(crate) route_capabilities: BTreeMap<String, String>,
    pub(crate) route_labels: BTreeMap<String, String>,
    pub(crate) device_backend: Option<String>,
    pub(crate) loaded_models_after: Option<usize>,
    pub(crate) history_project: Option<String>,
    pub(crate) history_deployment: Option<String>,
    pub(crate) history_saved: bool,
    pub(crate) history_notice: Option<String>,
    pub(crate) not_exhaustive: bool,
    pub(crate) sequential_replay: bool,
    pub(crate) capture_issues: Vec<OptimizationCaptureIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RemoteFallback {
    pub(crate) provider_key: String,
    pub(crate) display_name: String,
    pub(crate) capabilities: Vec<String>,
    pub(crate) models: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OptimizationCaptureIssue {
    pub(crate) route_key: String,
    pub(crate) capability: String,
    pub(crate) provider_key: String,
    pub(crate) message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MeasuredCombination {
    pub(crate) plan: RouteCombination,
    pub(crate) outcome: BenchmarkOutcome,
    pub(crate) first_total_ms: Option<u64>,
    pub(crate) first_run_cold: Option<bool>,
    pub(crate) repeat_runs_resident: Option<bool>,
    pub(crate) repeat_total_ms: Option<MetricRange>,
    pub(crate) repeat_ttft_ms: Option<MetricRange>,
    pub(crate) tokens_per_second: Option<f64>,
    pub(crate) first_process_cpu_percent: Option<f64>,
    pub(crate) process_cpu_percent: Option<f64>,
    pub(crate) peak_rss_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BenchmarkOutcome {
    Passed,
    Failed(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CaptureDisposition {
    Captured,
    Dropped,
    Ignored,
}

impl OptimizationController {
    pub(crate) fn new(
        route_overrides: Arc<SessionRouteOverrides>,
        #[cfg(feature = "local-llama")] local_model_pool: LocalModelPool,
    ) -> Self {
        Self {
            state: Arc::new(Mutex::new(OptimizationState::default())),
            route_overrides,
            history_target: Arc::new(Mutex::new(None)),
            #[cfg(feature = "local-llama")]
            local_model_pool,
        }
    }

    pub(crate) fn route_overrides(&self) -> Arc<SessionRouteOverrides> {
        Arc::clone(&self.route_overrides)
    }

    #[cfg(feature = "local-llama")]
    pub(crate) fn note_active_route(&self, route_key: &str, provider_key: &str) {
        self.local_model_pool
            .set_active_route(route_key, provider_key);
    }

    pub(crate) fn configure_runtime_control(
        &self,
        server_url: &str,
        gateway_id: impl Into<String>,
        device_token: impl Into<String>,
    ) -> Result<(), String> {
        let client = RuntimeControlClient::new(server_url, device_token)?;
        *lock(&self.history_target) = Some(ComparisonHistoryTarget {
            gateway_id: gateway_id.into(),
            client,
        });
        Ok(())
    }

    pub(crate) fn clear_runtime_control(&self) {
        *lock(&self.history_target) = None;
    }

    pub(crate) fn configure(
        &self,
        providers: &[Arc<dyn AgentGatewayProvider>],
        agents: &[AgentDescriptor],
    ) -> u64 {
        self.configure_inner(providers, agents, true)
    }

    /// Refreshes provider instances after a transient gateway reconnect while
    /// retaining the process-local route activation for this Vifu session.
    pub(crate) fn refresh_providers(
        &self,
        providers: &[Arc<dyn AgentGatewayProvider>],
        agents: &[AgentDescriptor],
    ) -> u64 {
        self.configure_inner(providers, agents, false)
    }

    fn configure_inner(
        &self,
        providers: &[Arc<dyn AgentGatewayProvider>],
        agents: &[AgentDescriptor],
        reset_session: bool,
    ) -> u64 {
        let mut descriptors: HashMap<&str, Vec<&AgentDescriptor>> = HashMap::new();
        for agent in agents {
            descriptors
                .entry(descriptor_provider_key(agent))
                .or_default()
                .push(agent);
        }
        let configured = providers
            .iter()
            .filter_map(|provider| {
                let provider_descriptors = descriptors.get(provider.id())?;
                let capabilities = provider_descriptors
                    .iter()
                    .flat_map(|descriptor| {
                        descriptor
                            .metadata
                            .get("capabilities")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                    })
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<BTreeSet<_>>();
                let local_kind = provider_descriptors.iter().find_map(|descriptor| {
                    descriptor
                        .metadata
                        .get("localProviderType")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                });
                let execution_location = provider_descriptors.iter().find_map(|descriptor| {
                    descriptor
                        .metadata
                        .get("executionLocation")
                        .and_then(Value::as_str)
                });
                let replay_safe = matches!(local_kind.as_deref(), Some("llama" | "local-whisper"))
                    || (local_kind.as_deref() == Some("openai-compatible")
                        && execution_location == Some("local"));
                let allowed = match local_kind.as_deref() {
                    Some("llama") => &["chat", "embedding"][..],
                    Some("local-whisper") => &["transcription"][..],
                    Some("openai-compatible") if execution_location == Some("local") => {
                        &["chat", "embedding"][..]
                    }
                    _ => &[][..],
                };
                let replay_capabilities = capabilities
                    .iter()
                    .filter(|capability| allowed.contains(&capability.as_str()))
                    .cloned()
                    .collect();
                let display_name = provider_descriptors.first().map_or_else(
                    || provider.id().to_string(),
                    |descriptor| descriptor.name.clone(),
                );
                let models = provider_descriptors
                    .iter()
                    .flat_map(|descriptor| {
                        descriptor
                            .metadata
                            .get("models")
                            .and_then(Value::as_object)
                            .into_iter()
                            .flat_map(|models| models.values())
                            .chain(descriptor.metadata.get("model"))
                    })
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect();
                Some((
                    provider.id().to_string(),
                    ConfiguredProvider {
                        provider: Arc::clone(provider),
                        display_name,
                        capabilities,
                        models,
                        replay_safe,
                        replay_capabilities,
                        local_kind,
                    },
                ))
            })
            .collect();
        let mut state = lock(&self.state);
        state.config_epoch = state.config_epoch.saturating_add(1).max(1);
        state.providers = configured;
        state.in_flight.clear();
        state.corpus.clear();
        state.capture_issues.clear();
        state.next_capture_sequence = 0;
        let epoch = state.config_epoch;
        drop(state);
        if reset_session {
            self.route_overrides.clear();
            #[cfg(feature = "local-llama")]
            self.local_model_pool.clear_active_routes();
        }
        epoch
    }

    pub(crate) fn capture(&self, event: GatewayCaptureEvent) -> CaptureDisposition {
        let mut state = lock(&self.state);
        match event {
            GatewayCaptureEvent::InvocationStarted {
                config_epoch,
                request_id,
                binding_id,
                agent_id,
                provider_key,
                capability,
                binding,
                input,
                timeout_ms,
            } => {
                if config_epoch != state.config_epoch {
                    return CaptureDisposition::Ignored;
                }
                let route_key = binding_id.to_string();
                let captured_bytes = captured_value_bytes(&binding, &input, None);
                let in_flight_bytes = state
                    .in_flight
                    .values()
                    .map(|capture| capture.captured_bytes)
                    .sum::<usize>();
                if captured_bytes <= MAX_SINGLE_CAPTURE_BYTES
                    && state.in_flight.len() < MAX_IN_FLIGHT_CAPTURES
                    && in_flight_bytes.saturating_add(captured_bytes) <= MAX_IN_FLIGHT_CAPTURE_BYTES
                {
                    state.in_flight.insert(
                        request_id,
                        CapturedStart {
                            route_key,
                            agent_id,
                            provider_key,
                            capability,
                            binding,
                            input,
                            timeout_ms,
                            captured_bytes,
                        },
                    );
                    CaptureDisposition::Captured
                } else {
                    let key = corpus_key(&route_key, &capability);
                    state.corpus.remove(&key);
                    state.capture_issues.insert(
                        key,
                        OptimizationCaptureIssue {
                            route_key,
                            capability,
                            provider_key,
                            message: if captured_bytes > MAX_SINGLE_CAPTURE_BYTES {
                                format!(
                                    "capture is {} bytes; per-invocation limit is {} bytes",
                                    captured_bytes, MAX_SINGLE_CAPTURE_BYTES
                                )
                            } else {
                                format!(
                                    "capture admission exceeded the in-flight budget of {} calls or {} bytes",
                                    MAX_IN_FLIGHT_CAPTURES, MAX_IN_FLIGHT_CAPTURE_BYTES
                                )
                            },
                        },
                    );
                    CaptureDisposition::Dropped
                }
            }
            GatewayCaptureEvent::InvocationFinished {
                config_epoch,
                request_id,
                terminal: GatewayInvocationTerminal::Delivered,
                output: Some(output),
            } => {
                if config_epoch != state.config_epoch {
                    return CaptureDisposition::Ignored;
                }
                let Some(start) = state.in_flight.remove(&request_id) else {
                    return CaptureDisposition::Ignored;
                };
                let captured_bytes =
                    captured_value_bytes(&start.binding, &start.input, Some(&output));
                if captured_bytes > MAX_SINGLE_CAPTURE_BYTES {
                    let key = corpus_key(&start.route_key, &start.capability);
                    state.corpus.remove(&key);
                    state.capture_issues.insert(
                        key,
                        OptimizationCaptureIssue {
                            route_key: start.route_key,
                            capability: start.capability,
                            provider_key: start.provider_key,
                            message: format!(
                                "capture is {} bytes; per-invocation limit is {} bytes",
                                captured_bytes, MAX_SINGLE_CAPTURE_BYTES
                            ),
                        },
                    );
                    return CaptureDisposition::Dropped;
                }
                state.next_capture_sequence = state.next_capture_sequence.saturating_add(1);
                let sequence = state.next_capture_sequence;
                let key = corpus_key(&start.route_key, &start.capability);
                state.capture_issues.remove(&key);
                state.corpus.insert(
                    key,
                    CapturedCase {
                        sequence,
                        route_key: start.route_key,
                        agent_id: start.agent_id,
                        provider_key: start.provider_key,
                        capability: start.capability,
                        binding: start.binding,
                        input: start.input,
                        baseline_output: output,
                        timeout: Duration::from_millis(start.timeout_ms.max(1)),
                        captured_bytes,
                    },
                );
                for evicted in trim_corpus(&mut state.corpus) {
                    let key = corpus_key(&evicted.route_key, &evicted.capability);
                    state.capture_issues.insert(
                        key,
                        OptimizationCaptureIssue {
                            route_key: evicted.route_key,
                            capability: evicted.capability,
                            provider_key: evicted.provider_key,
                            message: format!(
                                "capture was evicted to keep the optimization corpus under {} bytes",
                                MAX_CAPTURED_BYTES
                            ),
                        },
                    );
                }
                CaptureDisposition::Captured
            }
            GatewayCaptureEvent::InvocationFinished {
                config_epoch,
                request_id,
                ..
            }
            | GatewayCaptureEvent::InvocationCancelled {
                config_epoch,
                request_id,
            } => {
                if config_epoch != state.config_epoch {
                    return CaptureDisposition::Ignored;
                }
                if state.in_flight.remove(&request_id).is_some() {
                    CaptureDisposition::Captured
                } else {
                    CaptureDisposition::Ignored
                }
            }
        }
    }

    pub(crate) fn activate(&self, combination: &RouteCombination) -> Result<u64, String> {
        let state = lock(&self.state);
        validate_activation(&state, combination)?;
        self.route_overrides.activate(combination.routes.clone())
    }

    pub(crate) fn undo(&self) -> Option<u64> {
        self.route_overrides.undo()
    }

    pub(crate) fn note_capture_dropped(
        &self,
        config_epoch: u64,
        request_id: Uuid,
        route_key: Option<String>,
        capability: Option<String>,
        provider_key: Option<String>,
    ) {
        let mut state = lock(&self.state);
        if config_epoch != state.config_epoch {
            return;
        }
        let captured = state.in_flight.remove(&request_id);
        let route_key = captured
            .as_ref()
            .map(|capture| capture.route_key.clone())
            .or(route_key);
        let capability = captured
            .as_ref()
            .map(|capture| capture.capability.clone())
            .or(capability);
        let provider_key = captured
            .as_ref()
            .map(|capture| capture.provider_key.clone())
            .or(provider_key);
        let (Some(route_key), Some(capability), Some(provider_key)) =
            (route_key, capability, provider_key)
        else {
            return;
        };
        let key = corpus_key(&route_key, &capability);
        state.corpus.remove(&key);
        state.capture_issues.insert(
            key,
            OptimizationCaptureIssue {
                route_key,
                capability,
                provider_key,
                message: "capture was dropped because the bounded optimization queue was full"
                    .to_string(),
            },
        );
    }

    pub(crate) fn discard_capture(&self, config_epoch: u64, request_id: Uuid) {
        let mut state = lock(&self.state);
        if config_epoch == state.config_epoch {
            state.in_flight.remove(&request_id);
        }
    }

    pub(crate) async fn benchmark(&self) -> Result<OptimizationReport, String> {
        let (providers, corpus, capture_issues) = {
            let state = lock(&self.state);
            (
                state.providers.clone(),
                state.corpus.values().cloned().collect::<Vec<_>>(),
                state.capture_issues.values().cloned().collect::<Vec<_>>(),
            )
        };
        let local_provider_keys = providers
            .iter()
            .filter(|(_, provider)| provider.replay_safe)
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        let remote_fallbacks = remote_fallbacks(&providers);
        let comparison_id = Uuid::new_v4();
        let started_at_ms = unix_timestamp_ms();
        let comparison_started = Instant::now();
        let device_backend = comparison_backend(&providers);
        let route_labels = comparison_route_labels(&corpus);
        if corpus.is_empty() {
            return Ok(OptimizationReport {
                comparison_id,
                started_at_ms,
                completed_at_ms: unix_timestamp_ms().max(started_at_ms),
                monotonic_duration_ms: elapsed_ms(comparison_started.elapsed()),
                coverage: OptimizationCoverage {
                    configured_models: local_provider_keys.len(),
                    tested_models: 0,
                    passed_models: 0,
                    expected_pairs: 0,
                    evaluated_pairs: 0,
                    passed_pairs: 0,
                    missing: Vec::new(),
                },
                candidate_evaluations: Vec::new(),
                combinations: Vec::new(),
                recommendation: None,
                corpus_agents: 0,
                configured_local_models: local_provider_keys.len(),
                remote_fallbacks,
                route_capabilities: capture_issues
                    .iter()
                    .map(|issue| (issue.route_key.clone(), issue.capability.clone()))
                    .collect(),
                route_labels,
                device_backend,
                loaded_models_after: self.loaded_models_after(),
                history_project: None,
                history_deployment: None,
                history_saved: false,
                history_notice: Some(
                    "Comparison completed locally; Dashboard history upload was skipped because no successful request corpus is available yet."
                        .to_string(),
                ),
                not_exhaustive: true,
                sequential_replay: true,
                capture_issues,
            });
        }

        let grouped_cases = cases_by_route(&corpus);
        let mut candidate_evaluations = Vec::new();
        for (route_key, route_cases) in &grouped_cases {
            for provider_key in &local_provider_keys {
                let provider = &providers[provider_key];
                let unsupported = route_cases
                    .iter()
                    .find(|case| !provider.capabilities.contains(&case.capability));
                let replay_disabled = route_cases
                    .iter()
                    .find(|case| !provider.replay_capabilities.contains(&case.capability));
                let outcome = if let Some(case) = unsupported {
                    CandidateOutcome::Excluded {
                        reason: ExclusionReason::CapabilityMismatch,
                        message: Some(format!(
                            "{provider_key} does not support {} required by this route",
                            case.capability,
                        )),
                    }
                } else if let Some(case) = replay_disabled {
                    CandidateOutcome::Excluded {
                        reason: ExclusionReason::Unavailable,
                        message: Some(format!(
                            "direct replay is disabled for the required {} capability",
                            case.capability,
                        )),
                    }
                } else {
                    #[cfg(feature = "local-llama")]
                    self.local_model_pool.evict_all_idle().await;
                    let mut measurements = Vec::with_capacity(route_cases.len());
                    let mut failure = None;
                    for case in route_cases {
                        match measure_candidate(provider.provider.as_ref(), case).await {
                            Ok(measurement) => measurements.push(measurement),
                            Err(error) => {
                                failure = Some((case.capability.clone(), error));
                                break;
                            }
                        }
                    }
                    if let Some((capability, failure)) = failure {
                        CandidateOutcome::Excluded {
                            reason: failure.reason,
                            message: Some(format!("{capability}: {}", failure.message)),
                        }
                    } else {
                        aggregate_candidate_measurements(&measurements)
                    }
                };
                candidate_evaluations.push(CandidateEvaluation {
                    agent_id: route_key.clone(),
                    provider_key: provider_key.clone(),
                    outcome,
                });
            }
        }

        let cases = grouped_agent_cases(&grouped_cases);
        let coverage = coverage(&local_provider_keys, &cases, &candidate_evaluations);
        if !coverage.missing.is_empty() {
            return Err("Optimization candidate coverage is incomplete".to_string());
        }
        let plans = generate_combinations(&cases, &candidate_evaluations)
            .into_iter()
            .filter(|plan| {
                plan.kind != CombinationKind::Current
                    || plan.routes.values().all(|provider_key| {
                        providers
                            .get(provider_key)
                            .is_some_and(|provider| provider.replay_safe)
                    })
            })
            .collect::<Vec<_>>();
        let mut combinations = Vec::with_capacity(plans.len());
        for plan in plans {
            #[cfg(feature = "local-llama")]
            let evicted = self.local_model_pool.evict_all_idle().await;
            #[cfg(feature = "local-llama")]
            let _ = evicted;
            let residency_known = plan_reports_load_events(&plan, &providers);
            let first = measure_combination(&plan, &providers, &corpus).await;
            let mut repeats = Vec::with_capacity(REPEAT_RUNS);
            if first.is_ok() {
                for _ in 0..REPEAT_RUNS {
                    match measure_combination(&plan, &providers, &corpus).await {
                        Ok(measurement) => repeats.push(measurement),
                        Err(error) => {
                            repeats.clear();
                            combinations.push(failed_combination(plan.clone(), error.message));
                            break;
                        }
                    }
                }
            }
            if repeats.len() == REPEAT_RUNS {
                combinations.push(verified_combination(
                    plan,
                    first.expect("first run passed"),
                    &repeats,
                    residency_known,
                ));
            } else if let Err(error) = first {
                combinations.push(failed_combination(plan, error.message));
            }
        }
        let recommendation = combinations
            .iter()
            .filter(|row| row.outcome == BenchmarkOutcome::Passed)
            .filter_map(|row| {
                row.repeat_total_ms
                    .as_ref()
                    .map(|metric| (row, metric.median))
            })
            .min_by_key(|(_, median)| *median)
            .map(|(row, _)| row.plan.id.clone());

        let completed_at_ms = unix_timestamp_ms().max(started_at_ms);
        let mut report = OptimizationReport {
            comparison_id,
            started_at_ms,
            completed_at_ms,
            monotonic_duration_ms: elapsed_ms(comparison_started.elapsed()),
            coverage,
            candidate_evaluations,
            combinations,
            recommendation,
            corpus_agents: cases.len(),
            configured_local_models: local_provider_keys.len(),
            remote_fallbacks,
            route_capabilities: cases
                .iter()
                .map(|case| (case.agent_id.clone(), case.capability.clone()))
                .collect(),
            route_labels,
            device_backend,
            loaded_models_after: self.loaded_models_after(),
            history_project: None,
            history_deployment: None,
            history_saved: false,
            history_notice: None,
            not_exhaustive: true,
            sequential_replay: true,
            capture_issues,
        };
        let history_target = lock(&self.history_target).clone();
        persist_comparison_history(&mut report, history_target).await;
        Ok(report)
    }

    fn loaded_models_after(&self) -> Option<usize> {
        #[cfg(feature = "local-llama")]
        {
            Some(self.local_model_pool.loaded_count())
        }
        #[cfg(not(feature = "local-llama"))]
        {
            None
        }
    }
}

fn remote_fallbacks(providers: &BTreeMap<String, ConfiguredProvider>) -> Vec<RemoteFallback> {
    providers
        .iter()
        .filter(|(_, provider)| !provider.replay_safe)
        .map(|(provider_key, provider)| RemoteFallback {
            provider_key: provider_key.clone(),
            display_name: provider.display_name.clone(),
            capabilities: provider.capabilities.iter().cloned().collect(),
            models: provider.models.iter().cloned().collect(),
        })
        .collect()
}

fn cases_by_route(corpus: &[CapturedCase]) -> BTreeMap<String, Vec<&CapturedCase>> {
    let mut grouped = BTreeMap::<String, Vec<&CapturedCase>>::new();
    for case in corpus {
        grouped
            .entry(case.route_key.clone())
            .or_default()
            .push(case);
    }
    grouped
}

fn grouped_agent_cases(grouped: &BTreeMap<String, Vec<&CapturedCase>>) -> Vec<AgentCase> {
    grouped
        .iter()
        .filter_map(|(route_key, route_cases)| {
            let latest = route_cases.iter().max_by_key(|case| case.sequence)?;
            let capabilities = route_cases
                .iter()
                .map(|case| case.capability.as_str())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
                .join(" + ");
            Some(AgentCase {
                agent_id: route_key.clone(),
                capability: capabilities,
                current_provider_key: latest.provider_key.clone(),
            })
        })
        .collect()
}

fn aggregate_candidate_measurements(measurements: &[RunMeasurement]) -> CandidateOutcome {
    let total_ms = measurements
        .iter()
        .map(|measurement| measurement.total_ms)
        .fold(0_u64, u64::saturating_add);
    let ttft_ms = measurements
        .iter()
        .filter_map(|measurement| measurement.ttft_ms)
        .reduce(u64::saturating_add);
    let output_tokens = measurements
        .iter()
        .filter_map(|measurement| measurement.output_tokens)
        .sum::<u64>();
    let decode_ms = measurements
        .iter()
        .filter_map(|measurement| measurement.decode_ms)
        .sum::<u64>();
    CandidateOutcome::Passed {
        total_latency_ms: one_sample(total_ms),
        ttft_ms: ttft_ms.map(one_sample),
        tokens_per_second: (decode_ms > 0)
            .then_some(output_tokens as f64 / (decode_ms as f64 / 1000.0)),
        rss_delta_bytes: measurements
            .iter()
            .filter_map(|measurement| measurement.rss_delta_bytes)
            .max(),
    }
}

async fn persist_comparison_history(
    report: &mut OptimizationReport,
    history_target: Option<ComparisonHistoryTarget>,
) {
    if report.combinations.is_empty() {
        report.history_notice = Some(
            "Comparison completed locally; Dashboard history upload was skipped because no route combination was measured."
                .to_string(),
        );
        return;
    }
    let Some(history_target) = history_target else {
        report.history_notice = Some(
            "Comparison completed locally; Dashboard history upload was skipped because this Gateway has no active Dashboard authorization."
                .to_string(),
        );
        return;
    };
    let configuration = match tokio::time::timeout(
        HISTORY_REQUEST_TIMEOUT,
        history_target.client.configuration(),
    )
    .await
    {
        Ok(Ok(configuration)) => configuration,
        Ok(Err(error)) => {
            report.history_notice = Some(format!(
                "Comparison completed locally; Dashboard history upload was skipped: {}",
                safe_error_message(&error)
            ));
            return;
        }
        Err(_) => {
            report.history_notice = Some(
                "Comparison completed locally; Dashboard history upload was skipped because the runtime configuration request timed out."
                    .to_string(),
            );
            return;
        }
    };
    if configuration.gateway_id != history_target.gateway_id {
        report.history_notice = Some(
            "Comparison completed locally; Dashboard history upload was skipped because the authorized Gateway identity changed."
                .to_string(),
        );
        return;
    }
    let route_ids = report.combinations[0]
        .plan
        .routes
        .keys()
        .map(|route| Uuid::parse_str(route))
        .collect::<Result<BTreeSet<_>, _>>();
    let Ok(route_ids) = route_ids else {
        report.history_notice = Some(
            "Comparison completed locally; Dashboard history upload was skipped because the measured routes are not canonical binding IDs."
                .to_string(),
        );
        return;
    };
    let Some((deployment_id, project_slug, deployment_name)) =
        unique_primary_deployment(&configuration, &route_ids)
    else {
        report.history_notice = Some(
            "Comparison completed locally; Dashboard history upload was skipped because the corpus does not map to exactly one primary Project deployment."
                .to_string(),
        );
        return;
    };
    report.history_project = Some(project_slug);
    report.history_deployment = Some(deployment_name);

    let upload = match comparison_upload(report, deployment_id) {
        Ok(upload) => upload,
        Err(error) => {
            report.history_notice = Some(format!(
                "Comparison completed locally; Dashboard history upload was skipped: {}",
                safe_error_message(&error)
            ));
            return;
        }
    };
    match tokio::time::timeout(
        HISTORY_REQUEST_TIMEOUT,
        history_target.client.upload_comparison(&upload),
    )
    .await
    {
        Ok(Ok(comparison_id)) => {
            report.history_saved = true;
            report.history_notice = Some(format!(
                "Saved comparison {comparison_id} to Dashboard history"
            ));
        }
        Ok(Err(error)) => {
            report.history_notice = Some(format!(
                "Comparison completed locally; Dashboard history upload was skipped: {}",
                safe_error_message(&error)
            ));
        }
        Err(_) => {
            report.history_notice = Some(
                "Comparison completed locally; Dashboard history upload was skipped because the upload timed out."
                    .to_string(),
            );
        }
    }
}

fn unique_primary_deployment(
    configuration: &GatewayRuntimeConfiguration,
    route_ids: &BTreeSet<Uuid>,
) -> Option<(Uuid, String, String)> {
    let mut matching = configuration.deployments.iter().filter(|deployment| {
        deployment.is_primary
            && route_ids
                .iter()
                .all(|route_id| deployment.binding_ids.contains(route_id))
    });
    let deployment = matching.next()?;
    if matching.next().is_some() {
        return None;
    }
    Some((
        deployment.deployment_id,
        deployment.project_slug.clone(),
        deployment.deployment.clone(),
    ))
}

fn comparison_upload(
    report: &OptimizationReport,
    deployment_id: Uuid,
) -> Result<RuntimeComparisonUpload, String> {
    let corpus_agents = u32::try_from(report.corpus_agents)
        .map_err(|_| "comparison corpus count exceeds the upload contract".to_string())?;
    let configured_models = u32::try_from(report.configured_local_models)
        .map_err(|_| "configured model count exceeds the upload contract".to_string())?;
    let tested_models = u32::try_from(report.coverage.tested_models)
        .map_err(|_| "tested model count exceeds the upload contract".to_string())?;
    let passed_models = u32::try_from(report.coverage.passed_models)
        .map_err(|_| "passing model count exceeds the upload contract".to_string())?;
    let runs = report
        .combinations
        .iter()
        .map(|measured| {
            let (outcome, error) = match &measured.outcome {
                BenchmarkOutcome::Passed => (RuntimeComparisonOutcome::Passed, None),
                BenchmarkOutcome::Failed(error) => (
                    RuntimeComparisonOutcome::Failed,
                    Some(safe_error_message(error)),
                ),
            };
            let route_labels = measured
                .plan
                .routes
                .keys()
                .map(|route| {
                    report
                        .route_labels
                        .get(route)
                        .cloned()
                        .map(|label| (route.clone(), label))
                        .ok_or_else(|| format!("route {route} has no display label"))
                })
                .collect::<Result<BTreeMap<_, _>, _>>()?;
            Ok(RuntimeComparisonRunUpload {
                id: Uuid::new_v4(),
                combination_id: measured.plan.id.clone(),
                label: bounded_display_text(&measured.plan.label, 128)
                    .unwrap_or_else(|| "measured combination".to_string()),
                rule: bounded_display_text(&measured.plan.explanation, 512)
                    .unwrap_or_else(|| "generated from passing local candidates".to_string()),
                routes: measured.plan.routes.clone(),
                route_labels,
                outcome,
                first_total_ms: measured.first_total_ms,
                first_run_cold: measured.first_run_cold,
                repeat_runs_resident: measured.repeat_runs_resident,
                repeat_total: measured.repeat_total_ms.clone(),
                repeat_ttft: measured.repeat_ttft_ms.clone(),
                tokens_per_second: measured.tokens_per_second,
                first_process_cpu_percent: measured.first_process_cpu_percent,
                process_cpu_percent: measured.process_cpu_percent,
                peak_rss_bytes: measured.peak_rss_bytes,
                error,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(RuntimeComparisonUpload {
        id: report.comparison_id,
        deployment_id,
        status: RuntimeComparisonStatus::Completed,
        recommendation: report.recommendation.clone(),
        not_exhaustive: report.not_exhaustive,
        sequential_replay: report.sequential_replay,
        corpus_agents,
        configured_models,
        tested_models,
        passed_models,
        device: RuntimeComparisonDevice {
            architecture: std::env::consts::ARCH.to_string(),
            backend: report.device_backend.clone(),
            os: Some(std::env::consts::OS.to_string()),
        },
        started_at_ms: report.started_at_ms,
        completed_at_ms: Some(report.completed_at_ms),
        monotonic_duration_ms: report.monotonic_duration_ms,
        runs,
    })
}

fn comparison_backend(providers: &BTreeMap<String, ConfiguredProvider>) -> Option<String> {
    let backends = providers
        .values()
        .filter(|provider| provider.replay_safe)
        .filter_map(|provider| provider.local_kind.as_deref())
        .map(|kind| match kind {
            "llama" => "llama.cpp",
            "local-whisper" => "whisper.cpp",
            "openai-compatible" => "local OpenAI-compatible",
            other => other,
        })
        .collect::<BTreeSet<_>>();
    bounded_display_text(&backends.into_iter().collect::<Vec<_>>().join(" + "), 128)
}

fn comparison_route_labels(corpus: &[CapturedCase]) -> BTreeMap<String, String> {
    let mut route_names = BTreeMap::<String, (String, BTreeSet<String>)>::new();
    for case in corpus {
        let name = ["agentName", "profileName", "profileSlug"]
            .into_iter()
            .find_map(|key| case.binding.get(key).and_then(Value::as_str))
            .and_then(|name| bounded_display_text(name, 88))
            .filter(|name| safe_error_message(name) == *name)
            .or_else(|| {
                bounded_display_text(&case.agent_id, 88)
                    .filter(|name| safe_error_message(name) == *name)
            })
            .unwrap_or_else(|| format!("Agent {}", shorten_identifier(&case.route_key)));
        let capability =
            bounded_display_text(&case.capability, 32).unwrap_or_else(|| "unknown".to_string());
        let entry = route_names
            .entry(case.route_key.clone())
            .or_insert_with(|| (name, BTreeSet::new()));
        entry.1.insert(capability);
    }
    route_names
        .into_iter()
        .map(|(route, (name, capabilities))| {
            let capabilities = capabilities.into_iter().collect::<Vec<_>>().join(" + ");
            let label = bounded_display_text(&format!("{name} · {capabilities}"), 128)
                .unwrap_or_else(|| format!("Agent {}", shorten_identifier(&route)));
            (route, label)
        })
        .collect()
}

fn bounded_display_text(value: &str, max_bytes: usize) -> Option<String> {
    let mut output = String::new();
    let mut pending_space = false;
    for character in value.trim().chars() {
        if character.is_control() || character.is_whitespace() {
            pending_space = !output.is_empty();
            continue;
        }
        let needed = character.len_utf8() + usize::from(pending_space);
        if output.len().saturating_add(needed) > max_bytes {
            break;
        }
        if pending_space {
            output.push(' ');
            pending_space = false;
        }
        output.push(character);
    }
    (!output.is_empty()).then_some(output)
}

fn shorten_identifier(value: &str) -> String {
    value.chars().take(8).collect()
}

fn unix_timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| duration.as_millis().try_into().ok())
        .filter(|timestamp| *timestamp > 0)
        .unwrap_or(1)
}

fn elapsed_ms(duration: Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}

fn descriptor_provider_key(agent: &AgentDescriptor) -> &str {
    agent
        .metadata
        .get("providerKey")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(&agent.id)
}

fn validate_activation(
    state: &OptimizationState,
    combination: &RouteCombination,
) -> Result<(), String> {
    if combination.routes.is_empty() {
        return Err("route combination must contain at least one route".to_string());
    }
    let expected_routes = state
        .corpus
        .values()
        .map(|case| case.route_key.as_str())
        .collect::<BTreeSet<_>>();
    for route_key in &expected_routes {
        if !combination.routes.contains_key(*route_key) {
            return Err(format!(
                "route {route_key} is missing; rerun Optimize for the current corpus"
            ));
        }
    }
    for (route_key, provider_key) in &combination.routes {
        let route_cases = state
            .corpus
            .values()
            .filter(|case| case.route_key == *route_key)
            .collect::<Vec<_>>();
        if route_cases.is_empty() {
            return Err(format!(
                "route {route_key} is no longer in the successful request corpus"
            ));
        }
        let provider = state
            .providers
            .get(provider_key)
            .ok_or_else(|| format!("configured provider {provider_key} is unavailable"))?;
        for case in route_cases {
            if !provider.capabilities.contains(&case.capability) {
                return Err(format!(
                    "provider {provider_key} does not support {} for route {route_key}",
                    case.capability
                ));
            }
        }
    }
    Ok(())
}

fn validate_replay_plan(
    plan: &RouteCombination,
    providers: &BTreeMap<String, ConfiguredProvider>,
    corpus: &[CapturedCase],
) -> Result<(), RunFailure> {
    for case in corpus {
        let provider_key = plan.routes.get(&case.route_key).ok_or_else(|| RunFailure {
            reason: ExclusionReason::Unavailable,
            message: format!("{} has no route in {}", case.route_key, plan.label),
        })?;
        let provider = providers.get(provider_key).ok_or_else(|| RunFailure {
            reason: ExclusionReason::Unavailable,
            message: format!("configured provider {provider_key} is unavailable"),
        })?;
        if !provider.replay_safe {
            return Err(RunFailure {
                reason: ExclusionReason::Unavailable,
                message: format!(
                    "direct replay excluded provider {provider_key}: only explicit local llama and local-whisper providers are replayed"
                ),
            });
        }
        if !provider.replay_capabilities.contains(&case.capability) {
            return Err(RunFailure {
                reason: ExclusionReason::Unavailable,
                message: format!(
                    "direct replay excluded provider {provider_key} for {}: only declared local chat/embedding or local transcription execution is replayed",
                    case.capability,
                ),
            });
        }
        if !provider.capabilities.contains(&case.capability) {
            return Err(RunFailure {
                reason: ExclusionReason::CapabilityMismatch,
                message: format!(
                    "provider {provider_key} does not support {}",
                    case.capability
                ),
            });
        }
    }
    Ok(())
}

fn plan_reports_load_events(
    plan: &RouteCombination,
    providers: &BTreeMap<String, ConfiguredProvider>,
) -> bool {
    !plan.routes.is_empty()
        && plan.routes.values().all(|provider_key| {
            providers
                .get(provider_key)
                .and_then(|provider| provider.local_kind.as_deref())
                == Some("llama")
        })
}

#[derive(Debug)]
struct RunFailure {
    reason: ExclusionReason,
    message: String,
}

#[derive(Debug, Clone)]
struct RunMeasurement {
    total_ms: u64,
    ttft_ms: Option<u64>,
    decode_ms: Option<u64>,
    output_tokens: Option<u64>,
    peak_rss_bytes: Option<u64>,
    rss_delta_bytes: Option<u64>,
    load_observed: bool,
    process_cpu_percent: Option<f64>,
}

#[derive(Default)]
struct RunTelemetry {
    ttft_ms: Option<u64>,
    decode_ms: Option<u64>,
    output_tokens: Option<u64>,
    load_observed: bool,
}

impl RunTelemetry {
    fn observe(&mut self, event: ProviderEvent) {
        let ProviderEvent::StageCompleted {
            stage,
            elapsed_ms,
            metadata,
        } = event
        else {
            return;
        };
        match stage {
            ProviderStage::Load => {
                self.load_observed |=
                    metadata.get("resident").and_then(Value::as_bool) == Some(false);
            }
            ProviderStage::FirstToken => {
                self.ttft_ms = metadata.get("requestElapsedMs").and_then(Value::as_u64);
            }
            ProviderStage::Decode => {
                self.decode_ms = Some(elapsed_ms);
                self.output_tokens = metadata.get("outputTokens").and_then(Value::as_u64);
            }
            _ => {}
        }
    }
}

async fn measure_candidate(
    provider: &dyn AgentGatewayProvider,
    case: &CapturedCase,
) -> Result<RunMeasurement, RunFailure> {
    let telemetry = Arc::new(Mutex::new(RunTelemetry::default()));
    let telemetry_for_sink = Arc::clone(&telemetry);
    let started = Instant::now();
    let events = ProviderEventSink::from_fn(move |mut event| {
        if matches!(&event, ProviderEvent::OutputDelta { .. }) {
            return;
        }
        if let ProviderEvent::StageCompleted {
            stage: ProviderStage::FirstToken,
            metadata,
            ..
        } = &mut event
        {
            let request_elapsed_ms = started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
            if !metadata.is_object() {
                *metadata = serde_json::json!({});
            }
            if let Some(metadata) = metadata.as_object_mut() {
                metadata.insert(
                    "requestElapsedMs".to_string(),
                    Value::from(request_elapsed_ms),
                );
            }
        }
        lock(&telemetry_for_sink).observe(event);
    });
    let rss_start = current_rss_bytes();
    let mut peak_rss_bytes = rss_start;
    let invocation = tokio::time::timeout(
        case.timeout,
        provider.invoke_with_events(
            &case.agent_id,
            &case.binding,
            &case.input,
            case.timeout,
            events,
        ),
    );
    tokio::pin!(invocation);
    let mut sample_tick = tokio::time::interval(Duration::from_millis(20));
    sample_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let output = loop {
        tokio::select! {
            result = &mut invocation => break result,
            _ = sample_tick.tick() => {
                peak_rss_bytes = max_option(peak_rss_bytes, current_rss_bytes());
            }
        }
    }
    .map_err(|_| RunFailure {
        reason: ExclusionReason::Unavailable,
        message: format!(
            "candidate replay timed out after {}ms including model load",
            case.timeout.as_millis()
        ),
    })?
    .map_err(|error| RunFailure {
        reason: provider_failure_reason(&error.message),
        message: error.message,
    })?;
    peak_rss_bytes = max_option(peak_rss_bytes, current_rss_bytes());
    validate_contract(
        &case.capability,
        &case.input,
        &case.baseline_output,
        &output,
    )
    .map_err(|message| RunFailure {
        reason: ExclusionReason::ContractFailure,
        message,
    })?;
    let telemetry = lock(&telemetry);
    Ok(RunMeasurement {
        total_ms: started.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
        ttft_ms: telemetry.ttft_ms,
        decode_ms: telemetry.decode_ms,
        output_tokens: telemetry.output_tokens,
        peak_rss_bytes,
        rss_delta_bytes: peak_rss_bytes
            .zip(rss_start)
            .map(|(peak, start)| peak.saturating_sub(start)),
        load_observed: telemetry.load_observed,
        process_cpu_percent: None,
    })
}

async fn measure_combination(
    plan: &RouteCombination,
    providers: &BTreeMap<String, ConfiguredProvider>,
    corpus: &[CapturedCase],
) -> Result<RunMeasurement, RunFailure> {
    validate_replay_plan(plan, providers, corpus)?;
    let wall_started = Instant::now();
    let process_cpu_started = current_process_cpu_time();
    let mut total_ms = 0_u64;
    let mut ttft_ms = 0_u64;
    let mut has_ttft = false;
    let mut decode_ms = 0_u64;
    let mut output_tokens = 0_u64;
    let mut has_decode = false;
    let mut peak_rss_bytes = None;
    let mut load_observed = false;
    for case in corpus {
        let provider_key = plan.routes.get(&case.route_key).ok_or_else(|| RunFailure {
            reason: ExclusionReason::Unavailable,
            message: format!("{} has no route in {}", case.route_key, plan.label),
        })?;
        let provider = providers.get(provider_key).ok_or_else(|| RunFailure {
            reason: ExclusionReason::Unavailable,
            message: format!("configured provider {provider_key} is unavailable"),
        })?;
        let measurement = measure_candidate(provider.provider.as_ref(), case).await?;
        total_ms = total_ms.saturating_add(measurement.total_ms);
        if let Some(value) = measurement.ttft_ms {
            has_ttft = true;
            ttft_ms = ttft_ms.saturating_add(value);
        }
        if let (Some(decode), Some(tokens)) = (measurement.decode_ms, measurement.output_tokens) {
            has_decode = true;
            decode_ms = decode_ms.saturating_add(decode);
            output_tokens = output_tokens.saturating_add(tokens);
        }
        peak_rss_bytes = max_option(peak_rss_bytes, measurement.peak_rss_bytes);
        load_observed |= measurement.load_observed;
    }
    Ok(RunMeasurement {
        total_ms,
        ttft_ms: has_ttft.then_some(ttft_ms),
        decode_ms: has_decode.then_some(decode_ms),
        output_tokens: has_decode.then_some(output_tokens),
        peak_rss_bytes,
        rss_delta_bytes: None,
        load_observed,
        process_cpu_percent: process_cpu_percent(
            process_cpu_started,
            current_process_cpu_time(),
            wall_started.elapsed(),
        ),
    })
}

fn passed_combination(
    plan: RouteCombination,
    first: RunMeasurement,
    repeats: &[RunMeasurement],
    residency_known: bool,
) -> MeasuredCombination {
    let repeat_total_ms = metric_range(repeats.iter().map(|sample| sample.total_ms));
    let repeat_ttft_ms = metric_range(repeats.iter().filter_map(|sample| sample.ttft_ms));
    let output_tokens = repeats
        .iter()
        .filter_map(|sample| sample.output_tokens)
        .sum::<u64>();
    let decode_ms = repeats
        .iter()
        .filter_map(|sample| sample.decode_ms)
        .sum::<u64>();
    let tokens_per_second =
        (decode_ms > 0).then_some(output_tokens as f64 / (decode_ms as f64 / 1000.0));
    let repeat_process_cpu_percent = repeats
        .iter()
        .map(|sample| sample.process_cpu_percent)
        .collect::<Option<Vec<_>>>()
        .and_then(median_f64);
    MeasuredCombination {
        plan,
        outcome: BenchmarkOutcome::Passed,
        first_total_ms: Some(first.total_ms),
        first_run_cold: residency_known.then_some(first.load_observed),
        repeat_runs_resident: residency_known
            .then_some(repeats.iter().all(|sample| !sample.load_observed)),
        repeat_total_ms,
        repeat_ttft_ms,
        tokens_per_second,
        first_process_cpu_percent: first.process_cpu_percent,
        process_cpu_percent: repeat_process_cpu_percent,
        peak_rss_bytes: repeats
            .iter()
            .filter_map(|sample| sample.peak_rss_bytes)
            .max()
            .or(first.peak_rss_bytes),
    }
}

fn verified_combination(
    plan: RouteCombination,
    first: RunMeasurement,
    repeats: &[RunMeasurement],
    residency_known: bool,
) -> MeasuredCombination {
    if residency_known && !first.load_observed {
        return failed_combination(
            plan,
            "cold verification failed: the first run did not observe a model load".to_string(),
        );
    }
    if residency_known && repeats.iter().any(|sample| sample.load_observed) {
        return failed_combination(
            plan,
            "warm verification failed: a repeat run reloaded a model".to_string(),
        );
    }
    passed_combination(plan, first, repeats, residency_known)
}

fn failed_combination(plan: RouteCombination, message: String) -> MeasuredCombination {
    MeasuredCombination {
        plan,
        outcome: BenchmarkOutcome::Failed(message),
        first_total_ms: None,
        first_run_cold: None,
        repeat_runs_resident: None,
        repeat_total_ms: None,
        repeat_ttft_ms: None,
        tokens_per_second: None,
        first_process_cpu_percent: None,
        process_cpu_percent: None,
        peak_rss_bytes: None,
    }
}

fn validate_contract(
    capability: &str,
    request: &Value,
    baseline: &Value,
    candidate: &Value,
) -> Result<(), String> {
    match capability {
        "chat" => {
            let baseline_content = chat_text_content(baseline);
            let candidate_content = chat_text_content(candidate);
            let baseline_tool_calls = chat_tool_calls(baseline);
            let candidate_tool_calls = chat_tool_calls(candidate);
            if baseline_content
                .as_deref()
                .is_some_and(|content| !content.trim().is_empty())
                && candidate_content
                    .as_deref()
                    .is_none_or(|content| content.trim().is_empty())
            {
                return Err(
                    "chat output is missing assistant text required by the successful baseline"
                        .to_string(),
                );
            }
            if baseline_tool_calls.is_some_and(|calls| !calls.is_empty())
                && candidate_tool_calls.is_none_or(|calls| calls.is_empty())
            {
                return Err(
                    "chat output is missing tool calls required by the successful baseline"
                        .to_string(),
                );
            }
            if candidate_content.is_none()
                && candidate_tool_calls.is_none()
                && candidate.get("structured").is_none()
            {
                return Err(
                    "chat output has no compatible assistant content or tool calls".to_string(),
                );
            }
            if baseline.get("structured").is_some() && candidate.get("structured").is_none() {
                return Err("chat output is missing the requested structured value".to_string());
            }
            if let Some(response_format) = request
                .get("response_format")
                .or_else(|| request.get("responseFormat"))
            {
                validate_chat_response_format(
                    response_format,
                    candidate_content.as_deref().unwrap_or_default(),
                    candidate,
                )?;
            }
        }
        "embedding" => {
            let expected_rows = embedding_input_rows(request)
                .or_else(|| baseline.get("data").and_then(Value::as_array).map(Vec::len))
                .unwrap_or(1);
            let requested_encoding = request
                .get("encoding_format")
                .or_else(|| request.get("encodingFormat"))
                .and_then(Value::as_str);
            let baseline_dimensions = embedding_dimensions(baseline, expected_rows, None)?;
            let candidate_dimensions =
                embedding_dimensions(candidate, expected_rows, requested_encoding)?;
            if baseline_dimensions != candidate_dimensions {
                return Err(format!(
                    "embedding dimension changed from {} to {candidate_dimensions}",
                    baseline_dimensions
                ));
            }
        }
        "transcription" => {
            let candidate_text =
                candidate
                    .get("text")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        "transcription output text is missing or is not a string".to_string()
                    })?;
            if baseline
                .get("text")
                .and_then(Value::as_str)
                .is_some_and(|text| !text.trim().is_empty())
                && candidate_text.trim().is_empty()
            {
                return Err(
                    "transcription output is empty but the successful baseline contains speech"
                        .to_string(),
                );
            }
        }
        _ if json_kind(baseline) != json_kind(candidate) => {
            return Err("candidate output changed the top-level JSON type".to_string());
        }
        _ => {}
    }
    Ok(())
}

fn chat_text_content(response: &Value) -> Option<String> {
    let content = response
        .pointer("/choices/0/message/content")
        .or_else(|| response.get("text"))?;
    match content {
        Value::String(text) => Some(text.clone()),
        Value::Array(parts) => Some(
            parts
                .iter()
                .filter_map(|part| {
                    part.as_str()
                        .or_else(|| part.get("text").and_then(Value::as_str))
                        .or_else(|| part.pointer("/text/value").and_then(Value::as_str))
                })
                .collect::<Vec<_>>()
                .join(""),
        ),
        Value::Null => Some(String::new()),
        _ => None,
    }
}

fn chat_tool_calls(response: &Value) -> Option<&[Value]> {
    response
        .pointer("/choices/0/message/tool_calls")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
}

fn validate_chat_response_format(
    response_format: &Value,
    content: &str,
    candidate: &Value,
) -> Result<(), String> {
    let format_type = response_format
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !matches!(format_type, "json_object" | "json_schema") {
        return Ok(());
    }
    let parsed = candidate
        .get("structured")
        .cloned()
        .map(Ok)
        .unwrap_or_else(|| serde_json::from_str(content))
        .map_err(|_| {
            "chat output does not satisfy the requested JSON response format".to_string()
        })?;
    if format_type == "json_schema" {
        if let Some(schema) = response_format
            .pointer("/json_schema/schema")
            .or_else(|| response_format.pointer("/jsonSchema/schema"))
            .or_else(|| response_format.get("schema"))
        {
            validate_simple_json_schema(schema, &parsed, "$")?;
        }
    }
    Ok(())
}

fn validate_simple_json_schema(schema: &Value, value: &Value, path: &str) -> Result<(), String> {
    if let Some(allowed) = schema.get("enum").and_then(Value::as_array) {
        if !allowed.contains(value) {
            return Err(format!("chat structured output violates enum at {path}"));
        }
    }
    if let Some(kind) = schema.get("type").and_then(Value::as_str) {
        let matches = match kind {
            "object" => value.is_object(),
            "array" => value.is_array(),
            "string" => value.is_string(),
            "number" => value.as_f64().is_some_and(f64::is_finite),
            "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
            "boolean" => value.is_boolean(),
            "null" => value.is_null(),
            _ => true,
        };
        if !matches {
            return Err(format!(
                "chat structured output has the wrong type at {path}"
            ));
        }
    }
    if let Some(object) = value.as_object() {
        if let Some(required) = schema.get("required").and_then(Value::as_array) {
            for required in required.iter().filter_map(Value::as_str) {
                if !object.contains_key(required) {
                    return Err(format!(
                        "chat structured output is missing required field {path}.{required}"
                    ));
                }
            }
        }
        if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
            for (name, property_schema) in properties {
                if let Some(property) = object.get(name) {
                    validate_simple_json_schema(
                        property_schema,
                        property,
                        &format!("{path}.{name}"),
                    )?;
                }
            }
        }
    }
    if let (Some(values), Some(item_schema)) = (value.as_array(), schema.get("items")) {
        for (index, item) in values.iter().enumerate() {
            validate_simple_json_schema(item_schema, item, &format!("{path}[{index}]"))?;
        }
    }
    Ok(())
}

fn embedding_input_rows(request: &Value) -> Option<usize> {
    let input = request.get("input")?;
    match input {
        Value::String(_) => Some(1),
        Value::Array(values) if values.first().is_some_and(Value::is_number) => Some(1),
        Value::Array(values) => Some(values.len()),
        _ => None,
    }
}

fn embedding_dimensions(
    value: &Value,
    expected_rows: usize,
    requested_encoding: Option<&str>,
) -> Result<usize, String> {
    let rows = value
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| "embedding output has no data rows".to_string())?;
    if rows.len() != expected_rows {
        return Err(format!(
            "embedding output returned {} rows for {expected_rows} inputs",
            rows.len()
        ));
    }
    let mut dimensions = None;
    for (index, row) in rows.iter().enumerate() {
        let embedding = row
            .get("embedding")
            .ok_or_else(|| format!("embedding row {index} has no vector"))?;
        let row_dimensions = if let Some(values) = embedding.as_array() {
            if requested_encoding == Some("base64") {
                return Err(format!("embedding row {index} is not base64 encoded"));
            }
            if values.is_empty()
                || values
                    .iter()
                    .any(|value| !value.as_f64().is_some_and(f64::is_finite))
            {
                return Err(format!("embedding row {index} has an invalid float vector"));
            }
            values.len()
        } else if let Some(encoded) = embedding.as_str() {
            if requested_encoding == Some("float") {
                return Err(format!("embedding row {index} is not a float array"));
            }
            decode_base64_float32_dimensions(encoded)
                .map_err(|error| format!("embedding row {index} {error}"))?
        } else {
            return Err(format!("embedding row {index} has an invalid vector type"));
        };
        if let Some(expected) = dimensions {
            if expected != row_dimensions {
                return Err(format!(
                    "embedding row {index} changed dimension from {expected} to {row_dimensions}"
                ));
            }
        } else {
            dimensions = Some(row_dimensions);
        }
    }
    dimensions.ok_or_else(|| "embedding output has no usable vector".to_string())
}

fn decode_base64_float32_dimensions(encoded: &str) -> Result<usize, String> {
    let bytes = encoded.as_bytes();
    if bytes.is_empty() || !bytes.len().is_multiple_of(4) {
        return Err("has invalid base64 float32 data".to_string());
    }
    let mut decoded = Vec::with_capacity(bytes.len() / 4 * 3);
    for (chunk_index, chunk) in bytes.chunks_exact(4).enumerate() {
        let last = chunk_index + 1 == bytes.len() / 4;
        let padding = chunk.iter().rev().take_while(|byte| **byte == b'=').count();
        if padding > 2 || (!last && padding > 0) || chunk[..4 - padding].contains(&b'=') {
            return Err("has invalid base64 padding".to_string());
        }
        let a = base64_value(chunk[0])?;
        let b = base64_value(chunk[1])?;
        let c = if padding >= 2 {
            0
        } else {
            base64_value(chunk[2])?
        };
        let d = if padding >= 1 {
            0
        } else {
            base64_value(chunk[3])?
        };
        decoded.push((a << 2) | (b >> 4));
        if padding < 2 {
            decoded.push((b << 4) | (c >> 2));
        }
        if padding == 0 {
            decoded.push((c << 6) | d);
        }
    }
    if decoded.is_empty() || decoded.len() % std::mem::size_of::<f32>() != 0 {
        return Err("does not contain whole float32 values".to_string());
    }
    for bytes in decoded.chunks_exact(4) {
        let value = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        if !value.is_finite() {
            return Err("contains a non-finite float32 value".to_string());
        }
    }
    Ok(decoded.len() / std::mem::size_of::<f32>())
}

fn base64_value(byte: u8) -> Result<u8, String> {
    match byte {
        b'A'..=b'Z' => Ok(byte - b'A'),
        b'a'..=b'z' => Ok(byte - b'a' + 26),
        b'0'..=b'9' => Ok(byte - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        _ => Err("has invalid base64 characters".to_string()),
    }
}

fn json_kind(value: &Value) -> u8 {
    match value {
        Value::Null => 0,
        Value::Bool(_) => 1,
        Value::Number(_) => 2,
        Value::String(_) => 3,
        Value::Array(_) => 4,
        Value::Object(_) => 5,
    }
}

fn provider_failure_reason(message: &str) -> ExclusionReason {
    let message = message.to_ascii_lowercase();
    if message.contains("memory") || message.contains("alloc") {
        ExclusionReason::InsufficientMemory
    } else if message.contains("model") || message.contains("load") || message.contains("backend") {
        ExclusionReason::LoadFailure
    } else {
        ExclusionReason::Unavailable
    }
}

fn one_sample(value: u64) -> MetricRange {
    MetricRange {
        median: value,
        min: value,
        max: value,
        samples: 1,
    }
}

fn metric_range(values: impl Iterator<Item = u64>) -> Option<MetricRange> {
    let mut values = values.collect::<Vec<_>>();
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    Some(MetricRange {
        median: values[values.len() / 2],
        min: values[0],
        max: values[values.len() - 1],
        samples: values.len().try_into().unwrap_or(u32::MAX),
    })
}

fn median_f64(mut values: Vec<f64>) -> Option<f64> {
    if values.is_empty()
        || values
            .iter()
            .any(|value| !value.is_finite() || *value < 0.0)
    {
        return None;
    }
    values.sort_by(f64::total_cmp);
    Some(values[values.len() / 2])
}

fn process_cpu_percent(
    started: Option<Duration>,
    completed: Option<Duration>,
    wall_elapsed: Duration,
) -> Option<f64> {
    let process_elapsed = completed?.checked_sub(started?)?;
    let wall_seconds = wall_elapsed.as_secs_f64();
    if wall_seconds <= 0.0 {
        return None;
    }
    let percent = process_elapsed.as_secs_f64() / wall_seconds * 100.0;
    (percent.is_finite() && percent >= 0.0).then_some(percent)
}

fn max_option(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (left, right) => left.or(right),
    }
}

fn corpus_key(agent_id: &str, capability: &str) -> String {
    format!("{agent_id}\u{0}{capability}")
}

fn captured_value_bytes(binding: &Value, input: &Value, output: Option<&Value>) -> usize {
    serde_json::to_vec(binding)
        .map_or(0, |value| value.len())
        .saturating_add(serde_json::to_vec(input).map_or(0, |value| value.len()))
        .saturating_add(
            output
                .and_then(|value| serde_json::to_vec(value).ok())
                .map_or(0, |value| value.len()),
        )
}

fn trim_corpus(corpus: &mut BTreeMap<String, CapturedCase>) -> Vec<CapturedCase> {
    let mut evicted = Vec::new();
    while corpus.len() > MAX_CAPTURED_CASES
        || corpus
            .values()
            .map(|case| case.captured_bytes)
            .sum::<usize>()
            > MAX_CAPTURED_BYTES
    {
        let Some(oldest) = corpus
            .iter()
            .min_by_key(|(_, case)| case.sequence)
            .map(|(key, _)| key.clone())
        else {
            break;
        };
        if let Some(case) = corpus.remove(&oldest) {
            evicted.push(case);
        }
    }
    evicted
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;

    use serde_json::json;
    use std::collections::BTreeMap;
    use std::time::Duration;
    use vifu_gateway::optimization::{
        CandidateOutcome, CombinationKind, RouteCombination, SessionRouteOverrides,
    };
    use vifu_gateway::protocol::AgentDescriptor;
    use vifu_gateway::relay::{
        AgentGatewayProvider, GatewayCaptureEvent, GatewayProviderError, ProviderEvent,
        ProviderStage,
    };

    use super::{
        comparison_route_labels, corpus_key, lock, measure_candidate, median_f64, metric_range,
        process_cpu_percent, trim_corpus, unique_primary_deployment, validate_contract,
        validate_replay_plan, verified_combination, BenchmarkOutcome, CapturedCase,
        ConfiguredProvider, OptimizationController, RunMeasurement, RunTelemetry,
    };

    struct MockProvider {
        id: &'static str,
    }

    struct SlowProvider;

    struct MultiCapabilityProvider {
        id: &'static str,
        embedding_valid: bool,
    }

    impl AgentGatewayProvider for MockProvider {
        fn id(&self) -> &str {
            self.id
        }

        fn provider_type(&self) -> &str {
            "test"
        }

        fn invoke<'a>(
            &'a self,
            _agent_id: &'a str,
            _binding: &'a serde_json::Value,
            _input: &'a serde_json::Value,
            _timeout: Duration,
        ) -> Pin<
            Box<dyn Future<Output = Result<serde_json::Value, GatewayProviderError>> + Send + 'a>,
        > {
            Box::pin(async { Ok(json!({"choices": [{"message": {"content": "ok"}}]})) })
        }
    }

    impl AgentGatewayProvider for SlowProvider {
        fn id(&self) -> &str {
            "slow-local"
        }

        fn provider_type(&self) -> &str {
            "test"
        }

        fn invoke<'a>(
            &'a self,
            _agent_id: &'a str,
            _binding: &'a serde_json::Value,
            _input: &'a serde_json::Value,
            _timeout: Duration,
        ) -> Pin<
            Box<dyn Future<Output = Result<serde_json::Value, GatewayProviderError>> + Send + 'a>,
        > {
            Box::pin(async {
                tokio::time::sleep(Duration::from_millis(50)).await;
                Ok(json!({"choices": [{"message": {"content": "late"}}]}))
            })
        }
    }

    impl AgentGatewayProvider for MultiCapabilityProvider {
        fn id(&self) -> &str {
            self.id
        }

        fn provider_type(&self) -> &str {
            "test"
        }

        fn invoke<'a>(
            &'a self,
            _agent_id: &'a str,
            binding: &'a serde_json::Value,
            _input: &'a serde_json::Value,
            _timeout: Duration,
        ) -> Pin<
            Box<dyn Future<Output = Result<serde_json::Value, GatewayProviderError>> + Send + 'a>,
        > {
            Box::pin(async move {
                if binding
                    .get("capability")
                    .and_then(serde_json::Value::as_str)
                    == Some("embedding")
                {
                    if self.embedding_valid {
                        Ok(json!({"data": [{"embedding": [1.0, 2.0]}]}))
                    } else {
                        Ok(json!({"data": [{"embedding": [1.0]}]}))
                    }
                } else {
                    Ok(json!({"choices": [{"message": {"content": "ok"}}]}))
                }
            })
        }
    }

    fn controller() -> OptimizationController {
        let route_overrides = Arc::new(SessionRouteOverrides::default());
        OptimizationController::new(
            route_overrides,
            #[cfg(feature = "local-llama")]
            super::LocalModelPool::for_device(),
        )
    }

    fn route_plan(route: &str, provider: &str) -> RouteCombination {
        RouteCombination {
            id: "test".to_string(),
            label: "test".to_string(),
            kind: CombinationKind::Current,
            explanation: "test plan".to_string(),
            routes: BTreeMap::from([(route.to_string(), provider.to_string())]),
        }
    }

    fn load_measurement(load_observed: bool) -> RunMeasurement {
        RunMeasurement {
            total_ms: 10,
            ttft_ms: Some(2),
            decode_ms: Some(5),
            output_tokens: Some(2),
            peak_rss_bytes: Some(1024),
            rss_delta_bytes: Some(512),
            load_observed,
            process_cpu_percent: Some(100.0),
        }
    }

    #[test]
    fn repeat_metric_range_uses_measured_median_and_range() {
        let metric = metric_range([40, 10, 30].into_iter()).unwrap();
        assert_eq!(metric.median, 30);
        assert_eq!(metric.min, 10);
        assert_eq!(metric.max, 40);
        assert_eq!(metric.samples, 3);
    }

    #[test]
    fn load_telemetry_requires_a_cold_first_run() {
        let measured = verified_combination(
            route_plan("route", "local"),
            load_measurement(false),
            &[
                load_measurement(false),
                load_measurement(false),
                load_measurement(false),
            ],
            true,
        );

        assert!(matches!(
            measured.outcome,
            BenchmarkOutcome::Failed(ref message) if message.contains("first run")
        ));
    }

    #[test]
    fn load_telemetry_requires_resident_repeat_runs() {
        let measured = verified_combination(
            route_plan("route", "local"),
            load_measurement(true),
            &[
                load_measurement(false),
                load_measurement(true),
                load_measurement(false),
            ],
            true,
        );

        assert!(matches!(
            measured.outcome,
            BenchmarkOutcome::Failed(ref message) if message.contains("repeat run")
        ));
    }

    #[test]
    fn providers_without_load_telemetry_remain_comparable_without_cold_claims() {
        let measured = verified_combination(
            route_plan("route", "local-http"),
            load_measurement(false),
            &[
                load_measurement(false),
                load_measurement(false),
                load_measurement(false),
            ],
            false,
        );

        assert_eq!(measured.outcome, BenchmarkOutcome::Passed);
        assert_eq!(measured.first_run_cold, None);
        assert_eq!(measured.repeat_runs_resident, None);
    }

    #[test]
    fn process_cpu_uses_the_full_wall_window_and_warm_median() {
        assert_eq!(
            process_cpu_percent(
                Some(Duration::from_secs(1)),
                Some(Duration::from_secs(3)),
                Duration::from_secs(1),
            ),
            Some(200.0)
        );
        assert_eq!(median_f64(vec![180.0, 90.0, 140.0]), Some(140.0));
        assert_eq!(median_f64(vec![90.0, f64::NAN]), None);
    }

    #[test]
    fn route_labels_prefer_binding_profile_names() {
        let route_id = uuid::Uuid::new_v4();
        let corpus = vec![CapturedCase {
            sequence: 1,
            route_key: route_id.to_string(),
            agent_id: "provider-agent-17".to_string(),
            provider_key: "local".to_string(),
            capability: "chat".to_string(),
            binding: Arc::new(json!({"profileName": "NPC Planner"})),
            input: Arc::new(json!({})),
            baseline_output: Arc::new(json!({})),
            timeout: Duration::from_secs(1),
            captured_bytes: 1,
        }];

        assert_eq!(
            comparison_route_labels(&corpus).get(&route_id.to_string()),
            Some(&"NPC Planner · chat".to_string())
        );
    }

    #[test]
    fn dashboard_history_requires_exactly_one_matching_primary_deployment() {
        let route_id = uuid::Uuid::new_v4();
        let deployment = |name: &str| {
            json!({
                "deploymentId": uuid::Uuid::new_v4(),
                "deployment": name,
                "projectId": uuid::Uuid::new_v4(),
                "projectSlug": format!("project-{name}"),
                "projectName": name,
                "isPrimary": true,
                "bindingIds": [route_id],
                "policies": {
                    "configSync": false,
                    "traceMode": "off",
                    "remoteInvocation": false
                },
                "release": null
            })
        };
        let parse = |deployments: Vec<serde_json::Value>| {
            serde_json::from_value::<vifu_gateway::control::GatewayRuntimeConfiguration>(json!({
                "gatewayId": "gateway-test",
                "deployments": deployments
            }))
            .unwrap()
        };
        let routes = [route_id].into_iter().collect();

        assert!(unique_primary_deployment(&parse(Vec::new()), &routes).is_none());
        let one = unique_primary_deployment(&parse(vec![deployment("a")]), &routes).unwrap();
        assert_eq!(one.2, "a");
        assert!(
            unique_primary_deployment(&parse(vec![deployment("a"), deployment("b")]), &routes)
                .is_none()
        );
    }

    #[test]
    fn telemetry_uses_resident_metadata_and_request_elapsed_ttft() {
        let mut telemetry = RunTelemetry::default();
        telemetry.observe(ProviderEvent::StageStarted {
            stage: ProviderStage::Load,
            metadata: json!({}),
        });
        telemetry.observe(ProviderEvent::StageCompleted {
            stage: ProviderStage::Load,
            elapsed_ms: 2,
            metadata: json!({"resident": true}),
        });
        telemetry.observe(ProviderEvent::StageCompleted {
            stage: ProviderStage::FirstToken,
            elapsed_ms: 3,
            metadata: json!({}),
        });
        assert!(!telemetry.load_observed);
        assert_eq!(telemetry.ttft_ms, None);

        telemetry.observe(ProviderEvent::StageCompleted {
            stage: ProviderStage::FirstToken,
            elapsed_ms: 4,
            metadata: json!({"requestElapsedMs": 41}),
        });
        telemetry.observe(ProviderEvent::StageCompleted {
            stage: ProviderStage::Load,
            elapsed_ms: 5,
            metadata: json!({"resident": false}),
        });
        assert!(telemetry.load_observed);
        assert_eq!(telemetry.ttft_ms, Some(41));
    }

    #[test]
    fn contract_preserves_successful_chat_shape_and_embedding_dimensions() {
        assert!(validate_contract(
            "chat",
            &json!({}),
            &json!({"choices": [{"message": {"content": "ok"}}]}),
            &json!({"choices": []}),
        )
        .is_err());
        assert!(validate_contract(
            "chat",
            &json!({}),
            &json!({
                "choices": [{
                    "message": {
                        "content": null,
                        "tool_calls": [{"type": "function"}]
                    }
                }]
            }),
            &json!({
                "choices": [{
                    "message": {
                        "content": null,
                        "tool_calls": [{"type": "function"}]
                    }
                }]
            }),
        )
        .is_ok());
        assert!(validate_contract(
            "chat",
            &json!({}),
            &json!({"choices": [{"message": {"content": ""}}]}),
            &json!({"choices": [{"message": {"content": ""}}]}),
        )
        .is_ok());
        assert!(validate_contract(
            "embedding",
            &json!({"input": "hello"}),
            &json!({"data": [{"embedding": [1.0, 2.0]}]}),
            &json!({"data": [{"embedding": [1.0]}]}),
        )
        .is_err());
    }

    #[test]
    fn transcription_contract_allows_silence_but_not_lost_baseline_speech() {
        assert!(validate_contract(
            "transcription",
            &json!({}),
            &json!({"text": ""}),
            &json!({"text": ""}),
        )
        .is_ok());
        assert!(validate_contract(
            "transcription",
            &json!({}),
            &json!({"text": "hello"}),
            &json!({"text": ""}),
        )
        .is_err());
    }

    #[test]
    fn corpus_keeps_the_latest_success_for_each_agent_capability() {
        let mut corpus = BTreeMap::new();
        for sequence in 0..130_u64 {
            corpus.insert(
                format!("agent-{sequence}"),
                CapturedCase {
                    sequence,
                    route_key: format!("profile-{sequence}"),
                    agent_id: format!("agent-{sequence}"),
                    provider_key: "local".to_string(),
                    capability: "chat".to_string(),
                    binding: Arc::new(json!({})),
                    input: Arc::new(json!({})),
                    baseline_output: Arc::new(json!({})),
                    timeout: Duration::from_secs(1),
                    captured_bytes: 1,
                },
            );
        }
        trim_corpus(&mut corpus);
        assert_eq!(corpus.len(), 128);
        assert!(!corpus.contains_key("agent-0"));
        assert!(!corpus.contains_key("agent-1"));
    }

    #[test]
    fn in_flight_capture_admission_is_bounded_by_count_and_bytes() {
        let count_controller = controller();
        lock(&count_controller.state).config_epoch = 1;
        let capture = |input: serde_json::Value| GatewayCaptureEvent::InvocationStarted {
            config_epoch: 1,
            request_id: uuid::Uuid::new_v4(),
            binding_id: uuid::Uuid::new_v4(),
            agent_id: "agent".to_string(),
            provider_key: "local".to_string(),
            capability: "chat".to_string(),
            binding: Arc::new(json!({"capability": "chat"})),
            input: Arc::new(input),
            timeout_ms: 1_000,
        };
        for _ in 0..super::MAX_IN_FLIGHT_CAPTURES {
            assert_eq!(
                count_controller.capture(capture(json!({"input": "small"}))),
                super::CaptureDisposition::Captured
            );
        }
        assert_eq!(
            count_controller.capture(capture(json!({"input": "one too many"}))),
            super::CaptureDisposition::Dropped
        );
        assert_eq!(
            lock(&count_controller.state).in_flight.len(),
            super::MAX_IN_FLIGHT_CAPTURES
        );

        let byte_controller = controller();
        lock(&byte_controller.state).config_epoch = 1;
        assert_eq!(
            byte_controller.capture(capture(json!({
                "input": "x".repeat(5 * 1024 * 1024)
            }))),
            super::CaptureDisposition::Captured
        );
        assert_eq!(
            byte_controller.capture(capture(json!({
                "input": "x".repeat(4 * 1024 * 1024)
            }))),
            super::CaptureDisposition::Dropped
        );
        let state = lock(&byte_controller.state);
        assert!(
            state
                .in_flight
                .values()
                .map(|item| item.captured_bytes)
                .sum::<usize>()
                <= super::MAX_IN_FLIGHT_CAPTURE_BYTES
        );
        assert!(state
            .capture_issues
            .values()
            .any(|issue| issue.message.contains("in-flight budget")));
    }

    #[test]
    fn configure_uses_provider_key_and_unions_agent_capabilities() {
        let controller = controller();
        let provider: Arc<dyn AgentGatewayProvider> = Arc::new(MockProvider { id: "shared-local" });
        controller.configure(
            &[provider],
            &[
                AgentDescriptor {
                    id: "chat-agent".to_string(),
                    name: "Chat".to_string(),
                    metadata: json!({
                        "providerKey": "shared-local",
                        "localProviderType": "llama",
                        "capabilities": ["chat"]
                    }),
                },
                AgentDescriptor {
                    id: "embedding-agent".to_string(),
                    name: "Embedding".to_string(),
                    metadata: json!({
                        "providerKey": "shared-local",
                        "localProviderType": "llama",
                        "capabilities": ["embedding"]
                    }),
                },
            ],
        );

        let state = lock(&controller.state);
        let configured = &state.providers["shared-local"];
        assert!(configured.replay_safe);
        assert_eq!(
            configured.capabilities,
            ["chat".to_string(), "embedding".to_string()]
                .into_iter()
                .collect()
        );
    }

    #[test]
    fn activation_rejects_unknown_routes_providers_and_capability_mismatches() {
        let controller = controller();
        let provider: Arc<dyn AgentGatewayProvider> = Arc::new(MockProvider { id: "local-chat" });
        controller.configure(
            &[provider],
            &[AgentDescriptor {
                id: "chat-agent".to_string(),
                name: "Chat".to_string(),
                metadata: json!({
                    "providerKey": "local-chat",
                    "localProviderType": "llama",
                    "capabilities": ["chat"]
                }),
            }],
        );
        let route = uuid::Uuid::from_u128(1).to_string();
        lock(&controller.state).corpus.insert(
            corpus_key(&route, "chat"),
            CapturedCase {
                sequence: 1,
                route_key: route.to_string(),
                agent_id: "chat-agent".to_string(),
                provider_key: "local-chat".to_string(),
                capability: "chat".to_string(),
                binding: Arc::new(json!({"capability": "chat"})),
                input: Arc::new(json!({})),
                baseline_output: Arc::new(json!({"choices": [{"message": {"content": "ok"}}]})),
                timeout: Duration::from_secs(1),
                captured_bytes: 1,
            },
        );

        assert!(controller
            .activate(&route_plan("unknown-route", "local-chat"))
            .unwrap_err()
            .contains("missing"));
        assert!(controller
            .activate(&route_plan(&route, "missing-provider"))
            .unwrap_err()
            .contains("unavailable"));
        assert!(controller
            .activate(&route_plan(&route, "local-chat"))
            .is_ok());

        lock(&controller.state)
            .corpus
            .get_mut(&corpus_key(&route, "chat"))
            .unwrap()
            .capability = "embedding".to_string();
        assert!(controller
            .activate(&route_plan(&route, "local-chat"))
            .unwrap_err()
            .contains("does not support embedding"));
    }

    #[test]
    fn transient_provider_refresh_preserves_session_route_activation() {
        let controller = controller();
        let provider: Arc<dyn AgentGatewayProvider> = Arc::new(MockProvider { id: "local-chat" });
        let descriptor = AgentDescriptor {
            id: "chat-agent".to_string(),
            name: "Chat".to_string(),
            metadata: json!({
                "providerKey": "local-chat",
                "localProviderType": "openai-compatible",
                "executionLocation": "local",
                "capabilities": ["chat"]
            }),
        };
        controller.configure(
            std::slice::from_ref(&provider),
            std::slice::from_ref(&descriptor),
        );
        let routes = BTreeMap::from([("route-a".to_string(), "local-chat".to_string())]);
        controller.route_overrides.activate(routes.clone()).unwrap();

        controller.refresh_providers(
            std::slice::from_ref(&provider),
            std::slice::from_ref(&descriptor),
        );

        assert_eq!(controller.route_overrides.snapshot().routes, routes);
        controller.configure(&[provider], &[descriptor]);
        assert!(controller.route_overrides.snapshot().routes.is_empty());
    }

    #[tokio::test]
    async fn replay_timeout_covers_the_entire_provider_future() {
        let case = CapturedCase {
            sequence: 1,
            route_key: "route".to_string(),
            agent_id: "chat-agent".to_string(),
            provider_key: "slow-local".to_string(),
            capability: "chat".to_string(),
            binding: Arc::new(json!({"capability": "chat"})),
            input: Arc::new(json!({})),
            baseline_output: Arc::new(json!({"choices": [{"message": {"content": "ok"}}]})),
            timeout: Duration::from_millis(5),
            captured_bytes: 1,
        };

        let error = measure_candidate(&SlowProvider, &case).await.unwrap_err();

        assert!(error.message.contains("including model load"));
    }

    #[tokio::test]
    async fn multi_capability_routes_only_admit_providers_that_pass_every_capability() {
        let controller = controller();
        let partial: Arc<dyn AgentGatewayProvider> = Arc::new(MultiCapabilityProvider {
            id: "partial",
            embedding_valid: false,
        });
        let complete: Arc<dyn AgentGatewayProvider> = Arc::new(MultiCapabilityProvider {
            id: "complete",
            embedding_valid: true,
        });
        let descriptors = ["partial", "complete"].map(|provider| AgentDescriptor {
            id: provider.to_string(),
            name: provider.to_string(),
            metadata: json!({
                "providerKey": provider,
                "localProviderType": "openai-compatible",
                "executionLocation": "local",
                "capabilities": ["chat", "embedding"]
            }),
        });
        controller.configure(&[partial, complete], &descriptors);
        let route = uuid::Uuid::new_v4().to_string();
        {
            let mut state = lock(&controller.state);
            state.corpus.insert(
                corpus_key(&route, "chat"),
                CapturedCase {
                    sequence: 1,
                    route_key: route.clone(),
                    agent_id: "multi-agent".to_string(),
                    provider_key: "partial".to_string(),
                    capability: "chat".to_string(),
                    binding: Arc::new(json!({"capability": "chat", "profileName": "Multi"})),
                    input: Arc::new(json!({})),
                    baseline_output: Arc::new(json!({"choices": [{"message": {"content": "ok"}}]})),
                    timeout: Duration::from_secs(1),
                    captured_bytes: 1,
                },
            );
            state.corpus.insert(
                corpus_key(&route, "embedding"),
                CapturedCase {
                    sequence: 2,
                    route_key: route.clone(),
                    agent_id: "multi-agent".to_string(),
                    provider_key: "partial".to_string(),
                    capability: "embedding".to_string(),
                    binding: Arc::new(json!({"capability": "embedding", "profileName": "Multi"})),
                    input: Arc::new(json!({"input": "hello"})),
                    baseline_output: Arc::new(json!({"data": [{"embedding": [1.0, 2.0]}]})),
                    timeout: Duration::from_secs(1),
                    captured_bytes: 1,
                },
            );
        }

        let report = controller.benchmark().await.unwrap();

        assert_eq!(report.coverage.expected_pairs, 2);
        assert_eq!(report.coverage.evaluated_pairs, 2);
        assert!(report
            .candidate_evaluations
            .iter()
            .find(|evaluation| evaluation.provider_key == "partial")
            .is_some_and(|evaluation| {
                matches!(evaluation.outcome, CandidateOutcome::Excluded { .. })
            }));
        assert!(report
            .candidate_evaluations
            .iter()
            .find(|evaluation| evaluation.provider_key == "complete")
            .is_some_and(|evaluation| {
                matches!(evaluation.outcome, CandidateOutcome::Passed { .. })
            }));
        assert!(report
            .combinations
            .iter()
            .filter(|combination| combination.outcome == super::BenchmarkOutcome::Passed)
            .all(
                |combination| combination.plan.routes.get(&route).map(String::as_str)
                    == Some("complete")
            ));
        let recommendation = report.recommendation.as_deref().unwrap();
        let recommended = report
            .combinations
            .iter()
            .find(|combination| combination.plan.id == recommendation)
            .unwrap();
        assert_eq!(
            recommended.plan.routes.get(&route).map(String::as_str),
            Some("complete")
        );
    }

    #[test]
    fn replay_plan_excludes_remote_or_ambiguous_providers_before_invocation() {
        let route = "route";
        let provider: Arc<dyn AgentGatewayProvider> = Arc::new(MockProvider { id: "remote" });
        let providers = BTreeMap::from([(
            "remote".to_string(),
            ConfiguredProvider {
                provider,
                display_name: "Remote".to_string(),
                capabilities: ["chat".to_string()].into_iter().collect(),
                models: ["remote-model".to_string()].into_iter().collect(),
                replay_safe: false,
                replay_capabilities: Default::default(),
                local_kind: Some("openai-compatible".to_string()),
            },
        )]);
        let corpus = vec![CapturedCase {
            sequence: 1,
            route_key: route.to_string(),
            agent_id: "chat-agent".to_string(),
            provider_key: "remote".to_string(),
            capability: "chat".to_string(),
            binding: Arc::new(json!({"capability": "chat"})),
            input: Arc::new(json!({})),
            baseline_output: Arc::new(json!({"choices": [{"message": {"content": "ok"}}]})),
            timeout: Duration::from_secs(1),
            captured_bytes: 1,
        }];

        let error =
            validate_replay_plan(&route_plan(route, "remote"), &providers, &corpus).unwrap_err();

        assert!(error
            .message
            .contains("direct replay excluded provider remote"));
    }

    #[tokio::test]
    async fn remote_provider_is_inventory_only_and_not_a_failed_current_plan() {
        let controller = controller();
        let provider: Arc<dyn AgentGatewayProvider> = Arc::new(MockProvider { id: "remote" });
        controller.configure(
            &[provider],
            &[AgentDescriptor {
                id: "remote-agent".to_string(),
                name: "Remote fallback".to_string(),
                metadata: json!({
                    "providerKey": "remote",
                    "localProviderType": "openai-compatible",
                    "executionLocation": "remote",
                    "capabilities": ["chat"],
                    "models": {"chat": "remote-chat"}
                }),
            }],
        );
        let route = uuid::Uuid::new_v4().to_string();
        lock(&controller.state).corpus.insert(
            corpus_key(&route, "chat"),
            CapturedCase {
                sequence: 1,
                route_key: route,
                agent_id: "remote-agent".to_string(),
                provider_key: "remote".to_string(),
                capability: "chat".to_string(),
                binding: Arc::new(json!({"capability": "chat"})),
                input: Arc::new(json!({})),
                baseline_output: Arc::new(json!({"choices": [{"message": {"content": "ok"}}]})),
                timeout: Duration::from_secs(1),
                captured_bytes: 1,
            },
        );

        let report = controller.benchmark().await.unwrap();

        assert_eq!(report.configured_local_models, 0);
        assert_eq!(report.remote_fallbacks.len(), 1);
        assert_eq!(report.remote_fallbacks[0].provider_key, "remote");
        assert_eq!(report.remote_fallbacks[0].models, vec!["remote-chat"]);
        assert!(report.candidate_evaluations.is_empty());
        assert!(report.combinations.is_empty());
    }
}
