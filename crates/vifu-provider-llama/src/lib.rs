//! Local llama.cpp provider for the embedded Vifu runtime.

use std::fmt;
use std::num::NonZeroU32;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use llama_cpp_2::context::params::{LlamaContextParams, LlamaPoolingType};
use llama_cpp_2::context::LlamaContext;
use llama_cpp_2::json_schema_to_grammar;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, LlamaChatMessage, LlamaModel};
use llama_cpp_2::mtmd::{
    mtmd_default_marker, MtmdBitmap, MtmdContext, MtmdContextParams, MtmdInputText,
};
use llama_cpp_2::sampling::LlamaSampler;
use llama_cpp_2::token::LlamaToken;
use llama_cpp_2::{send_logs_to_tracing, LogOptions};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use tokio::sync::Semaphore;
use vifu_runtime::{
    AgentProvider, CancellationToken, InvocationData, ProviderEventSink, ProviderFuture,
    ProviderRequest, ProviderResponse, ProviderStage, RuntimeError,
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
const MAX_EMBEDDING_INPUTS: usize = 256;
const DEFAULT_MAX_IMAGES: usize = 8;
const MAX_IMAGES: usize = 16;
const DEFAULT_MAX_MEDIA_BYTES: usize = 16 * 1024 * 1024;
const MAX_MEDIA_BYTES: usize = 32 * 1024 * 1024;
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

#[derive(Clone)]
pub struct LlamaMultimodalConfig {
    pub mmproj_path: PathBuf,
    pub use_gpu: bool,
    pub image_min_tokens: i32,
    pub image_max_tokens: i32,
    pub max_images: usize,
    pub max_media_bytes: usize,
}

impl LlamaMultimodalConfig {
    pub fn new(mmproj_path: impl Into<PathBuf>) -> Self {
        Self {
            mmproj_path: mmproj_path.into(),
            use_gpu: true,
            image_min_tokens: -1,
            image_max_tokens: -1,
            max_images: DEFAULT_MAX_IMAGES,
            max_media_bytes: DEFAULT_MAX_MEDIA_BYTES,
        }
    }

    fn validate(&self) -> Result<(), LlamaProviderError> {
        if self.mmproj_path.as_os_str().is_empty() {
            return Err(LlamaProviderError::InvalidConfig(
                "mmprojPath must not be empty".to_string(),
            ));
        }
        if self.image_min_tokens == 0 || self.image_min_tokens < -1 {
            return Err(LlamaProviderError::InvalidConfig(
                "imageMinTokens must be -1 or greater than zero".to_string(),
            ));
        }
        if self.image_max_tokens == 0 || self.image_max_tokens < -1 {
            return Err(LlamaProviderError::InvalidConfig(
                "imageMaxTokens must be -1 or greater than zero".to_string(),
            ));
        }
        if self.image_min_tokens > 0
            && self.image_max_tokens > 0
            && self.image_min_tokens > self.image_max_tokens
        {
            return Err(LlamaProviderError::InvalidConfig(
                "imageMinTokens must not exceed imageMaxTokens".to_string(),
            ));
        }
        if !(1..=MAX_IMAGES).contains(&self.max_images) {
            return Err(LlamaProviderError::InvalidConfig(format!(
                "maxImages must be between 1 and {MAX_IMAGES}"
            )));
        }
        if !(1..=MAX_MEDIA_BYTES).contains(&self.max_media_bytes) {
            return Err(LlamaProviderError::InvalidConfig(format!(
                "maxMediaBytes must be between 1 and {MAX_MEDIA_BYTES}"
            )));
        }
        Ok(())
    }
}

impl fmt::Debug for LlamaMultimodalConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LlamaMultimodalConfig")
            .field("mmproj_path", &"[REDACTED]")
            .field("use_gpu", &self.use_gpu)
            .field("image_min_tokens", &self.image_min_tokens)
            .field("image_max_tokens", &self.image_max_tokens)
            .field("max_images", &self.max_images)
            .field("max_media_bytes", &self.max_media_bytes)
            .finish()
    }
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

#[derive(Clone, Deserialize)]
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
    #[serde(default)]
    mmproj_path: Option<PathBuf>,
    #[serde(default)]
    mmproj_use_gpu: Option<bool>,
    #[serde(default)]
    image_min_tokens: Option<i32>,
    #[serde(default)]
    image_max_tokens: Option<i32>,
    #[serde(default)]
    max_images: Option<usize>,
    #[serde(default)]
    max_media_bytes: Option<usize>,
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
        provider_configs(value, base_dir).map(|(config, _multimodal)| config)
    }

    fn from_file_config(
        file: &LlamaProviderFileConfig,
        base_dir: &Path,
    ) -> Result<Self, LlamaProviderError> {
        if file.model_path.as_os_str().is_empty() {
            return Err(LlamaProviderError::InvalidConfig(
                "modelPath must not be empty".to_string(),
            ));
        }
        let model_path = if file.model_path.is_absolute() {
            file.model_path.clone()
        } else {
            base_dir.join(&file.model_path)
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
    #[error("multimodal projector file does not exist")]
    ProjectorNotFound,
    #[error("multimodal projector could not be loaded: {0}")]
    Multimodal(String),
}

struct MultimodalRuntime {
    context: Mutex<MtmdContext>,
    max_images: usize,
    max_media_bytes: usize,
}

#[derive(Clone, Copy)]
struct ImageInputLimits {
    max_images: usize,
    max_media_bytes: usize,
}

impl MultimodalRuntime {
    fn input_limits(&self) -> ImageInputLimits {
        ImageInputLimits {
            max_images: self.max_images,
            max_media_bytes: self.max_media_bytes,
        }
    }
}

pub struct LlamaProvider {
    backend: Arc<LlamaBackend>,
    model: Arc<LlamaModel>,
    context_size: NonZeroU32,
    default_max_tokens: u32,
    concurrency: Arc<Semaphore>,
    multimodal: Option<Arc<MultimodalRuntime>>,
}

impl LlamaProvider {
    pub fn load(config: LlamaProviderConfig) -> Result<Self, LlamaProviderError> {
        Self::load_with_multimodal(config, None)
    }

    pub fn load_multimodal(
        config: LlamaProviderConfig,
        multimodal: LlamaMultimodalConfig,
    ) -> Result<Self, LlamaProviderError> {
        Self::load_with_multimodal(config, Some(multimodal))
    }

    pub fn load_from_provider_config(
        value: &Value,
        base_dir: &Path,
    ) -> Result<Self, LlamaProviderError> {
        let (config, multimodal) = provider_configs(value, base_dir)?;
        Self::load_with_multimodal(config, multimodal)
    }

    #[must_use]
    pub fn supports_vision(&self) -> bool {
        self.multimodal.is_some()
    }

    fn load_with_multimodal(
        config: LlamaProviderConfig,
        multimodal: Option<LlamaMultimodalConfig>,
    ) -> Result<Self, LlamaProviderError> {
        config.validate()?;
        if !Path::new(&config.model_path).is_file() {
            return Err(LlamaProviderError::ModelNotFound);
        }
        if let Some(multimodal) = multimodal.as_ref() {
            multimodal.validate()?;
            if !multimodal.mmproj_path.is_file() {
                return Err(LlamaProviderError::ProjectorNotFound);
            }
        }
        let context_size =
            NonZeroU32::new(config.context_size).ok_or(LlamaProviderError::InvalidContextSize)?;
        let backend = match LLAMA_BACKEND.get_or_init(|| {
            send_logs_to_tracing(LogOptions::default());
            LlamaBackend::init()
                .map(Arc::new)
                .map_err(|error| error.to_string())
        }) {
            Ok(backend) => Arc::clone(backend),
            Err(message) => return Err(LlamaProviderError::Backend(message.clone())),
        };
        let model_params = LlamaModelParams::default().with_n_gpu_layers(config.gpu_layers);
        let model = Arc::new(
            LlamaModel::load_from_file(&backend, &config.model_path, &model_params)
                .map_err(|error| LlamaProviderError::Model(error.to_string()))?,
        );
        let multimodal = multimodal
            .map(|multimodal| {
                let mmproj_path = multimodal.mmproj_path.to_str().ok_or_else(|| {
                    LlamaProviderError::InvalidConfig("mmprojPath must be valid UTF-8".to_string())
                })?;
                let params = MtmdContextParams {
                    use_gpu: multimodal.use_gpu,
                    print_timings: false,
                    image_min_tokens: multimodal.image_min_tokens,
                    image_max_tokens: multimodal.image_max_tokens,
                    ..MtmdContextParams::default()
                };
                let context = MtmdContext::init_from_file(mmproj_path, &model, &params)
                    .map_err(|error| LlamaProviderError::Multimodal(error.to_string()))?;
                if !context.support_vision() {
                    return Err(LlamaProviderError::Multimodal(
                        "projector does not support image input".to_string(),
                    ));
                }
                Ok(Arc::new(MultimodalRuntime {
                    context: Mutex::new(context),
                    max_images: multimodal.max_images,
                    max_media_bytes: multimodal.max_media_bytes,
                }))
            })
            .transpose()?;
        Ok(Self {
            backend,
            model,
            context_size,
            default_max_tokens: config.default_max_tokens,
            concurrency: Arc::new(Semaphore::new(config.max_concurrency)),
            multimodal,
        })
    }
}

fn provider_configs(
    value: &Value,
    base_dir: &Path,
) -> Result<(LlamaProviderConfig, Option<LlamaMultimodalConfig>), LlamaProviderError> {
    let file = serde_json::from_value::<LlamaProviderFileConfig>(value.clone())
        .map_err(|error| LlamaProviderError::InvalidConfig(error.to_string()))?;
    let config = LlamaProviderConfig::from_file_config(&file, base_dir)?;
    let use_mmproj_gpu = file.mmproj_use_gpu.unwrap_or(config.gpu_layers > 0);
    let multimodal = file.mmproj_path.map(|mmproj_path| {
        let mut multimodal = LlamaMultimodalConfig::new(if mmproj_path.is_absolute() {
            mmproj_path
        } else {
            base_dir.join(mmproj_path)
        });
        multimodal.use_gpu = use_mmproj_gpu;
        multimodal.image_min_tokens = file.image_min_tokens.unwrap_or(-1);
        multimodal.image_max_tokens = file.image_max_tokens.unwrap_or(-1);
        multimodal.max_images = file.max_images.unwrap_or(DEFAULT_MAX_IMAGES);
        multimodal.max_media_bytes = file.max_media_bytes.unwrap_or(DEFAULT_MAX_MEDIA_BYTES);
        multimodal
    });
    if let Some(multimodal) = multimodal.as_ref() {
        multimodal.validate()?;
    }
    Ok((config, multimodal))
}

impl AgentProvider for LlamaProvider {
    fn supports(&self, capability: &str) -> bool {
        matches!(capability, "chat" | "embedding")
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
        let multimodal = self.multimodal.clone();
        Box::pin(async move {
            let queue_started = Instant::now();
            events.stage_started(ProviderStage::Queue, Value::Null);
            let permit = match concurrency.try_acquire_owned() {
                Ok(permit) => {
                    events.stage_completed(
                        ProviderStage::Queue,
                        elapsed_ms(queue_started),
                        Value::Null,
                    );
                    permit
                }
                Err(_error) => {
                    let error = provider_error("local model concurrency limit reached");
                    events.stage_failed(
                        ProviderStage::Queue,
                        elapsed_ms(queue_started),
                        error.to_string(),
                        Value::Null,
                    );
                    return Err(error);
                }
            };
            let task = tokio::task::spawn_blocking(move || {
                let _permit = permit;
                match request.capability.as_str() {
                    "chat" => generate_chat(
                        ChatRuntime {
                            backend: &backend,
                            model: &model,
                            context_size,
                            default_max_tokens,
                            multimodal: multimodal.as_deref(),
                        },
                        request,
                        &cancellation,
                        &events,
                    ),
                    "embedding" => generate_embeddings(
                        &backend,
                        &model,
                        context_size,
                        request,
                        &cancellation,
                        &events,
                    ),
                    capability => Err(provider_error(&format!(
                        "capability {capability} is not supported"
                    ))),
                }
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

#[derive(Deserialize)]
struct EmbeddingRequest {
    input: EmbeddingInput,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    dimensions: Option<usize>,
    #[serde(default, alias = "encoding_format")]
    encoding_format: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum EmbeddingInput {
    Text(String),
    Texts(Vec<String>),
    Tokens(Vec<i32>),
    TokenBatches(Vec<Vec<i32>>),
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

struct ChatRuntime<'a> {
    backend: &'a LlamaBackend,
    model: &'a LlamaModel,
    context_size: NonZeroU32,
    default_max_tokens: u32,
    multimodal: Option<&'a MultimodalRuntime>,
}

fn generate_chat(
    runtime: ChatRuntime<'_>,
    request: ProviderRequest,
    cancellation: &CancellationToken,
    events: &ProviderEventSink,
) -> Result<ProviderResponse, RuntimeError> {
    let ChatRuntime {
        backend,
        model,
        context_size,
        default_max_tokens,
        multimodal,
    } = runtime;
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
    let mut image_buffers = Vec::new();
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
        let content = chat_message_content(
            &message.content,
            multimodal.map(MultimodalRuntime::input_limits),
            &mut image_buffers,
        )?;
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
    let image_count = image_buffers.len();

    let max_tokens = input
        .max_tokens
        .unwrap_or(default_max_tokens)
        .clamp(1, MAX_GENERATED_TOKENS);
    let mut context = model
        .new_context(
            backend,
            LlamaContextParams::default().with_n_ctx(Some(context_size)),
        )
        .map_err(|_error| provider_error("model context could not be created"))?;
    let (input_tokens, mut position) = if image_buffers.is_empty() {
        evaluate_text_prompt(
            model,
            &mut context,
            context_size,
            max_tokens,
            &prompt,
            events,
        )?
    } else {
        let multimodal = multimodal.ok_or_else(|| {
            provider_error("local text model does not accept image message content")
        })?;
        evaluate_multimodal_prompt(
            &mut context,
            context_size,
            max_tokens,
            multimodal,
            MultimodalPromptInput {
                text: &prompt,
                images: &image_buffers,
            },
            cancellation,
            events,
        )?
    };
    let mut batch = LlamaBatch::new(1, 1);

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
    let mut stopped_on_eog = false;
    let mut stopped_on_valid_json = false;
    let decode_started = Instant::now();
    let mut first_token_observed = false;
    let mut first_token_ms = None;
    events.stage_started(ProviderStage::FirstToken, Value::Null);
    events.stage_started(ProviderStage::Decode, Value::Null);
    let decode_result = (|| {
        while generated_tokens < max_tokens {
            if cancellation.is_cancelled() {
                return Err(RuntimeError::Cancelled);
            }
            let token = sampler.sample(&context, -1);
            if model.is_eog_token(token) {
                stopped_on_eog = true;
                break;
            }
            let piece = model
                .token_to_piece(token, &mut decoder, true, None)
                .map_err(|_error| provider_error("model output could not be decoded"))?;
            if !first_token_observed {
                first_token_observed = true;
                let completion_start_ms = elapsed_ms(decode_started);
                first_token_ms = Some(completion_start_ms);
                events.stage_completed(ProviderStage::FirstToken, completion_start_ms, Value::Null);
            }
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
        Ok::<_, RuntimeError>(())
    })();
    if let Err(error) = decode_result {
        if !first_token_observed {
            events.stage_failed(
                ProviderStage::FirstToken,
                elapsed_ms(decode_started),
                error.to_string(),
                Value::Null,
            );
        }
        events.stage_failed(
            ProviderStage::Decode,
            elapsed_ms(decode_started),
            error.to_string(),
            json!({ "outputTokens": generated_tokens }),
        );
        return Err(error);
    }
    if !first_token_observed {
        events.stage_completed(
            ProviderStage::FirstToken,
            elapsed_ms(decode_started),
            json!({ "empty": true }),
        );
    }
    events.stage_completed(
        ProviderStage::Decode,
        elapsed_ms(decode_started),
        json!({ "outputTokens": generated_tokens }),
    );

    let validate_started = Instant::now();
    events.stage_started(
        ProviderStage::Validate,
        json!({ "structured": structured_output.is_some() }),
    );
    let structured = structured_output
        .as_ref()
        .map(|_format| {
            serde_json::from_str::<Value>(output.trim()).map_err(|_error| {
                provider_error("structured response ended before producing valid JSON")
            })
        })
        .transpose();
    let structured = match structured {
        Ok(structured) => {
            events.stage_completed(
                ProviderStage::Validate,
                elapsed_ms(validate_started),
                json!({ "structured": structured_output.is_some() }),
            );
            structured
        }
        Err(error) => {
            events.stage_failed(
                ProviderStage::Validate,
                elapsed_ms(validate_started),
                error.to_string(),
                json!({ "structured": true }),
            );
            return Err(error);
        }
    };
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
            "imageCount": image_count,
            "finishReason": finish_reason,
            "completionStartMs": first_token_ms,
        }),
        state: None,
    })
}

fn evaluate_text_prompt(
    model: &LlamaModel,
    context: &mut LlamaContext<'_>,
    context_size: NonZeroU32,
    max_tokens: u32,
    prompt: &str,
    events: &ProviderEventSink,
) -> Result<(i32, i32), RuntimeError> {
    let tokenize_started = Instant::now();
    events.stage_started(ProviderStage::Tokenize, Value::Null);
    let tokens = match model
        .str_to_token(prompt, AddBos::Always)
        .map_err(|_error| provider_error("chat prompt could not be tokenized"))
    {
        Ok(tokens) => tokens,
        Err(error) => {
            events.stage_failed(
                ProviderStage::Tokenize,
                elapsed_ms(tokenize_started),
                error.to_string(),
                Value::Null,
            );
            return Err(error);
        }
    };
    if tokens.is_empty() {
        let error = provider_error("chat prompt produced no tokens");
        events.stage_failed(
            ProviderStage::Tokenize,
            elapsed_ms(tokenize_started),
            error.to_string(),
            Value::Null,
        );
        return Err(error);
    }
    events.stage_completed(
        ProviderStage::Tokenize,
        elapsed_ms(tokenize_started),
        json!({ "inputTokens": tokens.len() }),
    );
    let prefill_started = Instant::now();
    events.stage_started(
        ProviderStage::Prefill,
        json!({ "inputTokens": tokens.len() }),
    );
    let result = (|| {
        ensure_prompt_fits_context(tokens.len(), context_size, max_tokens)?;
        let prompt_ranges = prompt_chunk_ranges(tokens.len(), context.n_batch())?;
        let prompt_batch_size = prompt_ranges.first().map_or(1, |range| range.len()).max(1);
        let mut batch = LlamaBatch::new(prompt_batch_size, 1);
        for range in prompt_ranges {
            batch.clear();
            for (offset, token) in tokens[range.clone()].iter().copied().enumerate() {
                let absolute_position = range.start + offset;
                let position = i32::try_from(absolute_position)
                    .map_err(|_error| provider_error("chat prompt is too long"))?;
                batch
                    .add(token, position, &[0], absolute_position + 1 == tokens.len())
                    .map_err(|_error| provider_error("chat prompt could not be prepared"))?;
            }
            context
                .decode(&mut batch)
                .map_err(|_error| provider_error("chat prompt inference failed"))?;
        }
        let input_tokens = i32::try_from(tokens.len())
            .map_err(|_error| provider_error("chat prompt is too long"))?;
        Ok::<_, RuntimeError>((input_tokens, input_tokens))
    })();
    match result {
        Ok(result) => {
            events.stage_completed(
                ProviderStage::Prefill,
                elapsed_ms(prefill_started),
                json!({ "inputTokens": tokens.len() }),
            );
            Ok(result)
        }
        Err(error) => {
            events.stage_failed(
                ProviderStage::Prefill,
                elapsed_ms(prefill_started),
                error.to_string(),
                json!({ "inputTokens": tokens.len() }),
            );
            Err(error)
        }
    }
}

struct MultimodalPromptInput<'a> {
    text: &'a str,
    images: &'a [Vec<u8>],
}

fn evaluate_multimodal_prompt(
    context: &mut LlamaContext<'_>,
    context_size: NonZeroU32,
    max_tokens: u32,
    multimodal: &MultimodalRuntime,
    input: MultimodalPromptInput<'_>,
    cancellation: &CancellationToken,
    events: &ProviderEventSink,
) -> Result<(i32, i32), RuntimeError> {
    if cancellation.is_cancelled() {
        return Err(RuntimeError::Cancelled);
    }
    let tokenize_started = Instant::now();
    events.stage_started(
        ProviderStage::Tokenize,
        json!({ "imageCount": input.images.len() }),
    );
    let tokenization = (|| {
        let mtmd = multimodal
            .context
            .lock()
            .map_err(|_error| provider_error("multimodal context stopped"))?;
        let bitmaps = input
            .images
            .iter()
            .map(|image| {
                MtmdBitmap::from_buffer(&mtmd, image, false)
                    .map_err(|_error| provider_error("image content could not be decoded"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let bitmap_refs = bitmaps.iter().collect::<Vec<_>>();
        let chunks = mtmd
            .tokenize(
                MtmdInputText {
                    text: input.text.to_string(),
                    add_special: true,
                    parse_special: true,
                },
                &bitmap_refs,
            )
            .map_err(|_error| provider_error("multimodal prompt could not be tokenized"))?;
        Ok::<_, RuntimeError>((mtmd, chunks))
    })();
    let (mtmd, chunks) = match tokenization {
        Ok(tokenization) => {
            events.stage_completed(
                ProviderStage::Tokenize,
                elapsed_ms(tokenize_started),
                json!({
                    "imageCount": input.images.len(),
                    "inputTokens": tokenization.1.total_tokens(),
                }),
            );
            tokenization
        }
        Err(error) => {
            events.stage_failed(
                ProviderStage::Tokenize,
                elapsed_ms(tokenize_started),
                error.to_string(),
                json!({ "imageCount": input.images.len() }),
            );
            return Err(error);
        }
    };
    let prefill_started = Instant::now();
    events.stage_started(
        ProviderStage::Prefill,
        json!({ "inputTokens": chunks.total_tokens() }),
    );
    let prefill = (|| {
        ensure_prompt_fits_context(chunks.total_tokens(), context_size, max_tokens)?;
        let input_tokens = i32::try_from(chunks.total_tokens())
            .map_err(|_error| provider_error("multimodal prompt is too long"))?;
        let n_batch = i32::try_from(context.n_batch())
            .map_err(|_error| provider_error("model batch size is unsupported"))?;
        let next_position = chunks
            .eval_chunks(&mtmd, context, 0, 0, n_batch, true)
            .map_err(|_error| provider_error("multimodal prompt inference failed"))?;
        if cancellation.is_cancelled() {
            return Err(RuntimeError::Cancelled);
        }
        Ok::<_, RuntimeError>((input_tokens, next_position))
    })();
    match prefill {
        Ok(result) => {
            events.stage_completed(
                ProviderStage::Prefill,
                elapsed_ms(prefill_started),
                json!({ "inputTokens": chunks.total_tokens() }),
            );
            Ok(result)
        }
        Err(error) => {
            events.stage_failed(
                ProviderStage::Prefill,
                elapsed_ms(prefill_started),
                error.to_string(),
                json!({ "inputTokens": chunks.total_tokens() }),
            );
            Err(error)
        }
    }
}

fn ensure_prompt_fits_context(
    input_tokens: usize,
    context_size: NonZeroU32,
    max_tokens: u32,
) -> Result<(), RuntimeError> {
    let context_tokens = usize::try_from(context_size.get())
        .map_err(|_error| provider_error("context size is unsupported"))?;
    if input_tokens.saturating_add(max_tokens as usize) > context_tokens {
        return Err(provider_error(
            "chat prompt exceeds the configured context size",
        ));
    }
    Ok(())
}

fn generate_embeddings(
    backend: &LlamaBackend,
    model: &LlamaModel,
    context_size: NonZeroU32,
    request: ProviderRequest,
    cancellation: &CancellationToken,
    events: &ProviderEventSink,
) -> Result<ProviderResponse, RuntimeError> {
    let input = match request.data {
        InvocationData::Json(value) => serde_json::from_value::<EmbeddingRequest>(value)
            .map_err(|_error| provider_error("embedding input is invalid"))?,
        InvocationData::Binary(_) => {
            return Err(provider_error("embedding input must be JSON"));
        }
    };
    let encode_base64 = match input.encoding_format.as_deref() {
        None | Some("float") => false,
        Some("base64") => true,
        Some(_format) => {
            return Err(provider_error(
                "local embeddings support encoding_format float or base64",
            ));
        }
    };
    let requested_dimensions = input.dimensions;
    let tokenize_started = Instant::now();
    events.stage_started(ProviderStage::Tokenize, Value::Null);
    let sequences = match embedding_sequences(model, input.input) {
        Ok(sequences) => sequences,
        Err(error) => {
            events.stage_failed(
                ProviderStage::Tokenize,
                elapsed_ms(tokenize_started),
                error.to_string(),
                Value::Null,
            );
            return Err(error);
        }
    };
    if sequences.is_empty() {
        let error = provider_error("at least one embedding input is required");
        events.stage_failed(
            ProviderStage::Tokenize,
            elapsed_ms(tokenize_started),
            error.to_string(),
            Value::Null,
        );
        return Err(error);
    }
    if sequences.len() > MAX_EMBEDDING_INPUTS {
        let error = provider_error("too many embedding inputs");
        events.stage_failed(
            ProviderStage::Tokenize,
            elapsed_ms(tokenize_started),
            error.to_string(),
            json!({ "inputCount": sequences.len() }),
        );
        return Err(error);
    }
    let token_count = sequences
        .iter()
        .map(Vec::len)
        .fold(0_usize, usize::saturating_add);
    events.stage_completed(
        ProviderStage::Tokenize,
        elapsed_ms(tokenize_started),
        json!({ "inputCount": sequences.len(), "inputTokens": token_count }),
    );
    let prefill_started = Instant::now();
    events.stage_started(
        ProviderStage::Prefill,
        json!({ "inputCount": sequences.len(), "inputTokens": token_count }),
    );
    let inference = (|| {
        let context_tokens = usize::try_from(context_size.get())
            .map_err(|_error| provider_error("context size is unsupported"))?;
        if sequences.iter().any(|tokens| tokens.len() > context_tokens) {
            return Err(provider_error(
                "embedding input exceeds the configured context size",
            ));
        }
        let mut prompt_tokens = 0_usize;
        let mut data = Vec::with_capacity(sequences.len());
        let mut dimensions = 0_usize;
        for (index, tokens) in sequences.iter().enumerate() {
            if cancellation.is_cancelled() {
                return Err(RuntimeError::Cancelled);
            }
            prompt_tokens = prompt_tokens.saturating_add(tokens.len());
            let mut batch = LlamaBatch::new(tokens.len().max(1), 1);
            batch
                .add_sequence(tokens, 0, false)
                .map_err(|_error| provider_error("embedding input could not be prepared"))?;
            let params = || {
                LlamaContextParams::default()
                    .with_n_ctx(Some(context_size))
                    .with_n_batch(context_size.get())
                    .with_embeddings(true)
                    .with_pooling_type(LlamaPoolingType::Mean)
            };
            let mut context = model
                .new_context(backend, params())
                .map_err(|_error| provider_error("embedding context could not be created"))?;
            if model_uses_encoder(model) {
                context
                    .encode(&mut batch)
                    .map_err(|_error| provider_error("embedding inference failed"))?;
            } else {
                context
                    .decode(&mut batch)
                    .map_err(|_error| provider_error("embedding inference failed"))?;
            }
            let embedding = normalize_embedding(
                context
                    .embeddings_seq_ith(0)
                    .map_err(|_error| provider_error("model did not produce an embedding"))?,
            )?;
            if let Some(requested_dimensions) = requested_dimensions {
                if requested_dimensions != embedding.len() {
                    return Err(provider_error(
                        "requested embedding dimensions do not match the local model",
                    ));
                }
            }
            dimensions = dimensions.max(embedding.len());
            let embedding = if encode_base64 {
                Value::String(encode_embedding_base64(&embedding))
            } else {
                json!(embedding)
            };
            data.push(json!({
                "object": "embedding",
                "index": index,
                "embedding": embedding,
            }));
        }

        Ok::<_, RuntimeError>((prompt_tokens, data, dimensions))
    })();
    let (prompt_tokens, data, dimensions) = match inference {
        Ok(result) => {
            events.stage_completed(
                ProviderStage::Prefill,
                elapsed_ms(prefill_started),
                json!({ "inputCount": sequences.len(), "inputTokens": result.0 }),
            );
            result
        }
        Err(error) => {
            events.stage_failed(
                ProviderStage::Prefill,
                elapsed_ms(prefill_started),
                error.to_string(),
                json!({ "inputCount": sequences.len() }),
            );
            return Err(error);
        }
    };
    let validate_started = Instant::now();
    events.stage_started(ProviderStage::Validate, Value::Null);
    let model_name = input.model.unwrap_or_else(|| request.agent.id.clone());
    let response = ProviderResponse {
        data: InvocationData::Json(json!({
            "object": "list",
            "data": data,
            "model": model_name,
            "usage": {
                "prompt_tokens": prompt_tokens,
                "total_tokens": prompt_tokens,
            },
        })),
        metadata: json!({
            "inputTokens": prompt_tokens,
            "inputCount": sequences.len(),
            "embeddingDimensions": dimensions,
        }),
        state: None,
    };
    events.stage_completed(
        ProviderStage::Validate,
        elapsed_ms(validate_started),
        json!({ "dimensions": dimensions }),
    );
    Ok(response)
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn model_uses_encoder(model: &LlamaModel) -> bool {
    model
        .meta_val_str("general.architecture")
        .is_ok_and(|architecture| architecture_uses_encoder(&architecture))
}

fn architecture_uses_encoder(architecture: &str) -> bool {
    matches!(architecture, "t5" | "t5encoder")
}

fn embedding_sequences(
    model: &LlamaModel,
    input: EmbeddingInput,
) -> Result<Vec<Vec<LlamaToken>>, RuntimeError> {
    match input {
        EmbeddingInput::Text(text) => Ok(vec![tokenize_embedding_text(model, &text)?]),
        EmbeddingInput::Texts(texts) => texts
            .iter()
            .map(|text| tokenize_embedding_text(model, text))
            .collect(),
        EmbeddingInput::Tokens(tokens) => Ok(vec![embedding_tokens(model, tokens)?]),
        EmbeddingInput::TokenBatches(batches) => batches
            .into_iter()
            .map(|tokens| embedding_tokens(model, tokens))
            .collect(),
    }
}

fn tokenize_embedding_text(
    model: &LlamaModel,
    text: &str,
) -> Result<Vec<LlamaToken>, RuntimeError> {
    let tokens = model
        .str_to_token(text, AddBos::Always)
        .map_err(|_error| provider_error("embedding input could not be tokenized"))?;
    if tokens.is_empty() {
        return Err(provider_error("embedding input produced no tokens"));
    }
    Ok(tokens)
}

fn embedding_tokens(model: &LlamaModel, tokens: Vec<i32>) -> Result<Vec<LlamaToken>, RuntimeError> {
    if tokens.is_empty() {
        return Err(provider_error("embedding token input is invalid"));
    }
    tokenize_embedding_text(model, &openai_token_fallback_text(&tokens))
}

fn openai_token_fallback_text(tokens: &[i32]) -> String {
    let mut text = String::from("openai token ids:");
    for token in tokens {
        text.push(' ');
        text.push_str(&token.to_string());
    }
    text
}

fn normalize_embedding(embedding: &[f32]) -> Result<Vec<f32>, RuntimeError> {
    let norm = embedding
        .iter()
        .map(|value| value * value)
        .sum::<f32>()
        .sqrt();
    if !norm.is_finite() || norm <= f32::EPSILON {
        return Err(provider_error("model produced an invalid embedding"));
    }
    embedding
        .iter()
        .map(|value| value / norm)
        .map(|value| {
            value
                .is_finite()
                .then_some(value)
                .ok_or_else(|| provider_error("model produced an invalid embedding"))
        })
        .collect()
}

fn encode_embedding_base64(embedding: &[f32]) -> String {
    let mut bytes = Vec::with_capacity(std::mem::size_of_val(embedding));
    for value in embedding {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    encode_standard_base64(&bytes)
}

fn encode_standard_base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        let buffer = ((first as u32) << 16) | ((second as u32) << 8) | third as u32;

        encoded.push(ALPHABET[((buffer >> 18) & 0x3f) as usize] as char);
        encoded.push(ALPHABET[((buffer >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            encoded.push(ALPHABET[((buffer >> 6) & 0x3f) as usize] as char);
        } else {
            encoded.push('=');
        }
        if chunk.len() > 2 {
            encoded.push(ALPHABET[(buffer & 0x3f) as usize] as char);
        } else {
            encoded.push('=');
        }
    }
    encoded
}

fn prompt_chunk_ranges(
    token_count: usize,
    batch_size: u32,
) -> Result<Vec<Range<usize>>, RuntimeError> {
    let batch_size = usize::try_from(batch_size)
        .map_err(|_error| provider_error("model batch size is unsupported"))?;
    if batch_size == 0 {
        return Err(provider_error("model batch size must be greater than zero"));
    }
    Ok((0..token_count)
        .step_by(batch_size)
        .map(|start| start..start.saturating_add(batch_size).min(token_count))
        .collect())
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

fn chat_message_content(
    value: &Value,
    image_limits: Option<ImageInputLimits>,
    image_buffers: &mut Vec<Vec<u8>>,
) -> Result<String, RuntimeError> {
    if let Some(content) = value.as_str() {
        validate_reserved_media_marker(content, image_limits)?;
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
            Some("text") => {
                let text = part
                    .get("text")
                    .and_then(Value::as_str)
                    .ok_or_else(|| provider_error("chat text content is invalid"))?;
                validate_reserved_media_marker(text, image_limits)?;
                content.push(text);
            }
            Some("image_url") => {
                let image_limits = image_limits.ok_or_else(|| {
                    provider_error("local text model does not accept image message content")
                })?;
                if image_buffers.len() >= image_limits.max_images {
                    return Err(provider_error("chat request contains too many images"));
                }
                let image_url = part
                    .get("image_url")
                    .and_then(Value::as_object)
                    .and_then(|image| image.get("url"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| provider_error("chat image URL is invalid"))?;
                let remaining_bytes = image_limits
                    .max_media_bytes
                    .saturating_sub(image_buffers.iter().map(Vec::len).sum::<usize>());
                let image = decode_image_data_url(image_url, remaining_bytes)?;
                image_buffers.push(image);
                content.push(mtmd_default_marker());
            }
            _ => return Err(provider_error("chat message content type is unsupported")),
        }
    }
    Ok(content.join("\n"))
}

fn validate_reserved_media_marker(
    text: &str,
    image_limits: Option<ImageInputLimits>,
) -> Result<(), RuntimeError> {
    if image_limits.is_some() && text.contains(mtmd_default_marker()) {
        return Err(provider_error(
            "chat text contains the reserved multimodal marker",
        ));
    }
    Ok(())
}

fn decode_image_data_url(url: &str, max_bytes: usize) -> Result<Vec<u8>, RuntimeError> {
    let data = url
        .strip_prefix("data:")
        .ok_or_else(|| provider_error("local vision model accepts only base64 data image URLs"))?;
    let (metadata, encoded) = data
        .split_once(',')
        .ok_or_else(|| provider_error("chat image data URL is invalid"))?;
    let mut metadata_parts = metadata.split(';');
    let media_type = metadata_parts
        .next()
        .filter(|media_type| media_type.starts_with("image/"))
        .ok_or_else(|| provider_error("chat image data URL must contain an image"))?;
    if media_type.len() > 64 || !metadata_parts.any(|part| part == "base64") {
        return Err(provider_error("chat image data URL is invalid"));
    }
    decode_standard_base64(encoded, max_bytes)
}

fn decode_standard_base64(encoded: &str, max_bytes: usize) -> Result<Vec<u8>, RuntimeError> {
    if encoded.is_empty() || !encoded.len().is_multiple_of(4) {
        return Err(provider_error("chat image base64 data is invalid"));
    }
    let max_encoded_bytes = max_bytes.saturating_add(2) / 3 * 4;
    if encoded.len() > max_encoded_bytes.saturating_add(4) {
        return Err(provider_error("chat image data exceeds maxMediaBytes"));
    }
    let chunks = encoded.as_bytes().chunks_exact(4);
    if !chunks.remainder().is_empty() {
        return Err(provider_error("chat image base64 data is invalid"));
    }
    let chunk_count = encoded.len() / 4;
    let mut decoded = Vec::with_capacity(encoded.len() / 4 * 3);
    for (index, chunk) in chunks.enumerate() {
        let last = index + 1 == chunk_count;
        let first = base64_value(chunk[0])?;
        let second = base64_value(chunk[1])?;
        let third_padding = chunk[2] == b'=';
        let fourth_padding = chunk[3] == b'=';
        if third_padding {
            if !last || !fourth_padding {
                return Err(provider_error("chat image base64 data is invalid"));
            }
            decoded.push((first << 2) | (second >> 4));
            continue;
        }
        let third = base64_value(chunk[2])?;
        decoded.push((first << 2) | (second >> 4));
        decoded.push(((second & 0x0f) << 4) | (third >> 2));
        if fourth_padding {
            if !last {
                return Err(provider_error("chat image base64 data is invalid"));
            }
            continue;
        }
        let fourth = base64_value(chunk[3])?;
        decoded.push(((third & 0x03) << 6) | fourth);
    }
    if decoded.len() > max_bytes {
        return Err(provider_error("chat image data exceeds maxMediaBytes"));
    }
    Ok(decoded)
}

fn base64_value(byte: u8) -> Result<u8, RuntimeError> {
    match byte {
        b'A'..=b'Z' => Ok(byte - b'A'),
        b'a'..=b'z' => Ok(byte - b'a' + 26),
        b'0'..=b'9' => Ok(byte - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        _ => Err(provider_error("chat image base64 data is invalid")),
    }
}

fn agent_instructions(metadata: &Value) -> Option<String> {
    metadata
        .get("instructions")
        .or_else(|| metadata.get("systemPrompt"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn provider_error(message: &str) -> RuntimeError {
    if std::env::var_os("VIFU_LLAMA_DIAGNOSTICS").is_some() {
        eprintln!("local llama provider request rejected: {message}");
    }
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
    fn accepts_openai_embedding_input_shapes() {
        let text = serde_json::from_value::<EmbeddingRequest>(json!({
            "model": "farm-embedding",
            "input": "parsnip",
        }))
        .unwrap();
        let batch = serde_json::from_value::<EmbeddingRequest>(json!({
            "model": "farm-embedding",
            "input": ["parsnip", "watering can"],
            "encoding_format": "float",
        }))
        .unwrap();

        assert!(matches!(text.input, EmbeddingInput::Text(_)));
        assert!(matches!(batch.input, EmbeddingInput::Texts(_)));
    }

    #[test]
    fn converts_openai_token_ids_to_stable_text_for_local_fallback() {
        assert_eq!(
            openai_token_fallback_text(&[646, 321, 12]),
            "openai token ids: 646 321 12"
        );
    }

    #[test]
    fn encodes_embedding_base64_as_float32_bytes() {
        assert_eq!(encode_embedding_base64(&[1.0]), "AACAPw==");
    }

    #[test]
    fn normalizes_embeddings_for_cosine_similarity() {
        let embedding = normalize_embedding(&[3.0, 4.0]).unwrap();

        assert!((embedding[0] - 0.6).abs() < f32::EPSILON);
        assert!((embedding[1] - 0.8).abs() < f32::EPSILON);
    }

    #[test]
    fn selects_embedding_inference_for_the_model_architecture() {
        assert!(architecture_uses_encoder("t5"));
        assert!(architecture_uses_encoder("t5encoder"));
        assert!(!architecture_uses_encoder("qwen3"));
        assert!(!architecture_uses_encoder("llama"));
    }

    #[test]
    fn splits_long_prompts_into_context_decode_batches() {
        assert_eq!(
            prompt_chunk_ranges(4_097, 2_048).unwrap(),
            vec![0..2_048, 2_048..4_096, 4_096..4_097]
        );
    }

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
        let mut images = Vec::new();
        let content = chat_message_content(
            &json!([
                { "type": "text", "text": "Current task" },
                { "type": "text", "text": "Clear five stones" }
            ]),
            None,
            &mut images,
        )
        .unwrap();
        assert_eq!(content, "Current task\nClear five stones");
    }

    #[test]
    fn rejects_image_content_for_a_text_only_model() {
        let mut images = Vec::new();
        let error = chat_message_content(
            &json!([{
                "type": "image_url",
                "image_url": { "url": "data:image/png;base64,abc" }
            }]),
            None,
            &mut images,
        )
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("local text model does not accept image message content"));
    }

    #[test]
    fn converts_openai_data_images_to_multimodal_markers() {
        let mut images = Vec::new();
        let content = chat_message_content(
            &json!([
                { "type": "text", "text": "What is here?" },
                {
                    "type": "image_url",
                    "image_url": { "url": "data:image/png;base64,AQID" }
                }
            ]),
            Some(ImageInputLimits {
                max_images: 2,
                max_media_bytes: 16,
            }),
            &mut images,
        )
        .unwrap();

        assert_eq!(content, format!("What is here?\n{}", mtmd_default_marker()));
        assert_eq!(images, vec![vec![1, 2, 3]]);
    }

    #[test]
    fn rejects_remote_images_for_an_in_process_model() {
        let mut images = Vec::new();
        let error = chat_message_content(
            &json!([{
                "type": "image_url",
                "image_url": { "url": "https://example.invalid/frame.jpg" }
            }]),
            Some(ImageInputLimits {
                max_images: 2,
                max_media_bytes: 16,
            }),
            &mut images,
        )
        .unwrap_err();

        assert!(error.to_string().contains("base64 data image URLs"));
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
    fn provider_config_accepts_multimodal_model_fields() {
        let (_config, multimodal) = provider_configs(
            &json!({
                "modelPath": "models/qwen-vl.gguf",
                "mmprojPath": "models/mmproj-qwen-vl.gguf",
                "imageMaxTokens": 512
            }),
            Path::new("/opt/vifu"),
        )
        .unwrap();

        let multimodal = multimodal.unwrap();
        assert_eq!(
            multimodal.mmproj_path,
            Path::new("/opt/vifu/models/mmproj-qwen-vl.gguf")
        );
        assert_eq!(multimodal.image_max_tokens, 512);
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
