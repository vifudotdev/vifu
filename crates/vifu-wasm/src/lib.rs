//! WebAssembly facade for embedding Vifu Runtime in JavaScript hosts.

#[cfg(target_arch = "wasm32")]
use std::cell::RefCell;
#[cfg(target_arch = "wasm32")]
use std::rc::Rc;
#[cfg(target_arch = "wasm32")]
use std::sync::Arc;

#[cfg(target_arch = "wasm32")]
use js_sys::{Function, Promise, JSON};
use serde_json::Value;
use vifu_runtime::{
    AgentDefinition, EndpointDefinition, InvocationData, InvocationInput, RuntimeError, VifuRuntime,
};
#[cfg(target_arch = "wasm32")]
use vifu_runtime::{
    AgentProvider, CancellationToken, ProviderEventSink, ProviderFuture, ProviderRequest,
    ProviderStage,
};
use wasm_bindgen::prelude::*;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_futures::JsFuture;

#[wasm_bindgen(js_name = VifuRuntime)]
pub struct VifuWasmRuntime {
    runtime: VifuRuntime,
}

#[wasm_bindgen(js_class = VifuRuntime)]
impl VifuWasmRuntime {
    #[wasm_bindgen(constructor)]
    pub fn new(app_id: String) -> Result<VifuWasmRuntime, JsValue> {
        Ok(Self {
            runtime: VifuRuntime::new(app_id).map_err(runtime_error)?,
        })
    }

    #[wasm_bindgen(getter, js_name = appId)]
    pub fn app_id(&self) -> String {
        self.runtime.project_id().to_string()
    }

    #[wasm_bindgen(js_name = registerProvider)]
    #[cfg(target_arch = "wasm32")]
    pub fn register_provider(
        &self,
        provider_id: String,
        callback: Function,
    ) -> Result<(), JsValue> {
        self.runtime
            .register_provider(provider_id, Arc::new(JavaScriptProvider { callback }))
            .map_err(runtime_error)
    }

    #[wasm_bindgen(js_name = registerAgent)]
    pub fn register_agent(
        &self,
        agent_id: String,
        name: String,
        provider_id: String,
        capabilities_json: String,
        metadata_json: String,
    ) -> Result<(), JsValue> {
        let capabilities = serde_json::from_str(&capabilities_json)
            .map_err(|error| js_error(format!("agent capabilities are invalid: {error}")))?;
        let metadata = parse_json(&metadata_json, "agent metadata")?;
        self.runtime
            .register_agent(AgentDefinition {
                id: agent_id,
                name,
                provider: provider_id,
                capabilities,
                metadata,
            })
            .map_err(runtime_error)
    }

    #[wasm_bindgen(js_name = registerEndpoint)]
    pub fn register_endpoint(
        &self,
        name: String,
        agent_id: String,
        capability: String,
        timeout_ms: u64,
    ) -> Result<(), JsValue> {
        self.runtime
            .register_endpoint(EndpointDefinition {
                name,
                agent: agent_id,
                capability,
                timeout_ms,
            })
            .map_err(runtime_error)
    }

    #[wasm_bindgen(js_name = invoke)]
    pub async fn invoke(
        &self,
        endpoint: String,
        session_id: String,
        input_json: String,
        metadata_json: String,
    ) -> Result<String, JsValue> {
        let output = self
            .runtime
            .invoke(InvocationInput {
                endpoint,
                session_id,
                data: InvocationData::Json(parse_json(&input_json, "invocation input")?),
                metadata: parse_json(&metadata_json, "invocation metadata")?,
            })
            .await
            .map_err(runtime_error)?;
        serde_json::to_string(&output)
            .map_err(|error| js_error(format!("invocation output could not be encoded: {error}")))
    }

    #[wasm_bindgen(js_name = exportSnapshot)]
    pub fn export_snapshot(&self) -> Result<Vec<u8>, JsValue> {
        self.runtime.export_snapshot().map_err(runtime_error)
    }

    #[wasm_bindgen(js_name = restoreSnapshot)]
    pub fn restore_snapshot(&self, bytes: Vec<u8>) -> Result<(), JsValue> {
        self.runtime.restore_snapshot(&bytes).map_err(runtime_error)
    }

    #[wasm_bindgen(js_name = pendingTraces)]
    pub fn pending_traces(&self, limit: usize) -> Result<String, JsValue> {
        let traces = self.runtime.pending_traces(limit).map_err(runtime_error)?;
        serde_json::to_string(&traces)
            .map_err(|error| js_error(format!("runtime traces could not be encoded: {error}")))
    }

    #[wasm_bindgen(js_name = acknowledgeTraces)]
    pub fn acknowledge_traces(&self, trace_ids_json: String) -> Result<(), JsValue> {
        let trace_ids = serde_json::from_str::<Vec<String>>(&trace_ids_json)
            .map_err(|error| js_error(format!("trace IDs are invalid: {error}")))?;
        self.runtime
            .acknowledge_traces(&trace_ids)
            .map_err(runtime_error)
    }
}

#[cfg(target_arch = "wasm32")]
struct JavaScriptProvider {
    callback: Function,
}

#[cfg(target_arch = "wasm32")]
impl AgentProvider for JavaScriptProvider {
    fn supports(&self, _capability: &str) -> bool {
        true
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
        request: ProviderRequest,
        cancellation: CancellationToken,
        events: ProviderEventSink,
    ) -> ProviderFuture<'a> {
        Box::pin(async move {
            if cancellation.is_cancelled() {
                return Err(RuntimeError::Cancelled);
            }
            let request = provider_request_json(request);
            let request = json_to_js(&request).map_err(provider_error)?;
            let event_error = Rc::new(RefCell::new(None));
            let event_error_for_callback = Rc::clone(&event_error);
            let emit = Closure::<dyn Fn(JsValue)>::new(move |value| {
                if event_error_for_callback.borrow().is_some() {
                    return;
                }
                let result = js_to_json(&value).and_then(|value| provider_event(value, &events));
                if let Err(error) = result {
                    *event_error_for_callback.borrow_mut() = Some(error);
                }
            });
            let returned = self
                .callback
                .call2(&JsValue::UNDEFINED, &request, emit.as_ref())
                .map_err(|error| provider_error(js_value_message(error)))?;
            let resolved = JsFuture::from(Promise::resolve(&returned))
                .await
                .map_err(|error| provider_error(js_value_message(error)))?;
            if let Some(error) = event_error.borrow_mut().take() {
                return Err(provider_error(error));
            }
            if cancellation.is_cancelled() {
                return Err(RuntimeError::Cancelled);
            }
            let response = js_to_json(&resolved).map_err(provider_error)?;
            provider_response(response)
        })
    }
}

#[cfg(target_arch = "wasm32")]
fn provider_event(value: Value, events: &ProviderEventSink) -> Result<(), String> {
    let event = serde_json::from_value::<JavaScriptProviderEvent>(value)
        .map_err(|error| format!("provider trace event is invalid: {error}"))?;
    match event {
        JavaScriptProviderEvent::Activity => events.activity(),
        JavaScriptProviderEvent::OutputDelta { value } => {
            events.output_delta(InvocationData::Json(value))
        }
        JavaScriptProviderEvent::StageStarted { stage, metadata } => {
            events.stage_started(provider_stage(&stage)?, metadata)
        }
        JavaScriptProviderEvent::StageCompleted {
            stage,
            elapsed_ms,
            metadata,
        } => events.stage_completed(provider_stage(&stage)?, elapsed_ms, metadata),
        JavaScriptProviderEvent::StageFailed {
            stage,
            elapsed_ms,
            error,
            metadata,
        } => events.stage_failed(provider_stage(&stage)?, elapsed_ms, error, metadata),
    }
    Ok(())
}

#[cfg(target_arch = "wasm32")]
#[derive(serde::Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum JavaScriptProviderEvent {
    Activity,
    OutputDelta {
        value: Value,
    },
    StageStarted {
        stage: String,
        #[serde(default)]
        metadata: Value,
    },
    StageCompleted {
        stage: String,
        elapsed_ms: u64,
        #[serde(default)]
        metadata: Value,
    },
    StageFailed {
        stage: String,
        elapsed_ms: u64,
        error: String,
        #[serde(default)]
        metadata: Value,
    },
}

#[cfg(target_arch = "wasm32")]
fn provider_stage(value: &str) -> Result<ProviderStage, String> {
    match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
        "queue" => Ok(ProviderStage::Queue),
        "load" => Ok(ProviderStage::Load),
        "tokenize" => Ok(ProviderStage::Tokenize),
        "prefill" => Ok(ProviderStage::Prefill),
        "first_token" => Ok(ProviderStage::FirstToken),
        "decode" => Ok(ProviderStage::Decode),
        "validate" => Ok(ProviderStage::Validate),
        _ => Err(
            "provider stage must be queue, load, tokenize, prefill, first_token, decode, or validate"
                .to_string(),
        ),
    }
}

#[cfg(target_arch = "wasm32")]
fn provider_request_json(request: ProviderRequest) -> Value {
    serde_json::json!({
        "appId": request.project_id,
        "endpoint": request.endpoint,
        "sessionId": request.session_id,
        "agent": request.agent,
        "capability": request.capability,
        "input": invocation_data_json(request.data),
        "metadata": request.metadata,
        "state": request.snapshot.state,
        "stateRevision": request.snapshot.revision,
    })
}

#[cfg(target_arch = "wasm32")]
fn invocation_data_json(data: InvocationData) -> Value {
    match data {
        InvocationData::Json(value) => value,
        InvocationData::Binary(bytes) => serde_json::json!({
            "_vifuBinary": true,
            "bytes": bytes,
        }),
    }
}

#[cfg(any(test, target_arch = "wasm32"))]
fn provider_response(value: Value) -> Result<vifu_runtime::ProviderResponse, RuntimeError> {
    let Value::Object(mut object) = value else {
        return Ok(vifu_runtime::ProviderResponse::json(value));
    };
    if !object.contains_key("output") {
        return Ok(vifu_runtime::ProviderResponse::json(Value::Object(object)));
    }
    let output = object.remove("output").unwrap_or(Value::Null);
    let metadata = object
        .remove("metadata")
        .unwrap_or_else(|| serde_json::json!({}));
    let state = object.remove("state");
    Ok(vifu_runtime::ProviderResponse {
        data: InvocationData::Json(output),
        metadata,
        state,
    })
}

#[cfg(target_arch = "wasm32")]
fn json_to_js(value: &Value) -> Result<JsValue, String> {
    let encoded = serde_json::to_string(value).map_err(|error| error.to_string())?;
    JSON::parse(&encoded).map_err(js_value_message)
}

#[cfg(target_arch = "wasm32")]
fn js_to_json(value: &JsValue) -> Result<Value, String> {
    let encoded = JSON::stringify(value).map_err(js_value_message)?;
    serde_json::from_str(&String::from(encoded)).map_err(|error| error.to_string())
}

fn parse_json(value: &str, label: &str) -> Result<Value, JsValue> {
    serde_json::from_str(value).map_err(|error| js_error(format!("{label} is invalid: {error}")))
}

fn runtime_error(error: RuntimeError) -> JsValue {
    js_error(error.public_message())
}

#[cfg(target_arch = "wasm32")]
fn provider_error(message: String) -> RuntimeError {
    RuntimeError::provider("javascript", message)
}

fn js_error(message: impl AsRef<str>) -> JsValue {
    js_sys::Error::new(message.as_ref()).into()
}

#[cfg(target_arch = "wasm32")]
fn js_value_message(value: JsValue) -> String {
    value
        .as_string()
        .or_else(|| {
            js_sys::Reflect::get(&value, &JsValue::from_str("message"))
                .ok()
                .and_then(|message| message.as_string())
        })
        .unwrap_or_else(|| "JavaScript provider failed".to_string())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::provider_response;
    use vifu_runtime::InvocationData;

    #[test]
    fn provider_response_accepts_plain_values() {
        let response = provider_response(json!("hello")).unwrap();
        assert_eq!(response.data, InvocationData::Json(json!("hello")));
    }

    #[test]
    fn provider_response_extracts_output_metadata_and_state() {
        let response = provider_response(json!({
            "output": { "text": "hello" },
            "metadata": { "model": "test" },
            "state": { "turn": 1 }
        }))
        .unwrap();
        assert_eq!(
            response.data,
            InvocationData::Json(json!({ "text": "hello" }))
        );
        assert_eq!(response.metadata, json!({ "model": "test" }));
        assert_eq!(response.state, Some(json!({ "turn": 1 })));
    }
}
