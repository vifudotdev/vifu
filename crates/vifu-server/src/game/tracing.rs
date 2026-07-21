use std::time::Instant;

use serde_json::{json, Value};
use tracing::warn;
use uuid::Uuid;
use vifu_game_runtime::{RuntimeAdvance, SessionStatus};

use crate::db as trace_db;
use crate::AppState;

use super::models::GameEffectTrace;

pub struct GameTrace {
    request_id: Uuid,
    trace_id: Uuid,
    root_span_id: Uuid,
    operation: &'static str,
    started_at: Instant,
}

impl GameTrace {
    pub fn effect_context(&self) -> GameEffectTrace {
        GameEffectTrace {
            trace_id: self.trace_id,
            parent_span_id: self.root_span_id,
        }
    }
}

pub async fn start(
    state: &AppState,
    project_id: Uuid,
    operation: &'static str,
    selection_key: Option<&str>,
    request_summary: &Value,
) -> Option<GameTrace> {
    let request_id = Uuid::new_v4();
    let trace_id = match trace_db::create_trace(
        &state.pool,
        trace_db::NewTrace {
            request_id,
            endpoint_id: None,
            project_id: Some(project_id),
            gateway_session_id: None,
            profile_id: None,
            profile_version_id: None,
            operation,
            provider_key: None,
            capability_kind: Some("game"),
            selection_key,
            request: request_summary,
        },
    )
    .await
    {
        Ok(trace_id) => trace_id,
        Err(error) => {
            warn!(%error, %project_id, operation, "could not create game trace");
            return None;
        }
    };
    let root_span_id = match trace_db::create_trace_span(
        &state.pool,
        trace_db::NewTraceSpan {
            trace_id,
            parent_span_id: None,
            name: operation,
            kind: "runtime",
            provider_key: None,
            capability_kind: Some("game"),
            input_summary: Some(request_summary),
            attributes: &json!({"projectId": project_id}),
        },
    )
    .await
    {
        Ok(span_id) => span_id,
        Err(error) => {
            warn!(%error, %project_id, operation, "could not create game root span");
            let _ = trace_db::complete_trace(
                &state.pool,
                request_id,
                "failed",
                0,
                None,
                Some("trace root span could not be created"),
            )
            .await;
            return None;
        }
    };
    Some(GameTrace {
        request_id,
        trace_id,
        root_span_id,
        operation,
        started_at: Instant::now(),
    })
}

pub async fn complete(
    state: &AppState,
    trace: Option<GameTrace>,
    advance: Option<&RuntimeAdvance>,
    response_summary: &Value,
) {
    let Some(trace) = trace else {
        return;
    };
    if let Some(advance) = advance {
        record_node_spans(state, &trace, advance).await;
    }
    let failed = advance.is_some_and(|advance| advance.snapshot.status == SessionStatus::Failed);
    let status = if failed { "failed" } else { "completed" };
    let error = advance
        .and_then(|advance| advance.snapshot.failure.as_ref())
        .map(|failure| failure.message.as_str());
    let duration_ms = trace_db::elapsed_millis(trace.started_at);
    if let Err(persist_error) = trace_db::complete_trace_span(
        &state.pool,
        trace.root_span_id,
        status,
        duration_ms,
        Some(response_summary),
        error,
    )
    .await
    {
        warn!(error = %persist_error, operation = trace.operation, "could not complete game root span");
    }
    if let Err(persist_error) = trace_db::complete_trace(
        &state.pool,
        trace.request_id,
        status,
        duration_ms,
        Some(response_summary),
        error,
    )
    .await
    {
        warn!(error = %persist_error, operation = trace.operation, "could not complete game trace");
    }
}

pub async fn fail(state: &AppState, trace: Option<GameTrace>, error: &str) {
    let Some(trace) = trace else {
        return;
    };
    let duration_ms = trace_db::elapsed_millis(trace.started_at);
    if let Err(persist_error) = trace_db::complete_trace_span(
        &state.pool,
        trace.root_span_id,
        "failed",
        duration_ms,
        None,
        Some(error),
    )
    .await
    {
        warn!(error = %persist_error, operation = trace.operation, "could not fail game root span");
    }
    if let Err(persist_error) = trace_db::complete_trace(
        &state.pool,
        trace.request_id,
        "failed",
        duration_ms,
        None,
        Some(error),
    )
    .await
    {
        warn!(error = %persist_error, operation = trace.operation, "could not fail game trace");
    }
}

async fn record_node_spans(state: &AppState, trace: &GameTrace, advance: &RuntimeAdvance) {
    for execution in &advance.node_executions {
        let attributes = json!({
            "nodeId": execution.node_id,
            "nodeType": execution.node_type,
            "ordinal": execution.ordinal,
            "sequence": execution.sequence,
        });
        let name = format!("game.node.{}", execution.node_type);
        let span_id = match trace_db::create_trace_span(
            &state.pool,
            trace_db::NewTraceSpan {
                trace_id: trace.trace_id,
                parent_span_id: Some(trace.root_span_id),
                name: &name,
                kind: "node",
                provider_key: None,
                capability_kind: Some("game"),
                input_summary: None,
                attributes: &attributes,
            },
        )
        .await
        {
            Ok(span_id) => span_id,
            Err(error) => {
                warn!(%error, node_id = execution.node_id, "could not create game node span");
                continue;
            }
        };
        if let Err(error) = trace_db::complete_trace_span(
            &state.pool,
            span_id,
            "completed",
            0,
            Some(&json!({"sequence": execution.sequence})),
            None,
        )
        .await
        {
            warn!(%error, node_id = execution.node_id, "could not complete game node span");
        }
    }
}
