use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

use reqwest::header::{HeaderMap, CONTENT_TYPE};
use serde_json::{json, Value};

use crate::{
    AgentProvider, CancellationToken, InvocationData, ProviderFuture, ProviderRequest,
    ProviderResponse, RuntimeError,
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
                    ProviderResponse {
                        data: InvocationData::Json(
                            openai_chat_completion(
                                &self.base_url,
                                self.token.as_deref(),
                                model,
                                payload,
                                persona,
                            )
                            .await
                            .map_err(|message| RuntimeError::provider(&self.name, message))?,
                        ),
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
                    ProviderResponse {
                        data: InvocationData::Json(
                            openai_audio_transcription(
                                &self.base_url,
                                self.token.as_deref(),
                                model,
                                audio.clone(),
                                file_name,
                                content_type,
                            )
                            .await
                            .map_err(|message| RuntimeError::provider(&self.name, message))?,
                        ),
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
                    ProviderResponse {
                        data: InvocationData::Json(json!({
                            "text": local_whisper_transcription(
                                model_path,
                                audio,
                                language.as_deref(),
                            )
                            .map_err(|message| RuntimeError::provider(&self.name, message))?,
                        })),
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
    let mut request = request.clone();
    apply_persona_to_chat_request(&mut request, persona)?;
    request
        .as_object_mut()
        .ok_or_else(|| "chat completion request must be an object".to_string())?
        .insert("model".to_string(), Value::String(model.to_string()));

    let response = authorized(
        reqwest::Client::new().post(provider_url(base_url, "chat/completions")?),
        token,
    )
    .json(&request)
    .send()
    .await
    .map_err(|error| format!("provider request failed: {error}"))?;
    decode_json_response(response, "chat completion").await
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
    let response = authorized(reqwest::Client::new().post(url), token)
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
    let part = reqwest::multipart::Part::bytes(audio)
        .file_name(file_name.to_string())
        .mime_str(content_type)
        .map_err(|error| format!("audio content type is invalid: {error}"))?;
    let form = reqwest::multipart::Form::new()
        .text("model", model.to_string())
        .part("file", part);
    let response = authorized(
        reqwest::Client::new().post(provider_url(base_url, "audio/transcriptions")?),
        token,
    )
    .multipart(form)
    .send()
    .await
    .map_err(|error| format!("transcription provider request failed: {error}"))?;
    decode_json_response(response, "audio transcription").await
}

pub async fn probe_openai_compatible(base_url: &str, token: Option<&str>) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(PROVIDER_PROBE_TIMEOUT)
        .build()
        .map_err(|error| format!("provider client could not be created: {error}"))?;
    let response = authorized(client.get(provider_url(base_url, "models")?), token)
        .send()
        .await
        .map_err(|error| format!("provider probe failed: {error}"))?;
    require_success(response, "probe").await
}

pub async fn probe_elevenlabs(base_url: &str, token: Option<&str>) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(PROVIDER_PROBE_TIMEOUT)
        .build()
        .map_err(|error| format!("provider client could not be created: {error}"))?;
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

fn provider_url(base_url: &str, path: &str) -> Result<String, String> {
    let base_url = base_url.trim();
    if !(base_url.starts_with("http://") || base_url.starts_with("https://")) {
        return Err("provider URL must use http or https".to_string());
    }
    Ok(format!("{}/{}", base_url.trim_end_matches('/'), path))
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
) -> Result<Value, String> {
    let status = response.status();
    let body = response
        .bytes()
        .await
        .map_err(|error| format!("{operation} response could not be read: {error}"))?;
    if !status.is_success() {
        return Err(provider_error(operation, status.as_u16(), &body));
    }
    serde_json::from_slice(&body)
        .map_err(|error| format!("{operation} response is not valid JSON: {error}"))
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
    use serde_json::json;

    use super::{persona_prompt, provider_url, resolve_local_model_path};

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
    fn keeps_local_models_inside_the_vifu_model_directory() {
        let path =
            resolve_local_model_path(std::path::Path::new("/tmp/.vifu"), "tiny.bin").unwrap();
        assert_eq!(path, std::path::Path::new("/tmp/.vifu/models/tiny.bin"));
        assert!(resolve_local_model_path(std::path::Path::new("/tmp/.vifu"), "../key").is_err());
    }
}
