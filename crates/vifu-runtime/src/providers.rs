use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Instant;

use reqwest::header::{HeaderMap, CONTENT_TYPE};
use serde_json::{json, Value};

use crate::{
    AgentProvider, CancellationToken, InvocationData, ProviderEventSink, ProviderFuture,
    ProviderRequest, ProviderResponse, ProviderStage, RuntimeError,
};

const PROVIDER_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

pub struct BinaryProviderResponse {
    pub content_type: String,
    pub body: Vec<u8>,
}

/// Capability protocol used by [`HttpCapabilityProvider`].
#[derive(Clone, PartialEq)]
pub enum HttpCapabilityRoute {
    OpenAiChat {
        model: String,
        persona: Value,
    },
    OpenAiEmbedding {
        model: String,
    },
    ElevenLabsSpeech {
        voice_id: String,
    },
    OpenAiTranscription {
        model: String,
        file_name: String,
        content_type: String,
    },
    #[cfg(feature = "local-whisper")]
    LocalWhisper {
        model_path: PathBuf,
        language: Option<String>,
    },
}

impl fmt::Debug for HttpCapabilityRoute {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OpenAiChat { model, .. } => formatter
                .debug_struct("OpenAiChat")
                .field("model", model)
                .field("persona", &"[REDACTED]")
                .finish(),
            Self::OpenAiEmbedding { model } => formatter
                .debug_struct("OpenAiEmbedding")
                .field("model", model)
                .finish(),
            Self::ElevenLabsSpeech { voice_id } => formatter
                .debug_struct("ElevenLabsSpeech")
                .field("voice_id", voice_id)
                .finish(),
            Self::OpenAiTranscription {
                model,
                file_name,
                content_type,
            } => formatter
                .debug_struct("OpenAiTranscription")
                .field("model", model)
                .field("file_name", file_name)
                .field("content_type", content_type)
                .finish(),
            #[cfg(feature = "local-whisper")]
            Self::LocalWhisper { language, .. } => formatter
                .debug_struct("LocalWhisper")
                .field("model_path", &"[REDACTED]")
                .field("language", language)
                .finish(),
        }
    }
}

/// A runtime-registered provider assembled from capability protocol routes.
///
/// This is one provider object regardless of vendor count. Add routes at
/// runtime instead of selecting provider-specific Cargo features.
pub struct HttpCapabilityProvider {
    name: String,
    base_url: String,
    token: Option<String>,
    routes: HashMap<String, HttpCapabilityRoute>,
}

impl fmt::Debug for HttpCapabilityProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HttpCapabilityProvider")
            .field("name", &self.name)
            .field("base_url", &self.base_url)
            .field("token", &self.token.as_ref().map(|_| "[REDACTED]"))
            .field("capabilities", &self.routes.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl HttpCapabilityProvider {
    pub fn new(
        name: impl Into<String>,
        base_url: impl Into<String>,
        token: Option<String>,
    ) -> Result<Self, RuntimeError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(RuntimeError::InvalidDefinition(
                "provider name is required".to_string(),
            ));
        }
        let base_url = base_url.into();
        provider_url(&base_url, "models").map_err(RuntimeError::InvalidDefinition)?;
        Ok(Self {
            name,
            base_url,
            token: token
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
            routes: HashMap::new(),
        })
    }

    pub fn local(name: impl Into<String>) -> Result<Self, RuntimeError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(RuntimeError::InvalidDefinition(
                "provider name is required".to_string(),
            ));
        }
        Ok(Self {
            name,
            base_url: String::new(),
            token: None,
            routes: HashMap::new(),
        })
    }

    pub fn add_route(
        &mut self,
        capability: impl Into<String>,
        route: HttpCapabilityRoute,
    ) -> Result<(), RuntimeError> {
        let capability = capability.into().trim().to_ascii_lowercase();
        if capability.is_empty() || capability.len() > 128 {
            return Err(RuntimeError::InvalidDefinition(
                "provider capability is invalid".to_string(),
            ));
        }
        self.routes.insert(capability, route);
        Ok(())
    }

    pub fn with_route(
        mut self,
        capability: impl Into<String>,
        route: HttpCapabilityRoute,
    ) -> Result<Self, RuntimeError> {
        self.add_route(capability, route)?;
        Ok(self)
    }
}

impl AgentProvider for HttpCapabilityProvider {
    fn supports(&self, capability: &str) -> bool {
        self.routes.contains_key(capability)
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
            let route = self.routes.get(&request.capability).ok_or_else(|| {
                RuntimeError::CapabilityUnavailable {
                    provider: self.name.clone(),
                    capability: request.capability.clone(),
                }
            })?;
            if cancellation.is_cancelled() {
                return Err(RuntimeError::Cancelled);
            }
            let response = match route {
                HttpCapabilityRoute::OpenAiChat { model, persona } => {
                    let InvocationData::Json(payload) = &request.data else {
                        return Err(RuntimeError::InvalidDefinition(
                            "chat capability requires JSON input".to_string(),
                        ));
                    };
                    let data = provider_json_result(
                        &self.name,
                        &events,
                        "chat",
                        openai_chat_completion_result(
                            &self.base_url,
                            self.token.as_deref(),
                            model,
                            payload,
                            persona,
                        )
                        .await,
                    )?;
                    if cancellation.is_cancelled() {
                        return Err(RuntimeError::Cancelled);
                    }
                    validate_with_events(&self.name, &events, "chat", || {
                        validate_openai_chat_response(&data)
                    })?;
                    ProviderResponse {
                        data: InvocationData::Json(data),
                        metadata: json!({ "contentType": "application/json" }),
                        state: None,
                    }
                }
                HttpCapabilityRoute::OpenAiEmbedding { model } => {
                    let InvocationData::Json(payload) = &request.data else {
                        return Err(RuntimeError::InvalidDefinition(
                            "embedding capability requires JSON input".to_string(),
                        ));
                    };
                    let data = provider_json_result(
                        &self.name,
                        &events,
                        "embedding",
                        openai_embeddings_result(
                            &self.base_url,
                            self.token.as_deref(),
                            model,
                            payload,
                        )
                        .await,
                    )?;
                    if cancellation.is_cancelled() {
                        return Err(RuntimeError::Cancelled);
                    }
                    validate_with_events(&self.name, &events, "embedding", || {
                        validate_openai_embedding_response(&data)
                    })?;
                    ProviderResponse {
                        data: InvocationData::Json(data),
                        metadata: json!({ "contentType": "application/json" }),
                        state: None,
                    }
                }
                HttpCapabilityRoute::ElevenLabsSpeech { voice_id } => {
                    let InvocationData::Json(payload) = &request.data else {
                        return Err(RuntimeError::InvalidDefinition(
                            "speech capability requires JSON input".to_string(),
                        ));
                    };
                    let response =
                        elevenlabs_speech(&self.base_url, self.token.as_deref(), voice_id, payload)
                            .await
                            .map_err(|message| RuntimeError::provider(&self.name, message))?;
                    ProviderResponse {
                        data: InvocationData::Binary(response.body),
                        metadata: json!({ "contentType": response.content_type }),
                        state: None,
                    }
                }
                HttpCapabilityRoute::OpenAiTranscription {
                    model,
                    file_name,
                    content_type,
                } => {
                    let InvocationData::Binary(audio) = &request.data else {
                        return Err(RuntimeError::InvalidDefinition(
                            "transcription capability requires binary input".to_string(),
                        ));
                    };
                    let data = provider_json_result(
                        &self.name,
                        &events,
                        "transcription",
                        openai_audio_transcription_result(
                            &self.base_url,
                            self.token.as_deref(),
                            model,
                            audio.clone(),
                            file_name,
                            content_type,
                        )
                        .await,
                    )?;
                    if cancellation.is_cancelled() {
                        return Err(RuntimeError::Cancelled);
                    }
                    validate_with_events(&self.name, &events, "transcription", || {
                        validate_transcription_response(&data)
                    })?;
                    ProviderResponse {
                        data: InvocationData::Json(data),
                        metadata: json!({ "contentType": "application/json" }),
                        state: None,
                    }
                }
                #[cfg(feature = "local-whisper")]
                HttpCapabilityRoute::LocalWhisper {
                    model_path,
                    language,
                } => {
                    let InvocationData::Binary(audio) = &request.data else {
                        return Err(RuntimeError::InvalidDefinition(
                            "transcription capability requires binary input".to_string(),
                        ));
                    };
                    let data = json!({
                        "text": local_whisper_transcription(
                            model_path,
                            audio,
                            request
                                .metadata
                                .pointer("/binding/language")
                                .and_then(Value::as_str)
                                .or(language.as_deref()),
                        )
                        .map_err(|message| RuntimeError::provider(&self.name, message))?,
                    });
                    if cancellation.is_cancelled() {
                        return Err(RuntimeError::Cancelled);
                    }
                    validate_with_events(&self.name, &events, "transcription", || {
                        validate_transcription_response(&data)
                    })?;
                    ProviderResponse {
                        data: InvocationData::Json(data),
                        metadata: json!({ "contentType": "application/json" }),
                        state: None,
                    }
                }
            };
            if cancellation.is_cancelled() {
                return Err(RuntimeError::Cancelled);
            }
            Ok(response)
        })
    }
}

pub async fn openai_chat_completion(
    base_url: &str,
    token: Option<&str>,
    model: &str,
    request: &Value,
    persona: &Value,
) -> Result<Value, String> {
    openai_chat_completion_result(base_url, token, model, request, persona)
        .await
        .map_err(JsonProviderError::into_message)
}

async fn openai_chat_completion_result(
    base_url: &str,
    token: Option<&str>,
    model: &str,
    request: &Value,
    persona: &Value,
) -> Result<Value, JsonProviderError> {
    let mut request = request.clone();
    apply_persona_to_chat_request(&mut request, persona).map_err(JsonProviderError::Provider)?;
    request
        .as_object_mut()
        .ok_or_else(|| {
            JsonProviderError::Provider("chat completion request must be an object".to_string())
        })?
        .insert("model".to_string(), Value::String(model.to_string()));

    let client = provider_http_client(None).map_err(JsonProviderError::Provider)?;
    let response = authorized(
        client
            .post(provider_url(base_url, "chat/completions").map_err(JsonProviderError::Provider)?),
        token,
    )
    .json(&request)
    .send()
    .await
    .map_err(|error| JsonProviderError::Provider(format!("provider request failed: {error}")))?;
    decode_json_response(response, "chat completion").await
}

pub async fn openai_embeddings(
    base_url: &str,
    token: Option<&str>,
    model: &str,
    request: &Value,
) -> Result<Value, String> {
    openai_embeddings_result(base_url, token, model, request)
        .await
        .map_err(JsonProviderError::into_message)
}

async fn openai_embeddings_result(
    base_url: &str,
    token: Option<&str>,
    model: &str,
    request: &Value,
) -> Result<Value, JsonProviderError> {
    let mut request = request.clone();
    request
        .as_object_mut()
        .ok_or_else(|| {
            JsonProviderError::Provider("embedding request must be an object".to_string())
        })?
        .insert("model".to_string(), Value::String(model.to_string()));

    let client = provider_http_client(None).map_err(JsonProviderError::Provider)?;
    let response = authorized(
        client.post(provider_url(base_url, "embeddings").map_err(JsonProviderError::Provider)?),
        token,
    )
    .json(&request)
    .send()
    .await
    .map_err(|error| {
        JsonProviderError::Provider(format!("embedding provider request failed: {error}"))
    })?;
    decode_json_response(response, "embedding").await
}

pub fn apply_persona_to_chat_request(request: &mut Value, persona: &Value) -> Result<(), String> {
    let object = request
        .as_object_mut()
        .ok_or_else(|| "chat completion request must be an object".to_string())?;
    apply_persona(object, persona)
}

pub async fn elevenlabs_speech(
    base_url: &str,
    token: Option<&str>,
    voice_id: &str,
    request: &Value,
) -> Result<BinaryProviderResponse, String> {
    let url = format!(
        "{}/text-to-speech/{}",
        base_url.trim_end_matches('/'),
        encode_path_segment(voice_id)?
    );
    let client = provider_http_client(None)?;
    let response = authorized(client.post(url), token)
        .header("xi-api-key", token.unwrap_or_default())
        .json(request)
        .send()
        .await
        .map_err(|error| format!("speech provider request failed: {error}"))?;
    decode_binary_response(response, "speech synthesis").await
}

pub async fn openai_audio_transcription(
    base_url: &str,
    token: Option<&str>,
    model: &str,
    audio: Vec<u8>,
    file_name: &str,
    content_type: &str,
) -> Result<Value, String> {
    openai_audio_transcription_result(base_url, token, model, audio, file_name, content_type)
        .await
        .map_err(JsonProviderError::into_message)
}

async fn openai_audio_transcription_result(
    base_url: &str,
    token: Option<&str>,
    model: &str,
    audio: Vec<u8>,
    file_name: &str,
    content_type: &str,
) -> Result<Value, JsonProviderError> {
    let part = reqwest::multipart::Part::bytes(audio)
        .file_name(file_name.to_string())
        .mime_str(content_type)
        .map_err(|error| {
            JsonProviderError::Provider(format!("audio content type is invalid: {error}"))
        })?;
    let form = reqwest::multipart::Form::new()
        .text("model", model.to_string())
        .part("file", part);
    let client = provider_http_client(None).map_err(JsonProviderError::Provider)?;
    let response = authorized(
        client.post(
            provider_url(base_url, "audio/transcriptions").map_err(JsonProviderError::Provider)?,
        ),
        token,
    )
    .multipart(form)
    .send()
    .await
    .map_err(|error| {
        JsonProviderError::Provider(format!("transcription provider request failed: {error}"))
    })?;
    decode_json_response(response, "audio transcription").await
}

pub async fn probe_openai_compatible(base_url: &str, token: Option<&str>) -> Result<(), String> {
    let client = provider_http_client(Some(PROVIDER_PROBE_TIMEOUT))?;
    let response = authorized(client.get(provider_url(base_url, "models")?), token)
        .send()
        .await
        .map_err(|error| format!("provider probe failed: {error}"))?;
    require_success(response, "probe").await
}

pub async fn probe_elevenlabs(base_url: &str, token: Option<&str>) -> Result<(), String> {
    let client = provider_http_client(Some(PROVIDER_PROBE_TIMEOUT))?;
    let response = authorized(client.get(provider_url(base_url, "models")?), token)
        .header("xi-api-key", token.unwrap_or_default())
        .send()
        .await
        .map_err(|error| format!("provider probe failed: {error}"))?;
    require_success(response, "probe").await
}

#[cfg(feature = "local-whisper")]
pub fn local_whisper_transcription(
    model_path: &Path,
    wav: &[u8],
    language: Option<&str>,
) -> Result<String, String> {
    use std::io::Cursor;

    use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

    let mut reader = hound::WavReader::new(Cursor::new(wav))
        .map_err(|error| format!("audio must be a valid WAV file: {error}"))?;
    let spec = reader.spec();
    let channels = usize::from(spec.channels);
    if channels == 0 || spec.sample_rate == 0 {
        return Err("WAV audio has an invalid channel count or sample rate".to_string());
    }
    let interleaved = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("WAV samples could not be decoded: {error}"))?,
        hound::SampleFormat::Int => {
            let scale = 2_f32.powi(i32::from(spec.bits_per_sample.saturating_sub(1)));
            reader
                .samples::<i32>()
                .map(|sample| {
                    sample
                        .map(|sample| sample as f32 / scale)
                        .map_err(|error| format!("WAV samples could not be decoded: {error}"))
                })
                .collect::<Result<Vec<_>, _>>()?
        }
    };
    let mono = interleaved
        .chunks(channels)
        .map(|frame| frame.iter().copied().sum::<f32>() / frame.len() as f32)
        .collect::<Vec<_>>();
    let samples = resample_linear(&mono, spec.sample_rate, 16_000);
    if samples.is_empty() {
        return Err("WAV audio does not contain samples".to_string());
    }

    let model_path = model_path
        .to_str()
        .ok_or_else(|| "Whisper model path is not valid UTF-8".to_string())?;
    let context = WhisperContext::new_with_params(model_path, WhisperContextParameters::default())
        .map_err(|error| format!("Whisper model could not be loaded: {error}"))?;
    let mut state = context
        .create_state()
        .map_err(|error| format!("Whisper state could not be created: {error}"))?;
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_language(language);
    state
        .full(params, &samples)
        .map_err(|error| format!("Whisper transcription failed: {error}"))?;
    let segments = state
        .as_iter()
        .map(|segment| {
            segment
                .to_str_lossy()
                .map(|text| text.into_owned())
                .map_err(|error| format!("Whisper segment could not be decoded: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(segments.join("").trim().to_string())
}

#[cfg(not(feature = "local-whisper"))]
pub fn local_whisper_transcription(
    _model_path: &Path,
    _wav: &[u8],
    _language: Option<&str>,
) -> Result<String, String> {
    Err("this Vifu build does not include local Whisper support".to_string())
}

pub fn resolve_local_model_path(home_dir: &Path, model: &str) -> Result<PathBuf, String> {
    let model = model.trim();
    if model.is_empty()
        || model.len() > 255
        || model.contains('/')
        || model.contains('\\')
        || model == "."
        || model == ".."
    {
        return Err("local model must be a file name inside ~/.vifu/models".to_string());
    }
    Ok(home_dir.join("models").join(model))
}

fn apply_persona(
    request: &mut serde_json::Map<String, Value>,
    persona: &Value,
) -> Result<(), String> {
    let prompt = persona_prompt(persona);
    if prompt.is_empty() {
        return Ok(());
    }
    let messages = request
        .get_mut("messages")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "chat completion messages must be an array".to_string())?;
    messages.insert(0, json!({ "role": "system", "content": prompt }));
    Ok(())
}

fn persona_prompt(persona: &Value) -> String {
    let mut sections = Vec::new();
    if let Some(prompt) = persona
        .get("systemPrompt")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        sections.push(prompt.to_string());
    }
    if let Some(files) = persona.get("files").and_then(Value::as_object) {
        for (name, content) in files {
            let Some(content) = content
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            sections.push(format!("# {name}\n\n{content}"));
        }
    }
    sections.join("\n\n")
}

enum JsonProviderError {
    Provider(String),
    MalformedResponse(String),
}

impl JsonProviderError {
    fn into_message(self) -> String {
        match self {
            Self::Provider(message) | Self::MalformedResponse(message) => message,
        }
    }
}

fn provider_json_result(
    provider_name: &str,
    events: &ProviderEventSink,
    kind: &str,
    result: Result<Value, JsonProviderError>,
) -> Result<Value, RuntimeError> {
    match result {
        Ok(response) => Ok(response),
        Err(JsonProviderError::Provider(message)) => {
            Err(RuntimeError::provider(provider_name, message))
        }
        Err(JsonProviderError::MalformedResponse(message)) => {
            let started = Instant::now();
            events.stage_started(ProviderStage::Validate, json!({ "kind": kind }));
            Err(validation_error(
                provider_name,
                events,
                kind,
                started,
                message,
            ))
        }
    }
}

fn validate_with_events(
    provider_name: &str,
    events: &ProviderEventSink,
    kind: &str,
    validate: impl FnOnce() -> Result<Value, String>,
) -> Result<(), RuntimeError> {
    let started = Instant::now();
    events.stage_started(ProviderStage::Validate, json!({ "kind": kind }));
    match validate() {
        Ok(metadata) => {
            events.stage_completed(ProviderStage::Validate, elapsed_ms(started), metadata);
            Ok(())
        }
        Err(message) => Err(validation_error(
            provider_name,
            events,
            kind,
            started,
            message,
        )),
    }
}

fn validation_error(
    provider_name: &str,
    events: &ProviderEventSink,
    kind: &str,
    started: Instant,
    message: String,
) -> RuntimeError {
    let error = RuntimeError::provider(provider_name, message);
    events.stage_failed(
        ProviderStage::Validate,
        elapsed_ms(started),
        error.to_string(),
        json!({ "kind": kind }),
    );
    error
}

fn validate_openai_chat_response(response: &Value) -> Result<Value, String> {
    let choices = response
        .get("choices")
        .and_then(Value::as_array)
        .filter(|choices| !choices.is_empty())
        .ok_or_else(|| "chat response has no choices".to_string())?;
    let message = choices[0]
        .get("message")
        .and_then(Value::as_object)
        .ok_or_else(|| "chat response first choice has no assistant message".to_string())?;
    if let Some(role) = message.get("role") {
        if role.as_str() != Some("assistant") {
            return Err("chat response first message is not from the assistant".to_string());
        }
    }
    let content = message.get("content");
    if let Some(content) = content {
        if !matches!(content, Value::Null | Value::String(_) | Value::Array(_)) {
            return Err("chat response assistant content has an invalid type".to_string());
        }
    }
    let tool_calls = message.get("tool_calls");
    if tool_calls.is_some_and(|calls| !calls.is_array()) {
        return Err("chat response assistant tool_calls is not an array".to_string());
    }
    let function_call = message.get("function_call");
    if function_call.is_some_and(|call| !call.is_object() && !call.is_null()) {
        return Err("chat response assistant function_call is not an object".to_string());
    }
    if content.is_none() && tool_calls.is_none() && function_call.is_none() {
        return Err("chat response assistant message has no content or tool calls".to_string());
    }
    Ok(json!({
        "kind": "chat",
        "choices": choices.len(),
        "toolCalls": tool_calls.and_then(Value::as_array).map_or(0, Vec::len),
    }))
}

fn validate_openai_embedding_response(response: &Value) -> Result<Value, String> {
    let rows = response
        .get("data")
        .and_then(Value::as_array)
        .filter(|rows| !rows.is_empty())
        .ok_or_else(|| "embedding response has no data rows".to_string())?;
    let mut dimensions = None;
    let mut encoding = None;
    for (index, row) in rows.iter().enumerate() {
        let embedding = row
            .get("embedding")
            .ok_or_else(|| format!("embedding row {index} has no vector"))?;
        let (row_dimensions, row_encoding) = if let Some(values) = embedding.as_array() {
            if values.is_empty()
                || values
                    .iter()
                    .any(|value| !value.as_f64().is_some_and(f64::is_finite))
            {
                return Err(format!(
                    "embedding row {index} has an invalid numeric vector"
                ));
            }
            (values.len(), "float")
        } else if let Some(encoded) = embedding.as_str() {
            let row_dimensions = decode_base64_float32_dimensions(encoded)
                .map_err(|message| format!("embedding row {index} {message}"))?;
            (row_dimensions, "base64")
        } else {
            return Err(format!("embedding row {index} has an invalid vector type"));
        };
        if let Some(expected_dimensions) = dimensions {
            if expected_dimensions != row_dimensions {
                return Err(format!(
                    "embedding row {index} changed dimension from {expected_dimensions} to {row_dimensions}"
                ));
            }
        } else {
            dimensions = Some(row_dimensions);
        }
        if let Some(expected_encoding) = encoding {
            if expected_encoding != row_encoding {
                return Err(format!(
                    "embedding row {index} changed encoding from {expected_encoding} to {row_encoding}"
                ));
            }
        } else {
            encoding = Some(row_encoding);
        }
    }
    Ok(json!({
        "kind": "embedding",
        "rows": rows.len(),
        "dimensions": dimensions,
        "encoding": encoding,
    }))
}

fn validate_transcription_response(response: &Value) -> Result<Value, String> {
    let text = response
        .get("text")
        .and_then(Value::as_str)
        .ok_or_else(|| "transcription response text is missing or is not a string".to_string())?;
    Ok(json!({ "kind": "transcription", "characters": text.chars().count() }))
}

fn decode_base64_float32_dimensions(encoded: &str) -> Result<usize, String> {
    let mut bytes = encoded.as_bytes().to_vec();
    if bytes.is_empty() || bytes.len() % 4 == 1 {
        return Err("has invalid base64 float32 data".to_string());
    }
    match bytes.len() % 4 {
        2 => bytes.extend_from_slice(b"=="),
        3 => bytes.push(b'='),
        _ => {}
    }
    let mut decoded = Vec::with_capacity(bytes.len() / 4 * 3);
    let chunks = bytes.len() / 4;
    for (chunk_index, chunk) in bytes.chunks_exact(4).enumerate() {
        let last = chunk_index + 1 == chunks;
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
        if (padding == 2 && b & 0x0f != 0) || (padding == 1 && c & 0x03 != 0) {
            return Err("has non-canonical base64 padding".to_string());
        }
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
        b'+' | b'-' => Ok(62),
        b'/' | b'_' => Ok(63),
        _ => Err("has invalid base64 characters".to_string()),
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn provider_url(base_url: &str, path: &str) -> Result<String, String> {
    let base_url = base_url.trim();
    if !(base_url.starts_with("http://") || base_url.starts_with("https://")) {
        return Err("provider URL must use http or https".to_string());
    }
    Ok(format!("{}/{}", base_url.trim_end_matches('/'), path))
}

fn provider_http_client(timeout: Option<std::time::Duration>) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none());
    if let Some(timeout) = timeout {
        builder = builder.timeout(timeout);
    }
    builder
        .build()
        .map_err(|error| format!("provider client could not be created: {error}"))
}

fn authorized(builder: reqwest::RequestBuilder, token: Option<&str>) -> reqwest::RequestBuilder {
    match token.map(str::trim).filter(|token| !token.is_empty()) {
        Some(token) => builder.bearer_auth(token),
        None => builder,
    }
}

async fn decode_json_response(
    response: reqwest::Response,
    operation: &str,
) -> Result<Value, JsonProviderError> {
    let status = response.status();
    let body = response.bytes().await.map_err(|error| {
        JsonProviderError::Provider(format!("{operation} response could not be read: {error}"))
    })?;
    if !status.is_success() {
        return Err(JsonProviderError::Provider(provider_error(
            operation,
            status.as_u16(),
            &body,
        )));
    }
    serde_json::from_slice(&body).map_err(|error| {
        JsonProviderError::MalformedResponse(format!(
            "{operation} response is not valid JSON: {error}"
        ))
    })
}

async fn decode_binary_response(
    response: reqwest::Response,
    operation: &str,
) -> Result<BinaryProviderResponse, String> {
    let status = response.status();
    let content_type = response_content_type(response.headers());
    let body = response
        .bytes()
        .await
        .map_err(|error| format!("{operation} response could not be read: {error}"))?;
    if !status.is_success() {
        return Err(provider_error(operation, status.as_u16(), &body));
    }
    Ok(BinaryProviderResponse {
        content_type,
        body: body.to_vec(),
    })
}

async fn require_success(response: reqwest::Response, operation: &str) -> Result<(), String> {
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    let body = response
        .bytes()
        .await
        .map_err(|error| format!("provider {operation} response could not be read: {error}"))?;
    Err(provider_error(operation, status.as_u16(), &body))
}

fn response_content_type(headers: &HeaderMap) -> String {
    headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string()
}

fn provider_error(operation: &str, status: u16, body: &[u8]) -> String {
    let message = serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .pointer("/error/message")
                .or_else(|| value.get("error"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.chars().take(512).collect::<String>())
        })
        .unwrap_or_else(|| format!("HTTP {status}"));
    format!("provider {operation} failed: {message}")
}

fn encode_path_segment(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 256
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err("provider resource ID contains unsupported characters".to_string());
    }
    Ok(value.to_string())
}

#[cfg(feature = "local-whisper")]
fn resample_linear(input: &[f32], from_hz: u32, to_hz: u32) -> Vec<f32> {
    if input.is_empty() || from_hz == 0 || to_hz == 0 {
        return Vec::new();
    }
    if from_hz == to_hz {
        return input.to_vec();
    }
    let output_len = (input.len() as u64 * u64::from(to_hz) / u64::from(from_hz)) as usize;
    (0..output_len)
        .map(|index| {
            let source = index as f64 * f64::from(from_hz) / f64::from(to_hz);
            let left = source.floor() as usize;
            let right = (left + 1).min(input.len() - 1);
            let fraction = (source - left as f64) as f32;
            input[left] + (input[right] - input[left]) * fraction
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;

    use serde_json::json;

    use super::{
        openai_chat_completion, persona_prompt, probe_openai_compatible, provider_json_result,
        provider_url, resolve_local_model_path, validate_openai_chat_response,
        validate_openai_embedding_response, validate_transcription_response, validate_with_events,
        JsonProviderError,
    };
    use crate::{ProviderEvent, ProviderEventSink, ProviderStage, RuntimeError};

    #[test]
    fn builds_a_portable_persona_prompt() {
        assert_eq!(
            persona_prompt(&json!({
                "systemPrompt": "Stay concise.",
                "files": { "SOUL.md": "You are the steward." }
            })),
            "Stay concise.\n\n# SOUL.md\n\nYou are the steward."
        );
    }

    #[test]
    fn appends_openai_compatible_paths() {
        assert_eq!(
            provider_url("https://example.com/v1/", "chat/completions").unwrap(),
            "https://example.com/v1/chat/completions"
        );
    }

    #[test]
    fn appends_the_openai_embedding_path() {
        assert_eq!(
            provider_url("https://example.com/v1/", "embeddings").unwrap(),
            "https://example.com/v1/embeddings"
        );
    }

    #[tokio::test]
    async fn provider_requests_do_not_follow_redirects() {
        let (base_url, server) = redirecting_provider();

        let error = openai_chat_completion(
            &base_url,
            Some("test-token"),
            "local-model",
            &json!({"messages": [{"role": "user", "content": "hello"}]}),
            &json!({}),
        )
        .await
        .unwrap_err();

        server.join().unwrap();
        assert!(error.contains("307"), "unexpected provider error: {error}");
    }

    #[tokio::test]
    async fn provider_probes_do_not_follow_redirects() {
        let (base_url, server) = redirecting_provider();

        let error = probe_openai_compatible(&base_url, Some("test-token"))
            .await
            .unwrap_err();

        server.join().unwrap();
        assert!(error.contains("307"), "unexpected probe error: {error}");
    }

    fn redirecting_provider() -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request);
            stream
                .write_all(
                    b"HTTP/1.1 307 Temporary Redirect\r\nLocation: http://127.0.0.1:9/exfil\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
        });
        (format!("http://{address}/v1"), server)
    }

    #[test]
    fn keeps_local_models_inside_the_vifu_model_directory() {
        let path =
            resolve_local_model_path(std::path::Path::new("/tmp/.vifu"), "tiny.bin").unwrap();
        assert_eq!(path, std::path::Path::new("/tmp/.vifu/models/tiny.bin"));
        assert!(resolve_local_model_path(std::path::Path::new("/tmp/.vifu"), "../key").is_err());
    }

    #[test]
    fn validates_openai_chat_assistant_text() {
        let metadata = validate_openai_chat_response(&json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": [{ "type": "text", "text": "Ready." }]
                }
            }]
        }))
        .unwrap();

        assert_eq!(
            metadata,
            json!({ "kind": "chat", "choices": 1, "toolCalls": 0 })
        );
    }

    #[test]
    fn accepts_empty_and_tool_call_only_chat_outputs() {
        let empty = validate_openai_chat_response(&json!({
            "choices": [{ "message": { "role": "assistant", "content": "" } }]
        }))
        .unwrap();
        let tool_call = validate_openai_chat_response(&json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call-1",
                        "type": "function",
                        "function": { "name": "move", "arguments": "{}" }
                    }]
                }
            }]
        }))
        .unwrap();

        assert_eq!(empty["toolCalls"], 0);
        assert_eq!(tool_call["toolCalls"], 1);
    }

    #[test]
    fn validates_consistent_numeric_embedding_rows() {
        let metadata = validate_openai_embedding_response(&json!({
            "data": [
                { "embedding": [1, 2.5] },
                { "embedding": [-3, 4] }
            ]
        }))
        .unwrap();

        assert_eq!(
            metadata,
            json!({
                "kind": "embedding",
                "rows": 2,
                "dimensions": 2,
                "encoding": "float"
            })
        );
    }

    #[test]
    fn validates_consistent_base64_float32_embedding_rows() {
        let metadata = validate_openai_embedding_response(&json!({
            "data": [
                { "embedding": "AACAPwAAAMA=" },
                { "embedding": "AACAPwAAAMA=" }
            ]
        }))
        .unwrap();

        assert_eq!(metadata["dimensions"], 2);
        assert_eq!(metadata["encoding"], "base64");
    }

    #[test]
    fn rejects_embedding_rows_with_different_dimensions() {
        let error = validate_openai_embedding_response(&json!({
            "data": [
                { "embedding": [1, 2] },
                { "embedding": [3] }
            ]
        }))
        .unwrap_err();

        assert_eq!(error, "embedding row 1 changed dimension from 2 to 1");
    }

    #[test]
    fn rejects_malformed_base64_embedding_vectors() {
        let error = validate_openai_embedding_response(&json!({
            "data": [{ "embedding": "not base64" }]
        }))
        .unwrap_err();

        assert!(error.contains("invalid base64"));
    }

    #[test]
    fn accepts_silence_and_rejects_transcription_without_a_text_field() {
        assert_eq!(
            validate_transcription_response(&json!({ "text": "" })).unwrap(),
            json!({ "kind": "transcription", "characters": 0 })
        );
        let error = validate_transcription_response(&json!({})).unwrap_err();

        assert_eq!(
            error,
            "transcription response text is missing or is not a string"
        );
    }

    #[test]
    fn malformed_chat_emits_validate_failed_and_returns_provider_error() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let event_capture = Arc::clone(&captured);
        let events = ProviderEventSink::from_fn(move |event| {
            event_capture.lock().unwrap().push(event);
        });

        let error = validate_with_events("remote", &events, "chat", || {
            validate_openai_chat_response(&json!({
                "choices": [{ "message": { "role": "assistant", "content": 42 } }]
            }))
        })
        .unwrap_err();

        assert!(matches!(
            error,
            RuntimeError::Provider { ref provider, ref message }
                if provider == "remote"
                    && message == "chat response assistant content has an invalid type"
        ));
        let captured = captured.lock().unwrap();
        assert_eq!(captured.len(), 2);
        assert!(matches!(
            captured[0],
            ProviderEvent::StageStarted {
                stage: ProviderStage::Validate,
                ..
            }
        ));
        assert!(matches!(
            captured[1],
            ProviderEvent::StageFailed {
                stage: ProviderStage::Validate,
                ..
            }
        ));
    }

    #[test]
    fn malformed_embedding_emits_validate_failed_and_returns_provider_error() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let event_capture = Arc::clone(&captured);
        let events = ProviderEventSink::from_fn(move |event| {
            event_capture.lock().unwrap().push(event);
        });

        let error = validate_with_events("remote", &events, "embedding", || {
            validate_openai_embedding_response(&json!({
                "data": [
                    { "embedding": [1, 2] },
                    { "embedding": [3] }
                ]
            }))
        })
        .unwrap_err();

        assert!(matches!(
            error,
            RuntimeError::Provider { ref provider, ref message }
                if provider == "remote"
                    && message == "embedding row 1 changed dimension from 2 to 1"
        ));
        let captured = captured.lock().unwrap();
        assert_eq!(captured.len(), 2);
        assert!(matches!(
            captured[0],
            ProviderEvent::StageStarted {
                stage: ProviderStage::Validate,
                ..
            }
        ));
        assert!(matches!(
            captured[1],
            ProviderEvent::StageFailed {
                stage: ProviderStage::Validate,
                ..
            }
        ));
    }

    #[test]
    fn invalid_json_output_emits_validate_failed() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let event_capture = Arc::clone(&captured);
        let events = ProviderEventSink::from_fn(move |event| {
            event_capture.lock().unwrap().push(event);
        });

        let error = provider_json_result(
            "remote",
            &events,
            "chat",
            Err(JsonProviderError::MalformedResponse(
                "chat completion response is not valid JSON".to_string(),
            )),
        )
        .unwrap_err();

        assert!(matches!(error, RuntimeError::Provider { .. }));
        let captured = captured.lock().unwrap();
        assert!(matches!(
            captured.as_slice(),
            [
                ProviderEvent::StageStarted {
                    stage: ProviderStage::Validate,
                    ..
                },
                ProviderEvent::StageFailed {
                    stage: ProviderStage::Validate,
                    ..
                }
            ]
        ));
    }
}
