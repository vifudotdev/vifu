use serde_json::json;
use uuid::Uuid;
use vifu_gateway::protocol::{
    validate_trace_telemetry_batch, TraceDeliveryStatus, TraceStageStatus, TraceTelemetry,
    TraceTelemetryBatch,
};
use vifu_runtime::ProviderStage;

use crate::db;
use crate::error::ApiError;

const MAX_TELEMETRY_DURATION_MS: u64 = 24 * 60 * 60 * 1_000;
const MAX_TELEMETRY_TOKEN_COUNT: u64 = 1_000_000_000;

pub(crate) async fn persist(
    storage: &db::Storage,
    request_id: Uuid,
    telemetry: TraceTelemetry,
) -> Result<(), ApiError> {
    match telemetry {
        TraceTelemetry::InvocationStarted {
            provider_key,
            capability,
            model,
        } => {
            db::update_trace_runtime_identity(
                storage,
                request_id,
                &provider_key,
                &capability,
                model.as_deref(),
            )
            .await
        }
        TraceTelemetry::ProviderStage {
            observation_id,
            stage,
            status,
            start_offset_ms,
            end_offset_ms,
            elapsed_ms,
            request_elapsed_ms,
            input_tokens,
            output_tokens,
            resident,
            error,
        } => {
            let start_offset_ms = bounded_duration("stage startOffsetMs", start_offset_ms)?;
            let end_offset_ms = end_offset_ms
                .map(|value| bounded_duration("stage endOffsetMs", value))
                .transpose()?;
            if end_offset_ms.is_some_and(|end| end < start_offset_ms) {
                return Err(ApiError::Invalid(
                    "stage endOffsetMs precedes startOffsetMs".to_string(),
                ));
            }
            let elapsed_ms = elapsed_ms
                .map(|value| bounded_duration("stage elapsedMs", value))
                .transpose()?;
            let request_elapsed_ms = request_elapsed_ms
                .map(|value| bounded_duration("stage requestElapsedMs", value))
                .transpose()?;
            let input_tokens = input_tokens
                .map(|value| bounded_tokens("stage inputTokens", value))
                .transpose()?;
            let output_tokens = output_tokens
                .map(|value| bounded_tokens("stage outputTokens", value))
                .transpose()?;
            let error = error.as_deref().map(safe_error_message);
            if status != TraceStageStatus::Started {
                db::merge_trace_runtime_generation(
                    storage,
                    request_id,
                    (stage == ProviderStage::FirstToken)
                        .then_some(request_elapsed_ms)
                        .flatten(),
                    input_tokens,
                    output_tokens,
                )
                .await?;
            }
            let Some(target) = db::get_runtime_trace_target(storage, request_id).await? else {
                return Ok(());
            };
            let attributes = json!({
                "source": "agentGateway",
                "stage": stage_name(stage),
                "startOffsetMs": start_offset_ms,
                "endOffsetMs": end_offset_ms,
                "requestElapsedMs": request_elapsed_ms,
                "inputTokens": input_tokens,
                "outputTokens": output_tokens,
                "resident": resident,
            });
            db::upsert_runtime_trace_observation(
                storage,
                db::RuntimeTraceObservation {
                    id: observation_id,
                    trace_id: target.trace_id,
                    parent_span_id: target.parent_span_id,
                    name: stage_name(stage),
                    kind: "provider_stage",
                    observation_type: "span",
                    provider_key: target.provider_key.as_deref(),
                    capability_kind: target.capability_kind.as_deref(),
                    model: target.model.as_deref(),
                    status: match status {
                        TraceStageStatus::Started => "pending",
                        TraceStageStatus::Completed => "completed",
                        TraceStageStatus::Failed => "failed",
                    },
                    duration_ms: elapsed_ms,
                    attributes: &attributes,
                    error: error.as_deref(),
                },
            )
            .await
        }
        TraceTelemetry::Delivery {
            observation_id,
            status,
            start_offset_ms,
            end_offset_ms,
            elapsed_ms,
            error,
        } => {
            let start_offset_ms = bounded_duration("delivery startOffsetMs", start_offset_ms)?;
            let end_offset_ms = bounded_duration("delivery endOffsetMs", end_offset_ms)?;
            if end_offset_ms < start_offset_ms {
                return Err(ApiError::Invalid(
                    "delivery endOffsetMs precedes startOffsetMs".to_string(),
                ));
            }
            let elapsed_ms = bounded_duration("delivery elapsedMs", elapsed_ms)?;
            let error = error.as_deref().map(safe_error_message);
            let Some(target) = db::get_runtime_trace_target(storage, request_id).await? else {
                return Ok(());
            };
            let attributes = json!({
                "source": "agentGateway",
                "deliveryStatus": match status {
                    TraceDeliveryStatus::Delivered => "delivered",
                    TraceDeliveryStatus::Failed => "failed",
                },
                "startOffsetMs": start_offset_ms,
                "endOffsetMs": end_offset_ms,
            });
            db::upsert_runtime_trace_observation(
                storage,
                db::RuntimeTraceObservation {
                    id: observation_id,
                    trace_id: target.trace_id,
                    parent_span_id: target.parent_span_id,
                    name: "Deliver",
                    kind: "delivery",
                    observation_type: "span",
                    provider_key: target.provider_key.as_deref(),
                    capability_kind: target.capability_kind.as_deref(),
                    model: target.model.as_deref(),
                    status: match status {
                        TraceDeliveryStatus::Delivered => "completed",
                        TraceDeliveryStatus::Failed => "failed",
                    },
                    duration_ms: Some(elapsed_ms),
                    attributes: &attributes,
                    error: error.as_deref(),
                },
            )
            .await
        }
    }
}

pub(crate) async fn persist_batch(
    storage: &db::Storage,
    request_id: Uuid,
    batch: TraceTelemetryBatch,
) -> Result<(), ApiError> {
    validate_trace_telemetry_batch(&batch).map_err(ApiError::Invalid)?;
    let TraceTelemetryBatch {
        events,
        dropped_events,
        root_input_summary,
        root_output_summary,
    } = batch;
    for event in events {
        persist(storage, request_id, event).await?;
    }
    if root_input_summary.is_some() || root_output_summary.is_some() {
        db::update_trace_runtime_io_summaries(
            storage,
            request_id,
            root_input_summary.as_ref().map(|summary| &summary.value),
            root_input_summary
                .as_ref()
                .is_some_and(|summary| summary.effective_truncated()),
            root_output_summary.as_ref().map(|summary| &summary.value),
            root_output_summary
                .as_ref()
                .is_some_and(|summary| summary.effective_truncated()),
        )
        .await?;
    }
    if dropped_events == 0 {
        return Ok(());
    }
    let Some(target) = db::get_runtime_trace_target(storage, request_id).await? else {
        return Ok(());
    };
    let attributes = json!({
        "source": "agentGateway",
        "droppedEvents": dropped_events,
    });
    db::upsert_runtime_trace_observation(
        storage,
        db::RuntimeTraceObservation {
            id: telemetry_gap_observation_id(request_id),
            trace_id: target.trace_id,
            parent_span_id: target.parent_span_id,
            name: "Telemetry dropped",
            kind: "telemetry_gap",
            observation_type: "event",
            provider_key: target.provider_key.as_deref(),
            capability_kind: target.capability_kind.as_deref(),
            model: target.model.as_deref(),
            status: "failed",
            duration_ms: Some(0),
            attributes: &attributes,
            error: Some("Agent Gateway omitted bounded telemetry observations"),
        },
    )
    .await
}

fn telemetry_gap_observation_id(request_id: Uuid) -> Uuid {
    const GAP_MASK: u128 = 0x6f62736572766174696f6e2d67617021;
    Uuid::from_u128(request_id.as_u128() ^ GAP_MASK)
}

fn bounded_duration(name: &str, value: u64) -> Result<i64, ApiError> {
    if value > MAX_TELEMETRY_DURATION_MS {
        return Err(ApiError::Invalid(format!(
            "{name} exceeds the supported telemetry window"
        )));
    }
    i64::try_from(value).map_err(|_| ApiError::Invalid(format!("{name} is out of range")))
}

fn bounded_tokens(name: &str, value: u64) -> Result<i64, ApiError> {
    if value > MAX_TELEMETRY_TOKEN_COUNT {
        return Err(ApiError::Invalid(format!("{name} is out of range")));
    }
    i64::try_from(value).map_err(|_| ApiError::Invalid(format!("{name} is out of range")))
}

fn stage_name(stage: ProviderStage) -> &'static str {
    match stage {
        ProviderStage::Queue => "Queue",
        ProviderStage::Load => "Load",
        ProviderStage::Tokenize => "Tokenize",
        ProviderStage::Prefill => "Prefill",
        ProviderStage::FirstToken => "First token",
        ProviderStage::Decode => "Decode",
        ProviderStage::Validate => "Validate",
    }
}

fn safe_error_message(error: &str) -> String {
    let lower = error.to_ascii_lowercase();
    if [
        "authorization",
        "bearer ",
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
    error
        .chars()
        .take(240)
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use uuid::Uuid;
    use vifu_gateway::protocol::{
        canonical_trace_io_summary, TraceDeliveryStatus, TraceStageStatus, TraceTelemetry,
        TraceTelemetryBatch,
    };
    use vifu_runtime::ProviderStage;

    use super::{
        bounded_duration, persist, persist_batch, safe_error_message, telemetry_gap_observation_id,
    };
    use crate::db::{self, NewProject, NewTrace, NewTraceScore, NewTraceSpan};

    #[test]
    fn hostile_telemetry_is_bounded_and_secrets_are_redacted_server_side() {
        assert!(bounded_duration("elapsedMs", u64::MAX).is_err());
        let message = safe_error_message("password=hunter2 session=private");
        assert_eq!(message, "Provider failed; sensitive details were redacted");
        assert!(!message.contains("hunter2"));
    }

    #[tokio::test]
    async fn canonical_root_io_wins_before_or_after_trace_completion_and_retries() {
        let storage = db::connect("sqlite::memory:", 1).await.unwrap();
        db::migrate(&storage).await.unwrap();
        let project_id = Uuid::new_v4();
        db::create_project(
            &storage,
            NewProject {
                id: project_id,
                owner_user_id: None,
                slug: "io-race-test",
                name: "I/O race test",
                description: None,
                gateway_id: "",
                binding_ids: &[],
            },
        )
        .await
        .unwrap();
        let input_summary = canonical_trace_io_summary(&json!({
            "messages": [{"content": "hello"}],
            "apiKey": "private-input",
        }));
        let output_summary = canonical_trace_io_summary(&json!({
            "choices": [{"message": {"content": "world"}}],
            "authorization": "Bearer private-output",
        }));

        for (index, upload_first) in [false, true].into_iter().enumerate() {
            let request_id = Uuid::new_v4();
            let trace_id = db::create_trace(
                &storage,
                NewTrace {
                    request_id,
                    endpoint_id: None,
                    project_id: Some(project_id),
                    gateway_session_id: None,
                    profile_id: None,
                    profile_version_id: None,
                    operation: "runtime.invoke",
                    provider_key: None,
                    capability_kind: None,
                    selection_key: None,
                    request: &json!({"legacy": index}),
                },
            )
            .await
            .unwrap();
            let root_span_id = db::create_trace_span_with_id(
                &storage,
                request_id,
                NewTraceSpan {
                    trace_id,
                    parent_span_id: None,
                    name: "runtime.invoke",
                    kind: "invocation",
                    observation_type: "generation",
                    provider_key: None,
                    capability_kind: None,
                    model: None,
                    model_parameters: None,
                    input_summary: Some(&json!({"legacy": "input"})),
                    attributes: &json!({}),
                },
            )
            .await
            .unwrap();
            let batch = TraceTelemetryBatch {
                events: vec![TraceTelemetry::InvocationStarted {
                    provider_key: "local-llama".to_string(),
                    capability: "chat".to_string(),
                    model: Some("qwen2.5-2b".to_string()),
                }],
                dropped_events: 0,
                root_input_summary: Some(input_summary.clone()),
                root_output_summary: Some(output_summary.clone()),
            };
            if upload_first {
                persist_batch(&storage, request_id, batch.clone())
                    .await
                    .unwrap();
            }
            db::complete_trace_span(
                &storage,
                root_span_id,
                "completed",
                10,
                Some(&json!({"legacy": "completion output"})),
                None,
            )
            .await
            .unwrap();
            if !upload_first {
                persist_batch(&storage, request_id, batch.clone())
                    .await
                    .unwrap();
            }

            let spans = db::list_trace_spans(&storage, trace_id).await.unwrap();
            let root = spans.iter().find(|span| span.id == root_span_id).unwrap();
            assert_eq!(root.input_summary.as_ref(), Some(&input_summary.value));
            assert_eq!(root.output_summary.as_ref(), Some(&output_summary.value));
            assert_eq!(root.attributes["_vifuTraceIo"]["inputCanonical"], true);
            assert_eq!(root.attributes["_vifuTraceIo"]["outputCanonical"], true);

            persist_batch(&storage, request_id, batch).await.unwrap();
            let spans = db::list_trace_spans(&storage, trace_id).await.unwrap();
            let root = spans.iter().find(|span| span.id == root_span_id).unwrap();
            assert_eq!(root.input_summary.as_ref(), Some(&input_summary.value));
            assert_eq!(root.output_summary.as_ref(), Some(&output_summary.value));
        }
    }

    #[tokio::test]
    async fn telemetry_populates_trace_identity_generation_stages_and_list_aggregates() {
        let storage = db::connect("sqlite::memory:", 1).await.unwrap();
        db::migrate(&storage).await.unwrap();
        let project_id = Uuid::new_v4();
        db::create_project(
            &storage,
            NewProject {
                id: project_id,
                owner_user_id: None,
                slug: "telemetry-test",
                name: "Telemetry test",
                description: None,
                gateway_id: "",
                binding_ids: &[],
            },
        )
        .await
        .unwrap();
        let request_id = Uuid::new_v4();
        let trace_id = db::create_trace(
            &storage,
            NewTrace {
                request_id,
                endpoint_id: None,
                project_id: Some(project_id),
                gateway_session_id: None,
                profile_id: None,
                profile_version_id: None,
                operation: "runtime.invoke",
                provider_key: None,
                capability_kind: None,
                selection_key: None,
                request: &json!({"messages": [{"role": "user", "content": "hello"}]}),
            },
        )
        .await
        .unwrap();
        let root_span_id = db::create_trace_span_with_id(
            &storage,
            request_id,
            NewTraceSpan {
                trace_id,
                parent_span_id: None,
                name: "runtime.invoke",
                kind: "invocation",
                observation_type: "generation",
                provider_key: None,
                capability_kind: None,
                model: None,
                model_parameters: None,
                input_summary: None,
                attributes: &json!({}),
            },
        )
        .await
        .unwrap();
        assert_eq!(root_span_id, request_id);

        persist(
            &storage,
            request_id,
            TraceTelemetry::InvocationStarted {
                provider_key: "local-llama".to_string(),
                capability: "chat".to_string(),
                model: Some("qwen2.5-2b".to_string()),
            },
        )
        .await
        .unwrap();
        let gap_batch = TraceTelemetryBatch {
            events: vec![TraceTelemetry::InvocationStarted {
                provider_key: "local-llama".to_string(),
                capability: "chat".to_string(),
                model: Some("qwen2.5-2b".to_string()),
            }],
            dropped_events: 2,
            root_input_summary: None,
            root_output_summary: None,
        };
        persist_batch(&storage, request_id, gap_batch.clone())
            .await
            .unwrap();
        persist_batch(&storage, request_id, gap_batch)
            .await
            .unwrap();
        let first_token_observation_id = Uuid::new_v4();
        persist(
            &storage,
            request_id,
            TraceTelemetry::ProviderStage {
                observation_id: first_token_observation_id,
                stage: ProviderStage::FirstToken,
                status: TraceStageStatus::Completed,
                start_offset_ms: 38,
                end_offset_ms: Some(42),
                elapsed_ms: Some(4),
                request_elapsed_ms: Some(42),
                input_tokens: None,
                output_tokens: None,
                resident: Some(true),
                error: None,
            },
        )
        .await
        .unwrap();
        let decode_observation_id = Uuid::new_v4();
        persist(
            &storage,
            request_id,
            TraceTelemetry::ProviderStage {
                observation_id: decode_observation_id,
                stage: ProviderStage::Decode,
                status: TraceStageStatus::Started,
                start_offset_ms: 42,
                end_offset_ms: None,
                elapsed_ms: None,
                request_elapsed_ms: None,
                input_tokens: None,
                output_tokens: None,
                resident: Some(true),
                error: None,
            },
        )
        .await
        .unwrap();
        persist(
            &storage,
            request_id,
            TraceTelemetry::ProviderStage {
                observation_id: decode_observation_id,
                stage: ProviderStage::Decode,
                status: TraceStageStatus::Completed,
                start_offset_ms: 42,
                end_offset_ms: Some(117),
                elapsed_ms: Some(75),
                request_elapsed_ms: Some(117),
                input_tokens: Some(12),
                output_tokens: Some(8),
                resident: Some(true),
                error: None,
            },
        )
        .await
        .unwrap();
        persist(
            &storage,
            request_id,
            TraceTelemetry::Delivery {
                observation_id: Uuid::new_v4(),
                status: TraceDeliveryStatus::Delivered,
                start_offset_ms: 117,
                end_offset_ms: 120,
                elapsed_ms: 3,
                error: None,
            },
        )
        .await
        .unwrap();
        db::upsert_trace_score(
            &storage,
            NewTraceScore {
                trace_id,
                span_id: Some(root_span_id),
                name: "OUTPUT_ACCEPTED",
                data_type: "categorical",
                value: &json!("fail"),
                source: "application",
            },
        )
        .await
        .unwrap();
        db::complete_trace_span(
            &storage,
            root_span_id,
            "failed",
            0,
            None,
            Some("Authorization: Bearer private-token"),
        )
        .await
        .unwrap();
        let diagnostic_span_id = db::create_trace_span(
            &storage,
            NewTraceSpan {
                trace_id,
                parent_span_id: Some(root_span_id),
                name: "Diagnostic",
                kind: "event",
                observation_type: "event",
                provider_key: None,
                capability_kind: None,
                model: None,
                model_parameters: None,
                input_summary: None,
                attributes: &json!({}),
            },
        )
        .await
        .unwrap();
        db::complete_trace_span(
            &storage,
            diagnostic_span_id,
            "failed",
            0,
            None,
            Some("model returned invalid JSON"),
        )
        .await
        .unwrap();

        let traces = db::list_traces(
            &storage,
            db::TraceListOptions {
                endpoint_id: None,
                project_id: Some(project_id),
                request_id: Some(request_id),
                trace_id: None,
                allowed_profile_ids: None,
                created_from: None,
                created_before: None,
                cursor: None,
                limit: 10,
            },
        )
        .await
        .unwrap();
        assert_eq!(traces.len(), 1);
        let trace = &traces[0];
        assert_eq!(trace.provider_key.as_deref(), Some("local-llama"));
        assert_eq!(trace.capability_kind.as_deref(), Some("chat"));
        assert_eq!(trace.model.as_deref(), Some("qwen2.5-2b"));
        assert_eq!(trace.completion_start_ms, Some(42));
        assert_eq!(
            trace.usage,
            Some(json!({"inputTokens": 12, "outputTokens": 8}))
        );
        assert_eq!(trace.decode_ms, Some(75));
        assert_eq!(trace.app_outcome.as_deref(), Some("fail"));

        let spans = db::list_trace_spans(&storage, trace_id).await.unwrap();
        let root = spans.iter().find(|span| span.id == root_span_id).unwrap();
        assert_eq!(
            root.error.as_deref(),
            Some("[REDACTED sensitive trace error]")
        );
        assert!(!root.error.as_deref().unwrap().contains("private-token"));
        assert_eq!(
            spans
                .iter()
                .find(|span| span.id == diagnostic_span_id)
                .and_then(|span| span.error.as_deref()),
            Some("model returned invalid JSON")
        );
        assert!(spans.iter().any(|span| {
            span.name == "First token"
                && span.parent_span_id == Some(root_span_id)
                && span.status == "completed"
        }));
        assert!(spans.iter().any(|span| {
            span.id == decode_observation_id
                && span.name == "Decode"
                && span.duration_ms == Some(75)
                && span.status == "completed"
                && span.attributes["startOffsetMs"] == 42
                && span.attributes["endOffsetMs"] == 117
        }));
        assert!(spans.iter().any(|span| {
            span.name == "Deliver" && span.duration_ms == Some(3) && span.status == "completed"
        }));
        let gaps = spans
            .iter()
            .filter(|span| span.name == "Telemetry dropped")
            .collect::<Vec<_>>();
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].id, telemetry_gap_observation_id(request_id));
        assert_eq!(gaps[0].attributes["droppedEvents"], 2);
    }
}
