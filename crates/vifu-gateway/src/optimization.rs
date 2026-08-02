//! Small, explainable optimization planner for configured Agent Providers.
//!
//! This module does not execute benchmarks. It turns real per-agent candidate
//! results into at most eight route combinations and owns the in-memory session
//! override used to activate one of those combinations.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::RwLock;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const MAX_COMBINATIONS: usize = 8;
const MAX_SUBSTITUTION_COMBINATIONS: usize = 4;
pub const MAX_COMPARISON_AGENTS: usize = 128;
pub const MAX_COMPARISON_MODELS: u32 = 512;
pub const MAX_COMPARISON_DURATION_MS: u64 = 24 * 60 * 60 * 1_000;
pub const MAX_COMPARISON_UPLOAD_BYTES: usize = 1024 * 1024;
const REQUIRED_COMPARISON_REPEAT_SAMPLES: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeComparisonStatus {
    Completed,
    Failed,
}

impl RuntimeComparisonStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeComparisonOutcome {
    Passed,
    Failed,
}

impl RuntimeComparisonOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeComparisonDevice {
    pub architecture: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeComparisonRunUpload {
    pub id: Uuid,
    pub combination_id: String,
    pub label: String,
    pub rule: String,
    pub routes: BTreeMap<String, String>,
    pub route_labels: BTreeMap<String, String>,
    pub outcome: RuntimeComparisonOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_total_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_run_cold: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat_runs_resident: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat_total: Option<MetricRange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat_ttft: Option<MetricRange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_per_second: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_process_cpu_percent: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_cpu_percent: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peak_rss_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeComparisonUpload {
    pub id: Uuid,
    pub deployment_id: Uuid,
    pub status: RuntimeComparisonStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recommendation: Option<String>,
    pub not_exhaustive: bool,
    pub sequential_replay: bool,
    pub corpus_agents: u32,
    pub configured_models: u32,
    pub tested_models: u32,
    pub passed_models: u32,
    pub device: RuntimeComparisonDevice,
    pub started_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at_ms: Option<u64>,
    pub monotonic_duration_ms: u64,
    pub runs: Vec<RuntimeComparisonRunUpload>,
}

impl RuntimeComparisonUpload {
    pub fn validate(&self) -> Result<(), String> {
        if self.corpus_agents as usize > MAX_COMPARISON_AGENTS {
            return Err(format!(
                "comparison corpus must contain at most {MAX_COMPARISON_AGENTS} agents"
            ));
        }
        if self.configured_models > MAX_COMPARISON_MODELS
            || self.tested_models > self.configured_models
            || self.passed_models > self.tested_models
        {
            return Err("comparison model coverage is invalid".to_string());
        }
        validate_comparison_text("device architecture", &self.device.architecture, 64)?;
        if let Some(backend) = self.device.backend.as_deref() {
            validate_comparison_text("device backend", backend, 128)?;
        }
        if let Some(os) = self.device.os.as_deref() {
            validate_comparison_text("device OS", os, 64)?;
        }
        if self.started_at_ms == 0 || self.started_at_ms > i64::MAX as u64 {
            return Err("comparison start timestamp is invalid".to_string());
        }
        let completed_at_ms = self
            .completed_at_ms
            .ok_or_else(|| "comparison completion timestamp is required".to_string())?;
        if completed_at_ms < self.started_at_ms || completed_at_ms > i64::MAX as u64 {
            return Err("comparison completion timestamp is invalid".to_string());
        }
        if self.monotonic_duration_ms > MAX_COMPARISON_DURATION_MS
            || completed_at_ms.saturating_sub(self.started_at_ms) > MAX_COMPARISON_DURATION_MS
        {
            return Err("comparison duration exceeds the supported window".to_string());
        }
        if self.runs.len() > MAX_COMBINATIONS {
            return Err(format!(
                "comparison must contain at most {MAX_COMBINATIONS} runs"
            ));
        }
        if self.status == RuntimeComparisonStatus::Completed && self.runs.is_empty() {
            return Err("completed comparison must contain at least one run".to_string());
        }

        let mut run_ids = BTreeSet::new();
        let mut combination_ids = BTreeSet::new();
        let mut expected_routes: Option<Vec<&str>> = None;
        for run in &self.runs {
            validate_comparison_run(run)?;
            if !run_ids.insert(run.id) || !combination_ids.insert(run.combination_id.as_str()) {
                return Err("comparison run identifiers must be unique".to_string());
            }
            let routes = run.routes.keys().map(String::as_str).collect::<Vec<_>>();
            if expected_routes
                .as_ref()
                .is_some_and(|expected| expected != &routes)
            {
                return Err("comparison runs must cover the same routes".to_string());
            }
            expected_routes.get_or_insert(routes);
        }
        if let Some(recommendation) = self.recommendation.as_deref() {
            validate_comparison_text("recommendation", recommendation, 128)?;
            let recommended_passed = self.runs.iter().any(|run| {
                run.combination_id == recommendation
                    && run.outcome == RuntimeComparisonOutcome::Passed
            });
            if !recommended_passed {
                return Err("comparison recommendation must identify a passing run".to_string());
            }
        }
        if self.status == RuntimeComparisonStatus::Failed && self.recommendation.is_some() {
            return Err("failed comparison cannot have a recommendation".to_string());
        }
        Ok(())
    }
}

fn validate_comparison_run(run: &RuntimeComparisonRunUpload) -> Result<(), String> {
    validate_comparison_text("combination id", &run.combination_id, 128)?;
    validate_comparison_text("comparison label", &run.label, 128)?;
    validate_comparison_text("comparison rule", &run.rule, 512)?;
    if run.routes.is_empty() || run.routes.len() > MAX_COMPARISON_AGENTS {
        return Err(format!(
            "comparison run routes must contain between 1 and {MAX_COMPARISON_AGENTS} entries"
        ));
    }
    let mut route_bytes = 0_usize;
    for (route, provider) in &run.routes {
        validate_comparison_text("comparison route", route, 128)?;
        validate_comparison_text("comparison provider", provider, 128)?;
        route_bytes = route_bytes
            .saturating_add(route.len())
            .saturating_add(provider.len());
    }
    if route_bytes > 32 * 1024 {
        return Err("comparison routes are too large".to_string());
    }
    if run.route_labels.len() != run.routes.len() || run.route_labels.keys().ne(run.routes.keys()) {
        return Err("comparison route labels must match route binding IDs".to_string());
    }
    for label in run.route_labels.values() {
        validate_comparison_text("comparison route label", label, 128)?;
    }
    if let Some(value) = run.first_total_ms {
        validate_duration(value)?;
    }
    if let Some(range) = run.repeat_total.as_ref() {
        validate_metric_range(range)?;
    }
    if let Some(range) = run.repeat_ttft.as_ref() {
        validate_metric_range(range)?;
    }
    if let Some(value) = run.tokens_per_second {
        if !value.is_finite() || !(0.0..=1_000_000_000.0).contains(&value) {
            return Err("comparison token rate is invalid".to_string());
        }
    }
    validate_process_cpu(run.first_process_cpu_percent)?;
    validate_process_cpu(run.process_cpu_percent)?;
    if run
        .peak_rss_bytes
        .is_some_and(|value| value > i64::MAX as u64)
    {
        return Err("comparison peak RSS is invalid".to_string());
    }
    if let Some(error) = run.error.as_deref() {
        validate_comparison_text("comparison error", error, 512)?;
        if contains_sensitive_marker(error) {
            return Err("comparison error contains sensitive data".to_string());
        }
    }
    match run.outcome {
        RuntimeComparisonOutcome::Passed => {
            if run.first_total_ms.is_none() || run.repeat_total.is_none() || run.error.is_some() {
                return Err("passing comparison run is missing measured totals".to_string());
            }
            if run
                .repeat_total
                .as_ref()
                .is_some_and(|range| range.samples != REQUIRED_COMPARISON_REPEAT_SAMPLES)
            {
                return Err(format!(
                    "passing comparison run must include exactly {REQUIRED_COMPARISON_REPEAT_SAMPLES} repeat samples"
                ));
            }
        }
        RuntimeComparisonOutcome::Failed => {
            if run.error.is_none() {
                return Err("failed comparison run must include a bounded error".to_string());
            }
        }
    }
    Ok(())
}

fn validate_process_cpu(value: Option<f64>) -> Result<(), String> {
    if value.is_some_and(|value| !value.is_finite() || !(0.0..=1_000_000.0).contains(&value)) {
        Err("comparison process CPU is invalid".to_string())
    } else {
        Ok(())
    }
}

fn validate_metric_range(range: &MetricRange) -> Result<(), String> {
    if range.samples == 0
        || range.samples > 64
        || range.min > range.median
        || range.median > range.max
    {
        return Err("comparison metric range is invalid".to_string());
    }
    validate_duration(range.max)
}

fn validate_duration(value: u64) -> Result<(), String> {
    if value > MAX_COMPARISON_DURATION_MS {
        Err("comparison duration exceeds the supported window".to_string())
    } else {
        Ok(())
    }
}

fn validate_comparison_text(name: &str, value: &str, max_len: usize) -> Result<(), String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > max_len || trimmed.chars().any(char::is_control) {
        return Err(format!("{name} is invalid"));
    }
    Ok(())
}

fn contains_sensitive_marker(value: &str) -> bool {
    let lower = value.trim().to_ascii_lowercase();
    lower.starts_with("data:")
        || lower.starts_with("bearer ")
        || lower.starts_with("basic ")
        || [
            "authorization:",
            "authorization=",
            "api_key=",
            "api-key=",
            "apikey=",
            "access_token=",
            "access token:",
            "token=",
            "token:",
            "secret=",
            "secret:",
            "password=",
            "password:",
            "credential=",
            "credential:",
            "cookie=",
            "cookie:",
            "session=",
            "session:",
            "vifu_pk_",
            "vifu_gw_",
        ]
        .iter()
        .any(|marker| lower.contains(marker))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCase {
    pub agent_id: String,
    pub capability: String,
    pub current_provider_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricRange {
    pub median: u64,
    pub min: u64,
    pub max: u64,
    pub samples: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum CandidateOutcome {
    Passed {
        total_latency_ms: MetricRange,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ttft_ms: Option<MetricRange>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tokens_per_second: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rss_delta_bytes: Option<u64>,
    },
    Excluded {
        reason: ExclusionReason,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExclusionReason {
    CapabilityMismatch,
    Unavailable,
    LoadFailure,
    InsufficientMemory,
    ContractFailure,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateEvaluation {
    pub agent_id: String,
    pub provider_key: String,
    pub outcome: CandidateOutcome,
}

impl CandidateEvaluation {
    fn passed(&self) -> Option<PassedMetrics<'_>> {
        match &self.outcome {
            CandidateOutcome::Passed {
                total_latency_ms,
                rss_delta_bytes,
                ..
            } => Some(PassedMetrics {
                total_latency_ms,
                rss_delta_bytes: *rss_delta_bytes,
            }),
            CandidateOutcome::Excluded { .. } => None,
        }
    }
}

#[derive(Clone, Copy)]
struct PassedMetrics<'a> {
    total_latency_ms: &'a MetricRange,
    rss_delta_bytes: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CombinationKind {
    Current,
    FastestLocal,
    LowestMemory,
    SharedModel,
    Substitution,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteCombination {
    pub id: String,
    pub label: String,
    pub kind: CombinationKind,
    pub explanation: String,
    pub routes: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptimizationCoverage {
    pub configured_models: usize,
    pub tested_models: usize,
    pub passed_models: usize,
    pub expected_pairs: usize,
    pub evaluated_pairs: usize,
    pub passed_pairs: usize,
    pub missing: Vec<(String, String)>,
}

/// Reports whether every configured candidate has a result for every compatible
/// agent case. The caller supplies compatibility so capability mismatches can be
/// recorded once as explicit exclusions instead of silently disappearing.
pub fn coverage(
    configured_provider_keys: &[String],
    cases: &[AgentCase],
    evaluations: &[CandidateEvaluation],
) -> OptimizationCoverage {
    let expected = cases
        .iter()
        .flat_map(|case| {
            configured_provider_keys
                .iter()
                .map(move |provider| (case.agent_id.as_str(), provider.as_str()))
        })
        .collect::<BTreeSet<_>>();
    let evaluated = evaluations
        .iter()
        .filter(|evaluation| {
            expected.contains(&(
                evaluation.agent_id.as_str(),
                evaluation.provider_key.as_str(),
            ))
        })
        .map(|evaluation| {
            (
                evaluation.agent_id.as_str(),
                evaluation.provider_key.as_str(),
            )
        })
        .collect::<BTreeSet<_>>();
    let passed_pairs = evaluations
        .iter()
        .filter(|evaluation| evaluation.passed().is_some())
        .map(|evaluation| {
            (
                evaluation.agent_id.as_str(),
                evaluation.provider_key.as_str(),
            )
        })
        .filter(|pair| expected.contains(pair))
        .collect::<BTreeSet<_>>();
    let mut missing = Vec::new();
    for case in cases {
        for provider_key in configured_provider_keys {
            if !evaluated.contains(&(case.agent_id.as_str(), provider_key.as_str())) {
                missing.push((case.agent_id.clone(), provider_key.clone()));
            }
        }
    }
    OptimizationCoverage {
        configured_models: configured_provider_keys.len(),
        tested_models: configured_provider_keys
            .iter()
            .filter(|provider| {
                cases
                    .iter()
                    .all(|case| evaluated.contains(&(case.agent_id.as_str(), provider.as_str())))
            })
            .count(),
        passed_models: configured_provider_keys
            .iter()
            .filter(|provider| {
                passed_pairs
                    .iter()
                    .any(|(_, passed_provider)| *passed_provider == provider.as_str())
            })
            .count(),
        expected_pairs: expected.len(),
        evaluated_pairs: evaluated.len(),
        passed_pairs: passed_pairs.len(),
        missing,
    }
}

/// Generates the spec's bounded, explainable combination set from real passed
/// candidate results. No synthetic aggregate metrics are produced here; each
/// returned combination still needs one cold and three warm full-corpus runs.
pub fn generate_combinations(
    cases: &[AgentCase],
    evaluations: &[CandidateEvaluation],
) -> Vec<RouteCombination> {
    let by_agent = passed_by_agent(evaluations);
    let mut combinations = Vec::with_capacity(MAX_COMBINATIONS);

    push_unique(
        &mut combinations,
        RouteCombination {
            id: "current".to_string(),
            label: "current".to_string(),
            kind: CombinationKind::Current,
            explanation: "Current session routes".to_string(),
            routes: cases
                .iter()
                .map(|case| (case.agent_id.clone(), case.current_provider_key.clone()))
                .collect(),
        },
    );

    let fastest = select_routes(cases, &by_agent, CandidateOrder::Latency);
    if fastest.len() == cases.len() {
        push_unique(
            &mut combinations,
            RouteCombination {
                id: "fastest-local".to_string(),
                label: "fastest-local".to_string(),
                kind: CombinationKind::FastestLocal,
                explanation: "Fastest passing configured candidate for every agent".to_string(),
                routes: fastest.clone(),
            },
        );
    }

    let lowest_memory = select_routes(cases, &by_agent, CandidateOrder::Memory);
    if lowest_memory.len() == cases.len() {
        push_unique(
            &mut combinations,
            RouteCombination {
                id: "lowest-memory".to_string(),
                label: "lowest-memory".to_string(),
                kind: CombinationKind::LowestMemory,
                explanation: "Lowest observed Vifu OS-process RSS delta among passing candidates"
                    .to_string(),
                routes: lowest_memory,
            },
        );
    }

    if let Some(shared) = shared_routes(cases, &by_agent) {
        push_unique(
            &mut combinations,
            RouteCombination {
                id: "shared-model".to_string(),
                label: "shared-model".to_string(),
                kind: CombinationKind::SharedModel,
                explanation: "Reuses the passing provider that covers the most agents".to_string(),
                routes: shared,
            },
        );
    }

    let mut slowest_agents = fastest
        .iter()
        .filter_map(|(agent_id, provider_key)| {
            evaluation_for(&by_agent, agent_id, provider_key).map(|evaluation| {
                (
                    agent_id.clone(),
                    evaluation
                        .passed()
                        .expect("passed index only contains passing evaluations")
                        .total_latency_ms
                        .median,
                )
            })
        })
        .collect::<Vec<_>>();
    slowest_agents.sort_unstable_by_key(|item| std::cmp::Reverse(item.1));
    for (index, (agent_id, _)) in slowest_agents
        .into_iter()
        .take(MAX_SUBSTITUTION_COMBINATIONS)
        .enumerate()
    {
        let Some(alternatives) = by_agent.get(agent_id.as_str()) else {
            continue;
        };
        let mut alternatives = alternatives.clone();
        alternatives.sort_by(candidate_latency_order);
        let Some(second) = alternatives.get(1) else {
            continue;
        };
        let mut routes = fastest.clone();
        routes.insert(agent_id.clone(), second.provider_key.clone());
        push_unique(
            &mut combinations,
            RouteCombination {
                id: format!("alternative-{}", index + 1),
                label: format!("alternative-{}", index + 1),
                kind: CombinationKind::Substitution,
                explanation: format!("Second-fastest passing candidate for slow agent {agent_id}"),
                routes,
            },
        );
        if combinations.len() == MAX_COMBINATIONS {
            break;
        }
    }

    combinations
}

type PassedByAgent<'a> = HashMap<&'a str, Vec<&'a CandidateEvaluation>>;

fn passed_by_agent(evaluations: &[CandidateEvaluation]) -> PassedByAgent<'_> {
    let mut by_agent: PassedByAgent<'_> = HashMap::new();
    for evaluation in evaluations {
        if evaluation.passed().is_some() {
            let candidates = by_agent.entry(evaluation.agent_id.as_str()).or_default();
            if let Some(existing) = candidates
                .iter_mut()
                .find(|candidate| candidate.provider_key == evaluation.provider_key)
            {
                if evaluation
                    .passed()
                    .expect("candidate is passing")
                    .total_latency_ms
                    .median
                    < existing
                        .passed()
                        .expect("passed index contains passing candidates")
                        .total_latency_ms
                        .median
                {
                    *existing = evaluation;
                }
            } else {
                candidates.push(evaluation);
            }
        }
    }
    by_agent
}

#[derive(Clone, Copy)]
enum CandidateOrder {
    Latency,
    Memory,
}

fn select_routes(
    cases: &[AgentCase],
    by_agent: &PassedByAgent<'_>,
    order: CandidateOrder,
) -> BTreeMap<String, String> {
    cases
        .iter()
        .filter_map(|case| {
            let candidates = by_agent.get(case.agent_id.as_str())?;
            let selected = candidates
                .iter()
                .copied()
                .min_by(|left, right| match order {
                    CandidateOrder::Latency => candidate_latency_order(left, right),
                    CandidateOrder::Memory => candidate_memory_order(left, right),
                })?;
            Some((case.agent_id.clone(), selected.provider_key.clone()))
        })
        .collect()
}

fn shared_routes(
    cases: &[AgentCase],
    by_agent: &PassedByAgent<'_>,
) -> Option<BTreeMap<String, String>> {
    let mut coverage: HashMap<&str, (usize, u64)> = HashMap::new();
    for candidates in by_agent.values() {
        for candidate in candidates {
            let metrics = candidate
                .passed()
                .expect("passed index only contains passing evaluations");
            let entry = coverage
                .entry(candidate.provider_key.as_str())
                .or_insert((0, 0));
            entry.0 += 1;
            entry.1 = entry.1.saturating_add(metrics.total_latency_ms.median);
        }
    }
    let shared_provider = coverage
        .into_iter()
        .max_by(|left, right| {
            left.1
                 .0
                .cmp(&right.1 .0)
                .then_with(|| right.1 .1.cmp(&left.1 .1))
                .then_with(|| right.0.cmp(left.0))
        })?
        .0;
    let fastest = select_routes(cases, by_agent, CandidateOrder::Latency);
    let routes = cases
        .iter()
        .filter_map(|case| {
            let provider = if evaluation_for(by_agent, &case.agent_id, shared_provider).is_some() {
                shared_provider.to_string()
            } else {
                fastest.get(&case.agent_id)?.clone()
            };
            Some((case.agent_id.clone(), provider))
        })
        .collect::<BTreeMap<_, _>>();
    (routes.len() == cases.len()).then_some(routes)
}

fn evaluation_for<'a>(
    by_agent: &PassedByAgent<'a>,
    agent_id: &str,
    provider_key: &str,
) -> Option<&'a CandidateEvaluation> {
    by_agent
        .get(agent_id)?
        .iter()
        .copied()
        .find(|evaluation| evaluation.provider_key == provider_key)
}

fn candidate_latency_order(
    left: &&CandidateEvaluation,
    right: &&CandidateEvaluation,
) -> std::cmp::Ordering {
    let left = left
        .passed()
        .expect("passed index only contains passing evaluations");
    let right = right
        .passed()
        .expect("passed index only contains passing evaluations");
    left.total_latency_ms
        .median
        .cmp(&right.total_latency_ms.median)
}

fn candidate_memory_order(
    left: &&CandidateEvaluation,
    right: &&CandidateEvaluation,
) -> std::cmp::Ordering {
    let left = left
        .passed()
        .expect("passed index only contains passing evaluations");
    let right = right
        .passed()
        .expect("passed index only contains passing evaluations");
    left.rss_delta_bytes
        .unwrap_or(u64::MAX)
        .cmp(&right.rss_delta_bytes.unwrap_or(u64::MAX))
        .then_with(|| {
            left.total_latency_ms
                .median
                .cmp(&right.total_latency_ms.median)
        })
}

fn push_unique(combinations: &mut Vec<RouteCombination>, candidate: RouteCombination) {
    if combinations.len() < MAX_COMBINATIONS
        && !combinations
            .iter()
            .any(|existing| existing.routes == candidate.routes)
    {
        combinations.push(candidate);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteOverrideSnapshot {
    pub generation: u64,
    pub routes: BTreeMap<String, String>,
    pub can_undo: bool,
}

#[derive(Default)]
struct RouteOverrideState {
    generation: u64,
    routes: BTreeMap<String, String>,
    previous: Option<BTreeMap<String, String>>,
}

/// Atomic, process-local route overrides for the current gateway session.
///
/// Relay code clones one provider key at the invocation boundary. Existing
/// calls therefore keep their original provider while new calls see an
/// activated combination immediately.
#[derive(Default)]
pub struct SessionRouteOverrides {
    state: RwLock<RouteOverrideState>,
}

impl SessionRouteOverrides {
    pub fn provider_for(&self, agent_id: &str) -> Option<String> {
        self.state
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .routes
            .get(agent_id)
            .cloned()
    }

    pub fn snapshot(&self) -> RouteOverrideSnapshot {
        let state = self
            .state
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        RouteOverrideSnapshot {
            generation: state.generation,
            routes: state.routes.clone(),
            can_undo: state.previous.is_some(),
        }
    }

    /// Clears all routes when the provider configuration changes. Returning a
    /// generation even for a clear keeps TUI state changes monotonic.
    pub fn clear(&self) -> u64 {
        let mut state = self
            .state
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.routes.clear();
        state.previous = None;
        state.generation = state.generation.saturating_add(1);
        state.generation
    }

    pub fn activate(&self, routes: BTreeMap<String, String>) -> Result<u64, String> {
        if routes.is_empty() {
            return Err("route combination must contain at least one agent".to_string());
        }
        if routes
            .iter()
            .any(|(agent, provider)| agent.trim().is_empty() || provider.trim().is_empty())
        {
            return Err("route combination contains an empty agent or provider key".to_string());
        }
        let mut state = self
            .state
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.previous = Some(std::mem::replace(&mut state.routes, routes));
        state.generation = state.generation.saturating_add(1);
        Ok(state.generation)
    }

    pub fn undo(&self) -> Option<u64> {
        let mut state = self
            .state
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = state.previous.take()?;
        state.routes = previous;
        state.generation = state.generation.saturating_add(1);
        Some(state.generation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn passed(agent: &str, provider: &str, latency: u64, rss: u64) -> CandidateEvaluation {
        CandidateEvaluation {
            agent_id: agent.to_string(),
            provider_key: provider.to_string(),
            outcome: CandidateOutcome::Passed {
                total_latency_ms: MetricRange {
                    median: latency,
                    min: latency - 1,
                    max: latency + 1,
                    samples: 3,
                },
                ttft_ms: None,
                tokens_per_second: None,
                rss_delta_bytes: Some(rss),
            },
        }
    }

    fn cases() -> Vec<AgentCase> {
        vec![
            AgentCase {
                agent_id: "planner".to_string(),
                capability: "chat".to_string(),
                current_provider_key: "large".to_string(),
            },
            AgentCase {
                agent_id: "memory".to_string(),
                capability: "embedding".to_string(),
                current_provider_key: "embed".to_string(),
            },
        ]
    }

    fn comparison_upload() -> RuntimeComparisonUpload {
        RuntimeComparisonUpload {
            id: Uuid::new_v4(),
            deployment_id: Uuid::new_v4(),
            status: RuntimeComparisonStatus::Completed,
            recommendation: Some("fastest-local".to_string()),
            not_exhaustive: true,
            sequential_replay: true,
            corpus_agents: 1,
            configured_models: 2,
            tested_models: 2,
            passed_models: 1,
            device: RuntimeComparisonDevice {
                architecture: "aarch64".to_string(),
                backend: Some("llama.cpp".to_string()),
                os: Some("linux".to_string()),
            },
            started_at_ms: 1_700_000_000_000,
            completed_at_ms: Some(1_700_000_001_000),
            monotonic_duration_ms: 1_000,
            runs: vec![RuntimeComparisonRunUpload {
                id: Uuid::new_v4(),
                combination_id: "fastest-local".to_string(),
                label: "fastest-local".to_string(),
                rule: "Fastest passing configured local candidate".to_string(),
                routes: BTreeMap::from([("planner".to_string(), "qwen-2b".to_string())]),
                route_labels: BTreeMap::from([(
                    "planner".to_string(),
                    "NPC planner · chat".to_string(),
                )]),
                outcome: RuntimeComparisonOutcome::Passed,
                first_total_ms: Some(400),
                first_run_cold: Some(true),
                repeat_runs_resident: Some(true),
                repeat_total: Some(MetricRange {
                    median: 200,
                    min: 190,
                    max: 220,
                    samples: 3,
                }),
                repeat_ttft: Some(MetricRange {
                    median: 50,
                    min: 45,
                    max: 55,
                    samples: 3,
                }),
                tokens_per_second: Some(32.0),
                first_process_cpu_percent: Some(190.0),
                process_cpu_percent: Some(175.0),
                peak_rss_bytes: Some(512 * 1024 * 1024),
                error: None,
            }],
        }
    }

    fn maximum_comparison_upload() -> RuntimeComparisonUpload {
        let routes = (0..MAX_COMPARISON_AGENTS)
            .map(|index| {
                (
                    Uuid::from_u128(index as u128 + 1).to_string(),
                    "\\".repeat(128),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let route_labels = routes
            .keys()
            .map(|route| (route.clone(), "\\".repeat(128)))
            .collect::<BTreeMap<_, _>>();
        let runs = (0..MAX_COMBINATIONS)
            .map(|index| RuntimeComparisonRunUpload {
                id: Uuid::from_u128(10_000 + index as u128),
                combination_id: format!("combination-{index}"),
                label: "\\".repeat(128),
                rule: "\\".repeat(512),
                routes: routes.clone(),
                route_labels: route_labels.clone(),
                outcome: RuntimeComparisonOutcome::Passed,
                first_total_ms: Some(1),
                first_run_cold: Some(true),
                repeat_runs_resident: Some(true),
                repeat_total: Some(MetricRange {
                    median: 1,
                    min: 1,
                    max: 1,
                    samples: REQUIRED_COMPARISON_REPEAT_SAMPLES,
                }),
                repeat_ttft: None,
                tokens_per_second: None,
                first_process_cpu_percent: None,
                process_cpu_percent: None,
                peak_rss_bytes: None,
                error: None,
            })
            .collect();
        RuntimeComparisonUpload {
            id: Uuid::from_u128(20_000),
            deployment_id: Uuid::from_u128(20_001),
            status: RuntimeComparisonStatus::Completed,
            recommendation: Some("combination-0".to_string()),
            not_exhaustive: false,
            sequential_replay: true,
            corpus_agents: MAX_COMPARISON_AGENTS as u32,
            configured_models: MAX_COMPARISON_MODELS,
            tested_models: MAX_COMPARISON_MODELS,
            passed_models: MAX_COMPARISON_MODELS,
            device: RuntimeComparisonDevice {
                architecture: "\\".repeat(64),
                backend: Some("\\".repeat(128)),
                os: Some("\\".repeat(64)),
            },
            started_at_ms: 1,
            completed_at_ms: Some(2),
            monotonic_duration_ms: 1,
            runs,
        }
    }

    #[test]
    fn comparison_upload_accepts_bounded_runtime_evidence() {
        assert_eq!(comparison_upload().validate(), Ok(()));
    }

    #[test]
    fn comparison_upload_rejects_incomplete_passing_repeat_evidence() {
        let mut upload = comparison_upload();
        upload.runs[0].repeat_total.as_mut().unwrap().samples = 2;

        assert_eq!(
            upload.validate(),
            Err("passing comparison run must include exactly 3 repeat samples".to_string())
        );
    }

    #[test]
    fn maximum_valid_comparison_fits_the_shared_upload_limit() {
        let upload = maximum_comparison_upload();

        assert_eq!(upload.validate(), Ok(()));
        let serialized_size = serde_json::to_vec(&upload).unwrap().len();
        assert!(serialized_size > 512 * 1024);
        assert!(serialized_size <= MAX_COMPARISON_UPLOAD_BYTES);
    }

    #[test]
    fn comparison_upload_rejects_canonical_sensitive_error_markers() {
        for error in [
            "Authorization=Bearer private-token",
            "password=hunter2",
            "credential: private-token",
            "cookie=session-value",
            "session=private-session",
        ] {
            let mut upload = comparison_upload();
            upload.status = RuntimeComparisonStatus::Failed;
            upload.recommendation = None;
            upload.runs[0].outcome = RuntimeComparisonOutcome::Failed;
            upload.runs[0].error = Some(error.to_string());

            assert_eq!(
                upload.validate(),
                Err("comparison error contains sensitive data".to_string()),
                "sensitive error was accepted: {error}"
            );
        }
    }

    #[test]
    fn combinations_are_bounded_explainable_and_only_use_passed_candidates() {
        let mut evaluations = vec![
            passed("planner", "large", 80, 900),
            passed("planner", "shared", 30, 400),
            passed("planner", "small", 40, 200),
            passed("memory", "embed", 25, 300),
            passed("memory", "shared", 35, 400),
            passed("memory", "tiny", 45, 100),
        ];
        evaluations.push(CandidateEvaluation {
            agent_id: "planner".to_string(),
            provider_key: "broken".to_string(),
            outcome: CandidateOutcome::Excluded {
                reason: ExclusionReason::ContractFailure,
                message: Some("actions is missing".to_string()),
            },
        });

        let combinations = generate_combinations(&cases(), &evaluations);

        assert!(combinations.len() <= MAX_COMBINATIONS);
        assert!(combinations
            .iter()
            .all(|combination| !combination.explanation.is_empty()));
        assert!(combinations
            .iter()
            .all(|combination| !combination.routes.values().any(|value| value == "broken")));
        let fastest = combinations
            .iter()
            .find(|combination| combination.kind == CombinationKind::FastestLocal)
            .unwrap();
        assert_eq!(fastest.routes["planner"], "shared");
        assert_eq!(fastest.routes["memory"], "embed");
        let lowest = combinations
            .iter()
            .find(|combination| combination.kind == CombinationKind::LowestMemory)
            .unwrap();
        assert_eq!(lowest.routes["planner"], "small");
        assert_eq!(lowest.routes["memory"], "tiny");
        let shared = combinations
            .iter()
            .find(|combination| combination.kind == CombinationKind::SharedModel)
            .unwrap();
        assert!(shared.routes.values().all(|provider| provider == "shared"));
    }

    #[test]
    fn coverage_exposes_every_untested_agent_model_pair() {
        let providers = vec!["large".to_string(), "small".to_string()];
        let report = coverage(&providers, &cases(), &[passed("planner", "large", 80, 900)]);

        assert_eq!(report.configured_models, 2);
        assert_eq!(report.tested_models, 0);
        assert_eq!(report.evaluated_pairs, 1);
        assert_eq!(report.passed_pairs, 1);
        assert_eq!(report.missing.len(), 3);
        assert!(report
            .missing
            .contains(&("planner".to_string(), "small".to_string())));
    }

    #[test]
    fn activation_is_atomic_and_one_step_undo_restores_routes() {
        let overrides = SessionRouteOverrides::default();
        let first = BTreeMap::from([("planner".to_string(), "small".to_string())]);
        let second = BTreeMap::from([("planner".to_string(), "large".to_string())]);

        assert_eq!(overrides.activate(first.clone()).unwrap(), 1);
        assert_eq!(overrides.provider_for("planner").as_deref(), Some("small"));
        assert_eq!(overrides.activate(second).unwrap(), 2);
        assert_eq!(overrides.provider_for("planner").as_deref(), Some("large"));
        assert_eq!(overrides.undo(), Some(3));
        assert_eq!(overrides.snapshot().routes, first);
        assert!(!overrides.snapshot().can_undo);
    }
}
