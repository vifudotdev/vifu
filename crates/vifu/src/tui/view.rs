use std::collections::HashSet;
use std::time::{Duration, Instant};

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, Wrap};
use ratatui::Frame;
use uuid::Uuid;

use crate::monitor::{FeedbackEvent, FeedbackOutcome, RuntimeStage, RuntimeTerminal, StageStatus};

use super::model::{
    AgentLane, App, ComparisonRow, FeedbackRecord, LaneFilter, LaneOutcome, MetricSummary,
    ObservationType, SystemMetrics, TraceObservation, TraceRecord, TraceTab, View,
};

const RAIL_QUANTUM: Duration = Duration::from_millis(250);

pub(crate) fn render(frame: &mut Frame<'_>, app: &mut App, now: Instant, no_color: bool) {
    let area = frame.area();
    if area.width < 42 || area.height < 11 {
        render_too_small(frame, area);
        return;
    }

    match app.view.clone() {
        View::Main => render_main(frame, app, area, now, no_color),
        View::Agent { agent_key } => render_agent_live(frame, app, area, &agent_key, now, no_color),
        View::Trace { .. } => render_trace(frame, app, area, now, no_color),
        View::Optimize => render_optimize(frame, app, area, no_color),
    }

    if app.quit_confirmation {
        render_quit_confirmation(frame, area, app.active_invocations(), app.override_active);
    }
}

fn render_main(frame: &mut Frame<'_>, app: &mut App, area: Rect, now: Instant, no_color: bool) {
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Min(4),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(area);

    render_runtime_header(frame, app, sections[0], no_color);
    render_counts(frame, app, sections[1]);

    let visible_rows = sections[2].height.saturating_sub(1) as usize;
    app.ensure_lane_visible(now, visible_rows);
    render_lanes(frame, app, sections[2], now, no_color);
    render_selected_stage_strip(frame, app, sections[3], no_color);
    render_footer(
        frame,
        sections[4],
        if app.search_active {
            "Search: type · Enter Keep · Esc Cancel"
        } else {
            "↑↓ Select  → Focus  O Optimize  F Filter  / Search  S Sort  B Dashboard  Q Quit"
        },
        app.notice.as_deref(),
    );
}

fn render_runtime_header(frame: &mut Frame<'_>, app: &App, area: Rect, no_color: bool) {
    let project = app.project.as_deref().unwrap_or("project pending");
    let dashboard = app
        .project_dashboard_url
        .as_deref()
        .unwrap_or("dashboard unavailable");
    let deployment = app.deployment.as_deref().unwrap_or("deployment pending");
    let first = Line::styled(
        format!(" {project} / {dashboard} / {deployment}"),
        if no_color {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        },
    );
    let second = Line::raw(format_system_metrics(
        app.metrics,
        app.loaded_models,
        std::env::consts::ARCH,
        &app.runtime_backends,
        area.width,
    ));
    frame.render_widget(Paragraph::new(vec![first, second]), area);
}

fn render_counts(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let counts = app.counts();
    let first = Line::raw(format!(
        " {} AGENTS   RUNNING {}   PASS {}   FAILED {}   TIMEOUT {}   UNKNOWN {}",
        counts.total, counts.running, counts.passed, counts.failed, counts.timeout, counts.unknown
    ));
    let search = if app.search.is_empty() {
        String::new()
    } else {
        format!("   Search: {}", app.search)
    };
    let second = Line::raw(format!(
        " Filter: {}{}                                      Sort: {}",
        app.filter.label(),
        search,
        app.sort.label()
    ));
    frame.render_widget(Paragraph::new(vec![first, second]), area);
}

fn render_lanes(frame: &mut Frame<'_>, app: &App, area: Rect, now: Instant, no_color: bool) {
    let keys = app.visible_lane_keys(now);
    let available_rows = area.height.saturating_sub(1) as usize;
    let visible = keys
        .iter()
        .enumerate()
        .skip(app.scroll_offset)
        .take(available_rows)
        .filter_map(|(index, key)| app.lane(key).map(|lane| (index, lane)));
    let selected = app.selected_lane.as_deref();
    let wide = area.width >= 112;
    let medium = area.width >= 76;

    let rows = visible.map(|(index, lane)| {
        let trace = lane.representative();
        let never_run = trace.is_none();
        let outcome = lane.outcome();
        let stage = trace
            .map(TraceRecord::current_stage)
            .unwrap_or(RuntimeStage::Connect);
        let elapsed = trace.map(|trace| trace.elapsed(now));
        let concurrency = (lane.concurrency() > 1).then(|| format!(" ×{}", lane.concurrency()));
        let request = elapsed.map_or_else(
            || "— NOT RUN YET".to_string(),
            |elapsed| elapsed_rail(outcome, elapsed, stage, if wide { 24 } else { 17 }),
        );
        let is_selected = selected == Some(lane.key.as_str());
        let mut cells = vec![
            Cell::from(Line::from(vec![
                selection_marker(is_selected, no_color),
                Span::raw(format!("{:02}", index + 1)),
            ])),
            Cell::from(format!("{}{}", lane.name, concurrency.unwrap_or_default())),
        ];
        if medium {
            cells.push(Cell::from(shorten_capability(
                trace.map_or(lane.capability.as_str(), |trace| trace.capability.as_str()),
            )));
            cells.push(Cell::from(
                trace.map_or(lane.model.as_str(), |trace| trace.model.as_str()),
            ));
        }
        cells.push(Cell::from(request));
        if wide {
            cells.push(Cell::from(format_optional_duration(
                trace.and_then(|trace| trace.ttft),
            )));
            cells.push(Cell::from(format_rate(
                trace.and_then(|trace| trace.tokens_per_second),
            )));
        }
        cells.push(Cell::from(if never_run {
            "—".to_string()
        } else {
            format!("{} {}", outcome.symbol(), outcome.label())
        }));
        let style = if is_selected {
            outcome_style(outcome, no_color).add_modifier(Modifier::BOLD)
        } else {
            outcome_style(outcome, no_color)
        };
        Row::new(cells).style(style)
    });

    let (headers, widths) = if wide {
        (
            vec![
                "#",
                "AGENT",
                "CAP",
                "MODEL",
                "REQUEST / TIME",
                "TTFT",
                "RATE",
                "RESULT",
            ],
            vec![
                Constraint::Length(4),
                Constraint::Min(18),
                Constraint::Length(9),
                Constraint::Length(16),
                Constraint::Min(24),
                Constraint::Length(9),
                Constraint::Length(9),
                Constraint::Length(10),
            ],
        )
    } else if medium {
        (
            vec!["#", "AGENT", "CAP", "MODEL", "REQUEST / TIME", "RESULT"],
            vec![
                Constraint::Length(4),
                Constraint::Min(16),
                Constraint::Length(9),
                Constraint::Length(15),
                Constraint::Min(17),
                Constraint::Length(10),
            ],
        )
    } else {
        (
            vec!["#", "AGENT", "REQUEST / TIME", "RESULT"],
            vec![
                Constraint::Length(4),
                Constraint::Min(13),
                Constraint::Min(17),
                Constraint::Length(10),
            ],
        )
    };

    let table = Table::new(rows, widths)
        .header(Row::new(headers).style(Style::default().add_modifier(Modifier::BOLD)))
        .column_spacing(1);
    frame.render_widget(table, area);

    if keys.is_empty() {
        let message = if app.filter == LaneFilter::Live && app.search.is_empty() {
            "Waiting for live Agent activity…"
        } else if app.counts().total == 0 {
            "Waiting for configured Agents and real invocations…"
        } else {
            "No Agents match the current filter/search."
        };
        let empty = Rect {
            x: area.x,
            y: area.y.saturating_add(2),
            width: area.width,
            height: 1,
        };
        frame.render_widget(Paragraph::new(message).alignment(Alignment::Center), empty);
    }
}

fn render_selected_stage_strip(frame: &mut Frame<'_>, app: &App, area: Rect, no_color: bool) {
    let Some(lane) = app.selected_lane() else {
        frame.render_widget(Paragraph::new(" SELECTED —\n No real Agent selected"), area);
        return;
    };
    let trace = lane.representative();
    let identity = trace.map_or_else(
        || format!("SELECTED {} · no invocation yet", lane.name),
        |trace| format!("SELECTED {} · {}", lane.name, short_uuid(trace.id)),
    );
    let mut stages = Vec::new();
    for stage in RuntimeStage::ORDERED {
        let status = trace
            .map(|trace| trace.observation_status(stage))
            .unwrap_or(StageStatus::Unknown);
        stages.push(Span::styled(
            format!("{} {}  ", stage.label(), stage_status_symbol(status)),
            stage_style(status, no_color),
        ));
    }
    frame.render_widget(
        Paragraph::new(vec![Line::raw(identity), Line::from(stages)])
            .block(Block::default().borders(Borders::TOP))
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn render_agent_live(
    frame: &mut Frame<'_>,
    app: &App,
    area: Rect,
    agent_key: &str,
    now: Instant,
    no_color: bool,
) {
    let Some(lane) = app.lane(agent_key) else {
        frame.render_widget(Paragraph::new("Agent is no longer available"), area);
        return;
    };
    let capability_height = u16::try_from(lane.capabilities().len())
        .unwrap_or(u16::MAX)
        .saturating_add(1)
        .min(area.height.saturating_sub(9).max(2));
    let capability_noun = if lane.capabilities().len() == 1 {
        "capability"
    } else {
        "capabilities"
    };
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(capability_height),
            Constraint::Min(4),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(area);
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(
                format!(" {}", lane.name),
                if no_color {
                    Style::default().add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                },
            ),
            Line::raw(format!(
                " Agent {} · {} {} · {} active requests",
                lane.agent_id,
                lane.capabilities().len(),
                capability_noun,
                lane.concurrency()
            )),
        ]),
        sections[0],
    );

    render_agent_capabilities(frame, lane, sections[1], now, no_color);
    render_agent_requests(frame, app, lane, sections[2], now, no_color);
    render_agent_selected_request(frame, app, lane, sections[3], now, no_color);
    render_footer(
        frame,
        sections[4],
        "↑↓ Select  →/Enter Inspect  ←/Esc Agents  O Optimize  B Dashboard",
        app.notice.as_deref(),
    );
}

fn render_agent_capabilities(
    frame: &mut Frame<'_>,
    lane: &AgentLane,
    area: Rect,
    now: Instant,
    no_color: bool,
) {
    let wide = area.width >= 96;
    let rows = lane.capabilities().iter().map(|capability| {
        let trace = lane.live_trace_for_capability(capability);
        let active = lane.active_for_capability(capability);
        let outcome = if active > 0 {
            LaneOutcome::Running
        } else {
            trace.map_or(LaneOutcome::Unknown, |trace| trace.outcome)
        };
        let request = trace.map_or_else(
            || "— waiting".to_string(),
            |trace| {
                elapsed_rail(
                    outcome,
                    trace.elapsed(now),
                    trace.current_stage(),
                    if wide { 22 } else { 16 },
                )
            },
        );
        let mut cells = vec![
            Cell::from(shorten_capability(capability)),
            Cell::from(active.to_string()),
            Cell::from(request),
        ];
        if wide {
            cells.push(Cell::from(format_optional_duration(
                trace.and_then(|trace| trace.ttft),
            )));
            cells.push(Cell::from(format_rate(
                trace.and_then(|trace| trace.tokens_per_second),
            )));
        }
        cells.push(Cell::from(if trace.is_none() {
            "— WAITING".to_string()
        } else {
            format!("{} {}", outcome.symbol(), outcome.label())
        }));
        Row::new(cells).style(outcome_style(outcome, no_color))
    });
    let (headers, widths) = if wide {
        (
            vec![
                "CAPABILITY",
                "ACTIVE",
                "REQUEST / TIME",
                "TTFT",
                "RATE",
                "RESULT",
            ],
            vec![
                Constraint::Length(13),
                Constraint::Length(7),
                Constraint::Min(22),
                Constraint::Length(9),
                Constraint::Length(9),
                Constraint::Length(11),
            ],
        )
    } else {
        (
            vec!["CAPABILITY", "ACTIVE", "REQUEST / TIME", "RESULT"],
            vec![
                Constraint::Length(13),
                Constraint::Length(7),
                Constraint::Min(16),
                Constraint::Length(11),
            ],
        )
    };
    frame.render_widget(
        Table::new(rows, widths)
            .header(Row::new(headers).style(Style::default().add_modifier(Modifier::BOLD)))
            .column_spacing(1),
        area,
    );
}

fn render_agent_requests(
    frame: &mut Frame<'_>,
    app: &App,
    lane: &AgentLane,
    area: Rect,
    now: Instant,
    no_color: bool,
) {
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(area);
    frame.render_widget(
        Paragraph::new(" LIVE REQUESTS").style(Style::default().add_modifier(Modifier::BOLD)),
        sections[0],
    );
    let traces = lane.live_traces();
    let visible_rows = sections[1].height.saturating_sub(1) as usize;
    let selected_index = app
        .selected_trace
        .and_then(|selected| traces.iter().position(|trace| trace.id == selected))
        .unwrap_or_default();
    let row_start = selection_window_start(traces.len(), selected_index, visible_rows);
    let rows = traces
        .iter()
        .enumerate()
        .skip(row_start)
        .take(visible_rows)
        .map(|(index, trace)| {
            let selected = app.selected_trace == Some(trace.id);
            let mut cells = vec![
                Cell::from(Line::from(vec![
                    selection_marker(selected, no_color),
                    Span::raw(format!("{:02}", index + 1)),
                ])),
                Cell::from(shorten_capability(&trace.capability)),
                Cell::from(short_uuid(trace.id)),
                Cell::from(trace.model.clone()),
                Cell::from(elapsed_rail(
                    trace.outcome,
                    trace.elapsed(now),
                    trace.current_stage(),
                    20,
                )),
            ];
            if area.width >= 112 {
                cells.push(Cell::from(format_optional_duration(trace.ttft)));
                cells.push(Cell::from(format_rate(trace.tokens_per_second)));
            }
            cells.push(Cell::from(format!(
                "{} {}",
                trace.outcome.symbol(),
                trace.outcome.label()
            )));
            Row::new(cells).style(if selected {
                outcome_style(trace.outcome, no_color).add_modifier(Modifier::BOLD)
            } else {
                outcome_style(trace.outcome, no_color)
            })
        });
    let (headers, widths) = if area.width >= 112 {
        (
            vec![
                "#",
                "TYPE",
                "REQUEST",
                "MODEL",
                "REQUEST / TIME",
                "TTFT",
                "RATE",
                "RESULT",
            ],
            vec![
                Constraint::Length(5),
                Constraint::Length(11),
                Constraint::Length(14),
                Constraint::Length(18),
                Constraint::Min(20),
                Constraint::Length(9),
                Constraint::Length(9),
                Constraint::Length(11),
            ],
        )
    } else {
        (
            vec!["#", "TYPE", "REQUEST", "MODEL", "REQUEST / TIME", "RESULT"],
            vec![
                Constraint::Length(5),
                Constraint::Length(11),
                Constraint::Length(14),
                Constraint::Length(16),
                Constraint::Min(16),
                Constraint::Length(11),
            ],
        )
    };
    frame.render_widget(
        Table::new(rows, widths)
            .header(Row::new(headers).style(Style::default().add_modifier(Modifier::BOLD)))
            .column_spacing(1),
        sections[1],
    );
    if traces.is_empty() {
        frame.render_widget(
            Paragraph::new("Waiting for this Agent's first live request…")
                .alignment(Alignment::Center),
            sections[1],
        );
    }
}

fn render_agent_selected_request(
    frame: &mut Frame<'_>,
    app: &App,
    lane: &AgentLane,
    area: Rect,
    now: Instant,
    no_color: bool,
) {
    let trace = app.selected_trace.and_then(|trace_id| lane.trace(trace_id));
    let identity = trace.map_or_else(
        || "SELECTED — waiting for a live request".to_string(),
        |trace| {
            format!(
                "SELECTED {} · {} · {} · {}",
                shorten_capability(&trace.capability),
                short_uuid(trace.id),
                trace.current_stage().label(),
                format_duration(trace.elapsed(now))
            )
        },
    );
    let stages = RuntimeStage::ORDERED.into_iter().map(|stage| {
        let status = trace.map_or(StageStatus::Unknown, |trace| {
            trace.observation_status(stage)
        });
        Span::styled(
            format!("{} {}  ", stage.label(), stage_status_symbol(status)),
            stage_style(status, no_color),
        )
    });
    frame.render_widget(
        Paragraph::new(vec![
            Line::raw(identity),
            Line::from(stages.collect::<Vec<_>>()),
        ])
        .block(Block::default().borders(Borders::TOP))
        .wrap(Wrap { trim: true }),
        area,
    );
}

fn render_trace(frame: &mut Frame<'_>, app: &App, area: Rect, now: Instant, no_color: bool) {
    let View::Trace {
        agent_key,
        trace_id,
        tab,
        timeline,
        observation_cursor,
        selected_observation,
    } = &app.view
    else {
        return;
    };
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Min(5),
            Constraint::Length(1),
        ])
        .split(area);
    let Some(trace) = app.trace(agent_key, *trace_id) else {
        frame.render_widget(Paragraph::new("Trace is no longer available"), area);
        return;
    };
    let agent_name = app
        .lane(agent_key)
        .map_or(trace.agent_id.as_str(), |lane| lane.name.as_str());
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(
                format!(
                    " TRACE {}  {} {}  {}",
                    short_uuid(trace.id),
                    trace.outcome.symbol(),
                    trace.outcome.label(),
                    format_duration(trace.elapsed(now))
                ),
                outcome_style(trace.outcome, no_color).add_modifier(Modifier::BOLD),
            ),
            Line::raw(format!(
                " {} · {} · {} · {}",
                agent_name, trace.capability, trace.provider, trace.model
            )),
        ]),
        sections[0],
    );
    let tabs = [
        TraceTab::Summary,
        TraceTab::Io,
        TraceTab::Metadata,
        TraceTab::Scores,
        TraceTab::Events,
    ]
    .into_iter()
    .map(|item| {
        if item == *tab {
            Span::styled(
                format!("[{}] ", item.label()),
                selected_style(Style::default(), no_color),
            )
        } else {
            Span::raw(format!(" {}  ", item.label()))
        }
    })
    .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(Line::from(tabs)), sections[1]);

    let body = if area.width >= 82 {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
            .split(sections[2])
            .to_vec()
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(48), Constraint::Percentage(52)])
            .split(sections[2])
            .to_vec()
    };
    render_observations(
        frame,
        TraceObservationPane {
            trace,
            area: body[0],
            timeline: *timeline,
            now,
            observation_cursor: *observation_cursor,
            selected_observation: *selected_observation,
            search: &app.search,
            no_color,
        },
    );
    render_trace_detail(
        frame,
        trace,
        body[1],
        TraceDetailContext {
            tab: *tab,
            selected_observation: *selected_observation,
            now,
            search: &app.search,
            scroll: app.trace_detail_scroll,
            no_color,
        },
    );
    render_footer(
        frame,
        sections[3],
        if app.search_active {
            "Search Trace: type · Enter Keep · Esc Cancel"
        } else if area.width < 70 {
            "↑↓ Preview  Wheel/JK Scroll  Tab Detail  ← Back"
        } else {
            "↑↓ Preview  Wheel/JK Scroll  Tab Detail  T Tree/Timeline  / Search  E Editor  B Dashboard  ← Back"
        },
        app.notice.as_deref(),
    );
}

struct TraceObservationPane<'a> {
    trace: &'a TraceRecord,
    area: Rect,
    timeline: bool,
    now: Instant,
    observation_cursor: Option<Uuid>,
    selected_observation: Option<Uuid>,
    search: &'a str,
    no_color: bool,
}

fn render_observations(frame: &mut Frame<'_>, pane: TraceObservationPane<'_>) {
    let TraceObservationPane {
        trace,
        area,
        timeline,
        now,
        observation_cursor,
        selected_observation,
        search,
        no_color,
    } = pane;
    let searching = !search.trim().is_empty();
    let root_matches = trace.root_matches_search(search);
    let matching_observations = trace
        .observations
        .iter()
        .filter(|observation| trace.observation_matches_search(observation, search))
        .collect::<Vec<_>>();
    let match_count = usize::from(root_matches).saturating_add(matching_observations.len());
    let root_cursor = root_matches && observation_cursor.is_none();
    let root_selected = selected_observation.is_none();
    let root_style = selection_style(
        outcome_style(trace.outcome, no_color),
        root_cursor,
        root_selected,
    );
    let root = root_matches.then(|| {
        Row::new([
            Cell::from(Line::from(vec![
                selection_marker(root_cursor, no_color),
                Span::raw(format!(
                    "{}{} Generation",
                    if root_selected { "●" } else { " " },
                    if searching { " *" } else { "" },
                )),
            ])),
            Cell::from(format_duration(trace.elapsed(now))),
        ])
        .style(root_style)
    });
    let observations = matching_observations.iter().map(|observation| {
        let status = observation.status;
        let detail = if timeline {
            format!(
                "{} {}",
                stage_status_symbol(status),
                format_duration(observation.elapsed)
            )
        } else {
            format_duration(observation.elapsed)
        };
        let is_cursor = observation_cursor == Some(observation.id);
        let is_selected = selected_observation == Some(observation.id);
        let indent = "  ".repeat(observation_depth(trace, observation));
        Row::new([
            Cell::from(Line::from(vec![
                selection_marker(is_cursor, no_color),
                Span::raw(format!(
                    "{}{} {indent}{} {} ({})",
                    if is_selected { "●" } else { " " },
                    if searching { " *" } else { "" },
                    stage_status_symbol(status),
                    observation.name,
                    observation.observation_type.label(),
                )),
            ])),
            Cell::from(detail),
        ])
        .style(selection_style(
            stage_style(status, no_color),
            is_cursor,
            is_selected,
        ))
    });
    let visible_rows = area.height.saturating_sub(2) as usize;
    let cursor_index = observation_cursor
        .and_then(|cursor| {
            matching_observations
                .iter()
                .position(|observation| observation.id == cursor)
                .map(|index| index.saturating_add(usize::from(root_matches)))
        })
        .unwrap_or_default();
    let total_rows = match_count;
    let row_start = selection_window_start(total_rows, cursor_index, visible_rows);
    let no_match = (searching && match_count == 0).then(|| {
        Row::new([
            Cell::from("No matching observations"),
            Cell::from("Refine / search"),
        ])
    });
    let rows = no_match
        .into_iter()
        .chain(root)
        .chain(observations)
        .skip(row_start)
        .take(visible_rows);
    let mode = if timeline { "Timeline" } else { "Tree" };
    let title = if searching {
        match match_count {
            0 => format!(" {mode} · no matches "),
            1 => format!(" {mode} · 1 match "),
            count => format!(" {mode} · {count} matches "),
        }
    } else {
        format!(" {mode} ")
    };
    frame.render_widget(
        Table::new(rows, [Constraint::Min(18), Constraint::Length(21)])
            .block(Block::default().borders(Borders::ALL).title(title)),
        area,
    );
}

fn observation_depth(trace: &TraceRecord, observation: &TraceObservation) -> usize {
    let mut depth = 1_usize;
    let mut parent = observation.parent_observation_id;
    let mut visited = HashSet::new();
    while let Some(parent_id) = parent {
        if parent_id == trace.id || !visited.insert(parent_id) {
            break;
        }
        let Some(parent_observation) = trace.observation(parent_id) else {
            break;
        };
        depth = depth.saturating_add(1).min(trace.observations.len().max(1));
        parent = parent_observation.parent_observation_id;
    }
    depth
}

struct TraceDetailContext<'a> {
    tab: TraceTab,
    selected_observation: Option<Uuid>,
    now: Instant,
    search: &'a str,
    scroll: u16,
    no_color: bool,
}

fn render_trace_detail(
    frame: &mut Frame<'_>,
    trace: &TraceRecord,
    area: Rect,
    context: TraceDetailContext<'_>,
) {
    if let Some(observation_id) = context.selected_observation {
        render_observation_detail(frame, trace, area, observation_id, &context);
        return;
    }
    let TraceDetailContext {
        tab,
        now,
        search,
        scroll,
        no_color,
        ..
    } = context;
    let search_match = trace.matches_search(search);
    let lines = match tab {
        TraceTab::Summary => {
            let slowest = trace
                .observations
                .iter()
                .filter(|observation| observation.status != StageStatus::Unknown)
                .max_by_key(|observation| observation.elapsed);
            let first_failure = RuntimeStage::ORDERED
                .iter()
                .find(|stage| trace.observation_status(**stage) == StageStatus::Failed);
            let first_failed_index = RuntimeStage::ORDERED
                .iter()
                .position(|stage| trace.observation_status(*stage) == StageStatus::Failed);
            let problem_boundary = first_failed_index.or_else(|| {
                (trace.outcome == LaneOutcome::Timeout).then(|| {
                    RuntimeStage::ORDERED
                        .iter()
                        .position(|stage| *stage == RuntimeStage::Deliver)
                        .unwrap_or(RuntimeStage::ORDERED.len())
                })
            });
            let last_success = RuntimeStage::ORDERED
                .iter()
                .enumerate()
                .take(problem_boundary.unwrap_or(RuntimeStage::ORDERED.len()))
                .rev()
                .find(|(_, stage)| trace.observation_status(**stage) == StageStatus::Passed)
                .map(|(_, stage)| stage);
            let (blocked, unknown) = RuntimeStage::ORDERED.iter().enumerate().fold(
                (0_usize, 0_usize),
                |(blocked, unknown), (index, stage)| {
                    if trace.observation_status(*stage) != StageStatus::Unknown {
                        (blocked, unknown)
                    } else if first_failed_index.is_some_and(|failed| index > failed) {
                        (blocked + 1, unknown)
                    } else {
                        (blocked, unknown + 1)
                    }
                },
            );
            let first_problem = first_failure.map_or_else(
                || {
                    if trace.outcome == LaneOutcome::Timeout {
                        "request idle timeout".to_string()
                    } else {
                        "none observed".to_string()
                    }
                },
                |stage| {
                    let error = trace
                        .observation_for_stage(*stage)
                        .and_then(|observation| observation.error.as_deref());
                    error.map_or_else(
                        || stage.label().to_string(),
                        |error| format!("{}: {error}", stage.label()),
                    )
                },
            );
            let runtime_result = trace_result_summary(trace);
            let mut lines = vec![Line::raw(runtime_result)];
            lines.extend(trace_summary_io_lines(trace, no_color));
            lines.extend([
                Line::raw(format!("Total: {}", format_duration(trace.elapsed(now)))),
                Line::raw(format!(
                    "Longest observed: {}",
                    slowest.map_or_else(
                        || "unknown".to_string(),
                        |observation| format!(
                            "{} {}",
                            observation.name,
                            format_duration(observation.elapsed)
                        )
                    )
                )),
                Line::raw(format!(
                    "Last successful observation: {}",
                    last_success.map_or("none observed", |stage| stage.label())
                )),
                Line::raw(format!("First error/timeout: {first_problem}")),
                Line::raw(format!(
                    "Unobserved: {unknown} unknown · {blocked} blocked after failure"
                )),
                Line::raw(format!(
                    "Error: {}",
                    trace.error.as_deref().unwrap_or("none")
                )),
                search_status_line(search, search_match),
            ]);
            lines
        }
        TraceTab::Io => render_io_lines(trace, search, search_match, no_color),
        TraceTab::Metadata => vec![
            Line::raw(format!("Trace / root observation ID: {}", trace.id)),
            Line::raw(format!("Type: {}", ObservationType::Generation.label())),
            Line::raw("Name: Agent invocation"),
            Line::raw(format!("Agent: {}", trace.agent_id)),
            Line::raw(format!("Gateway Agent: {}", trace.source_agent_id)),
            Line::raw(format!("Capability: {}", trace.capability)),
            Line::raw(format!("Provider: {}", trace.provider)),
            Line::raw(format!("Model: {}", trace.model)),
            Line::raw(format!(
                "Model parameters: {}",
                compact_json(trace.model_parameters.as_ref())
            )),
            Line::raw(format!("Started (Unix ms): {}", trace.started_unix_ms)),
            Line::raw(format!("TTFT: {}", format_optional_duration(trace.ttft))),
            Line::raw(format!(
                "Decode rate: {}",
                format_rate(trace.tokens_per_second)
            )),
            Line::raw(format!(
                "I/O capture: {}",
                if trace.io_dropped {
                    "dropped (capture queue was full)"
                } else if trace.io_truncated {
                    "bounded/redacted (truncated)"
                } else {
                    "bounded/redacted"
                }
            )),
            Line::raw(format!(
                "Correlation: trace={} · root={}",
                trace.id, trace.id
            )),
            search_status_line(search, search_match),
        ],
        TraceTab::Scores => render_score_lines(trace, search, search_match),
        TraceTab::Events => trace
            .observations
            .iter()
            .filter(|observation| trace.observation_matches_search(observation, search))
            .map(|observation| {
                Line::raw(format!(
                    "{}  {}  {}{}{}",
                    format_duration(observation.elapsed),
                    observation.name,
                    stage_status_label(observation.status),
                    observation
                        .request_elapsed
                        .map_or_else(String::new, |elapsed| {
                            format!(" · request {}", format_duration(elapsed))
                        }),
                    observation
                        .error
                        .as_deref()
                        .map_or_else(String::new, |error| { format!(" · {error}") })
                ))
            })
            .collect(),
    };
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" {} · Trace ", tab.label())),
            )
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0)),
        area,
    );
}

fn trace_result_summary(trace: &TraceRecord) -> String {
    let runtime = match trace.terminal {
        Some(RuntimeTerminal::Delivered) => "Runtime passed",
        Some(RuntimeTerminal::TimedOut) => "Runtime timed out",
        Some(
            RuntimeTerminal::ProviderFailed
            | RuntimeTerminal::DeliveryFailed
            | RuntimeTerminal::PreflightFailed,
        ) => "Runtime failed",
        None => "Runtime running",
    };
    let outcomes = [
        FeedbackEvent::OutputAccepted,
        FeedbackEvent::ActionApplied,
        FeedbackEvent::FramePresented,
    ]
    .map(|event| {
        trace
            .feedback
            .iter()
            .rev()
            .find(|feedback| feedback.event == event)
            .map(|feedback| feedback.outcome)
    });
    let application = if outcomes.contains(&Some(FeedbackOutcome::Fail)) {
        "Application failed"
    } else if outcomes.iter().all(|outcome| {
        matches!(
            outcome,
            Some(FeedbackOutcome::Pass | FeedbackOutcome::NotApplicable)
        )
    }) {
        "Application passed"
    } else {
        "Application outcome unknown"
    };
    format!("{runtime} · {application}")
}

fn render_observation_detail(
    frame: &mut Frame<'_>,
    trace: &TraceRecord,
    area: Rect,
    observation_id: Uuid,
    context: &TraceDetailContext<'_>,
) {
    let TraceDetailContext {
        tab,
        search,
        scroll,
        no_color,
        ..
    } = context;
    let observation = trace.observation(observation_id);
    let search = search.trim();
    let search_match = observation
        .is_some_and(|observation| trace.observation_matches_search(observation, search));
    let lines = match (*tab, observation) {
        (_, None) => vec![
            Line::raw(format!(
                "Observation {observation_id} is no longer in this Trace"
            )),
            search_status_line(search, search_match),
        ],
        (TraceTab::Summary, Some(observation)) => vec![
            Line::raw(format!("Observation: {}", observation.name)),
            Line::raw(format!("Type: {}", observation.observation_type.label())),
            Line::raw(format!(
                "Status: {}",
                stage_status_label(observation.status)
            )),
            Line::raw(format!("Elapsed: {}", format_duration(observation.elapsed))),
            Line::raw(format!(
                "Request elapsed: {}",
                format_optional_duration(observation.request_elapsed)
            )),
            Line::raw(format!(
                "Error: {}",
                observation.error.as_deref().unwrap_or("none")
            )),
            search_status_line(search, search_match),
        ],
        (TraceTab::Io, Some(observation)) => {
            let mut lines = Vec::new();
            append_io_lines(&mut lines, "INPUT", observation.input.as_ref(), *no_color);
            append_io_lines(&mut lines, "OUTPUT", observation.output.as_ref(), *no_color);
            lines.push(search_status_line(search, search_match));
            lines
        }
        (TraceTab::Metadata, Some(observation)) => vec![
            Line::raw(format!("Trace: {}", trace.id)),
            Line::raw(format!("Observation ID: {}", observation.id)),
            Line::raw(format!(
                "Parent observation ID: {}",
                observation
                    .parent_observation_id
                    .map_or_else(|| "none".to_string(), |value| value.to_string())
            )),
            Line::raw(format!("Type: {}", observation.observation_type.label())),
            Line::raw(format!("Name: {}", observation.name)),
            Line::raw(format!(
                "Provider: {}",
                observation.provider.as_deref().unwrap_or("not reported")
            )),
            Line::raw(format!(
                "Model: {}",
                observation.model.as_deref().unwrap_or("not reported")
            )),
            Line::raw(format!(
                "Model parameters: {}",
                compact_json(observation.model_parameters.as_ref())
            )),
            Line::raw(format!(
                "Capability: {}",
                observation.capability.as_deref().unwrap_or("not reported")
            )),
            Line::raw(format!(
                "Request elapsed: {}",
                format_optional_duration(observation.request_elapsed)
            )),
            Line::raw(format!(
                "Input / output tokens: {} / {}",
                observation
                    .input_tokens
                    .map_or_else(|| "--".to_string(), |value| value.to_string()),
                observation
                    .output_tokens
                    .map_or_else(|| "--".to_string(), |value| value.to_string())
            )),
            Line::raw(format!(
                "Resident: {}",
                observation
                    .resident
                    .map_or("unknown", |resident| if resident { "yes" } else { "no" })
            )),
            Line::raw(format!(
                "Correlation: trace={} · observation={}",
                trace.id, observation.id
            )),
            Line::raw(format!(
                "Attributes: {}",
                compact_json(Some(&observation.attributes))
            )),
            search_status_line(search, search_match),
        ],
        (TraceTab::Scores, Some(_)) => {
            let feedback = trace
                .feedback
                .iter()
                .filter(|feedback| feedback.observation_id == observation_id)
                .collect::<Vec<_>>();
            if feedback.is_empty() {
                vec![
                    Line::raw("No Score is attached to this observation."),
                    search_status_line(search, search_match),
                ]
            } else {
                let mut lines = Vec::new();
                for feedback in feedback {
                    append_feedback_lines(&mut lines, feedback);
                }
                lines.push(search_status_line(search, search_match));
                lines
            }
        }
        (TraceTab::Events, Some(observation)) => vec![
            Line::raw(format!(
                "{}  {}  {}{}",
                format_duration(observation.elapsed),
                observation.name,
                stage_status_label(observation.status),
                observation
                    .error
                    .as_deref()
                    .map_or_else(String::new, |error| format!(" · {error}"))
            )),
            Line::raw(format!(
                "Attributes: {}",
                compact_json(Some(&observation.attributes))
            )),
            search_status_line(search, search_match),
        ],
    };
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title(format!(
                " {} · {} ",
                tab.label(),
                observation
                    .map(|item| item.name.as_str())
                    .unwrap_or("Observation")
            )))
            .wrap(Wrap { trim: false })
            .scroll((*scroll, 0)),
        area,
    );
}

fn compact_json(value: Option<&serde_json::Value>) -> String {
    value.map_or_else(|| "not reported".to_string(), serde_json::Value::to_string)
}

fn search_status_line(search: &str, matched: bool) -> Line<'static> {
    if search.trim().is_empty() {
        Line::raw("")
    } else if matched {
        Line::raw(format!("Search: {search} · match"))
    } else {
        Line::raw(format!("Search: {search} · no match"))
    }
}

fn render_io_lines(
    trace: &TraceRecord,
    search: &str,
    matched: bool,
    no_color: bool,
) -> Vec<Line<'static>> {
    let mut lines = vec![Line::raw(format!(
        "Capture: {}",
        if trace.io_dropped {
            "DROPPED — bounded capture queue was full"
        } else if trace.io_truncated {
            "REDACTED + TRUNCATED"
        } else {
            "REDACTED + BOUNDED"
        }
    ))];
    append_io_lines(&mut lines, "INPUT", trace.input.as_ref(), no_color);
    append_io_lines(&mut lines, "OUTPUT", trace.output.as_ref(), no_color);
    lines.push(search_status_line(search, matched));
    lines
}

fn trace_summary_io_lines(trace: &TraceRecord, no_color: bool) -> Vec<Line<'static>> {
    let input_messages = trace.input.as_ref().map_or_else(Vec::new, chat_messages);
    let output_messages = trace.output.as_ref().map_or_else(Vec::new, chat_messages);
    let top_level_system = trace
        .input
        .as_ref()
        .and_then(|value| value.get("system"))
        .filter(|_| !chat_messages_include_role(&input_messages, "system"));
    if top_level_system.is_none() && input_messages.is_empty() && output_messages.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![section_heading("CONVERSATION".to_string())];
    if let Some(system) = top_level_system {
        append_role_content(
            &mut lines,
            "SYSTEM",
            message_content(Some(system)),
            no_color,
        );
    }
    for (index, message) in input_messages
        .into_iter()
        .chain(output_messages)
        .enumerate()
    {
        if index > 0 || top_level_system.is_some() {
            lines.push(Line::raw(""));
        }
        append_chat_message(&mut lines, message, no_color);
    }
    lines
}

fn append_io_lines(
    lines: &mut Vec<Line<'static>>,
    label: &str,
    value: Option<&serde_json::Value>,
    no_color: bool,
) {
    let Some(value) = value else {
        lines.push(section_heading(format!("{label} · NOT CAPTURED")));
        return;
    };
    let messages = chat_messages(value);
    if !messages.is_empty() {
        lines.push(section_heading(format!("{label} · CHAT")));
        if let Some(model) = value.get("model").and_then(serde_json::Value::as_str) {
            lines.push(Line::raw(format!("MODEL  {model}")));
        }
        let top_level_system = value
            .get("system")
            .filter(|_| !chat_messages_include_role(&messages, "system"));
        if let Some(system) = top_level_system {
            append_role_content(lines, "SYSTEM", message_content(Some(system)), no_color);
        }
        for (index, message) in messages.into_iter().enumerate() {
            if index > 0 || top_level_system.is_some() {
                lines.push(Line::raw(""));
            }
            append_chat_message(lines, message, no_color);
        }
        return;
    }
    if append_embedding_lines(lines, label, value) {
        return;
    }
    lines.push(section_heading(format!("{label} · DATA")));
    append_structured_value(lines, value, 0);
}

fn section_heading(title: String) -> Line<'static> {
    Line::styled(title, Style::default().add_modifier(Modifier::BOLD))
}

fn chat_messages(value: &serde_json::Value) -> Vec<&serde_json::Value> {
    if let Some(messages) = value.get("messages").and_then(serde_json::Value::as_array) {
        return messages.iter().collect();
    }
    if let Some(choices) = value.get("choices").and_then(serde_json::Value::as_array) {
        let messages = choices
            .iter()
            .filter_map(|choice| choice.get("message"))
            .collect::<Vec<_>>();
        if !messages.is_empty() {
            return messages;
        }
    }
    if let Some(input) = value.get("input").and_then(serde_json::Value::as_array) {
        let messages = input
            .iter()
            .filter(|item| is_chat_or_tool_item(item))
            .collect::<Vec<_>>();
        if !messages.is_empty() {
            return messages;
        }
    }
    if let Some(output) = value.get("output").and_then(serde_json::Value::as_array) {
        let messages = output
            .iter()
            .filter(|item| is_chat_or_tool_item(item))
            .collect::<Vec<_>>();
        if !messages.is_empty() {
            return messages;
        }
    }
    if let Some(message) = value
        .get("message")
        .filter(|message| message.get("role").is_some())
    {
        return vec![message];
    }
    if value.get("role").is_some() {
        return vec![value];
    }
    value.as_array().map_or_else(Vec::new, |values| {
        values
            .iter()
            .filter(|item| is_chat_or_tool_item(item))
            .collect()
    })
}

fn is_chat_or_tool_item(value: &serde_json::Value) -> bool {
    value.get("role").is_some()
        || value
            .get("type")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|kind| {
                matches!(kind, "message" | "function_call" | "tool_call" | "tool_use")
            })
}

fn chat_messages_include_role(messages: &[&serde_json::Value], role: &str) -> bool {
    messages.iter().any(|message| {
        message
            .get("role")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message_role| message_role.eq_ignore_ascii_case(role))
    })
}

fn is_tool_call_item(value: &serde_json::Value) -> bool {
    value
        .get("type")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|kind| matches!(kind, "function_call" | "tool_call" | "tool_use"))
}

fn append_chat_message(
    lines: &mut Vec<Line<'static>>,
    message: &serde_json::Value,
    no_color: bool,
) {
    if message.get("role").is_none() && is_tool_call_item(message) {
        append_tool_call(lines, message, no_color);
        return;
    }
    let role = message
        .get("role")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("message")
        .to_ascii_uppercase();
    let content = message_content(
        message
            .get("content")
            .filter(|content| !content.is_null())
            .or_else(|| message.get("refusal")),
    );
    append_role_content(lines, &role, content, no_color);
    if let Some(tool_calls) = message
        .get("tool_calls")
        .or_else(|| message.get("toolCalls"))
        .and_then(serde_json::Value::as_array)
    {
        for tool_call in tool_calls {
            append_tool_call(lines, tool_call, no_color);
        }
    }
    if let Some(function_call) = message
        .get("function_call")
        .or_else(|| message.get("functionCall"))
    {
        append_tool_call(lines, function_call, no_color);
    }
    if let Some(parts) = message.get("content").and_then(serde_json::Value::as_array) {
        for part in parts.iter().filter(|part| is_tool_call_item(part)) {
            append_tool_call(lines, part, no_color);
        }
    }
}

fn append_role_content(
    lines: &mut Vec<Line<'static>>,
    role: &str,
    content: Vec<String>,
    no_color: bool,
) {
    lines.push(Line::styled(
        role.to_string(),
        chat_role_style(role, no_color, true),
    ));
    for text in content {
        lines.push(Line::styled(
            format!("  {text}"),
            chat_role_style(role, no_color, false),
        ));
    }
}

fn chat_role_style(role: &str, no_color: bool, bold: bool) -> Style {
    let mut style = Style::default();
    if !no_color {
        style = style.fg(match role.to_ascii_lowercase().as_str() {
            "system" | "developer" => Color::Magenta,
            "user" => Color::Cyan,
            "assistant" => Color::Green,
            "tool" | "function" => Color::Yellow,
            _ => Color::White,
        });
    }
    if bold {
        style.add_modifier(Modifier::BOLD)
    } else {
        style
    }
}

fn message_content(value: Option<&serde_json::Value>) -> Vec<String> {
    match value {
        None | Some(serde_json::Value::Null) => Vec::new(),
        Some(serde_json::Value::String(text)) => text.lines().map(str::to_string).collect(),
        Some(serde_json::Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| match part {
                serde_json::Value::String(text) => Some(text.clone()),
                serde_json::Value::Object(object) => object
                    .get("text")
                    .or_else(|| object.get("input_text"))
                    .or_else(|| object.get("output_text"))
                    .or_else(|| object.get("refusal"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
                    .or_else(|| {
                        object
                            .get("type")
                            .and_then(serde_json::Value::as_str)
                            .filter(|kind| kind.contains("image") || kind.contains("media"))
                            .map(|_| "[image/media omitted]".to_string())
                    }),
                _ => None,
            })
            .collect(),
        Some(value) => vec![scalar_text(value)],
    }
}

fn append_tool_call(lines: &mut Vec<Line<'static>>, tool_call: &serde_json::Value, no_color: bool) {
    let function = tool_call.get("function").unwrap_or(tool_call);
    let name = function
        .get("name")
        .or_else(|| function.get("toolName"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unnamed tool");
    lines.push(Line::styled(
        format!("TOOL  {name}"),
        chat_role_style("tool", no_color, true),
    ));
    let Some(arguments) = function
        .get("arguments")
        .or_else(|| function.get("args"))
        .or_else(|| function.get("input"))
    else {
        return;
    };
    if let Some(encoded) = arguments.as_str() {
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(encoded) {
            append_structured_value(lines, &parsed, 2);
        } else {
            lines.push(Line::raw(format!("  arguments: {encoded}")));
        }
    } else {
        append_structured_value(lines, arguments, 2);
    }
}

fn append_embedding_lines(
    lines: &mut Vec<Line<'static>>,
    label: &str,
    value: &serde_json::Value,
) -> bool {
    if let Some(input) = value.get("input") {
        lines.push(section_heading(format!("{label} · TEXT")));
        append_structured_value(lines, input, 0);
        return true;
    }
    let Some(data) = value.get("data").and_then(serde_json::Value::as_array) else {
        return false;
    };
    let Some(first_embedding) = data
        .first()
        .and_then(|item| item.get("embedding"))
        .and_then(serde_json::Value::as_array)
    else {
        return false;
    };
    lines.push(section_heading(format!("{label} · EMBEDDINGS")));
    lines.push(Line::raw(format!(
        "{} vectors · {} captured dimensions{}",
        data.len(),
        first_embedding.len(),
        if first_embedding.len() >= 16 {
            " (bounded)"
        } else {
            ""
        }
    )));
    true
}

fn append_structured_value(
    lines: &mut Vec<Line<'static>>,
    value: &serde_json::Value,
    indent: usize,
) {
    let padding = " ".repeat(indent);
    match value {
        serde_json::Value::Object(object) => {
            if object.is_empty() {
                lines.push(Line::raw(format!("{padding}<empty>")));
                return;
            }
            for (key, value) in object {
                if is_scalar(value) {
                    lines.push(Line::raw(format!(
                        "{padding}{}: {}",
                        readable_key(key),
                        scalar_text(value)
                    )));
                } else {
                    lines.push(Line::raw(format!("{padding}{}:", readable_key(key))));
                    append_structured_value(lines, value, indent.saturating_add(2));
                }
            }
        }
        serde_json::Value::Array(values) => {
            if values.is_empty() {
                lines.push(Line::raw(format!("{padding}<empty>")));
            } else if values.iter().all(serde_json::Value::is_number) {
                lines.push(Line::raw(format!(
                    "{padding}<{} numeric values>",
                    values.len()
                )));
            } else if values.iter().all(is_scalar) {
                for value in values {
                    lines.push(Line::raw(format!("{padding}• {}", scalar_text(value))));
                }
            } else {
                for (index, value) in values.iter().enumerate() {
                    lines.push(Line::raw(format!("{padding}#{}", index + 1)));
                    append_structured_value(lines, value, indent.saturating_add(2));
                }
            }
        }
        value => lines.push(Line::raw(format!("{padding}{}", scalar_text(value)))),
    }
}

fn is_scalar(value: &serde_json::Value) -> bool {
    matches!(
        value,
        serde_json::Value::Null
            | serde_json::Value::Bool(_)
            | serde_json::Value::Number(_)
            | serde_json::Value::String(_)
    )
}

fn scalar_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::String(value) => value.clone(),
        serde_json::Value::Array(values) => format!("{} items", values.len()),
        serde_json::Value::Object(values) => format!("{} fields", values.len()),
    }
}

fn readable_key(key: &str) -> String {
    key.replace('_', " ")
}

fn render_score_lines(trace: &TraceRecord, search: &str, matched: bool) -> Vec<Line<'static>> {
    let mut lines = if trace.feedback.is_empty() {
        vec![
            Line::raw("No application feedback was observed."),
            Line::raw("Missing feedback is unknown, never a failure."),
        ]
    } else {
        let mut lines = Vec::new();
        for feedback in &trace.feedback {
            append_feedback_lines(&mut lines, feedback);
        }
        lines
    };
    lines.push(search_status_line(search, matched));
    lines
}

fn feedback_event_label(event: FeedbackEvent) -> &'static str {
    match event {
        FeedbackEvent::OutputAccepted => "output accepted",
        FeedbackEvent::ActionApplied => "action applied",
        FeedbackEvent::FramePresented => "frame presented",
    }
}

fn feedback_event_explanation(event: FeedbackEvent) -> &'static str {
    match event {
        FeedbackEvent::OutputAccepted => {
            "Whether the application accepted and parsed the model output"
        }
        FeedbackEvent::ActionApplied => {
            "Whether the requested tool or application action was applied"
        }
        FeedbackEvent::FramePresented => {
            "Whether the resulting frame became visible in the application"
        }
    }
}

fn append_feedback_lines(lines: &mut Vec<Line<'static>>, feedback: &FeedbackRecord) {
    lines.push(section_heading(format!(
        "{} · {}",
        feedback_event_label(feedback.event).to_ascii_uppercase(),
        feedback_outcome_label(feedback.outcome)
    )));
    lines.push(Line::raw(feedback_event_explanation(feedback.event)));
    if let Some(path) = feedback.path.as_deref() {
        lines.push(Line::raw(format!("Path: {path}")));
    }
    if let Some(message) = feedback.message.as_deref() {
        lines.push(Line::raw(format!("Detail: {message}")));
    }
}

fn feedback_outcome_label(outcome: FeedbackOutcome) -> &'static str {
    match outcome {
        FeedbackOutcome::Pass => "PASS",
        FeedbackOutcome::Fail => "FAIL",
        FeedbackOutcome::Unknown => "UNKNOWN",
        FeedbackOutcome::NotApplicable => "N/A",
    }
}

fn render_optimize(frame: &mut Frame<'_>, app: &App, area: Rect, no_color: bool) {
    let selected_height = if area.height >= 18 {
        6
    } else if area.height >= 14 {
        5
    } else {
        3
    };
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Min(4),
            Constraint::Length(selected_height),
            Constraint::Length(1),
        ])
        .split(area);
    let state = if app.optimization_running {
        "● MEASURING"
    } else {
        "READY"
    };
    let coverage = app.optimization_summary.as_ref().map_or_else(
        || "Coverage: waiting for a measured corpus".to_string(),
        |summary| {
            format!(
                "Corpus {} · models {}/{} tested ({} pass) · pairs {}/{} ({} pass, {} excluded) · COMBINATIONS {}/8",
                summary.corpus_agents,
                summary.tested_models,
                summary.configured_local_models,
                summary.passed_models,
                summary.evaluated_pairs,
                summary.expected_pairs,
                summary.passed_pairs,
                summary.evaluated_pairs.saturating_sub(summary.passed_pairs),
                app.comparison_rows.len()
            )
        },
    );
    let search_scope = app.optimization_summary.as_ref().map_or(
        "NOT EXHAUSTIVE · sequential replay · at most eight explainable combinations",
        |summary| {
            if summary.not_exhaustive {
                "NOT EXHAUSTIVE · sequential runtime/contract replay"
            } else if summary.sequential_replay {
                "Sequential replay · all configured combinations measured"
            } else {
                "All configured combinations measured"
            }
        },
    );
    let override_status = if app.override_active {
        format!(
            "ACTIVE OVERRIDE · {} routes · generation {}",
            app.override_route_count,
            app.override_generation.unwrap_or_default()
        )
    } else {
        "No active route override".to_string()
    };
    let remote_status = app.optimization_summary.as_ref().map_or_else(
        || {
            "REMOTE FALLBACKS — inventory pending · not measured (local optimization default)"
                .to_string()
        },
        |summary| {
            let inventory = if summary.remote_fallbacks.is_empty() {
                String::new()
            } else {
                format!(" · {}", summary.remote_fallbacks.join(" · "))
            };
            format!(
                "REMOTE FALLBACKS {}{inventory} · not measured (local optimization default)",
                summary.remote_fallbacks.len()
            )
        },
    );
    let search_scope = app.optimization_summary.as_ref().map_or_else(
        || search_scope.to_string(),
        |summary| {
            let backend = summary
                .device_backend
                .as_deref()
                .unwrap_or("backend unknown");
            format!(
                "{search_scope} · {} / {backend}",
                architecture_label(&summary.device_architecture)
            )
        },
    );
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(
                format!(" OPTIMIZE  {state}"),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Line::raw(format!(" {coverage}")),
            Line::raw(format!(" {search_scope}")),
            Line::raw(format!(" {remote_status}")),
            Line::raw(format!(" {override_status}")),
        ]),
        sections[0],
    );
    if app.comparison_rows.is_empty() {
        let message = if app.optimization_running {
            "Running real configured models…\n\nThe Runtime and Agent monitor remain live."
        } else {
            "No measured comparison exists yet.\n\nPress O after at least one successful real request."
        };
        frame.render_widget(
            Paragraph::new(message).alignment(Alignment::Center).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Configurations "),
            ),
            sections[1],
        );
    } else {
        let wide = area.width >= 104;
        let medium = area.width >= 72;
        let recommendation = app
            .optimization_summary
            .as_ref()
            .and_then(|summary| summary.recommendation.as_deref());
        let visible_rows = sections[1].height.saturating_sub(3) as usize;
        let row_start = selection_window_start(
            app.comparison_rows.len(),
            app.selected_comparison,
            visible_rows,
        );
        let rows = app
            .comparison_rows
            .iter()
            .enumerate()
            .skip(row_start)
            .take(visible_rows)
            .map(|(index, row)| {
                let recommended = recommendation == Some(row.id.as_str());
                let mut cells = vec![
                    Cell::from(format!(
                        "{}{}",
                        if index == app.selected_comparison {
                            "›"
                        } else {
                            " "
                        },
                        if recommended { "★" } else { " " }
                    )),
                    Cell::from(row.name.clone()),
                ];
                if wide {
                    cells.push(Cell::from(format_optional_duration(row.first_total)));
                }
                cells.push(Cell::from(format_optional_duration(row.total)));
                if wide {
                    cells.push(Cell::from(format_optional_duration(row.ttft)));
                    cells.push(Cell::from(format_rate(row.tokens_per_second)));
                    cells.push(Cell::from(format_process_cpu(row.process_cpu_percent)));
                }
                if medium {
                    cells.push(Cell::from(format_bytes(row.peak_rss_bytes)));
                }
                cells.push(Cell::from(format!(
                    "{} {}",
                    row.result.symbol(),
                    row.result.label()
                )));
                Row::new(cells).style(if index == app.selected_comparison {
                    selected_style(outcome_style(row.result, no_color), no_color)
                } else {
                    outcome_style(row.result, no_color)
                })
            });
        let (headers, widths) = if wide {
            (
                vec![
                    "",
                    "CONFIGURATION",
                    "FIRST",
                    "REPEAT",
                    "TTFT",
                    "RATE",
                    "PROC CPU",
                    "OS PEAK RSS",
                    "RESULT",
                ],
                vec![
                    Constraint::Length(3),
                    Constraint::Min(18),
                    Constraint::Length(9),
                    Constraint::Length(9),
                    Constraint::Length(9),
                    Constraint::Length(10),
                    Constraint::Length(9),
                    Constraint::Length(11),
                    Constraint::Length(10),
                ],
            )
        } else if medium {
            (
                vec!["", "CONFIGURATION", "REPEAT", "OS PEAK RSS", "RESULT"],
                vec![
                    Constraint::Length(3),
                    Constraint::Min(18),
                    Constraint::Length(9),
                    Constraint::Length(11),
                    Constraint::Length(10),
                ],
            )
        } else {
            (
                vec!["", "CONFIGURATION", "REPEAT", "RESULT"],
                vec![
                    Constraint::Length(3),
                    Constraint::Min(14),
                    Constraint::Length(9),
                    Constraint::Length(10),
                ],
            )
        };
        frame.render_widget(
            Table::new(rows, widths)
                .header(Row::new(headers).style(Style::default().add_modifier(Modifier::BOLD)))
                .block(Block::default().borders(Borders::ALL)),
            sections[1],
        );
    }
    let mut selected_detail = app.selected_comparison().map_or_else(
        || vec![Line::raw("No route combination selected")],
        |row| {
            let first_residency = match row.first_run_cold {
                Some(true) => "first run observed model Load",
                Some(false) => "first run used a resident model",
                None => "first-run residency unknown (no Load telemetry)",
            };
            let repeat_residency = match row.repeat_runs_resident {
                Some(true) => "repeats stayed resident",
                Some(false) => "a repeat observed model Load",
                None => "repeat residency unknown",
            };
            let verification = if row.result == LaneOutcome::Passed {
                "RUNTIME/CONTRACT VERIFIED · application path awaits a real request"
            } else {
                "RUNTIME/CONTRACT FAILED"
            };
            vec![
                Line::raw(format!(
                    "{} · {} · {} · first Vifu OS-process CPU {}",
                    row.detail,
                    first_residency,
                    repeat_residency,
                    format_process_cpu(row.first_process_cpu_percent)
                )),
                Line::raw(format!(
                    "Repeat total {} · TTFT {} · Vifu OS-process CPU {} (warm median) · {} · {verification}",
                    format_metric_summary(row.total_range.as_ref()),
                    format_metric_summary(row.ttft_range.as_ref()),
                    format_process_cpu(row.process_cpu_percent),
                    if app
                        .optimization_summary
                        .as_ref()
                        .and_then(|summary| summary.recommendation.as_deref())
                        == Some(row.id.as_str())
                    {
                        "recommended by repeat median"
                    } else {
                        "measured candidate"
                    }
                )),
                Line::raw(format!("Routes: {}", format_route_summary(row))),
            ]
        },
    );
    if selected_height >= 5 {
        if let Some(summary) = &app.optimization_summary {
            selected_detail.insert(0, Line::raw(format!(
                "Arm capture correlation: comparison {} · wall {}–{} Unix ms · monotonic duration {} ms · not an Arm tool metric",
                summary.comparison_id,
                summary.started_at_ms,
                summary.completed_at_ms,
                summary.monotonic_duration_ms
            )));
        }
    }
    if selected_height >= 6 {
        selected_detail.push(Line::raw(format_exclusion_summary(app)));
    }
    frame.render_widget(
        Paragraph::new(selected_detail)
            .block(Block::default().borders(Borders::TOP).title(" Selected "))
            .wrap(Wrap { trim: true }),
        sections[2],
    );
    render_footer(
        frame,
        sections[3],
        if area.width < 70 {
            "↑↓ Config  [ ] Exclusion  ← Back"
        } else {
            "↑↓ Select  [ ] Exclusion  O Measure  A Activate  U Undo  ← Back  B Dashboard"
        },
        app.notice.as_deref(),
    );
}

fn format_route_summary(row: &ComparisonRow) -> String {
    const DISPLAY_LIMIT: usize = 3;
    let mut routes = row
        .plan
        .routes
        .iter()
        .take(DISPLAY_LIMIT)
        .map(|(route, provider)| {
            let label = row
                .route_labels
                .get(route)
                .map_or_else(|| shorten_route(route), Clone::clone);
            format!("{label} → {provider}")
        })
        .collect::<Vec<_>>();
    let remaining = row.plan.routes.len().saturating_sub(routes.len());
    if remaining > 0 {
        routes.push(format!("+{remaining} more"));
    }
    routes.join(" · ")
}

fn format_metric_summary(metric: Option<&MetricSummary>) -> String {
    metric.map_or_else(
        || "unknown".to_string(),
        |metric| {
            format!(
                "{} [{}–{}], n={}",
                format_duration(metric.median),
                format_duration(metric.min),
                format_duration(metric.max),
                metric.samples
            )
        },
    )
}

fn format_exclusion_summary(app: &App) -> String {
    let Some(exclusion) = app.optimization_exclusions.get(app.selected_exclusion) else {
        return "Excluded candidate pairs: 0".to_string();
    };
    let message = exclusion
        .message
        .as_deref()
        .map_or(String::new(), |message| format!(" — {message}"));
    format!(
        "Excluded {}/{}: {} → {} ({}) · {}{}",
        app.selected_exclusion + 1,
        app.optimization_excluded_total,
        exclusion.provider,
        shorten_route(&exclusion.route),
        exclusion.capability,
        exclusion.reason,
        message,
    )
}

fn shorten_route(route: &str) -> String {
    if route.chars().count() <= 12 {
        route.to_string()
    } else {
        format!("{}…", route.chars().take(11).collect::<String>())
    }
}

fn selection_window_start(len: usize, selected: usize, visible_rows: usize) -> usize {
    if visible_rows == 0 || len <= visible_rows {
        return 0;
    }
    selected
        .min(len.saturating_sub(1))
        .saturating_add(1)
        .saturating_sub(visible_rows)
        .min(len.saturating_sub(visible_rows))
}

fn render_footer(frame: &mut Frame<'_>, area: Rect, legend: &str, notice: Option<&str>) {
    let text = notice.map_or_else(
        || legend.to_string(),
        |notice| {
            if legend.contains("Back") {
                format!("← Back · {notice}")
            } else {
                notice.to_string()
            }
        },
    );
    frame.render_widget(
        Paragraph::new(text).style(Style::default().add_modifier(Modifier::REVERSED)),
        area,
    );
}

fn render_quit_confirmation(
    frame: &mut Frame<'_>,
    area: Rect,
    active: usize,
    override_active: bool,
) {
    let popup = centered_rect(64, 6, area);
    frame.render_widget(Clear, popup);
    let invocation_warning = if active > 0 {
        format!("{active} invocation(s) are still active.")
    } else {
        "No invocation is active.".to_string()
    };
    let override_warning = if override_active {
        "A session route override is active."
    } else {
        "No session route override is active."
    };
    frame.render_widget(
        Paragraph::new(format!(
            "Quit Vifu and stop the Runtime?\n{invocation_warning}\n{override_warning}\n\nY Confirm   N/Esc Keep running"
        ))
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL).title(" Confirm quit ")),
        popup,
    );
}

fn render_too_small(frame: &mut Frame<'_>, area: Rect) {
    frame.render_widget(
        Paragraph::new(
            "Vifu TUI needs at least 42 × 11. Resize the terminal or use headless output.",
        )
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true }),
        area,
    );
}

pub(crate) fn elapsed_rail(
    outcome: LaneOutcome,
    elapsed: Duration,
    stage: RuntimeStage,
    max_width: usize,
) -> String {
    let quantum_ms = RAIL_QUANTUM.as_millis().max(1);
    let units = elapsed
        .as_millis()
        .saturating_div(quantum_ms)
        .saturating_add(1);
    let reserved = format_duration(elapsed).chars().count() + stage.label().chars().count() + 4;
    let available = max_width.saturating_sub(reserved).max(1);
    let rail_units = usize::try_from(units).unwrap_or(usize::MAX).min(available);
    let rail = "━".repeat(rail_units);
    format!(
        "{}{} {} {}",
        outcome.symbol(),
        rail,
        format_duration(elapsed),
        stage.label().to_ascii_uppercase()
    )
}

fn format_system_metrics(
    metrics: SystemMetrics,
    loaded_models: usize,
    arch: &str,
    runtime_backends: &[String],
    width: u16,
) -> String {
    let backends = if runtime_backends.is_empty() {
        "none configured".to_string()
    } else {
        runtime_backends.join(" + ")
    };
    let cpu = metrics
        .cpu_percent
        .map_or_else(|| "--".to_string(), |value| format!("{value:.0}%"));
    if width < 76 {
        format!(
            " {} · OS CPU {} · OS RSS {} · VIFU MODELS {}",
            architecture_label(arch),
            cpu,
            format_bytes(metrics.rss_bytes),
            loaded_models
        )
    } else {
        format!(
            " {} · BACKEND {} · OS PROCESS CPU {} · RSS {} / {} · VIFU RESIDENT MODELS {}",
            architecture_label(arch),
            backends,
            cpu,
            format_bytes(metrics.rss_bytes),
            format_bytes(metrics.total_memory_bytes),
            loaded_models
        )
    }
}

fn architecture_label(arch: &str) -> String {
    match arch.to_ascii_lowercase().as_str() {
        "aarch64" => "ARM64".to_string(),
        "arm" | "armv7" | "armv7l" => "ARM".to_string(),
        _ => arch.to_ascii_uppercase(),
    }
}

fn format_duration(duration: Duration) -> String {
    if duration < Duration::from_secs(1) {
        format!("{}ms", duration.as_millis())
    } else if duration < Duration::from_secs(10) {
        format!("{:.1}s", duration.as_secs_f64())
    } else {
        format!("{}s", duration.as_secs())
    }
}

fn format_optional_duration(duration: Option<Duration>) -> String {
    duration.map_or_else(|| "--".to_string(), format_duration)
}

fn format_rate(rate: Option<f64>) -> String {
    rate.map_or_else(|| "--".to_string(), |value| format!("{value:.1} t/s"))
}

fn format_process_cpu(cpu_percent: Option<f64>) -> String {
    cpu_percent.map_or_else(|| "--".to_string(), |value| format!("{value:.0}%"))
}

fn format_bytes(bytes: Option<u64>) -> String {
    let Some(bytes) = bytes else {
        return "--".to_string();
    };
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    if bytes as f64 >= GIB {
        format!("{:.1} GiB", bytes as f64 / GIB)
    } else {
        format!("{:.0} MiB", bytes as f64 / MIB)
    }
}

fn outcome_style(outcome: LaneOutcome, no_color: bool) -> Style {
    if no_color {
        return Style::default();
    }
    match outcome {
        LaneOutcome::Running => Style::default().fg(Color::Cyan),
        LaneOutcome::Passed => Style::default().fg(Color::Green),
        LaneOutcome::Failed | LaneOutcome::Timeout => Style::default().fg(Color::Red),
        LaneOutcome::Unknown | LaneOutcome::Skipped => Style::default().fg(Color::DarkGray),
    }
}

fn stage_style(status: StageStatus, no_color: bool) -> Style {
    if no_color {
        return Style::default();
    }
    match status {
        StageStatus::Active => Style::default().fg(Color::Cyan),
        StageStatus::Passed => Style::default().fg(Color::Green),
        StageStatus::Failed => Style::default().fg(Color::Red),
        StageStatus::Skipped | StageStatus::Unknown => Style::default().fg(Color::DarkGray),
    }
}

fn selection_style(style: Style, cursor: bool, selected: bool) -> Style {
    if selected || cursor {
        style.add_modifier(Modifier::BOLD)
    } else {
        style
    }
}

fn selection_marker(selected: bool, no_color: bool) -> Span<'static> {
    if selected {
        Span::styled("›", selected_style(Style::default(), no_color))
    } else {
        Span::raw(" ")
    }
}

fn selected_style(style: Style, no_color: bool) -> Style {
    let style = style.add_modifier(Modifier::BOLD);
    if no_color {
        style
    } else {
        style.fg(Color::Cyan)
    }
}

fn stage_status_symbol(status: StageStatus) -> &'static str {
    match status {
        StageStatus::Active => "●",
        StageStatus::Passed => "✓",
        StageStatus::Failed => "✕",
        StageStatus::Skipped => "–",
        StageStatus::Unknown => "?",
    }
}

fn stage_status_label(status: StageStatus) -> &'static str {
    match status {
        StageStatus::Active => "running",
        StageStatus::Passed => "pass",
        StageStatus::Failed => "failed",
        StageStatus::Skipped => "skipped",
        StageStatus::Unknown => "unknown",
    }
}

fn shorten_capability(capability: &str) -> String {
    match capability {
        "embedding" => "embed".to_string(),
        "transcription" => "stt".to_string(),
        value => value.to_string(),
    }
}

fn short_uuid(id: uuid::Uuid) -> String {
    format!("inv_{}", &id.simple().to_string()[..10])
}

fn centered_rect(width_percent: u16, height: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(height.min(area.height)),
            Constraint::Min(0),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - width_percent) / 2),
            Constraint::Percentage(width_percent),
            Constraint::Percentage((100 - width_percent) / 2),
        ])
        .split(vertical[1])[1]
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::time::{Duration, Instant};

    use ratatui::backend::TestBackend;
    use ratatui::style::{Color, Modifier};
    use ratatui::Terminal;
    use serde_json::json;
    use uuid::Uuid;
    use vifu_gateway::optimization::{CombinationKind, RouteCombination};

    use crate::monitor::{
        FeedbackEvent, FeedbackOutcome, ProjectProfileRegistration, RegisteredAgent, RuntimeEvent,
        RuntimeStage, RuntimeTerminal, StageStatus,
    };

    use super::{
        append_chat_message, append_io_lines, chat_messages, chat_role_style, elapsed_rail, render,
        short_uuid,
    };
    use crate::tui::model::{
        App, ComparisonRow, LaneFilter, LaneOutcome, OptimizationSummary, TraceTab, View,
    };

    fn rendered_content(app: &mut App, now: Instant, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render(frame, app, now, true))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    fn comparison_row(index: usize) -> ComparisonRow {
        let id = format!("config-{index}");
        ComparisonRow {
            id: id.clone(),
            name: id.clone(),
            plan: RouteCombination {
                id: id.clone(),
                label: id,
                kind: CombinationKind::Substitution,
                explanation: format!("measured configuration {index}"),
                routes: BTreeMap::from([("route".to_string(), format!("provider-{index}"))]),
            },
            first_total: Some(Duration::from_millis(20)),
            first_run_cold: Some(true),
            repeat_runs_resident: Some(true),
            total: Some(Duration::from_millis(10)),
            total_range: None,
            ttft: None,
            ttft_range: None,
            tokens_per_second: None,
            first_process_cpu_percent: None,
            process_cpu_percent: None,
            peak_rss_bytes: None,
            route_labels: BTreeMap::new(),
            result: LaneOutcome::Passed,
            detail: format!("detail-{index}"),
        }
    }

    #[test]
    fn rail_should_grow_in_elapsed_time_quanta_without_a_percentage() {
        let short = elapsed_rail(
            LaneOutcome::Running,
            Duration::from_millis(250),
            RuntimeStage::Decode,
            32,
        );
        let long = elapsed_rail(
            LaneOutcome::Running,
            Duration::from_secs(2),
            RuntimeStage::Decode,
            32,
        );

        assert!(long.matches('━').count() > short.matches('━').count());
        assert!(!long.contains('%'));
    }

    #[test]
    fn main_screen_should_render_only_the_visible_window_for_one_hundred_agents() {
        let mut app = App::default();
        let now = Instant::now();
        app.apply(
            RuntimeEvent::AgentsRegistered(vec![RegisteredAgent {
                id: "local".to_string(),
                name: "Local Provider".to_string(),
                provider: "local".to_string(),
                capability: "chat".to_string(),
                model: "qwen".to_string(),
                local_model_loaded: true,
            }]),
            now,
        );
        for index in 0..100 {
            app.apply(
                RuntimeEvent::InvocationStarted {
                    invocation_id: Uuid::new_v4(),
                    agent_id: format!("agent-{index:03}"),
                    agent_name: format!("Agent {index:03}"),
                    source_agent_id: "local".to_string(),
                    capability: "chat".to_string(),
                    provider: "local".to_string(),
                    model: "qwen".to_string(),
                    started_unix_ms: 1,
                },
                now,
            );
        }
        app.apply(
            RuntimeEvent::BackendsChanged(vec!["llama.cpp".to_string()]),
            now,
        );
        let backend = TestBackend::new(120, 20);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| render(frame, &mut app, now, true))
            .unwrap();
        let content = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(content.contains("100 AGENTS"));
        assert!(content.contains("BACKEND llama.cpp"));
        assert!(content.contains("Agent 000"));
        assert!(!content.contains("Agent 099"));
    }

    #[test]
    fn no_color_mode_should_still_render_status_text_and_symbols() {
        let mut app = App::default();
        let now = Instant::now();
        app.apply(
            RuntimeEvent::AgentsRegistered(vec![RegisteredAgent {
                id: "local".to_string(),
                name: "Local Provider".to_string(),
                provider: "local".to_string(),
                capability: "chat".to_string(),
                model: "qwen".to_string(),
                local_model_loaded: false,
            }]),
            now,
        );
        app.apply(
            RuntimeEvent::InvocationStarted {
                invocation_id: Uuid::new_v4(),
                agent_id: "planner".to_string(),
                agent_name: "Planner".to_string(),
                source_agent_id: "local".to_string(),
                capability: "chat".to_string(),
                provider: "local".to_string(),
                model: "qwen".to_string(),
                started_unix_ms: 1,
            },
            now,
        );
        let backend = TestBackend::new(100, 18);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| render(frame, &mut app, now, true))
            .unwrap();
        let content = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(content.contains("● RUNNING"));
    }

    #[test]
    fn main_header_should_show_project_url_and_deployment_without_runtime_branding() {
        let mut app = App::default();
        app.project = Some("stardew-valley".to_string());
        app.deployment = Some("development".to_string());
        app.project_dashboard_url =
            Some("http://127.0.0.1:6790/project/stardew-valley".to_string());

        let content = rendered_content(&mut app, Instant::now(), 120, 18);

        assert!(content.contains(
            "stardew-valley / http://127.0.0.1:6790/project/stardew-valley / development"
        ));
        assert!(!content.contains("VIFU ARM RUNTIME"));
        assert!(!content.contains("● LIVE"));
    }

    #[test]
    fn never_run_project_agent_should_render_as_inactive() {
        let mut app = App::default();
        app.filter = LaneFilter::All;
        let now = Instant::now();
        app.apply(
            RuntimeEvent::ProjectProfilesRegistered(vec![ProjectProfileRegistration {
                id: "profile-1".to_string(),
                name: "Farming 0".to_string(),
                provider: "stardew-valley/development".to_string(),
                capabilities: vec!["chat".to_string()],
                model: "stardew-valley-farming-0".to_string(),
            }]),
            now,
        );

        let content = rendered_content(&mut app, now, 120, 18);

        assert!(content.contains("Farming 0"));
        assert!(content.contains("NOT RUN YET"));
    }

    #[test]
    fn agent_live_view_should_render_capabilities_requests_and_the_selected_stage() {
        let mut app = App::default();
        let now = Instant::now();
        app.apply(
            RuntimeEvent::ProjectProfilesRegistered(vec![ProjectProfileRegistration {
                id: "profile-1".to_string(),
                name: "Farmhand".to_string(),
                provider: "stardew-valley/development".to_string(),
                capabilities: vec!["chat".to_string(), "embedding".to_string()],
                model: "stardew-valley-farmhand".to_string(),
            }]),
            now,
        );
        let invocation_id = Uuid::new_v4();
        app.apply(
            RuntimeEvent::InvocationStarted {
                invocation_id,
                agent_id: "profile-1".to_string(),
                agent_name: "Farmhand".to_string(),
                source_agent_id: "local-qwen".to_string(),
                capability: "chat".to_string(),
                provider: "local-qwen".to_string(),
                model: "qwen".to_string(),
                started_unix_ms: 100,
            },
            now,
        );
        app.apply(
            RuntimeEvent::StageChanged {
                invocation_id,
                observation_id: Uuid::new_v4(),
                stage: RuntimeStage::Prefill,
                status: StageStatus::Active,
                start_offset: Duration::from_millis(10),
                end_offset: None,
                elapsed: Duration::from_millis(20),
                request_elapsed: Some(Duration::from_millis(30)),
                input_tokens: Some(20),
                output_tokens: None,
                resident: Some(true),
                error: None,
            },
            now,
        );
        app.open_selected_agent();

        let content = rendered_content(&mut app, now, 140, 22);

        assert!(content.contains("2 capabilities"));
        assert!(content.contains("CAPABILITY"));
        assert!(content.contains("embed"));
        assert!(content.contains("LIVE REQUESTS"));
        assert!(content.contains(&short_uuid(invocation_id)));
        assert!(content.contains("PREFILL"));
        assert!(content.contains("←/Esc Agents"));
    }

    #[test]
    fn agent_live_view_should_show_declared_capabilities_before_the_first_request() {
        let mut app = App::default();
        app.filter = LaneFilter::All;
        let now = Instant::now();
        app.apply(
            RuntimeEvent::ProjectProfilesRegistered(vec![ProjectProfileRegistration {
                id: "profile-1".to_string(),
                name: "Farmhand".to_string(),
                provider: "stardew-valley/development".to_string(),
                capabilities: vec!["chat".to_string(), "embedding".to_string()],
                model: "stardew-valley-farmhand".to_string(),
            }]),
            now,
        );
        app.open_selected_agent();

        let content = rendered_content(&mut app, now, 120, 18);

        assert!(content.contains("chat"));
        assert!(content.contains("embed"));
        assert!(content.contains("WAITING"));
        assert!(content.contains("Waiting for this Agent's first live request"));
    }

    #[test]
    fn agent_live_view_should_render_the_newest_request_as_the_first_selected_row() {
        let mut app = App::default();
        let now = Instant::now();
        let older = Uuid::new_v4();
        let newest = Uuid::new_v4();
        for (invocation_id, started_unix_ms) in [(older, 100), (newest, 200)] {
            app.apply(
                RuntimeEvent::InvocationStarted {
                    invocation_id,
                    agent_id: "profile-1".to_string(),
                    agent_name: "Farmhand".to_string(),
                    source_agent_id: "local-qwen".to_string(),
                    capability: "chat".to_string(),
                    provider: "local-qwen".to_string(),
                    model: "qwen".to_string(),
                    started_unix_ms,
                },
                now,
            );
        }
        app.open_selected_agent();

        let content = rendered_content(&mut app, now, 120, 18);
        let cursor = content.find("›01").unwrap();
        let newest = content.find(&short_uuid(newest)).unwrap();
        let older = content.find(&short_uuid(older)).unwrap();

        assert!(cursor < newest && newest < older);
    }

    #[test]
    fn selected_agent_should_use_an_accent_foreground_without_reverse_video() {
        let mut app = App::default();
        let now = Instant::now();
        app.apply(
            RuntimeEvent::InvocationStarted {
                invocation_id: Uuid::new_v4(),
                agent_id: "planner".to_string(),
                agent_name: "Planner".to_string(),
                source_agent_id: "local".to_string(),
                capability: "chat".to_string(),
                provider: "local".to_string(),
                model: "qwen".to_string(),
                started_unix_ms: 1,
            },
            now,
        );
        let backend = TestBackend::new(100, 18);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| render(frame, &mut app, now, false))
            .unwrap();
        let selected = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .find(|cell| cell.symbol() == "›")
            .expect("selected Agent marker");

        assert_eq!(selected.fg, Color::Cyan);
        assert!(!selected.modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn selected_failed_request_keeps_its_semantic_red_color() {
        let mut app = App::default();
        let now = Instant::now();
        let invocation_id = Uuid::new_v4();
        app.apply(
            RuntimeEvent::InvocationStarted {
                invocation_id,
                agent_id: "planner".to_string(),
                agent_name: "Planner".to_string(),
                source_agent_id: "local".to_string(),
                capability: "chat".to_string(),
                provider: "local".to_string(),
                model: "qwen".to_string(),
                started_unix_ms: 1,
            },
            now,
        );
        app.apply(
            RuntimeEvent::InvocationFinished {
                invocation_id,
                elapsed: Duration::from_millis(10),
                terminal: RuntimeTerminal::ProviderFailed,
                error: Some("provider request failed".to_string()),
            },
            now,
        );
        let backend = TestBackend::new(120, 20);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| render(frame, &mut app, now, false))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let selected = buffer
            .content
            .iter()
            .find(|cell| cell.symbol() == "›")
            .expect("selected Agent marker");
        let failed = buffer
            .content
            .iter()
            .find(|cell| cell.symbol() == "✕")
            .expect("failed result marker");

        assert_eq!(selected.fg, Color::Cyan);
        assert_eq!(failed.fg, Color::Red);

        app.open_selected_agent();
        terminal
            .draw(|frame| render(frame, &mut app, now, false))
            .unwrap();
        let failed = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .find(|cell| cell.symbol() == "✕")
            .expect("failed Agent request marker");

        assert_eq!(failed.fg, Color::Red);
    }

    #[test]
    fn trace_io_renders_chat_messages_and_tool_calls_as_readable_content() {
        let mut app = App::default();
        let now = Instant::now();
        let trace_id = Uuid::new_v4();
        app.apply(
            RuntimeEvent::InvocationStarted {
                invocation_id: trace_id,
                agent_id: "planner".to_string(),
                agent_name: "Planner".to_string(),
                source_agent_id: "remote".to_string(),
                capability: "chat".to_string(),
                provider: "openai-compatible".to_string(),
                model: "gpt-test".to_string(),
                started_unix_ms: 1,
            },
            now,
        );
        app.apply(
            RuntimeEvent::IoCaptured {
                invocation_id: trace_id,
                input: Some(json!({
                    "model": "gpt-test",
                    "messages": [
                        {"role": "system", "content": "Keep actions grounded."},
                        {"role": "user", "content": "Cut the grass."}
                    ]
                })),
                output: Some(json!({
                    "choices": [{
                        "message": {
                            "role": "assistant",
                            "content": "I will move east.",
                            "tool_calls": [{
                                "function": {
                                    "name": "move",
                                    "arguments": "{\"direction\":\"east\"}"
                                }
                            }]
                        }
                    }]
                })),
                truncated: false,
            },
            now,
        );
        app.view = View::Trace {
            agent_key: "planner\0".to_string(),
            trace_id,
            tab: TraceTab::Io,
            timeline: false,
            observation_cursor: None,
            selected_observation: None,
        };

        let content = rendered_content(&mut app, now, 180, 36);

        assert!(content.contains("INPUT · CHAT"));
        assert!(content.contains("SYSTEM"));
        assert!(content.contains("Keep actions grounded."));
        assert!(content.contains("USER"));
        assert!(content.contains("Cut the grass."));
        assert!(content.contains("OUTPUT · CHAT"));
        assert!(content.contains("ASSISTANT"));
        assert!(content.contains("I will move east."));
        assert!(content.contains("TOOL  move"));
        assert!(content.contains("direction: east"));
    }

    #[test]
    fn chat_roles_use_distinct_semantic_colors() {
        assert_eq!(
            chat_role_style("SYSTEM", false, false).fg,
            Some(Color::Magenta)
        );
        assert_eq!(chat_role_style("USER", false, false).fg, Some(Color::Cyan));
        assert_eq!(
            chat_role_style("ASSISTANT", false, false).fg,
            Some(Color::Green)
        );
        assert_ne!(
            chat_role_style("SYSTEM", false, false).fg,
            chat_role_style("USER", false, false).fg
        );
        assert_eq!(chat_role_style("SYSTEM", true, false).fg, None);
    }

    #[test]
    fn anthropic_chat_system_and_tool_use_share_the_generic_conversation_view() {
        let request = json!({
            "system": "Stay within the farm.",
            "messages": [{"role": "user", "content": "Cut one weed."}]
        });
        let response = json!({
            "role": "assistant",
            "content": [
                {"type": "text", "text": "I will cut left."},
                {"type": "tool_use", "name": "use", "input": {"direction": "left"}}
            ]
        });
        let mut lines = Vec::new();
        append_io_lines(&mut lines, "INPUT", Some(&request), false);
        for message in chat_messages(&response) {
            append_chat_message(&mut lines, message, false);
        }
        let content = lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(content.contains("SYSTEM"));
        assert!(content.contains("Stay within the farm."));
        assert!(content.contains("USER"));
        assert!(content.contains("Cut one weed."));
        assert!(content.contains("ASSISTANT"));
        assert!(content.contains("I will cut left."));
        assert!(content.contains("TOOL  use"));
        assert!(content.contains("direction: left"));
    }

    #[test]
    fn trace_summary_leads_with_the_current_conversation_and_agent_name() {
        let mut app = App::default();
        let now = Instant::now();
        let trace_id = Uuid::new_v4();
        let profile_id = Uuid::new_v4().to_string();
        app.apply(
            RuntimeEvent::InvocationStarted {
                invocation_id: trace_id,
                agent_id: profile_id.clone(),
                agent_name: "Stardew Valley farming/0".to_string(),
                source_agent_id: "remote".to_string(),
                capability: "chat".to_string(),
                provider: "openai-compatible".to_string(),
                model: "gpt-test".to_string(),
                started_unix_ms: 1,
            },
            now,
        );
        app.apply(
            RuntimeEvent::IoCaptured {
                invocation_id: trace_id,
                input: Some(json!({
                    "messages": [
                        {"role": "system", "content": "Long instructions"},
                        {"role": "user", "content": "Cut the next patch of grass."}
                    ]
                })),
                output: Some(json!({
                    "choices": [{"message": {"role": "assistant", "content": "Moving east."}}]
                })),
                truncated: false,
            },
            now,
        );
        app.view = View::Trace {
            agent_key: format!("{profile_id}\0"),
            trace_id,
            tab: TraceTab::Summary,
            timeline: false,
            observation_cursor: None,
            selected_observation: None,
        };

        let content = rendered_content(&mut app, now, 180, 36);

        assert!(content.contains("Stardew Valley farming/0 · chat"));
        assert!(content.contains("CONVERSATION"));
        assert!(content.contains("SYSTEM"));
        assert!(content.contains("Long instructions"));
        assert!(content.contains("USER"));
        assert!(content.contains("Cut the next patch of grass."));
        assert!(content.contains("ASSISTANT"));
        assert!(content.contains("Moving east."));
    }

    #[test]
    fn trace_io_renders_responses_api_function_calls() {
        let mut app = App::default();
        let now = Instant::now();
        let trace_id = Uuid::new_v4();
        app.apply(
            RuntimeEvent::InvocationStarted {
                invocation_id: trace_id,
                agent_id: "planner".to_string(),
                agent_name: "Planner".to_string(),
                source_agent_id: "remote".to_string(),
                capability: "chat".to_string(),
                provider: "openai-compatible".to_string(),
                model: "gpt-test".to_string(),
                started_unix_ms: 1,
            },
            now,
        );
        app.apply(
            RuntimeEvent::IoCaptured {
                invocation_id: trace_id,
                input: None,
                output: Some(json!({
                    "output": [
                        {
                            "type": "message",
                            "role": "assistant",
                            "content": [{"type": "output_text", "text": "Moving now."}]
                        },
                        {
                            "type": "function_call",
                            "name": "move",
                            "arguments": "{\"x\":1,\"y\":0}"
                        }
                    ]
                })),
                truncated: false,
            },
            now,
        );
        app.view = View::Trace {
            agent_key: "planner\0".to_string(),
            trace_id,
            tab: TraceTab::Io,
            timeline: false,
            observation_cursor: None,
            selected_observation: None,
        };

        let content = rendered_content(&mut app, now, 180, 36);

        assert!(content.contains("ASSISTANT"));
        assert!(content.contains("Moving now."));
        assert!(content.contains("TOOL  move"));
        assert!(content.contains("x: 1"));
        assert!(content.contains("y: 0"));
    }

    #[test]
    fn score_view_explains_the_application_check() {
        let mut app = App::default();
        let now = Instant::now();
        let trace_id = Uuid::new_v4();
        app.apply(
            RuntimeEvent::InvocationStarted {
                invocation_id: trace_id,
                agent_id: "planner".to_string(),
                agent_name: "Planner".to_string(),
                source_agent_id: "local".to_string(),
                capability: "chat".to_string(),
                provider: "local".to_string(),
                model: "qwen".to_string(),
                started_unix_ms: 1,
            },
            now,
        );
        app.apply(
            RuntimeEvent::ApplicationFeedback {
                invocation_id: trace_id,
                observation_id: Uuid::new_v4(),
                start_offset: Duration::from_millis(10),
                end_offset: Duration::from_millis(11),
                event: FeedbackEvent::OutputAccepted,
                outcome: FeedbackOutcome::Fail,
                message: Some("could not parse action".to_string()),
                path: Some("$.action".to_string()),
            },
            now,
        );
        app.view = View::Trace {
            agent_key: "planner\0".to_string(),
            trace_id,
            tab: TraceTab::Scores,
            timeline: false,
            observation_cursor: None,
            selected_observation: None,
        };

        let content = rendered_content(&mut app, now, 160, 28);

        assert!(content.contains("OUTPUT ACCEPTED · FAIL"));
        assert!(content.contains("Whether the application accepted and parsed the model output"));
        assert!(content.contains("Path: $.action"));
        assert!(content.contains("could not parse action"));
    }

    #[test]
    fn narrow_main_header_keeps_cpu_rss_and_model_count_visible() {
        let mut app = App::default();
        let now = Instant::now();
        app.apply(
            RuntimeEvent::BackendsChanged(vec!["llama.cpp".to_string()]),
            now,
        );

        let content = rendered_content(&mut app, now, 52, 18);

        assert!(content.contains("OS CPU"));
        assert!(content.contains("OS RSS"));
        assert!(content.contains("VIFU MODELS"));
    }

    #[test]
    fn optimize_view_labels_remote_providers_as_unmeasured_fallbacks() {
        let mut app = App::default();
        app.view = View::Optimize;
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
            recommendation: None,
            not_exhaustive: true,
            sequential_replay: true,
            device_architecture: "aarch64".to_string(),
            device_backend: Some("llama.cpp".to_string()),
            remote_fallbacks: vec!["Remote Chat (remote) · hosted-model · chat".to_string()],
        });

        let content = rendered_content(&mut app, Instant::now(), 160, 20);

        assert!(content.contains("REMOTE FALLBACKS 1"));
        assert!(content.contains("Remote Chat (remote)"));
        assert!(content.contains("not measured (local optimization default)"));
        assert!(content.contains("Arm capture correlation"));
        assert!(content.contains("not an Arm tool metric"));
    }

    #[test]
    fn trace_summary_separates_delivered_runtime_from_application_parser_failure() {
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
        app.apply(
            RuntimeEvent::ApplicationFeedback {
                invocation_id: trace_id,
                observation_id: Uuid::new_v4(),
                start_offset: Duration::from_millis(20),
                end_offset: Duration::from_millis(20),
                event: FeedbackEvent::OutputAccepted,
                outcome: FeedbackOutcome::Fail,
                message: Some("application could not parse response".to_string()),
                path: Some("$.action".to_string()),
            },
            now,
        );
        app.apply(
            RuntimeEvent::InvocationFinished {
                invocation_id: trace_id,
                elapsed: Duration::from_millis(50),
                terminal: RuntimeTerminal::Delivered,
                error: None,
            },
            now,
        );
        app.view = View::Trace {
            agent_key: "planner\0".to_string(),
            trace_id,
            tab: TraceTab::Summary,
            timeline: false,
            observation_cursor: None,
            selected_observation: None,
        };

        let content = rendered_content(&mut app, now, 120, 26);

        assert!(content.contains("Runtime passed · Application failed"));
        assert!(content.contains("First error/timeout: App accepted"));
    }

    #[test]
    fn trace_summary_keeps_partial_application_feedback_unknown() {
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
        app.apply(
            RuntimeEvent::ApplicationFeedback {
                invocation_id: trace_id,
                observation_id: Uuid::new_v4(),
                start_offset: Duration::from_millis(40),
                end_offset: Duration::from_millis(41),
                event: FeedbackEvent::OutputAccepted,
                outcome: FeedbackOutcome::Pass,
                message: None,
                path: None,
            },
            now,
        );
        app.apply(
            RuntimeEvent::InvocationFinished {
                invocation_id: trace_id,
                elapsed: Duration::from_millis(50),
                terminal: RuntimeTerminal::Delivered,
                error: None,
            },
            now,
        );
        app.view = View::Trace {
            agent_key: "planner\0".to_string(),
            trace_id,
            tab: TraceTab::Summary,
            timeline: false,
            observation_cursor: None,
            selected_observation: None,
        };

        let content = rendered_content(&mut app, now, 120, 26);

        assert!(content.contains("Runtime passed · Application outcome unknown"));
    }

    #[test]
    fn selected_observation_scores_and_io_are_scoped_by_exact_uuid() {
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
        let selected_id = Uuid::new_v4();
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
            RuntimeEvent::ApplicationFeedback {
                invocation_id: trace_id,
                observation_id: selected_id,
                start_offset: Duration::from_millis(10),
                end_offset: Duration::from_millis(11),
                event: FeedbackEvent::OutputAccepted,
                outcome: FeedbackOutcome::Pass,
                message: Some("selected score".to_string()),
                path: Some("$.action".to_string()),
            },
            now,
        );
        app.apply(
            RuntimeEvent::ApplicationFeedback {
                invocation_id: trace_id,
                observation_id: Uuid::new_v4(),
                start_offset: Duration::from_millis(12),
                end_offset: Duration::from_millis(13),
                event: FeedbackEvent::ActionApplied,
                outcome: FeedbackOutcome::Fail,
                message: Some("different score".to_string()),
                path: Some("$.other".to_string()),
            },
            now,
        );
        app.view = View::Trace {
            agent_key: "planner\0".to_string(),
            trace_id,
            tab: TraceTab::Scores,
            timeline: false,
            observation_cursor: Some(selected_id),
            selected_observation: Some(selected_id),
        };

        let scores = rendered_content(&mut app, now, 120, 26);
        assert!(scores.contains("selected score"));
        assert!(!scores.contains("different score"));

        if let View::Trace { tab, .. } = &mut app.view {
            *tab = TraceTab::Io;
        }
        let io = rendered_content(&mut app, now, 120, 26);
        assert!(io.contains("OUTPUT_ACCEPTED"));
        assert!(io.contains("pass"));
    }

    #[test]
    fn trace_view_highlights_and_scopes_detail_to_the_selected_observation() {
        let mut app = App::default();
        let now = Instant::now();
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
        let trace_id = uuid::Uuid::new_v4();
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
                stage: RuntimeStage::Decode,
                status: StageStatus::Active,
                start_offset: Duration::from_millis(10),
                end_offset: None,
                elapsed: Duration::from_millis(20),
                request_elapsed: Some(Duration::from_millis(30)),
                input_tokens: Some(8),
                output_tokens: Some(2),
                resident: Some(true),
                error: None,
            },
            now,
        );
        app.apply(
            RuntimeEvent::InvocationFinished {
                invocation_id: trace_id,
                elapsed: Duration::from_millis(50),
                terminal: crate::monitor::RuntimeTerminal::Delivered,
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
        let backend = TestBackend::new(110, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| render(frame, &mut app, now, true))
            .unwrap();
        let content = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(content.contains("Summary · Decode"));
        assert!(content.contains("Generation"));
        assert!(content.contains("Observation: Decode"));
        assert!(content.contains("›●   ● Decode"));
    }

    #[test]
    fn narrow_agent_live_view_keeps_the_selected_historical_request_visible() {
        let mut app = App::default();
        let now = Instant::now();
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
        let mut oldest = None;
        for index in 0..8_u128 {
            let trace_id = Uuid::from_u128((index + 1) << 120);
            oldest.get_or_insert(trace_id);
            app.apply(
                RuntimeEvent::InvocationStarted {
                    invocation_id: trace_id,
                    agent_id: "planner".to_string(),
                    agent_name: "Planner".to_string(),
                    source_agent_id: "planner".to_string(),
                    capability: "chat".to_string(),
                    provider: "local".to_string(),
                    model: "qwen".to_string(),
                    started_unix_ms: index as u64,
                },
                now,
            );
            app.apply(
                RuntimeEvent::InvocationFinished {
                    invocation_id: trace_id,
                    elapsed: Duration::from_millis(10),
                    terminal: RuntimeTerminal::Delivered,
                    error: None,
                },
                now,
            );
        }
        let oldest = oldest.unwrap();
        app.selected_trace = Some(oldest);
        app.view = View::Agent {
            agent_key: "planner\0".to_string(),
        };

        let content = rendered_content(&mut app, now, 120, 11);

        assert!(content.contains(&short_uuid(oldest)));
        assert!(content.contains("←/Esc Agents"));
    }

    #[test]
    fn narrow_trace_tree_keeps_the_last_observation_cursor_visible() {
        let mut app = App::default();
        let now = Instant::now();
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
        let trace_id = Uuid::from_u128(1 << 120);
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
        let mut last_observation = None;
        for (index, stage) in RuntimeStage::ORDERED.into_iter().enumerate() {
            let observation_id = Uuid::from_u128(((index as u128) + 2) << 120);
            last_observation = Some(observation_id);
            app.apply(
                RuntimeEvent::StageChanged {
                    invocation_id: trace_id,
                    observation_id,
                    stage,
                    status: StageStatus::Passed,
                    start_offset: Duration::from_millis(index as u64),
                    end_offset: Some(Duration::from_millis(index as u64 + 1)),
                    elapsed: Duration::from_millis(1),
                    request_elapsed: Some(Duration::from_millis(index as u64 + 1)),
                    input_tokens: None,
                    output_tokens: None,
                    resident: None,
                    error: None,
                },
                now,
            );
        }
        let last_observation = last_observation.unwrap();
        app.view = View::Trace {
            agent_key: "planner\0".to_string(),
            trace_id,
            tab: TraceTab::Summary,
            timeline: false,
            observation_cursor: Some(last_observation),
            selected_observation: Some(last_observation),
        };

        let content = rendered_content(&mut app, now, 52, 18);

        assert!(content.contains("›●   ✓ Frame"));
        assert!(content.contains("← Back"));
    }

    #[test]
    fn trace_search_filters_tree_selects_match_and_renders_no_match_state() {
        let mut app = App::default();
        let now = Instant::now();
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
        let trace_id = Uuid::from_u128(1 << 120);
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
        for (index, (stage, error)) in [
            (RuntimeStage::Prefill, None),
            (
                RuntimeStage::Validate,
                Some("response contract sentinel".to_string()),
            ),
        ]
        .into_iter()
        .enumerate()
        {
            app.apply(
                RuntimeEvent::StageChanged {
                    invocation_id: trace_id,
                    observation_id: Uuid::from_u128(((index as u128) + 2) << 120),
                    stage,
                    status: if error.is_some() {
                        StageStatus::Failed
                    } else {
                        StageStatus::Passed
                    },
                    start_offset: Duration::from_millis(index as u64),
                    end_offset: Some(Duration::from_millis(index as u64 + 1)),
                    elapsed: Duration::from_millis(1),
                    request_elapsed: Some(Duration::from_millis(index as u64 + 1)),
                    input_tokens: None,
                    output_tokens: None,
                    resident: None,
                    error,
                },
                now,
            );
        }
        app.view = View::Trace {
            agent_key: "planner\0".to_string(),
            trace_id,
            tab: TraceTab::Summary,
            timeline: false,
            observation_cursor: None,
            selected_observation: None,
        };
        app.search = "contract sentinel".to_string();
        app.normalize_search_selection(now);

        let matching = rendered_content(&mut app, now, 52, 18);

        assert!(matching.contains("Tree · 1 match"));
        assert!(matching.contains("Validate"));
        assert!(!matching.contains("Prefill"));
        assert!(matching.contains("← Back"));

        app.search = "absent observation".to_string();
        app.normalize_search_selection(now);
        let no_match = rendered_content(&mut app, now, 52, 18);

        assert!(no_match.contains("Tree · no matches"));
        assert!(no_match.contains("No matching observations"));
        assert!(no_match.contains("← Back"));
    }

    #[test]
    fn narrow_optimize_list_keeps_selection_override_and_back_visible() {
        let mut app = App::default();
        app.view = View::Optimize;
        app.comparison_rows = (0..8).map(comparison_row).collect();
        app.selected_comparison = 7;
        app.override_active = true;
        app.override_route_count = 2;
        app.override_generation = Some(3);

        let content = rendered_content(&mut app, Instant::now(), 52, 18);

        assert!(content.contains("config-7"));
        assert!(content.contains("ACTIVE OVERRIDE"));
        assert!(content.contains("← Back"));
    }

    #[test]
    fn narrow_trace_and_optimize_views_keep_the_back_action_visible() {
        let mut app = App::default();
        let now = Instant::now();
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
        let trace_id = uuid::Uuid::new_v4();
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
            RuntimeEvent::InvocationFinished {
                invocation_id: trace_id,
                elapsed: Duration::from_millis(50),
                terminal: crate::monitor::RuntimeTerminal::Delivered,
                error: None,
            },
            now,
        );
        app.view = View::Trace {
            agent_key: "planner\0".to_string(),
            trace_id,
            tab: TraceTab::Summary,
            timeline: false,
            observation_cursor: None,
            selected_observation: None,
        };
        let backend = TestBackend::new(52, 18);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render(frame, &mut app, now, true))
            .unwrap();
        let trace_content = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(trace_content.contains("← Back"));
        assert!(trace_content.contains("Runtime passed · Application outcome unknown"));

        app.open_optimize();
        terminal
            .draw(|frame| render(frame, &mut app, now, true))
            .unwrap();
        let optimize_content = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(optimize_content.contains("← Back"));
    }
}
