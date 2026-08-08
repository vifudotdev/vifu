pub(crate) mod model;
pub(crate) mod system;
mod view;

use std::fs::{File, OpenOptions};
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crossterm::cursor::{Hide, Show};
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, MouseEvent, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use uuid::Uuid;
use vifu_gateway::optimization::{CandidateOutcome, ExclusionReason, MetricRange};

use crate::benchmark::{BenchmarkOutcome, OptimizationController, OptimizationReport};
use crate::monitor::{RuntimeEventReceiver, RuntimeStage};

use self::model::{
    App, ComparisonRow, DevicePairingView, LaneOutcome, MetricSummary, OptimizationExclusion,
    OptimizationSummary, View,
};
use self::system::SystemSampler;

type TuiBackend = CrosstermBackend<Box<dyn Write>>;
const MAX_EVENTS_PER_TICK: usize = 128;

pub(crate) fn should_run() -> bool {
    io::stdin().is_terminal()
        && io::stdout().is_terminal()
        && std::env::var_os("CI").is_none()
        && std::env::var_os("TERM").is_none_or(|term| term != "dumb")
}

pub(crate) async fn run(
    mut events: RuntimeEventReceiver,
    dashboard_url: Option<String>,
    optimization: Option<OptimizationController>,
    device_pairing: Option<crate::gateway::DevicePairingController>,
) -> Result<(), String> {
    let mut terminal = TerminalSession::enter()?;
    let mut app = App::default();
    sync_override_state(&mut app, optimization.as_ref());
    let mut sampler = SystemSampler::default();
    app.metrics = sampler.sample();
    let no_color = std::env::var_os("NO_COLOR").is_some();
    let mut render_tick = tokio::time::interval(Duration::from_millis(100));
    render_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut metrics_tick = tokio::time::interval(Duration::from_secs(1));
    metrics_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut events_closed = false;
    let (optimization_result_tx, mut optimization_results) =
        tokio::sync::mpsc::unbounded_channel::<(u64, Result<OptimizationReport, String>)>();
    let (dashboard_result_tx, mut dashboard_results) =
        tokio::sync::mpsc::channel::<Result<(), String>>(1);
    let mut dashboard_opening = false;
    let (pairing_result_tx, mut pairing_results) = tokio::sync::mpsc::channel::<
        Result<vifu_gateway::control::GuestGatewayEnrollment, String>,
    >(1);
    let mut pairing_opening = false;

    loop {
        let mut action = UiAction::Continue;
        tokio::select! {
            biased;
            _ = render_tick.tick() => {
                while event::poll(Duration::ZERO).map_err(|error| error.to_string())? {
                    match event::read().map_err(|error| error.to_string())? {
                        Event::Key(key) if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) => {
                            action = handle_key(&mut app, key, Instant::now());
                            if action != UiAction::Continue {
                                break;
                            }
                        }
                        Event::Mouse(mouse) => handle_mouse(&mut app, mouse),
                        Event::Resize(_, _) => {}
                        _ => {}
                    }
                }
            }
            event = events.recv(), if !events_closed => {
                match event {
                    Some(event) => {
                        let now = Instant::now();
                        app.apply(event, now);
                        for _ in 1..MAX_EVENTS_PER_TICK {
                            let Ok(event) = events.try_recv() else {
                                break;
                            };
                            app.apply(event, now);
                        }
                    }
                    None => events_closed = true,
                }
            }
            _ = metrics_tick.tick() => {
                app.metrics = sampler.sample();
            }
            result = optimization_results.recv(), if app.optimization_running => {
                app.optimization_running = false;
                match result {
                    Some((generation, _)) if generation != app.inventory_generation => {
                        app.notice = Some(
                            "Provider inventory changed during comparison; run Optimize again"
                                .to_string(),
                        );
                    }
                    Some((_, Ok(report))) => apply_optimization_report(&mut app, report),
                    Some((_, Err(error))) => app.notice = Some(error),
                    None => app.notice = Some("Optimization worker stopped unexpectedly".to_string()),
                }
            }
            result = dashboard_results.recv(), if dashboard_opening => {
                dashboard_opening = false;
                app.notice = Some(match result {
                    Some(Ok(())) => "Dashboard opened".to_string(),
                    Some(Err(error)) => error,
                    None => "Dashboard opener stopped unexpectedly".to_string(),
                });
            }
            result = pairing_results.recv(), if pairing_opening => {
                pairing_opening = false;
                match result {
                    Some(Ok(enrollment)) => match enrollment.pairing {
                        Some(pairing) => match pairing.pairing_terminal_qr {
                            Some(terminal_qr) => {
                                app.device_pairing = Some(DevicePairingView {
                                    enrollment_id: enrollment.enrollment_id,
                                    server_url: pairing.server_url,
                                    terminal_qr,
                                    expires_at: enrollment.expires_at,
                                });
                                app.notice = None;
                            }
                            None => app.notice = Some(
                                "The configured Server did not provide a terminal pairing QR"
                                    .to_string(),
                            ),
                        },
                        None => app.notice = Some(
                            "The configured Server has no public device endpoint".to_string(),
                        ),
                    },
                    Some(Err(error)) => app.notice = Some(error),
                    None => app.notice = Some("Device pairing worker stopped unexpectedly".to_string()),
                }
            }
        }

        match action {
            UiAction::Continue => {}
            UiAction::Quit => return Ok(()),
            UiAction::Dashboard => {
                app.notice = if dashboard_opening {
                    Some("Opening Dashboard when ready…".to_string())
                } else {
                    match dashboard_url.as_deref() {
                        Some(url) => {
                            let target = dashboard_target_url(url, &app);
                            dashboard_opening = true;
                            let result_tx = dashboard_result_tx.clone();
                            std::mem::drop(tokio::spawn(async move {
                                let result = crate::launcher::open_browser_when_ready(target).await;
                                let _ = result_tx.send(result).await;
                            }));
                            Some("Opening Dashboard when ready…".to_string())
                        }
                        None => Some("No Dashboard URL is configured for this Runtime".to_string()),
                    }
                };
            }
            UiAction::PairDevice => {
                if pairing_opening {
                    app.notice = Some("Preparing a one-time device pairing code…".to_string());
                } else if let Some(controller) = device_pairing.as_ref() {
                    pairing_opening = true;
                    app.notice = Some("Preparing a one-time device pairing code…".to_string());
                    let controller = controller.clone();
                    let result_tx = pairing_result_tx.clone();
                    std::mem::drop(tokio::spawn(async move {
                        let _ = result_tx.send(controller.create_enrollment().await).await;
                    }));
                } else {
                    app.notice = Some(
                        "Device pairing is available when the Agent Gateway owns a Guest project"
                            .to_string(),
                    );
                }
            }
            UiAction::ExternalEditor => {
                app.notice =
                    match open_selected_trace_in_editor(&mut terminal, &mut app, &mut events).await
                    {
                        Ok(path) => Some(format!("Exported redacted Trace to {}", path.display())),
                        Err(error) => Some(error),
                    };
            }
            UiAction::Optimize => {
                app.open_optimize();
                if app.optimization_running {
                    app.notice = Some("A real model comparison is already running".to_string());
                } else if let Some(controller) = optimization.as_ref() {
                    app.optimization_running = true;
                    app.notice = Some(
                        "Sequentially replaying configured local models: one first run and three repeats…"
                            .to_string(),
                    );
                    let controller = controller.clone();
                    let result_tx = optimization_result_tx.clone();
                    let inventory_generation = app.inventory_generation;
                    std::mem::drop(tokio::spawn(async move {
                        let _ =
                            result_tx.send((inventory_generation, controller.benchmark().await));
                    }));
                } else {
                    app.notice = Some(
                        "Optimization is available when the Agent Gateway is running".to_string(),
                    );
                }
            }
            UiAction::Activate => {
                activate_selected_combination(&mut app, optimization.as_ref());
            }
            UiAction::Undo => {
                undo_active_combination(&mut app, optimization.as_ref());
            }
        }

        // Provider reconnects can either preserve or clear a session override.
        // Read the controller's authoritative snapshot before every draw so
        // the quit guard and header always reflect the routes actually in use.
        sync_override_state(&mut app, optimization.as_ref());
        app.project_dashboard_url = dashboard_url.as_deref().map(|base_url| {
            app.project.as_deref().map_or_else(
                || base_url.to_string(),
                |project| {
                    dashboard_project_url(base_url, project).unwrap_or_else(|| base_url.to_string())
                },
            )
        });
        let now = Instant::now();
        terminal
            .terminal
            .draw(|frame| view::render(frame, &mut app, now, no_color))
            .map_err(|error| format!("could not draw Vifu TUI: {error}"))?;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UiAction {
    Continue,
    Quit,
    Dashboard,
    PairDevice,
    ExternalEditor,
    Optimize,
    Activate,
    Undo,
}

fn handle_key(app: &mut App, key: KeyEvent, now: Instant) -> UiAction {
    if app.device_pairing.is_some() {
        if matches!(key.code, KeyCode::Esc | KeyCode::Char('p' | 'P')) {
            app.device_pairing = None;
        }
        return UiAction::Continue;
    }
    if app.quit_confirmation {
        return match key.code {
            KeyCode::Char('y' | 'Y') => UiAction::Quit,
            KeyCode::Char('n' | 'N') | KeyCode::Esc => {
                app.quit_confirmation = false;
                UiAction::Continue
            }
            _ => UiAction::Continue,
        };
    }

    if app.search_active {
        match key.code {
            KeyCode::Enter => app.search_active = false,
            KeyCode::Esc => {
                app.search.clear();
                app.search_active = false;
            }
            KeyCode::Backspace => {
                app.search.pop();
                app.normalize_search_selection(now);
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                app.search.push(character);
                app.normalize_search_selection(now);
            }
            _ => {}
        }
        return UiAction::Continue;
    }

    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return request_quit(app);
    }

    match app.view.clone() {
        View::Main => match key.code {
            KeyCode::Up => app.move_lane_selection(-1, now),
            KeyCode::Down => app.move_lane_selection(1, now),
            KeyCode::PageUp => app.move_lane_selection(-10, now),
            KeyCode::PageDown => app.move_lane_selection(10, now),
            KeyCode::Right | KeyCode::Enter => app.open_selected_agent(),
            KeyCode::Char('f' | 'F') => {
                app.filter = app.filter.next();
                app.normalize_lane_selection(now);
            }
            KeyCode::Char('s' | 'S') => {
                app.sort = app.sort.next();
                app.normalize_lane_selection(now);
            }
            KeyCode::Char('/') => app.search_active = true,
            KeyCode::Char('o' | 'O') => return UiAction::Optimize,
            KeyCode::Char('b' | 'B') => return UiAction::Dashboard,
            KeyCode::Char('p' | 'P') => return UiAction::PairDevice,
            KeyCode::Char('q' | 'Q') => return request_quit(app),
            _ => {}
        },
        View::Agent { .. } => match key.code {
            KeyCode::Up => app.move_agent_request_selection(-1),
            KeyCode::Down => app.move_agent_request_selection(1),
            KeyCode::PageUp => app.move_agent_request_selection(-10),
            KeyCode::PageDown => app.move_agent_request_selection(10),
            KeyCode::Right | KeyCode::Enter => app.open_selected_trace(),
            KeyCode::Left | KeyCode::Esc => app.go_back(),
            KeyCode::Char('o' | 'O') => return UiAction::Optimize,
            KeyCode::Char('b' | 'B') => return UiAction::Dashboard,
            KeyCode::Char('p' | 'P') => return UiAction::PairDevice,
            KeyCode::Char('q' | 'Q') => return request_quit(app),
            _ => {}
        },
        View::Trace { .. } => match key.code {
            KeyCode::Up => app.move_observation_cursor(-1),
            KeyCode::Down => app.move_observation_cursor(1),
            KeyCode::PageUp => app.move_observation_cursor(-8),
            KeyCode::PageDown => app.move_observation_cursor(8),
            KeyCode::Right | KeyCode::Enter => app.inspect_observation_cursor(),
            KeyCode::Tab => app.cycle_trace_tab(),
            KeyCode::Char('k' | 'K') => app.scroll_trace_detail(-3),
            KeyCode::Char('j' | 'J') => app.scroll_trace_detail(3),
            KeyCode::Char('t' | 'T') => app.toggle_timeline(),
            KeyCode::Char('/') => app.search_active = true,
            KeyCode::Char('e' | 'E') => return UiAction::ExternalEditor,
            KeyCode::Char('o' | 'O') => return UiAction::Optimize,
            KeyCode::Char('b' | 'B') => return UiAction::Dashboard,
            KeyCode::Char('p' | 'P') => return UiAction::PairDevice,
            KeyCode::Left | KeyCode::Esc => app.go_back(),
            KeyCode::Char('q' | 'Q') => return request_quit(app),
            _ => {}
        },
        View::Optimize => match key.code {
            KeyCode::Up => app.move_comparison_selection(-1),
            KeyCode::Down => app.move_comparison_selection(1),
            KeyCode::PageUp => app.move_comparison_selection(-8),
            KeyCode::PageDown => app.move_comparison_selection(8),
            KeyCode::Char('[') => app.move_exclusion_selection(-1),
            KeyCode::Char(']') => app.move_exclusion_selection(1),
            KeyCode::Char('o' | 'O') => return UiAction::Optimize,
            KeyCode::Char('a' | 'A') => return UiAction::Activate,
            KeyCode::Char('u' | 'U') => return UiAction::Undo,
            KeyCode::Left | KeyCode::Esc => app.go_back(),
            KeyCode::Char('b' | 'B') => return UiAction::Dashboard,
            KeyCode::Char('p' | 'P') => return UiAction::PairDevice,
            KeyCode::Char('q' | 'Q') => return request_quit(app),
            _ => {}
        },
    }
    UiAction::Continue
}

fn handle_mouse(app: &mut App, mouse: MouseEvent) {
    match mouse.kind {
        MouseEventKind::ScrollUp => app.scroll_trace_detail(-3),
        MouseEventKind::ScrollDown => app.scroll_trace_detail(3),
        _ => {}
    }
}

fn request_quit(app: &mut App) -> UiAction {
    if app.active_invocations() > 0 || app.optimization_running || app.override_active {
        app.quit_confirmation = true;
        UiAction::Continue
    } else {
        UiAction::Quit
    }
}

fn apply_optimization_report(app: &mut App, report: OptimizationReport) {
    let comparison_id = report.comparison_id;
    let started_at_ms = report.started_at_ms;
    let completed_at_ms = report.completed_at_ms;
    let monotonic_duration_ms = report.monotonic_duration_ms;
    let recommendation = report.recommendation.clone();
    let route_labels = report.route_labels.clone();
    let history_notice = report.history_notice.clone();
    let remote_fallbacks = report
        .remote_fallbacks
        .iter()
        .map(|fallback| {
            let models = if fallback.models.is_empty() {
                "models not reported".to_string()
            } else {
                fallback.models.join(", ")
            };
            let capabilities = if fallback.capabilities.is_empty() {
                "capabilities not reported".to_string()
            } else {
                fallback.capabilities.join("+")
            };
            format!(
                "{} ({}) · {} · {}",
                fallback.display_name, fallback.provider_key, models, capabilities
            )
        })
        .collect();
    if let Some(project) = report.history_project.clone() {
        app.project = Some(project);
    }
    if let Some(deployment) = report.history_deployment.clone() {
        app.deployment = Some(deployment);
    }
    if let Some(loaded_models) = report.loaded_models_after {
        app.loaded_models = loaded_models;
    }
    let mut exclusions = report
        .candidate_evaluations
        .iter()
        .filter_map(|evaluation| {
            let CandidateOutcome::Excluded { reason, message } = &evaluation.outcome else {
                return None;
            };
            Some(OptimizationExclusion {
                route: route_labels
                    .get(&evaluation.agent_id)
                    .cloned()
                    .unwrap_or_else(|| evaluation.agent_id.clone()),
                capability: report
                    .route_capabilities
                    .get(&evaluation.agent_id)
                    .cloned()
                    .unwrap_or_else(|| "unknown".to_string()),
                provider: evaluation.provider_key.clone(),
                reason: exclusion_reason_label(*reason).to_string(),
                message: message.as_deref().map(crate::monitor::safe_error_message),
            })
        })
        .collect::<Vec<_>>();
    exclusions.extend(report.capture_issues.iter().map(|issue| {
        OptimizationExclusion {
            route: route_labels
                .get(&issue.route_key)
                .cloned()
                .unwrap_or_else(|| issue.route_key.clone()),
            capability: issue.capability.clone(),
            provider: issue.provider_key.clone(),
            reason: "capture unavailable".to_string(),
            message: Some(crate::monitor::safe_error_message(&issue.message)),
        }
    }));
    app.optimization_excluded_total = exclusions.len();
    app.optimization_exclusions = exclusions;
    app.selected_exclusion = app
        .selected_exclusion
        .min(app.optimization_exclusions.len().saturating_sub(1));
    app.comparison_rows = report
        .combinations
        .into_iter()
        .map(|measured| {
            let measured_route_labels = measured
                .plan
                .routes
                .keys()
                .filter_map(|route| {
                    route_labels
                        .get(route)
                        .cloned()
                        .map(|label| (route.clone(), label))
                })
                .collect();
            let (result, failure) = match measured.outcome {
                BenchmarkOutcome::Passed => (LaneOutcome::Passed, None),
                BenchmarkOutcome::Failed(error) => (
                    LaneOutcome::Failed,
                    Some(crate::monitor::safe_error_message(&error)),
                ),
            };
            let detail = failure.map_or_else(
                || measured.plan.explanation.clone(),
                |error| format!("{} · {error}", measured.plan.explanation),
            );
            ComparisonRow {
                id: measured.plan.id.clone(),
                name: measured.plan.label.clone(),
                plan: measured.plan,
                first_total: measured.first_total_ms.map(Duration::from_millis),
                first_run_cold: measured.first_run_cold,
                repeat_runs_resident: measured.repeat_runs_resident,
                total: measured
                    .repeat_total_ms
                    .as_ref()
                    .map(|metric| Duration::from_millis(metric.median)),
                total_range: measured.repeat_total_ms.as_ref().map(metric_summary),
                ttft: measured
                    .repeat_ttft_ms
                    .as_ref()
                    .map(|metric| Duration::from_millis(metric.median)),
                ttft_range: measured.repeat_ttft_ms.as_ref().map(metric_summary),
                tokens_per_second: measured.tokens_per_second,
                first_process_cpu_percent: measured.first_process_cpu_percent,
                process_cpu_percent: measured.process_cpu_percent,
                peak_rss_bytes: measured.peak_rss_bytes,
                route_labels: measured_route_labels,
                result,
                detail,
            }
        })
        .collect();
    app.selected_comparison = recommendation
        .as_deref()
        .and_then(|id| app.comparison_rows.iter().position(|row| row.id == id))
        .unwrap_or_default();
    app.optimization_summary = Some(OptimizationSummary {
        comparison_id,
        started_at_ms,
        completed_at_ms,
        monotonic_duration_ms,
        corpus_agents: report.corpus_agents,
        configured_local_models: report.configured_local_models,
        tested_models: report.coverage.tested_models,
        passed_models: report.coverage.passed_models,
        expected_pairs: report.coverage.expected_pairs,
        evaluated_pairs: report.coverage.evaluated_pairs,
        passed_pairs: report.coverage.passed_pairs,
        recommendation,
        not_exhaustive: report.not_exhaustive,
        sequential_replay: report.sequential_replay,
        device_architecture: std::env::consts::ARCH.to_string(),
        device_backend: report.device_backend,
        remote_fallbacks,
    });
    app.notice = Some(history_notice.map_or_else(
        || {
            format!(
                "Measured {} real route combination(s)",
                app.comparison_rows.len()
            )
        },
        |notice| {
            format!(
                "Measured {} real route combination(s) · {notice}",
                app.comparison_rows.len()
            )
        },
    ));
}

fn metric_summary(metric: &MetricRange) -> MetricSummary {
    MetricSummary {
        median: Duration::from_millis(metric.median),
        min: Duration::from_millis(metric.min),
        max: Duration::from_millis(metric.max),
        samples: metric.samples,
    }
}

fn exclusion_reason_label(reason: ExclusionReason) -> &'static str {
    match reason {
        ExclusionReason::CapabilityMismatch => "capability mismatch",
        ExclusionReason::Unavailable => "unavailable or replay-unsafe",
        ExclusionReason::LoadFailure => "model load failure",
        ExclusionReason::InsufficientMemory => "insufficient memory",
        ExclusionReason::ContractFailure => "response contract failure",
    }
}

fn activate_selected_combination(app: &mut App, optimization: Option<&OptimizationController>) {
    let Some(controller) = optimization else {
        app.notice = Some("Optimization is unavailable without the Agent Gateway".to_string());
        return;
    };
    let Some(row) = app.selected_comparison() else {
        app.notice = Some("Run a successful comparison before activating a plan".to_string());
        return;
    };
    if row.result != LaneOutcome::Passed {
        app.notice = Some("A failed route combination cannot be activated".to_string());
        return;
    }
    let plan = row.plan.clone();
    let label = row.name.clone();
    match controller.activate(&plan) {
        Ok(generation) => {
            sync_override_state(app, Some(controller));
            app.notice = Some(format!(
                "Activated {label} for new invocations (generation {generation})"
            ));
        }
        Err(error) => app.notice = Some(error),
    }
}

fn undo_active_combination(app: &mut App, optimization: Option<&OptimizationController>) {
    let Some(controller) = optimization else {
        app.notice = Some("Optimization is unavailable without the Agent Gateway".to_string());
        return;
    };
    match controller.undo() {
        Some(generation) => {
            sync_override_state(app, Some(controller));
            app.notice = Some(format!(
                "Restored the previous routes (generation {generation})"
            ));
        }
        None => app.notice = Some("There is no earlier route plan to restore".to_string()),
    }
}

fn sync_override_state(app: &mut App, optimization: Option<&OptimizationController>) {
    let Some(controller) = optimization else {
        app.override_active = false;
        app.override_generation = None;
        app.override_route_count = 0;
        return;
    };
    let snapshot = controller.route_overrides().snapshot();
    app.override_active = !snapshot.routes.is_empty();
    app.override_generation = (snapshot.generation > 0).then_some(snapshot.generation);
    app.override_route_count = snapshot.routes.len();
}

fn dashboard_target_url(base_url: &str, app: &App) -> String {
    let project_base = app
        .project
        .as_deref()
        .and_then(|project| dashboard_project_url(base_url, project))
        .unwrap_or_else(|| base_url.to_string());
    let (trace_id, observation_id) = match &app.view {
        View::Trace { trace_id, .. } => (*trace_id, app.selected_observation_id()),
        View::Agent { .. } => {
            let Some(trace_id) = app.selected_trace else {
                return project_base;
            };
            (trace_id, None)
        }
        View::Main => {
            let Some(trace_id) = app
                .selected_lane()
                .and_then(|lane| lane.representative())
                .map(|trace| trace.id)
            else {
                return project_base;
            };
            (trace_id, None)
        }
        View::Optimize => return project_base,
    };
    let trace_base = app
        .project
        .as_deref()
        .and_then(|project| dashboard_project_logs_url(base_url, project))
        .unwrap_or_else(|| base_url.to_string());
    append_invocation_query(&trace_base, trace_id, observation_id)
}

fn dashboard_project_url(base_url: &str, project: &str) -> Option<String> {
    dashboard_project_logs_url(base_url, project)
        .and_then(|logs_url| logs_url.strip_suffix("/logs").map(str::to_string))
}

fn dashboard_project_logs_url(base_url: &str, project: &str) -> Option<String> {
    let (without_fragment, fragment) = base_url
        .split_once('#')
        .map_or((base_url, None), |(base, fragment)| (base, Some(fragment)));
    if fragment.is_some_and(|fragment| fragment.starts_with('/') || fragment.starts_with("!/")) {
        return None;
    }
    let without_query = without_fragment
        .split_once('?')
        .map_or(without_fragment, |(base, _)| base)
        .trim_end_matches('/');
    let scheme_end = without_query.find("://")? + 3;
    if !matches!(&without_query[..scheme_end - 3], "http" | "https") {
        return None;
    }
    let origin_end = without_query[scheme_end..]
        .find('/')
        .map_or(without_query.len(), |offset| scheme_end + offset);
    let origin = &without_query[..origin_end];
    Some(format!(
        "{origin}/project/{}/logs",
        encode_path_segment(project)
    ))
}

fn append_invocation_query(
    base_url: &str,
    trace_id: uuid::Uuid,
    observation_id: Option<uuid::Uuid>,
) -> String {
    let (without_fragment, fragment) = base_url
        .split_once('#')
        .map_or((base_url, None), |(base, fragment)| (base, Some(fragment)));
    let separator = if without_fragment.ends_with('?') || without_fragment.ends_with('&') {
        ""
    } else if without_fragment.contains('?') {
        "&"
    } else {
        "?"
    };
    let mut target = format!("{without_fragment}{separator}invocationId={trace_id}");
    if let Some(observation_id) = observation_id {
        target.push_str("&observationId=");
        target.push_str(&observation_id.to_string());
    }
    if let Some(fragment) = fragment {
        target.push('#');
        target.push_str(fragment);
    }
    target
}

fn encode_path_segment(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
    }
    encoded
}

async fn open_selected_trace_in_editor(
    terminal: &mut TerminalSession,
    app: &mut App,
    events: &mut RuntimeEventReceiver,
) -> Result<PathBuf, String> {
    let View::Trace {
        agent_key,
        trace_id,
        ..
    } = &app.view
    else {
        return Err("Select a Trace before opening the external editor".to_string());
    };
    let trace = app
        .trace(agent_key, *trace_id)
        .ok_or_else(|| "The selected Trace is no longer available".to_string())?;
    let export = trace_export_value(trace, Instant::now());
    let path = write_secure_trace_export(trace.id, &export)?;
    let editor = editor_command();

    terminal.suspend()?;
    let editor_path = path.clone();
    let mut editor_task = tokio::task::spawn_blocking(move || run_editor(&editor, &editor_path));
    let mut events_closed = false;
    let editor_result = loop {
        tokio::select! {
            result = &mut editor_task => {
                break match result {
                    Ok(result) => result,
                    Err(error) => Err(format!("external editor task failed: {error}")),
                };
            }
            event = events.recv(), if !events_closed => {
                match event {
                    Some(event) => app.apply(event, Instant::now()),
                    None => events_closed = true,
                }
            }
        }
    };
    let resume_result = terminal.resume();
    resume_result?;
    editor_result?;
    Ok(path)
}

fn trace_export_value(trace: &model::TraceRecord, now: Instant) -> serde_json::Value {
    let observations = trace
        .observations
        .iter()
        .map(|observation| {
            serde_json::json!({
                "id": observation.id,
                "parentObservationId": observation.parent_observation_id,
                "type": observation.observation_type.label(),
                "name": observation.name,
                "stage": observation.stage.map(RuntimeStage::label),
                "status": observation.status,
                "startOffsetMs": observation.start_offset.map(|value| value.as_millis()),
                "endOffsetMs": observation.end_offset.map(|value| value.as_millis()),
                "elapsedMs": observation.elapsed.as_millis(),
                "requestElapsedMs": observation.request_elapsed.map(|value| value.as_millis()),
                "provider": observation.provider,
                "model": observation.model,
                "modelParameters": observation.model_parameters,
                "capability": observation.capability,
                "input": observation.input,
                "output": observation.output,
                "usage": {
                    "inputTokens": observation.input_tokens,
                    "outputTokens": observation.output_tokens,
                },
                "resident": observation.resident,
                "attributes": observation.attributes,
                "error": observation.error,
            })
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "trace": {
            "id": trace.id,
            "rootObservationId": trace.id,
            "parentObservationId": null,
            "type": model::ObservationType::Generation.label(),
            "name": "Agent invocation",
            "agentId": trace.agent_id,
            "sourceAgentId": trace.source_agent_id,
            "capability": trace.capability,
            "provider": trace.provider,
            "model": trace.model,
            "modelParameters": trace.model_parameters,
            "startedUnixMs": trace.started_unix_ms,
            "elapsedMs": trace.elapsed(now).as_millis(),
            "result": trace.outcome,
            "error": trace.error,
            "terminal": trace.terminal,
            "ttftMs": trace.ttft.map(|value| value.as_millis()),
            "tokensPerSecond": trace.tokens_per_second,
            "input": trace.input,
            "output": trace.output,
            "correlation": {"traceId": trace.id, "rootObservationId": trace.id},
        },
        "observations": observations,
        "scores": trace.feedback.iter().map(|feedback| serde_json::json!({
            "observationId": feedback.observation_id,
            "event": feedback.event,
            "outcome": feedback.outcome,
            "message": feedback.message,
            "path": feedback.path,
        })).collect::<Vec<_>>(),
        "redaction": {
            "request": if trace.io_dropped { "capture dropped" } else { "bounded redacted summary" },
            "response": if trace.io_dropped { "capture dropped" } else { "bounded redacted summary" },
            "truncated": trace.io_truncated,
            "credentials": "removed"
        }
    })
}

fn write_secure_trace_export(id: uuid::Uuid, value: &serde_json::Value) -> Result<PathBuf, String> {
    for attempt in 0..10 {
        let path = std::env::temp_dir().join(format!("vifu-trace-{}-{attempt}.json", id.simple()));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = match options.open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("could not create Trace export: {error}")),
        };
        serde_json::to_writer_pretty(&mut file, value)
            .map_err(|error| format!("could not serialize Trace export: {error}"))?;
        file.write_all(b"\n")
            .map_err(|error| format!("could not finish Trace export: {error}"))?;
        return Ok(path);
    }
    Err("could not reserve a unique Trace export path".to_string())
}

fn editor_command() -> String {
    std::env::var("VISUAL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            std::env::var("EDITOR")
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
        .unwrap_or_else(|| "vi".to_string())
}

fn run_editor(editor: &str, path: &Path) -> Result<(), String> {
    let mut parts = editor.split_ascii_whitespace();
    let program = parts
        .next()
        .ok_or_else(|| "VISUAL/EDITOR does not name an executable".to_string())?;
    let mut command = Command::new(program);
    command.args(parts).arg(path);
    configure_editor_stdio(&mut command)?;
    let status = command
        .status()
        .map_err(|error| format!("could not open {program}: {error}"))?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| format!("{program} exited with {status}"))
}

#[cfg(unix)]
fn configure_editor_stdio(command: &mut Command) -> Result<(), String> {
    let input = OpenOptions::new()
        .read(true)
        .open("/dev/tty")
        .map_err(|error| format!("could not open terminal input for editor: {error}"))?;
    let output = OpenOptions::new()
        .write(true)
        .open("/dev/tty")
        .map_err(|error| format!("could not open terminal output for editor: {error}"))?;
    let error = output
        .try_clone()
        .map_err(|error| format!("could not clone terminal output for editor: {error}"))?;
    command
        .stdin(Stdio::from(input))
        .stdout(Stdio::from(output))
        .stderr(Stdio::from(error));
    Ok(())
}

#[cfg(not(unix))]
fn configure_editor_stdio(command: &mut Command) -> Result<(), String> {
    command
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    Ok(())
}

struct TerminalSession {
    terminal: Terminal<TuiBackend>,
    #[cfg(unix)]
    output_redirect: Option<OutputRedirect>,
}

impl TerminalSession {
    fn enter() -> Result<Self, String> {
        #[cfg(unix)]
        let writer: Box<dyn Write> = Box::new(
            OpenOptions::new()
                .write(true)
                .open("/dev/tty")
                .map_err(|error| format!("could not open terminal for Vifu TUI: {error}"))?,
        );
        #[cfg(not(unix))]
        let writer: Box<dyn Write> = Box::new(io::stdout());

        #[cfg(unix)]
        let output_redirect = OutputRedirect::start()?;

        enable_raw_mode()
            .map_err(|error| format!("could not enable terminal raw mode: {error}"))?;
        let mut terminal = Terminal::new(CrosstermBackend::new(writer)).map_err(|error| {
            let _ = disable_raw_mode();
            format!("could not initialize Vifu terminal: {error}")
        })?;
        if let Err(error) = execute!(
            terminal.backend_mut(),
            EnterAlternateScreen,
            EnableMouseCapture,
            Hide
        ) {
            let _ = execute!(
                terminal.backend_mut(),
                Show,
                DisableMouseCapture,
                LeaveAlternateScreen
            );
            let _ = disable_raw_mode();
            return Err(format!("could not enter Vifu terminal screen: {error}"));
        }
        if let Err(error) = terminal.clear() {
            let _ = execute!(
                terminal.backend_mut(),
                Show,
                DisableMouseCapture,
                LeaveAlternateScreen
            );
            let _ = disable_raw_mode();
            return Err(format!("could not clear Vifu terminal: {error}"));
        }

        Ok(Self {
            terminal,
            #[cfg(unix)]
            output_redirect: Some(output_redirect),
        })
    }

    fn suspend(&mut self) -> Result<(), String> {
        self.terminal
            .show_cursor()
            .map_err(|error| format!("could not show terminal cursor: {error}"))?;
        execute!(
            self.terminal.backend_mut(),
            Show,
            DisableMouseCapture,
            LeaveAlternateScreen
        )
        .map_err(|error| format!("could not suspend Vifu TUI: {error}"))?;
        disable_raw_mode().map_err(|error| format!("could not restore terminal mode: {error}"))
    }

    fn resume(&mut self) -> Result<(), String> {
        enable_raw_mode()
            .map_err(|error| format!("could not resume terminal raw mode: {error}"))?;
        execute!(
            self.terminal.backend_mut(),
            EnterAlternateScreen,
            EnableMouseCapture,
            Hide
        )
        .map_err(|error| format!("could not resume Vifu TUI: {error}"))?;
        self.terminal
            .clear()
            .map_err(|error| format!("could not redraw Vifu TUI: {error}"))
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = self.terminal.show_cursor();
        let _ = execute!(
            self.terminal.backend_mut(),
            Show,
            DisableMouseCapture,
            LeaveAlternateScreen
        );
        let _ = disable_raw_mode();
        #[cfg(unix)]
        {
            self.output_redirect.take();
        }
    }
}

#[cfg(unix)]
struct OutputRedirect {
    saved_stdout: std::os::fd::OwnedFd,
    saved_stderr: std::os::fd::OwnedFd,
    log: File,
    path: PathBuf,
}

#[cfg(unix)]
impl OutputRedirect {
    fn start() -> Result<Self, String> {
        use std::os::fd::{AsRawFd, FromRawFd};
        use std::os::unix::fs::OpenOptionsExt;

        let (path, log) = (0..16)
            .find_map(|_| {
                let path = std::env::temp_dir().join(format!(
                    "vifu-tui-{}-{}.log",
                    std::process::id(),
                    Uuid::new_v4()
                ));
                match OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .mode(0o600)
                    .open(&path)
                {
                    Ok(log) => Some(Ok((path, log))),
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => None,
                    Err(error) => Some(Err(error)),
                }
            })
            .unwrap_or_else(|| {
                Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "could not allocate a unique private log name",
                ))
            })
            .map_err(|error| format!("could not create private TUI runtime log: {error}"))?;
        // SAFETY: dup returns new owned descriptors or -1 without changing the originals.
        let stdout_fd = unsafe { libc::dup(libc::STDOUT_FILENO) };
        if stdout_fd < 0 {
            let _ = std::fs::remove_file(&path);
            return Err(format!(
                "could not preserve terminal stdout: {}",
                io::Error::last_os_error()
            ));
        }
        // SAFETY: stdout_fd was returned by dup and is now uniquely owned here.
        let saved_stdout = unsafe { std::os::fd::OwnedFd::from_raw_fd(stdout_fd) };
        // SAFETY: dup returns new owned descriptors or -1 without changing the originals.
        let stderr_fd = unsafe { libc::dup(libc::STDERR_FILENO) };
        if stderr_fd < 0 {
            let _ = std::fs::remove_file(&path);
            return Err(format!(
                "could not preserve terminal stderr: {}",
                io::Error::last_os_error()
            ));
        }
        // SAFETY: stderr_fd was returned by dup and is now uniquely owned here.
        let saved_stderr = unsafe { std::os::fd::OwnedFd::from_raw_fd(stderr_fd) };
        // SAFETY: all descriptors are valid; dup2 atomically replaces only this process's stdout.
        if unsafe { libc::dup2(log.as_raw_fd(), libc::STDOUT_FILENO) } < 0 {
            let _ = std::fs::remove_file(&path);
            return Err(format!(
                "could not redirect runtime stdout: {}",
                io::Error::last_os_error()
            ));
        }
        // SAFETY: all descriptors are valid; dup2 atomically replaces only this process's stderr.
        if unsafe { libc::dup2(log.as_raw_fd(), libc::STDERR_FILENO) } < 0 {
            // SAFETY: saved_stdout is a valid duplicate of the original stdout.
            let _ = unsafe { libc::dup2(saved_stdout.as_raw_fd(), libc::STDOUT_FILENO) };
            let _ = std::fs::remove_file(&path);
            return Err(format!(
                "could not redirect runtime stderr: {}",
                io::Error::last_os_error()
            ));
        }
        Ok(Self {
            saved_stdout,
            saved_stderr,
            log,
            path,
        })
    }
}

#[cfg(unix)]
impl Drop for OutputRedirect {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;

        let _ = io::stdout().flush();
        let _ = io::stderr().flush();
        // SAFETY: the saved descriptors remain valid for this object's lifetime.
        let _ = unsafe { libc::dup2(self.saved_stdout.as_raw_fd(), libc::STDOUT_FILENO) };
        // SAFETY: the saved descriptors remain valid for this object's lifetime.
        let _ = unsafe { libc::dup2(self.saved_stderr.as_raw_fd(), libc::STDERR_FILENO) };
        let _ = self.log.flush();
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::time::{Duration, Instant};

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
    use serde_json::json;
    use uuid::Uuid;
    use vifu_gateway::optimization::{
        CandidateEvaluation, CandidateOutcome, CombinationKind, ExclusionReason, MetricRange,
        OptimizationCoverage, RouteCombination,
    };

    use super::{
        apply_optimization_report, dashboard_target_url, handle_key, handle_mouse, request_quit,
        trace_export_value, write_secure_trace_export, UiAction,
    };
    use crate::benchmark::{
        BenchmarkOutcome, MeasuredCombination, OptimizationReport, RemoteFallback,
    };
    use crate::monitor::{RegisteredAgent, RuntimeEvent, RuntimeStage, StageStatus};
    use crate::tui::model::{App, TraceTab, View};

    #[test]
    fn mouse_wheel_scrolls_trace_detail_without_changing_other_views() {
        let mut app = App::default();
        let mouse = |kind| MouseEvent {
            kind,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        };

        handle_mouse(&mut app, mouse(MouseEventKind::ScrollDown));
        assert_eq!(app.trace_detail_scroll, 0);

        app.view = View::Trace {
            agent_key: "planner\0".to_string(),
            trace_id: Uuid::new_v4(),
            tab: TraceTab::Summary,
            timeline: false,
            observation_cursor: None,
            selected_observation: None,
        };
        handle_mouse(&mut app, mouse(MouseEventKind::ScrollDown));
        assert_eq!(app.trace_detail_scroll, 3);
        handle_mouse(&mut app, mouse(MouseEventKind::ScrollUp));
        assert_eq!(app.trace_detail_scroll, 0);
    }

    #[cfg(unix)]
    #[test]
    fn trace_export_should_be_owner_readable_and_writable_only() {
        use std::os::unix::fs::PermissionsExt;

        let path = write_secure_trace_export(Uuid::new_v4(), &json!({"trace": {}})).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        let _ = fs::remove_file(path);

        assert_eq!(mode, 0o600);
    }

    #[test]
    fn dashboard_target_uses_the_selected_recent_trace() {
        let mut app = App::default();
        assert_eq!(
            dashboard_target_url("http://127.0.0.1:8787", &app),
            "http://127.0.0.1:8787"
        );
        let trace_id = Uuid::from_u128(0x42);
        app.view = View::Agent {
            agent_key: "planner\0".to_string(),
        };
        app.selected_trace = Some(trace_id);
        app.project = Some("demo/東京".to_string());

        assert_eq!(
            dashboard_target_url("https://vifu.test/overview?source=tui#detail", &app),
            format!(
                "https://vifu.test/project/demo%2F%E6%9D%B1%E4%BA%AC/logs?invocationId={trace_id}"
            )
        );
        assert_eq!(
            dashboard_target_url("https://vifu.test/#/project/demo/logs", &app),
            format!("https://vifu.test/?invocationId={trace_id}#/project/demo/logs")
        );
    }

    #[test]
    fn dashboard_target_uses_the_current_project_when_main_view_has_no_trace() {
        let mut app = App::default();
        app.project = Some("stardew-valley".to_string());

        assert_eq!(
            dashboard_target_url("http://127.0.0.1:6790", &app),
            "http://127.0.0.1:6790/project/stardew-valley"
        );
    }

    #[test]
    fn dashboard_target_deep_links_the_selected_main_view_trace() {
        let now = Instant::now();
        let trace_id = Uuid::new_v4();
        let mut app = App::default();
        app.project = Some("stardew-valley".to_string());
        app.apply(
            RuntimeEvent::InvocationStarted {
                invocation_id: trace_id,
                agent_id: "planner".to_string(),
                agent_name: "Planner".to_string(),
                source_agent_id: "planner".to_string(),
                capability: "chat".to_string(),
                provider: "local".to_string(),
                model: "qwen".to_string(),
                started_unix_ms: 1,
            },
            now,
        );

        assert_eq!(
            dashboard_target_url("http://127.0.0.1:6790", &app),
            format!("http://127.0.0.1:6790/project/stardew-valley/logs?invocationId={trace_id}")
        );
    }

    #[test]
    fn dashboard_target_deep_links_the_selected_observation() {
        let now = Instant::now();
        let trace_id = Uuid::new_v4();
        let observation_id = Uuid::new_v4();
        let mut app = App::default();
        app.apply(
            RuntimeEvent::InvocationStarted {
                invocation_id: trace_id,
                agent_id: "planner".to_string(),
                agent_name: "Planner".to_string(),
                source_agent_id: "planner".to_string(),
                capability: "chat".to_string(),
                provider: "local".to_string(),
                model: "qwen".to_string(),
                started_unix_ms: 1,
            },
            now,
        );
        app.apply(
            RuntimeEvent::StageChanged {
                invocation_id: trace_id,
                observation_id,
                stage: RuntimeStage::Decode,
                status: StageStatus::Active,
                start_offset: Duration::from_millis(10),
                end_offset: None,
                elapsed: Duration::from_millis(2),
                request_elapsed: Some(Duration::from_millis(12)),
                input_tokens: None,
                output_tokens: Some(1),
                resident: Some(true),
                error: None,
            },
            now,
        );
        app.view = View::Trace {
            agent_key: "planner\0".to_string(),
            trace_id,
            tab: TraceTab::Summary,
            timeline: false,
            observation_cursor: Some(observation_id),
            selected_observation: Some(observation_id),
        };

        assert_eq!(
            dashboard_target_url("https://vifu.test", &app),
            format!("https://vifu.test?invocationId={trace_id}&observationId={observation_id}")
        );
    }

    #[test]
    fn trace_export_preserves_generic_observation_identity_and_order() {
        let now = Instant::now();
        let trace_id = Uuid::new_v4();
        let first_id = Uuid::new_v4();
        let second_id = Uuid::new_v4();
        let mut app = App::default();
        app.apply(
            RuntimeEvent::InvocationStarted {
                invocation_id: trace_id,
                agent_id: "planner".to_string(),
                agent_name: "Planner".to_string(),
                source_agent_id: "planner".to_string(),
                capability: "chat".to_string(),
                provider: "local".to_string(),
                model: "qwen".to_string(),
                started_unix_ms: 1,
            },
            now,
        );
        for (observation_id, stage) in [
            (first_id, RuntimeStage::Prefill),
            (second_id, RuntimeStage::Decode),
        ] {
            app.apply(
                RuntimeEvent::StageChanged {
                    invocation_id: trace_id,
                    observation_id,
                    stage,
                    status: StageStatus::Passed,
                    start_offset: Duration::ZERO,
                    end_offset: Some(Duration::from_millis(1)),
                    elapsed: Duration::from_millis(1),
                    request_elapsed: Some(Duration::from_millis(1)),
                    input_tokens: None,
                    output_tokens: None,
                    resident: None,
                    error: None,
                },
                now,
            );
        }
        let trace = app.trace("planner\0", trace_id).unwrap();

        let export = trace_export_value(trace, now);
        let observations = export["observations"].as_array().unwrap();

        assert_eq!(observations.len(), 2);
        assert_eq!(observations[0]["id"], json!(first_id));
        assert_eq!(observations[1]["id"], json!(second_id));
        assert_eq!(observations[0]["parentObservationId"], json!(trace_id));
        assert_eq!(observations[0]["type"], "span");
        assert_eq!(observations[0]["name"], "Prefill");
        assert_eq!(export["trace"]["type"], "generation");
    }

    #[test]
    fn active_route_override_requires_quit_confirmation() {
        let mut app = App::default();
        app.override_active = true;

        assert_eq!(request_quit(&mut app), UiAction::Continue);
        assert!(app.quit_confirmation);
    }

    #[test]
    fn running_optimization_requires_quit_confirmation() {
        let mut app = App::default();
        app.optimization_running = true;

        assert_eq!(request_quit(&mut app), UiAction::Continue);
        assert!(app.quit_confirmation);
    }

    #[test]
    fn trace_keyboard_movement_immediately_previews_the_observation() {
        let now = Instant::now();
        let mut app = App::default();
        app.apply(
            RuntimeEvent::AgentsRegistered(vec![RegisteredAgent {
                id: "planner".to_string(),
                name: "Planner".to_string(),
                provider: "local".to_string(),
                capability: "chat".to_string(),
                model: "qwen".to_string(),
                local_model_loaded: false,
            }]),
            now,
        );
        let trace_id = Uuid::new_v4();
        app.apply(
            RuntimeEvent::InvocationStarted {
                invocation_id: trace_id,
                agent_id: "planner".to_string(),
                agent_name: "Planner".to_string(),
                source_agent_id: "planner".to_string(),
                capability: "chat".to_string(),
                provider: "local".to_string(),
                model: "qwen".to_string(),
                started_unix_ms: 1,
            },
            now,
        );
        let observation_id = Uuid::new_v4();
        app.apply(
            RuntimeEvent::StageChanged {
                invocation_id: trace_id,
                observation_id,
                stage: RuntimeStage::Connect,
                status: StageStatus::Passed,
                start_offset: Duration::ZERO,
                end_offset: Some(Duration::from_millis(1)),
                elapsed: Duration::from_millis(1),
                request_elapsed: Some(Duration::from_millis(1)),
                input_tokens: None,
                output_tokens: None,
                resident: None,
                error: None,
            },
            now,
        );
        app.selected_lane = Some("planner\0".to_string());
        app.open_selected_agent();
        app.open_selected_trace();

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            now,
        );

        assert!(matches!(
            app.view,
            View::Trace {
                observation_cursor: Some(cursor),
                selected_observation: Some(selected),
                ..
            } if cursor == observation_id && selected == observation_id
        ));
    }

    #[test]
    fn optimization_report_keeps_ranges_and_redacted_exclusions() {
        let route = Uuid::from_u128(1).to_string();
        let plan = RouteCombination {
            id: "fastest-local".to_string(),
            label: "fastest-local".to_string(),
            kind: CombinationKind::FastestLocal,
            explanation: "Measured fastest local route".to_string(),
            routes: BTreeMap::from([(route.clone(), "qwen".to_string())]),
        };
        let comparison_id = Uuid::new_v4();
        let mut app = App::default();

        apply_optimization_report(
            &mut app,
            OptimizationReport {
                comparison_id,
                started_at_ms: 1_000,
                completed_at_ms: 1_900,
                monotonic_duration_ms: 900,
                coverage: OptimizationCoverage {
                    configured_models: 2,
                    tested_models: 2,
                    passed_models: 1,
                    expected_pairs: 2,
                    evaluated_pairs: 2,
                    passed_pairs: 1,
                    missing: Vec::new(),
                },
                candidate_evaluations: vec![CandidateEvaluation {
                    agent_id: route.clone(),
                    provider_key: "broken".to_string(),
                    outcome: CandidateOutcome::Excluded {
                        reason: ExclusionReason::LoadFailure,
                        message: Some("api_key=private model load failed".to_string()),
                    },
                }],
                combinations: vec![MeasuredCombination {
                    plan,
                    outcome: BenchmarkOutcome::Passed,
                    first_total_ms: Some(40),
                    first_run_cold: Some(true),
                    repeat_runs_resident: Some(true),
                    repeat_total_ms: Some(MetricRange {
                        median: 20,
                        min: 10,
                        max: 30,
                        samples: 3,
                    }),
                    repeat_ttft_ms: None,
                    tokens_per_second: Some(12.0),
                    first_process_cpu_percent: Some(180.0),
                    process_cpu_percent: Some(140.0),
                    peak_rss_bytes: Some(1024),
                }],
                recommendation: Some("fastest-local".to_string()),
                corpus_agents: 1,
                configured_local_models: 2,
                remote_fallbacks: vec![RemoteFallback {
                    provider_key: "remote-chat".to_string(),
                    display_name: "Remote Chat".to_string(),
                    capabilities: vec!["chat".to_string()],
                    models: vec!["hosted-model".to_string()],
                }],
                route_capabilities: BTreeMap::from([(route.clone(), "chat".to_string())]),
                route_labels: BTreeMap::from([(route, "NPC planner · chat".to_string())]),
                device_backend: Some("llama.cpp".to_string()),
                loaded_models_after: Some(1),
                history_project: Some("stardojo".to_string()),
                history_deployment: Some("local-a".to_string()),
                history_saved: true,
                history_notice: Some("Saved comparison to Dashboard history".to_string()),
                not_exhaustive: true,
                sequential_replay: true,
                capture_issues: Vec::new(),
            },
        );

        let range = app.comparison_rows[0].total_range.as_ref().unwrap();
        assert_eq!(range.samples, 3);
        assert_eq!(range.min, Duration::from_millis(10));
        assert_eq!(range.max, Duration::from_millis(30));
        assert_eq!(app.comparison_rows[0].process_cpu_percent, Some(140.0));
        assert_eq!(app.loaded_models, 1);
        let summary = app.optimization_summary.as_ref().unwrap();
        assert_eq!(summary.comparison_id, comparison_id);
        assert_eq!(summary.started_at_ms, 1_000);
        assert_eq!(summary.completed_at_ms, 1_900);
        assert_eq!(summary.monotonic_duration_ms, 900);
        assert_eq!(
            summary.remote_fallbacks,
            vec!["Remote Chat (remote-chat) · hosted-model · chat"]
        );
        assert_eq!(app.project.as_deref(), Some("stardojo"));
        assert_eq!(app.deployment.as_deref(), Some("local-a"));
        assert_eq!(app.optimization_excluded_total, 1);
        assert_eq!(
            app.optimization_exclusions[0].message.as_deref(),
            Some("Provider failed; sensitive details were redacted")
        );
    }
}
