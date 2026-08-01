//! Local llama.cpp provider for the embedded Vifu runtime.

use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, LlamaChatMessage, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;
use serde::Deserialize;
use serde_json::{json, Value};
use vifu_runtime::{
    AgentProvider, CancellationToken, InvocationData, ProviderEventSink, ProviderFuture,
    ProviderRequest, ProviderResponse, RuntimeError,
};

const DEFAULT_CONTEXT_SIZE: u32 = 4_096;
const DEFAULT_MAX_TOKENS: u32 = 256;
const MAX_GENERATED_TOKENS: u32 = 2_048;
const DEFAULT_TEMPERATURE: f32 = 0.7;
const DEFAULT_TOP_P: f32 = 0.9;
const THINK_OPEN_TAG: &str = "<think>";
const THINK_CLOSE_TAG: &str = "</think>";

static LLAMA_BACKEND: OnceLock<Result<Arc<LlamaBackend>, String>> = OnceLock::new();

#[derive(Clone, Debug)]
pub struct LlamaProviderConfig {
    pub model_path: PathBuf,
    pub context_size: u32,
    pub gpu_layers: u32,
    pub default_max_tokens: u32,
}

impl LlamaProviderConfig {
    pub fn new(model_path: impl Into<PathBuf>) -> Self {
        Self {
            model_path: model_path.into(),
            context_size: DEFAULT_CONTEXT_SIZE,
            gpu_layers: u32::MAX,
            default_max_tokens: DEFAULT_MAX_TOKENS,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LlamaProviderError {
    #[error("model file does not exist")]
    ModelNotFound,
    #[error("context size must be greater than zero")]
    InvalidContextSize,
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
}

impl LlamaProvider {
    pub fn load(config: LlamaProviderConfig) -> Result<Self, LlamaProviderError> {
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
            default_max_tokens: config.default_max_tokens.clamp(1, MAX_GENERATED_TOKENS),
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
        let context_size = self.context_size;
        let default_max_tokens = self.default_max_tokens;
        Box::pin(async move {
            let task = tokio::task::spawn_blocking(move || {
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
    #[serde(default)]
    max_tokens: Option<u32>,
    #[serde(default)]
    temperature: Option<f32>,
    #[serde(default)]
    top_p: Option<f32>,
    #[serde(default)]
    seed: Option<u32>,
}

#[derive(Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
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
        messages.push(
            LlamaChatMessage::new(message.role, message.content)
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
    let mut sampler = LlamaSampler::chain_simple([
        LlamaSampler::top_k(40),
        LlamaSampler::top_p(top_p, 1),
        LlamaSampler::temp(temperature),
        LlamaSampler::dist(input.seed.unwrap_or(0)),
    ]);
    let mut decoder = encoding_rs::UTF_8.new_decoder();
    let mut output = String::new();
    let mut visible_output = VisibleOutput::default();
    let mut generated_tokens = 0_u32;
    let mut position = batch.n_tokens();
    while generated_tokens < max_tokens {
        if cancellation.is_cancelled() {
            return Err(RuntimeError::Cancelled);
        }
        let token = sampler.sample(&context, batch.n_tokens() - 1);
        sampler.accept(token);
        if model.is_eog_token(token) {
            break;
        }
        let piece = model
            .token_to_piece(token, &mut decoder, true, None)
            .map_err(|_error| provider_error("model output could not be decoded"))?;
        if let Some(piece) = visible_output.push(&piece) {
            output.push_str(&piece);
            events.output_delta(InvocationData::Json(Value::String(piece)));
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

    Ok(ProviderResponse {
        data: InvocationData::Json(json!({
            "text": output,
            "message": {
                "role": "assistant",
                "content": output,
            },
        })),
        metadata: json!({
            "inputTokens": position - i32::try_from(generated_tokens).unwrap_or(i32::MAX),
            "outputTokens": generated_tokens,
        }),
        state: None,
    })
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
}
