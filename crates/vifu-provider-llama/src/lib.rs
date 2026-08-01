//! Local llama.cpp provider for the embedded Vifu runtime.

use std::fmt;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::json_schema_to_grammar;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, LlamaChatMessage, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use tokio::sync::Semaphore;
use vifu_runtime::{
    AgentProvider, CancellationToken, InvocationData, ProviderEventSink, ProviderFuture,
    ProviderRequest, ProviderResponse, RuntimeError,
};

const DEFAULT_CONTEXT_SIZE: u32 = 4_096;
const DEFAULT_MAX_TOKENS: u32 = 256;
const MAX_GENERATED_TOKENS: u32 = 2_048;
const DEFAULT_TEMPERATURE: f32 = 0.7;
const DEFAULT_TOP_P: f32 = 0.9;
const DEFAULT_MAX_CONCURRENCY: usize = 1;
const MAX_CONCURRENCY: usize = 64;
const MAX_JSON_SCHEMA_BYTES: usize = 64 * 1024;
const MAX_JSON_SCHEMA_NAME_BYTES: usize = 64;
const THINK_OPEN_TAG: &str = "<think>";
const THINK_CLOSE_TAG: &str = "</think>";

static LLAMA_BACKEND: OnceLock<Result<Arc<LlamaBackend>, String>> = OnceLock::new();

#[derive(Clone)]
pub struct LlamaProviderConfig {
    pub model_path: PathBuf,
    pub context_size: u32,
    pub gpu_layers: u32,
    pub default_max_tokens: u32,
    pub max_concurrency: usize,
}

impl fmt::Debug for LlamaProviderConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LlamaProviderConfig")
            .field("model_path", &"[REDACTED]")
            .field("context_size", &self.context_size)
            .field("gpu_layers", &self.gpu_layers)
            .field("default_max_tokens", &self.default_max_tokens)
            .field("max_concurrency", &self.max_concurrency)
            .finish()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct LlamaProviderFileConfig {
    model_path: PathBuf,
    #[serde(default)]
    context_size: Option<u32>,
    #[serde(default)]
    gpu_layers: Option<u32>,
    #[serde(default)]
    default_max_tokens: Option<u32>,
    #[serde(default)]
    max_concurrency: Option<usize>,
}

impl LlamaProviderConfig {
    pub fn new(model_path: impl Into<PathBuf>) -> Self {
        Self {
            model_path: model_path.into(),
            context_size: DEFAULT_CONTEXT_SIZE,
            gpu_layers: u32::MAX,
            default_max_tokens: DEFAULT_MAX_TOKENS,
            max_concurrency: DEFAULT_MAX_CONCURRENCY,
        }
    }

    pub fn from_provider_config(
        value: &Value,
        base_dir: &Path,
    ) -> Result<Self, LlamaProviderError> {
        let file = serde_json::from_value::<LlamaProviderFileConfig>(value.clone())
            .map_err(|error| LlamaProviderError::InvalidConfig(error.to_string()))?;
        if file.model_path.as_os_str().is_empty() {
            return Err(LlamaProviderError::InvalidConfig(
                "modelPath must not be empty".to_string(),
            ));
        }
        let model_path = if file.model_path.is_absolute() {
            file.model_path
        } else {
            base_dir.join(file.model_path)
        };
        let config = Self {
            model_path,
            context_size: file.context_size.unwrap_or(DEFAULT_CONTEXT_SIZE),
            gpu_layers: file.gpu_layers.unwrap_or(u32::MAX),
            default_max_tokens: file.default_max_tokens.unwrap_or(DEFAULT_MAX_TOKENS),
            max_concurrency: file.max_concurrency.unwrap_or(DEFAULT_MAX_CONCURRENCY),
        };
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), LlamaProviderError> {
        if self.context_size == 0 {
            return Err(LlamaProviderError::InvalidContextSize);
        }
        if !(1..=MAX_GENERATED_TOKENS).contains(&self.default_max_tokens) {
            return Err(LlamaProviderError::InvalidConfig(format!(
                "defaultMaxTokens must be between 1 and {MAX_GENERATED_TOKENS}"
            )));
        }
        if !(1..=MAX_CONCURRENCY).contains(&self.max_concurrency) {
            return Err(LlamaProviderError::InvalidConfig(format!(
                "maxConcurrency must be between 1 and {MAX_CONCURRENCY}"
            )));
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LlamaProviderError {
    #[error("model file does not exist")]
    ModelNotFound,
    #[error("context size must be greater than zero")]
    InvalidContextSize,
    #[error("provider configuration is invalid: {0}")]
    InvalidConfig(String),
    #[error("llama.cpp backend could not start: {0}")]
    Backend(String),
    #[error("GGUF model could not be loaded: {0}")]
    Model(String),
}

pub struct LlamaProvider {
    backend: Arc<LlamaBackend>,
    model: Arc<LlamaModel>,
    context_size: NonZeroU32,
    default_max_tokens: u32,
    concurrency: Arc<Semaphore>,
}

impl LlamaProvider {
    pub fn load(config: LlamaProviderConfig) -> Result<Self, LlamaProviderError> {
        config.validate()?;
        if !Path::new(&config.model_path).is_file() {
            return Err(LlamaProviderError::ModelNotFound);
        }
        let context_size =
            NonZeroU32::new(config.context_size).ok_or(LlamaProviderError::InvalidContextSize)?;
        let backend = match LLAMA_BACKEND.get_or_init(|| {
            LlamaBackend::init()
                .map(Arc::new)
                .map_err(|error| error.to_string())
        }) {
            Ok(backend) => Arc::clone(backend),
            Err(message) => return Err(LlamaProviderError::Backend(message.clone())),
        };
        let model_params = LlamaModelParams::default().with_n_gpu_layers(config.gpu_layers);
        let model = LlamaModel::load_from_file(&backend, &config.model_path, &model_params)
            .map_err(|error| LlamaProviderError::Model(error.to_string()))?;
        Ok(Self {
            backend,
            model: Arc::new(model),
            context_size,
            default_max_tokens: config.default_max_tokens,
            concurrency: Arc::new(Semaphore::new(config.max_concurrency)),
        })
    }
}

impl AgentProvider for LlamaProvider {
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
        request: ProviderRequest,
        cancellation: CancellationToken,
        events: ProviderEventSink,
    ) -> ProviderFuture<'a> {
        let backend = Arc::clone(&self.backend);
        let model = Arc::clone(&self.model);
        let concurrency = Arc::clone(&self.concurrency);
        let context_size = self.context_size;
        let default_max_tokens = self.default_max_tokens;
        Box::pin(async move {
            let permit = concurrency
                .try_acquire_owned()
                .map_err(|_error| provider_error("local model concurrency limit reached"))?;
            let task = tokio::task::spawn_blocking(move || {
                let _permit = permit;
                generate(
                    &backend,
                    &model,
                    context_size,
                    default_max_tokens,
                    request,
                    &cancellation,
                    &events,
                )
            });
            task.await
                .map_err(|_error| RuntimeError::provider("llama", "local model task stopped"))?
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChatRequest {
    messages: Vec<ChatMessage>,
    #[serde(default, alias = "max_tokens")]
    max_tokens: Option<u32>,
    #[serde(default)]
    temperature: Option<f32>,
    #[serde(default, alias = "top_p")]
    top_p: Option<f32>,
    #[serde(default)]
    seed: Option<u32>,
    #[serde(default, alias = "response_format")]
    response_format: Option<Value>,
}

#[derive(Deserialize)]
struct ChatMessage {
    role: String,
    content: Value,
}

#[derive(Debug, PartialEq)]
struct StructuredOutput {
    name: String,
    schema: Value,
}

#[derive(Debug, Default, PartialEq, Eq)]
enum OutputPhase {
    #[default]
    Detecting,
    Reasoning,
    Visible,
}

#[derive(Debug, Default)]
struct VisibleOutput {
    phase: OutputPhase,
    pending: String,
}

impl VisibleOutput {
    fn push(&mut self, piece: &str) -> Option<String> {
        match self.phase {
            OutputPhase::Detecting => {
                self.pending.push_str(piece);
                let candidate = self.pending.trim_start();
                if let Some(reasoning) = candidate.strip_prefix(THINK_OPEN_TAG) {
                    self.pending = reasoning.to_string();
                    self.phase = OutputPhase::Reasoning;
                    self.finish_reasoning()
                } else if THINK_OPEN_TAG.starts_with(candidate) {
                    None
                } else {
                    self.phase = OutputPhase::Visible;
                    Some(std::mem::take(&mut self.pending))
                }
            }
            OutputPhase::Reasoning => {
                self.pending.push_str(piece);
                self.finish_reasoning()
            }
            OutputPhase::Visible => Some(piece.to_string()),
        }
    }

    fn finish(&mut self) -> Option<String> {
        match self.phase {
            OutputPhase::Detecting => {
                self.phase = OutputPhase::Visible;
                non_empty(std::mem::take(&mut self.pending))
            }
            OutputPhase::Reasoning => None,
            OutputPhase::Visible => None,
        }
    }

    fn finish_reasoning(&mut self) -> Option<String> {
        let close_index = self.pending.find(THINK_CLOSE_TAG)?;
        let visible = self.pending[close_index + THINK_CLOSE_TAG.len()..]
            .trim_start()
            .to_string();
        self.pending.clear();
        self.phase = OutputPhase::Visible;
        non_empty(visible)
    }
}

fn generate(
    backend: &LlamaBackend,
    model: &LlamaModel,
    context_size: NonZeroU32,
    default_max_tokens: u32,
    request: ProviderRequest,
    cancellation: &CancellationToken,
    events: &ProviderEventSink,
) -> Result<ProviderResponse, RuntimeError> {
    let input = match request.data {
        InvocationData::Json(value) => serde_json::from_value::<ChatRequest>(value)
            .map_err(|_error| provider_error("chat input is invalid"))?,
        InvocationData::Binary(_) => {
            return Err(provider_error("chat input must be JSON"));
        }
    };
    if input.messages.is_empty() {
        return Err(provider_error("at least one chat message is required"));
    }
    let structured_output = input
        .response_format
        .as_ref()
        .map(parse_structured_output)
        .transpose()?;
    let grammar = structured_output
        .as_ref()
        .map(|format| {
            let schema = serde_json::to_string(&format.schema)
                .map_err(|_error| provider_error("response JSON schema could not be encoded"))?;
            json_schema_to_grammar(&schema)
                .map_err(|_error| provider_error("response JSON schema is unsupported"))
        })
        .transpose()?;
    let mut messages = Vec::with_capacity(input.messages.len() + 1);
    if let Some(instructions) = agent_instructions(&request.agent.metadata) {
        messages.push(
            LlamaChatMessage::new("system".to_string(), instructions)
                .map_err(|_error| provider_error("agent instructions are invalid"))?,
        );
    }
    for message in input.messages {
        if !matches!(message.role.as_str(), "system" | "user" | "assistant") {
            return Err(provider_error("chat message role is unsupported"));
        }
        let content = chat_message_content(&message.content)?;
        messages.push(
            LlamaChatMessage::new(message.role, content)
                .map_err(|_error| provider_error("chat message is invalid"))?,
        );
    }

    let template = model
        .chat_template(None)
        .map_err(|_error| provider_error("model does not include a supported chat template"))?;
    let prompt = model
        .apply_chat_template(&template, &messages, true)
        .map_err(|_error| provider_error("model chat template could not be applied"))?;
    let tokens = model
        .str_to_token(&prompt, AddBos::Always)
        .map_err(|_error| provider_error("chat prompt could not be tokenized"))?;
    if tokens.is_empty() {
        return Err(provider_error("chat prompt produced no tokens"));
    }

    let max_tokens = input
        .max_tokens
        .unwrap_or(default_max_tokens)
        .clamp(1, MAX_GENERATED_TOKENS);
    let context_tokens = usize::try_from(context_size.get())
        .map_err(|_error| provider_error("context size is unsupported"))?;
    if tokens.len().saturating_add(max_tokens as usize) > context_tokens {
        return Err(provider_error(
            "chat prompt exceeds the configured context size",
        ));
    }
    let mut context = model
        .new_context(
            backend,
            LlamaContextParams::default().with_n_ctx(Some(context_size)),
        )
        .map_err(|_error| provider_error("model context could not be created"))?;
    let mut batch = LlamaBatch::new(tokens.len().max(1), 1);
    let last_index = i32::try_from(tokens.len() - 1)
        .map_err(|_error| provider_error("chat prompt is too long"))?;
    for (position, token) in (0_i32..).zip(tokens) {
        batch
            .add(token, position, &[0], position == last_index)
            .map_err(|_error| provider_error("chat prompt could not be prepared"))?;
    }
    context
        .decode(&mut batch)
        .map_err(|_error| provider_error("chat prompt inference failed"))?;

    let temperature = input
        .temperature
        .unwrap_or(DEFAULT_TEMPERATURE)
        .clamp(0.0, 2.0);
    let top_p = input.top_p.unwrap_or(DEFAULT_TOP_P).clamp(0.0, 1.0);
    let mut samplers = Vec::with_capacity(5);
    if let Some(grammar) = grammar.as_deref() {
        samplers.push(
            LlamaSampler::grammar(model, grammar, "root")
                .map_err(|_error| provider_error("response JSON grammar could not be created"))?,
        );
    }
    samplers.extend([
        LlamaSampler::top_k(40),
        LlamaSampler::top_p(top_p, 1),
        LlamaSampler::temp(temperature),
        LlamaSampler::dist(input.seed.unwrap_or(0)),
    ]);
    let mut sampler = LlamaSampler::chain_simple(samplers);
    let mut decoder = encoding_rs::UTF_8.new_decoder();
    let mut output = String::new();
    let mut visible_output = VisibleOutput::default();
    let mut generated_tokens = 0_u32;
    let mut position = batch.n_tokens();
    let input_tokens = position;
    let mut stopped_on_eog = false;
    let mut stopped_on_valid_json = false;
    while generated_tokens < max_tokens {
        if cancellation.is_cancelled() {
            return Err(RuntimeError::Cancelled);
        }
        let token = sampler.sample(&context, batch.n_tokens() - 1);
        if model.is_eog_token(token) {
            stopped_on_eog = true;
            break;
        }
        let piece = model
            .token_to_piece(token, &mut decoder, true, None)
            .map_err(|_error| provider_error("model output could not be decoded"))?;
        if let Some(piece) = visible_output.push(&piece) {
            output.push_str(&piece);
            events.output_delta(InvocationData::Json(Value::String(piece)));
        }
        if structured_output.is_some() && structured_json_is_complete(&output) {
            generated_tokens += 1;
            stopped_on_valid_json = true;
            break;
        }
        batch.clear();
        batch
            .add(token, position, &[0], true)
            .map_err(|_error| provider_error("model output could not be prepared"))?;
        context
            .decode(&mut batch)
            .map_err(|_error| provider_error("model output inference failed"))?;
        position += 1;
        generated_tokens += 1;
    }
    if let Some(piece) = visible_output.finish() {
        output.push_str(&piece);
        events.output_delta(InvocationData::Json(Value::String(piece)));
    }

    let structured = structured_output
        .as_ref()
        .map(|_format| {
            serde_json::from_str::<Value>(output.trim()).map_err(|_error| {
                provider_error("structured response ended before producing valid JSON")
            })
        })
        .transpose()?;
    let finish_reason = if stopped_on_eog || stopped_on_valid_json {
        "stop"
    } else {
        "length"
    };
    let total_tokens = input_tokens.saturating_add(
        i32::try_from(generated_tokens).map_err(|_error| provider_error("token count overflow"))?,
    );
    let mut data = Map::from_iter([
        ("text".to_string(), Value::String(output.clone())),
        (
            "message".to_string(),
            json!({
                "role": "assistant",
                "content": output,
            }),
        ),
        (
            "choices".to_string(),
            json!([{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": output,
                },
                "finish_reason": finish_reason,
            }]),
        ),
        (
            "usage".to_string(),
            json!({
                "prompt_tokens": input_tokens,
                "completion_tokens": generated_tokens,
                "total_tokens": total_tokens,
            }),
        ),
    ]);
    if let (Some(format), Some(structured)) = (structured_output.as_ref(), structured) {
        data.insert(
            "structuredSchema".to_string(),
            Value::String(format.name.clone()),
        );
        data.insert("structured".to_string(), structured);
    }

    Ok(ProviderResponse {
        data: InvocationData::Json(Value::Object(data)),
        metadata: json!({
            "inputTokens": input_tokens,
            "outputTokens": generated_tokens,
            "finishReason": finish_reason,
        }),
        state: None,
    })
}

fn parse_structured_output(value: &Value) -> Result<StructuredOutput, RuntimeError> {
    let object = value
        .as_object()
        .ok_or_else(|| provider_error("response format must be an object"))?;
    let format_type = object
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| provider_error("response format type is required"))?;
    let definition = match format_type {
        "jsonSchema" => object,
        "json_schema" => object
            .get("json_schema")
            .and_then(Value::as_object)
            .ok_or_else(|| provider_error("response_format.json_schema is required"))?,
        _ => return Err(provider_error("response format type is unsupported")),
    };
    if definition
        .get("strict")
        .is_some_and(|strict| strict != true)
    {
        return Err(provider_error("structured response strict must be true"));
    }
    let name = definition
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| provider_error("response JSON schema name is required"))?;
    if name.len() > MAX_JSON_SCHEMA_NAME_BYTES {
        return Err(provider_error("response JSON schema name is too long"));
    }
    let schema = definition
        .get("schema")
        .filter(|schema| schema.is_object())
        .cloned()
        .ok_or_else(|| provider_error("response JSON schema must be an object"))?;
    let schema_bytes = serde_json::to_vec(&schema)
        .map_err(|_error| provider_error("response JSON schema could not be encoded"))?;
    if schema_bytes.len() > MAX_JSON_SCHEMA_BYTES {
        return Err(provider_error("response JSON schema is too large"));
    }
    Ok(StructuredOutput {
        name: name.to_string(),
        schema,
    })
}

fn chat_message_content(value: &Value) -> Result<String, RuntimeError> {
    if let Some(content) = value.as_str() {
        return Ok(content.to_string());
    }
    let parts = value
        .as_array()
        .ok_or_else(|| provider_error("chat message content must be text"))?;
    let mut content = Vec::with_capacity(parts.len());
    for part in parts {
        let part = part
            .as_object()
            .ok_or_else(|| provider_error("chat message content part is invalid"))?;
        match part.get("type").and_then(Value::as_str) {
            Some("text") => content.push(
                part.get("text")
                    .and_then(Value::as_str)
                    .ok_or_else(|| provider_error("chat text content is invalid"))?,
            ),
            Some("image_url") => {
                return Err(provider_error(
                    "local text model does not accept image message content",
                ));
            }
            _ => return Err(provider_error("chat message content type is unsupported")),
        }
    }
    Ok(content.join("\n"))
}

fn agent_instructions(metadata: &Value) -> Option<String> {
    metadata
        .get("instructions")
        .or_else(|| metadata.get("systemPrompt"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn provider_error(message: &str) -> RuntimeError {
    RuntimeError::provider("llama", message)
}

fn structured_json_is_complete(output: &str) -> bool {
    serde_json::from_str::<Value>(output.trim()).is_ok()
}

fn non_empty(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_portable_agent_instructions() {
        assert_eq!(
            agent_instructions(&json!({ "instructions": "Stay in character." })),
            Some("Stay in character.".to_string())
        );
    }

    #[test]
    fn ignores_non_string_agent_instructions() {
        assert_eq!(agent_instructions(&json!({ "instructions": 42 })), None);
    }

    #[test]
    fn preserves_normal_streamed_output() {
        let mut output = VisibleOutput::default();
        assert_eq!(output.push("Hello"), Some("Hello".to_string()));
        assert_eq!(output.push(" world"), Some(" world".to_string()));
        assert_eq!(output.finish(), None);
    }

    #[test]
    fn hides_reasoning_split_across_streamed_pieces() {
        let mut output = VisibleOutput::default();
        assert_eq!(output.push("\n<th"), None);
        assert_eq!(output.push("ink>private"), None);
        assert_eq!(output.push(" reasoning</thi"), None);
        assert_eq!(
            output.push("nk>\n\nVisible answer"),
            Some("Visible answer".to_string())
        );
        assert_eq!(output.push("."), Some(".".to_string()));
        assert_eq!(output.finish(), None);
    }

    #[test]
    fn flushes_an_incomplete_opening_tag_as_normal_output() {
        let mut output = VisibleOutput::default();
        assert_eq!(output.push("<thi"), None);
        assert_eq!(output.finish(), Some("<thi".to_string()));
    }

    #[test]
    fn accepts_the_embedded_json_schema_shape() {
        let format = parse_structured_output(&json!({
            "type": "jsonSchema",
            "name": "stardojo_action",
            "schema": {
                "type": "object",
                "properties": { "command": { "type": "string" } },
                "required": ["command"]
            }
        }))
        .unwrap();
        assert_eq!(format.name, "stardojo_action");
        assert_eq!(format.schema["type"], "object");
    }

    #[test]
    fn accepts_the_openai_json_schema_shape() {
        let format = parse_structured_output(&json!({
            "type": "json_schema",
            "json_schema": {
                "name": "stardojo_action",
                "strict": true,
                "schema": { "type": "object" }
            }
        }))
        .unwrap();
        assert_eq!(format.name, "stardojo_action");
    }

    #[test]
    fn deserializes_openai_request_aliases() {
        let request = serde_json::from_value::<ChatRequest>(json!({
            "messages": [{ "role": "user", "content": "Choose" }],
            "max_tokens": 64,
            "top_p": 0.8,
            "response_format": {
                "type": "json_schema",
                "json_schema": {
                    "name": "action",
                    "strict": true,
                    "schema": { "type": "object" }
                }
            }
        }))
        .unwrap();
        assert_eq!(request.max_tokens, Some(64));
        assert_eq!(request.top_p, Some(0.8));
        assert_eq!(
            request.response_format.unwrap()["type"],
            Value::String("json_schema".to_string())
        );
    }

    #[test]
    fn rejects_non_strict_openai_json_schema() {
        let error = parse_structured_output(&json!({
            "type": "json_schema",
            "json_schema": {
                "name": "stardojo_action",
                "strict": false,
                "schema": { "type": "object" }
            }
        }))
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("structured response strict must be true"));
    }

    #[test]
    fn stops_sampling_as_soon_as_a_structured_json_value_is_complete() {
        assert!(!structured_json_is_complete(r#"{"ok": tru"#));
        assert!(structured_json_is_complete(r#"{"ok": true}"#));
        assert!(!structured_json_is_complete(r#"{"ok": true} trailing"#));
    }

    #[test]
    fn joins_openai_text_content_parts() {
        let content = chat_message_content(&json!([
            { "type": "text", "text": "Current task" },
            { "type": "text", "text": "Clear five stones" }
        ]))
        .unwrap();
        assert_eq!(content, "Current task\nClear five stones");
    }

    #[test]
    fn rejects_image_content_for_a_text_only_model() {
        let error = chat_message_content(&json!([{
            "type": "image_url",
            "image_url": { "url": "data:image/png;base64,abc" }
        }]))
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("local text model does not accept image message content"));
    }

    #[test]
    fn provider_config_resolves_a_relative_model_path_from_the_registry() {
        let config = LlamaProviderConfig::from_provider_config(
            &json!({
                "modelPath": "models/qwen.gguf",
                "contextSize": 2048,
                "gpuLayers": 0,
                "defaultMaxTokens": 128,
                "maxConcurrency": 1
            }),
            Path::new("/opt/vifu"),
        )
        .unwrap();

        assert_eq!(config.model_path, Path::new("/opt/vifu/models/qwen.gguf"));
    }

    #[test]
    fn provider_config_rejects_zero_concurrency() {
        let error = LlamaProviderConfig::from_provider_config(
            &json!({
                "modelPath": "/models/qwen.gguf",
                "maxConcurrency": 0
            }),
            Path::new("/opt/vifu"),
        )
        .unwrap_err();

        assert!(error.to_string().contains("maxConcurrency"));
    }
}
