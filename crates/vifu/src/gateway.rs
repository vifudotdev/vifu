use std::fs;
use std::io::{self, IsTerminal};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(feature = "local-llama")]
use std::time::Instant;

use tokio::sync::watch;

use vifu_gateway::session_store::{
    gateway_session_state_key, GatewaySecretStorage, GatewaySessionStore,
};
use vifu_gateway::{config, openclaw, providers, relay, session};

#[cfg(feature = "local-llama")]
use vifu_provider_llama::{LlamaProvider, LlamaProviderConfig};

use config::{AgentProviderConfig, Config};
use openclaw::ProbeStatus;
use session::SessionSummary;

const PROVIDER_RETRY_DELAY: Duration = Duration::from_secs(10);

#[derive(Debug, Clone)]
pub struct GatewayRuntimeOptions {
    pub server_url: String,
    pub dashboard_url: Option<String>,
    pub allow_guest_bootstrap: bool,
    pub enrollment_token: Option<String>,
    pub session_scope: String,
}

impl GatewayRuntimeOptions {
    pub fn load_config(&self) -> Result<Config, String> {
        Config::load(self.server_url.clone(), self.enrollment_token.clone())
    }
}

pub async fn run(
    options: GatewayRuntimeOptions,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), String> {
    println!("Vifu");
    loop {
        if *shutdown.borrow() {
            return Ok(());
        }
        let mut config = options.load_config()?;
        ensure_home_dir(&config)?;
        print_server_config(&config)?;
        let openclaw_providers = config.openclaw_providers().cloned().collect::<Vec<_>>();
        let llama_providers = config.llama_providers().cloned().collect::<Vec<_>>();
        let local_whisper_providers = config
            .local_whisper_providers()
            .cloned()
            .collect::<Vec<_>>();
        let openai_compatible_providers = config
            .openai_compatible_providers()
            .cloned()
            .collect::<Vec<_>>();
        println!();
        println!("Providers");
        if config.agent_providers.is_empty() {
            print_agent_provider_config(&config);
        }
        let has_configured_providers = !openclaw_providers.is_empty()
            || !llama_providers.is_empty()
            || !local_whisper_providers.is_empty()
            || !openai_compatible_providers.is_empty();
        let mut runtime_providers: Vec<Arc<dyn relay::AgentGatewayProvider>> = Vec::new();
        let mut agents = Vec::new();
        for provider in llama_providers {
            let (runtime_provider, agent) = load_llama_provider(
                provider,
                config
                    .agent_providers_file
                    .parent()
                    .unwrap_or_else(|| Path::new(".")),
            )?;
            runtime_providers.push(runtime_provider);
            agents.push(agent);
        }
        for provider in local_whisper_providers {
            let (runtime_provider, agent) =
                load_local_whisper_provider(provider, &config.home_dir)?;
            runtime_providers.push(runtime_provider);
            agents.push(agent);
        }
        for provider in openai_compatible_providers {
            let probe =
                providers::probe_openai_compatible(&provider.url, provider.token.as_deref()).await;
            print_openai_compatible_report(&provider, &probe);
            if probe.is_err() {
                continue;
            }
            let (runtime_provider, agent) = load_openai_compatible_provider(provider)?;
            runtime_providers.push(runtime_provider);
            agents.push(agent);
        }
        for provider in openclaw_providers {
            let report = openclaw::probe(&provider.url).await;
            print_openclaw_report(&provider, &report);
            if !matches!(report.status, ProbeStatus::Online) {
                continue;
            }
            let mut discovered = match openclaw::discover_agents(
                &report.endpoint,
                provider.token.as_deref(),
            )
            .await
            {
                Ok(agents) => agents,
                Err(error) => {
                    eprintln!(
                        "OpenClaw provider {} agent discovery is unavailable ({error}).",
                        provider.id
                    );
                    continue;
                }
            };
            for agent in &mut discovered {
                if !agent.metadata.is_object() {
                    agent.metadata = serde_json::json!({});
                }
                let Some(metadata) = agent.metadata.as_object_mut() else {
                    continue;
                };
                metadata.insert(
                    "providerKey".to_string(),
                    serde_json::Value::String(provider.id.clone()),
                );
                metadata.insert(
                    "providerType".to_string(),
                    serde_json::Value::String(provider.provider_type.clone()),
                );
            }
            agents.extend(discovered);
            runtime_providers.push(Arc::new(relay::OpenClawGatewayProvider::new(
                provider.id,
                report.endpoint,
                provider.token,
            )));
        }
        if has_configured_providers && runtime_providers.is_empty() {
            wait_for_provider_retry("No configured Agent Provider is reachable.", &mut shutdown)
                .await;
            continue;
        }
        println!(
            "  Ready: {} providers, {} agents",
            runtime_providers.len(),
            agents.len()
        );
        let runtime_database_file = config.runtime_database_file();
        let session_store = GatewaySessionStore::open(&runtime_database_file)?;
        let session_key = gateway_session_state_key(&options.session_scope, &config.server_url)?;
        let mut session = load_or_create_session(&session_store, &session_key)?;
        print_session(
            &session,
            &config.server_url,
            options.dashboard_url.as_deref(),
        );
        let session_persistence =
            session_store.persistence(session_key, GatewaySecretStorage::Persisted);
        let runtime = relay::AgentGatewayRuntime {
            server_url: &config.server_url,
            dashboard_url: options.dashboard_url.as_deref(),
            agent_gateway_bootstrap_token: config.agent_gateway_bootstrap_token.as_deref(),
            enrollment_token: config.enrollment_token.take(),
            allow_guest_bootstrap: options.allow_guest_bootstrap,
            providers: &runtime_providers,
            agents: &agents,
            session_path: None,
            runtime_database_path: &runtime_database_file,
            embedded_runtime: None,
        };
        let guest_project_observer = options.dashboard_url.as_ref().map(|dashboard_url| {
            let dashboard_url = dashboard_url.clone();
            Arc::new(move |guest: &session::GuestProjectSummary| {
                print_guest_management_link(&dashboard_url, guest);
            }) as relay::GuestProjectObserver
        });
        let provider_file = config.agent_providers_file.clone();
        let provider_snapshot = fs::read(&provider_file).unwrap_or_default();
        let result = tokio::select! {
            result = relay::run_agent_gateway_with_session_persistence(
                runtime,
                &mut session,
                session_persistence,
                guest_project_observer,
                None,
                None,
            ) => Some(result),
            () = wait_for_provider_config_change(&provider_file, &provider_snapshot) => None,
            () = wait_for_shutdown(&mut shutdown) => return Ok(()),
        };
        let Some(result) = result else {
            println!("Agent provider configuration changed; reconnecting.");
            continue;
        };
        if let Err(error) = result {
            wait_for_provider_retry(&format!("Agent Gateway stopped ({error})."), &mut shutdown)
                .await;
        }
    }
}

async fn wait_for_provider_config_change(path: &Path, snapshot: &[u8]) {
    loop {
        tokio::time::sleep(Duration::from_secs(2)).await;
        if fs::read(path).unwrap_or_default() != snapshot {
            return;
        }
    }
}

async fn wait_for_provider_retry(message: &str, shutdown: &mut watch::Receiver<bool>) {
    eprintln!(
        "{message} Waiting {}s before retrying.",
        PROVIDER_RETRY_DELAY.as_secs()
    );
    tokio::select! {
        _ = tokio::time::sleep(PROVIDER_RETRY_DELAY) => {},
        () = wait_for_shutdown(shutdown) => {},
    }
}

async fn wait_for_shutdown(shutdown: &mut watch::Receiver<bool>) {
    if *shutdown.borrow() {
        return;
    }
    let _ = shutdown.changed().await;
}

pub async fn status(
    config: &Config,
    dashboard_url: Option<&str>,
    session_scope: &str,
) -> Result<(), String> {
    println!("Vifu Agent Gateway status");
    println!("State: {}", config.home_dir.display());
    print_server_config(config)?;
    println!();
    println!("Providers");
    if config.agent_providers.is_empty() || !has_supported_gateway_provider(config) {
        print_agent_provider_config(config);
    }
    print_llama_provider_status(config)?;
    print_local_whisper_provider_status(config)?;
    print_openai_compatible_provider_status(config).await;
    print_agent_provider_status(config).await;
    print_stored_session(config, dashboard_url, session_scope)?;
    Ok(())
}

pub async fn doctor(
    config: &Config,
    dashboard_url: Option<&str>,
    session_scope: &str,
) -> Result<(), String> {
    println!("Vifu Agent Gateway doctor");
    println!("State directory: {}", config.home_dir.display());
    print_server_config(config)?;
    println!();
    println!("Providers");
    if config.agent_providers.is_empty() || !has_supported_gateway_provider(config) {
        print_agent_provider_config(config);
    }
    print_llama_provider_status(config)?;
    print_local_whisper_provider_status(config)?;
    print_openai_compatible_provider_status(config).await;
    let providers = print_agent_provider_status(config).await;
    print_stored_session(config, dashboard_url, session_scope)?;
    for (provider, report) in providers {
        match report.status {
            ProbeStatus::Online => {
                match openclaw::discover_agents(&report.endpoint, provider.token.as_deref()).await {
                    Ok(agents) => println!(
                        "OpenClaw provider {}: ready ({} agents)",
                        provider.id,
                        agents.len()
                    ),
                    Err(error) => {
                        println!("OpenClaw provider {}: unavailable ({error})", provider.id)
                    }
                }
            }
            ProbeStatus::Offline(_) => {
                println!(
                    "OpenClaw provider {}: start its Gateway on loopback.",
                    provider.id
                );
            }
            ProbeStatus::Unsupported(_) => {
                println!(
                    "OpenClaw provider {}: use a loopback URL such as http://127.0.0.1:18789",
                    provider.id
                );
            }
        }
    }
    Ok(())
}

fn load_openai_compatible_provider(
    provider: AgentProviderConfig,
) -> Result<
    (
        Arc<dyn relay::AgentGatewayProvider>,
        vifu_gateway::protocol::AgentDescriptor,
    ),
    String,
> {
    let chat_model = provider_config_text(&provider.config, "chatModel")
        .or_else(|| provider_config_text(&provider.config, "model"));
    let embedding_model = provider_config_text(&provider.config, "embeddingModel")
        .or_else(|| provider_config_text(&provider.config, "embModel"));
    if chat_model.is_none() && embedding_model.is_none() {
        return Err(format!(
            "OpenAI-compatible provider {} requires config.chatModel or config.embeddingModel",
            provider.id
        ));
    }

    let mut http_provider =
        providers::HttpCapabilityProvider::new(&provider.id, provider.url.clone(), provider.token)
            .map_err(|error| error.public_message())?;
    let mut capabilities = Vec::new();
    if let Some(model) = chat_model {
        http_provider
            .add_route(
                "chat",
                providers::HttpCapabilityRoute::OpenAiChat {
                    model: model.clone(),
                    persona: serde_json::json!({}),
                },
            )
            .map_err(|error| error.public_message())?;
        capabilities.push("chat".to_string());
    }
    if let Some(model) = embedding_model {
        http_provider
            .add_route(
                "embedding",
                providers::HttpCapabilityRoute::OpenAiEmbedding { model },
            )
            .map_err(|error| error.public_message())?;
        capabilities.push("embedding".to_string());
    }

    let runtime_provider =
        relay::InProcessGatewayProvider::new(provider.id.clone(), Arc::new(http_provider))?;
    let name = provider.name.unwrap_or_else(|| provider.id.clone());
    let includes_chat = capabilities.iter().any(|capability| capability == "chat");
    let input_modalities = provider_input_modalities(&provider.config, includes_chat)?;
    let agent = vifu_gateway::protocol::AgentDescriptor {
        id: provider.id.clone(),
        name,
        metadata: serde_json::json!({
            "providerKey": provider.id,
            "providerType": "vifu-runtime",
            "localProviderType": "openai-compatible",
            "capabilities": capabilities,
            "inputModalities": input_modalities,
        }),
    };
    Ok((Arc::new(runtime_provider), agent))
}

fn provider_config_text(config: &serde_json::Value, field: &str) -> Option<String> {
    config
        .get(field)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn provider_input_modalities(
    config: &serde_json::Value,
    includes_chat: bool,
) -> Result<serde_json::Value, String> {
    let Some(value) = config.get("inputModalities") else {
        return Ok(if includes_chat {
            serde_json::json!(["text", "image"])
        } else {
            serde_json::json!(["text"])
        });
    };
    let modalities = value
        .as_array()
        .ok_or_else(|| {
            "OpenAI-compatible provider config.inputModalities must be an array".to_string()
        })?
        .iter()
        .map(|item| {
            item.as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .ok_or_else(|| {
                    "OpenAI-compatible provider config.inputModalities must contain strings"
                        .to_string()
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if modalities.is_empty() {
        return Err(
            "OpenAI-compatible provider config.inputModalities must not be empty".to_string(),
        );
    }
    Ok(serde_json::json!(modalities))
}

async fn print_agent_provider_status(
    config: &Config,
) -> Vec<(AgentProviderConfig, openclaw::ProbeReport)> {
    let providers = config.openclaw_providers().cloned().collect::<Vec<_>>();
    if providers.is_empty() {
        return Vec::new();
    }
    let mut reports = Vec::with_capacity(providers.len());
    for provider in providers {
        let report = openclaw::probe(&provider.url).await;
        print_openclaw_report(&provider, &report);
        reports.push((provider, report));
    }
    reports
}

fn has_supported_gateway_provider(config: &Config) -> bool {
    config.agent_providers.iter().any(|provider| {
        matches!(
            provider.provider_type.as_str(),
            "llama" | "local-whisper" | "openclaw" | "openai-compatible"
        )
    })
}

async fn print_openai_compatible_provider_status(config: &Config) {
    for provider in config.openai_compatible_providers() {
        let report =
            providers::probe_openai_compatible(&provider.url, provider.token.as_deref()).await;
        print_openai_compatible_report(provider, &report);
    }
}

fn ensure_home_dir(config: &Config) -> Result<(), String> {
    fs::create_dir_all(&config.home_dir).map_err(|error| error.to_string())
}

fn print_agent_provider_config(config: &Config) {
    if config.agent_providers.is_empty() {
        println!(
            "  None configured ({})",
            config.agent_providers_file.display()
        );
    } else {
        println!(
            "  {} configured; no supported provider is available yet",
            config.agent_providers.len()
        );
    }
}

#[cfg(feature = "local-llama")]
fn load_llama_provider(
    provider: AgentProviderConfig,
    base_dir: &Path,
) -> Result<
    (
        Arc<dyn relay::AgentGatewayProvider>,
        vifu_gateway::protocol::AgentDescriptor,
    ),
    String,
> {
    println!("  {} (Llama): loading local model", provider.id);
    let started = Instant::now();
    let llama = LlamaProvider::load_from_provider_config(&provider.config, base_dir)
        .map_err(|error| format!("llama provider {}: {error}", provider.id))?;
    let input_modalities = if llama.supports_vision() {
        serde_json::json!(["text", "image"])
    } else {
        serde_json::json!(["text"])
    };
    let runtime_provider =
        relay::InProcessGatewayProvider::new(provider.id.clone(), Arc::new(llama))?;
    let name = provider.name.unwrap_or_else(|| provider.id.clone());
    let agent = vifu_gateway::protocol::AgentDescriptor {
        id: provider.id.clone(),
        name,
        metadata: serde_json::json!({
            "providerKey": provider.id,
            "providerType": "vifu-runtime",
            "localProviderType": "llama",
            "capabilities": ["chat", "embedding"],
            "inputModalities": input_modalities,
        }),
    };
    println!(
        "  {} (Llama): ready in {}ms",
        agent.id,
        started.elapsed().as_millis()
    );
    Ok((Arc::new(runtime_provider), agent))
}

#[cfg(not(feature = "local-llama"))]
fn load_llama_provider(
    provider: AgentProviderConfig,
    _base_dir: &Path,
) -> Result<
    (
        Arc<dyn relay::AgentGatewayProvider>,
        vifu_gateway::protocol::AgentDescriptor,
    ),
    String,
> {
    Err(format!(
        "llama provider {} requires a Vifu build with the local-llama feature",
        provider.id
    ))
}

#[cfg(feature = "local-llama")]
fn print_llama_provider_status(config: &Config) -> Result<(), String> {
    let base_dir = config
        .agent_providers_file
        .parent()
        .unwrap_or_else(|| Path::new("."));
    for provider in config.llama_providers() {
        let llama = LlamaProviderConfig::from_provider_config(&provider.config, base_dir)
            .map_err(|error| format!("llama provider {}: {error}", provider.id))?;
        let status = if llama.model_path.is_file() {
            "model file ready"
        } else {
            "model file missing"
        };
        println!("  {} (Llama): {status}", provider.id);
    }
    Ok(())
}

#[cfg(not(feature = "local-llama"))]
fn print_llama_provider_status(config: &Config) -> Result<(), String> {
    for provider in config.llama_providers() {
        println!("  {} (Llama): unavailable in this Vifu build", provider.id);
    }
    Ok(())
}

#[cfg(feature = "local-whisper")]
fn load_local_whisper_provider(
    provider: AgentProviderConfig,
    home_dir: &Path,
) -> Result<
    (
        Arc<dyn relay::AgentGatewayProvider>,
        vifu_gateway::protocol::AgentDescriptor,
    ),
    String,
> {
    let model = provider_config_text(&provider.config, "model").ok_or_else(|| {
        format!(
            "local-whisper provider {} requires config.model",
            provider.id
        )
    })?;
    let model_path = providers::resolve_local_model_path(home_dir, &model)?;
    if !model_path.is_file() {
        return Err(format!(
            "local-whisper provider {} model file is missing in ~/.vifu/models: {}",
            provider.id, model
        ));
    }
    let language = provider_config_text(&provider.config, "language");
    let mut local_provider = providers::HttpCapabilityProvider::local(provider.id.clone())
        .map_err(|error| error.public_message())?;
    local_provider
        .add_route(
            "transcription",
            providers::HttpCapabilityRoute::LocalWhisper {
                model_path,
                language,
            },
        )
        .map_err(|error| error.public_message())?;
    let runtime_provider =
        relay::InProcessGatewayProvider::new(provider.id.clone(), Arc::new(local_provider))?;
    let name = provider.name.unwrap_or_else(|| provider.id.clone());
    let agent = vifu_gateway::protocol::AgentDescriptor {
        id: provider.id.clone(),
        name,
        metadata: serde_json::json!({
            "providerKey": provider.id,
            "providerType": "vifu-runtime",
            "localProviderType": "local-whisper",
            "capabilities": ["transcription"],
            "inputModalities": ["audio"],
        }),
    };
    Ok((Arc::new(runtime_provider), agent))
}

#[cfg(not(feature = "local-whisper"))]
fn load_local_whisper_provider(
    provider: AgentProviderConfig,
    _home_dir: &Path,
) -> Result<
    (
        Arc<dyn relay::AgentGatewayProvider>,
        vifu_gateway::protocol::AgentDescriptor,
    ),
    String,
> {
    Err(format!(
        "local-whisper provider {} requires a Vifu build with the local-whisper feature",
        provider.id
    ))
}

#[cfg(feature = "local-whisper")]
fn print_local_whisper_provider_status(config: &Config) -> Result<(), String> {
    for provider in config.local_whisper_providers() {
        let model = provider_config_text(&provider.config, "model").ok_or_else(|| {
            format!(
                "local-whisper provider {} requires config.model",
                provider.id
            )
        })?;
        let model_path = providers::resolve_local_model_path(&config.home_dir, &model)?;
        let status = if model_path.is_file() {
            "model file ready"
        } else {
            "model file missing"
        };
        println!("  {} (Local Whisper): {status}", provider.id);
    }
    Ok(())
}

#[cfg(not(feature = "local-whisper"))]
fn print_local_whisper_provider_status(config: &Config) -> Result<(), String> {
    for provider in config.local_whisper_providers() {
        println!(
            "  {} (Local Whisper): unavailable in this Vifu build",
            provider.id
        );
    }
    Ok(())
}

fn print_openclaw_report(provider: &AgentProviderConfig, report: &openclaw::ProbeReport) {
    match &report.status {
        ProbeStatus::Online => println!(
            "  {} (OpenClaw): connected at {}:{}",
            provider.id, report.endpoint.host, report.endpoint.port
        ),
        ProbeStatus::Offline(reason) => {
            println!("  {} (OpenClaw): offline", provider.id);
            tracing::debug!(
                provider_id = %provider.id,
                endpoint = %format!("{}:{}", report.endpoint.host, report.endpoint.port),
                %reason,
                "OpenClaw provider is offline"
            );
        }
        ProbeStatus::Unsupported(reason) => {
            println!("  {} (OpenClaw): needs configuration", provider.id);
            tracing::debug!(
                provider_id = %provider.id,
                %reason,
                "OpenClaw provider configuration is unsupported"
            );
        }
    }
}

fn print_openai_compatible_report(provider: &AgentProviderConfig, report: &Result<(), String>) {
    match report {
        Ok(()) => println!("  {} (OpenAI-compatible): connected", provider.id),
        Err(reason) => {
            println!("  {} (OpenAI-compatible): offline", provider.id);
            tracing::debug!(
                provider_id = %provider.id,
                %reason,
                "OpenAI-compatible provider is offline"
            );
        }
    }
}

fn print_server_config(config: &Config) -> Result<(), String> {
    println!("Runtime: {}", config.server_url);
    tracing::debug!(
        websocket = %relay::agent_gateway_websocket_url(&config.server_url)?,
        "Agent Gateway transport"
    );
    Ok(())
}

fn print_stored_session(
    config: &Config,
    dashboard_url: Option<&str>,
    session_scope: &str,
) -> Result<(), String> {
    let (store, key) = gateway_session_store(config, session_scope)?;
    match store.load(&key, None, None)? {
        Some(summary) => print_session(&summary, &config.server_url, dashboard_url),
        None => println!("Session: not established"),
    }
    Ok(())
}

fn print_session(session: &SessionSummary, server_url: &str, dashboard_url: Option<&str>) {
    println!();
    println!(
        "Gateway: {}",
        if session.resume_session_id.is_some() {
            "ready"
        } else {
            "registering"
        }
    );
    tracing::debug!(
        gateway_id = ?session.gateway_id,
        machine_id = %session.identity.machine_id,
        resume_session_id = ?session.resume_session_id,
        "Agent Gateway session"
    );
    if let Some(guest) = session.guest_project.as_ref() {
        let endpoint = relay::guest_endpoint_url(server_url, &guest.endpoint_path)
            .unwrap_or_else(|_| guest.endpoint_path.clone());
        println!();
        println!("Project");
        println!("  Name:     {}", guest.project_slug);
        println!("  Endpoint: {endpoint}");
        println!("  Expires:  {}", guest.expires_at);
        if let Some(dashboard_url) = dashboard_url {
            print_guest_management_link(dashboard_url, guest);
        }
    }
}

fn print_guest_management_link(dashboard_url: &str, guest: &session::GuestProjectSummary) {
    match relay::guest_claim_url(dashboard_url, &guest.claim_token) {
        Ok(url) => {
            println!();
            println!("Dashboard");
            println!("  {}", terminal_link(&url));
            println!(
                "  Sign in before {} to claim this project.",
                guest.expires_at
            );
        }
        Err(error) => eprintln!("Dashboard link is unavailable: {error}"),
    }
}

fn terminal_link(url: &str) -> String {
    format_terminal_link(url, io::stdout().is_terminal())
}

fn format_terminal_link(url: &str, hyperlinks: bool) -> String {
    if hyperlinks {
        format!("\u{1b}]8;;{url}\u{1b}\\{url}\u{1b}]8;;\u{1b}\\")
    } else {
        url.to_string()
    }
}

fn load_or_create_session(
    store: &GatewaySessionStore,
    state_key: &str,
) -> Result<SessionSummary, String> {
    match store.load(state_key, None, None)? {
        Some(summary) => Ok(summary),
        None => {
            let summary = SessionSummary::new(
                vifu_gateway::identity::MachineIdentity::generate()?,
                now_unix_seconds()?,
            )?;
            store
                .persistence(state_key, GatewaySecretStorage::Persisted)
                .save(&summary)?;
            Ok(summary)
        }
    }
}

fn gateway_session_store(
    config: &Config,
    session_scope: &str,
) -> Result<(GatewaySessionStore, String), String> {
    let path = config.runtime_database_file();
    let store = GatewaySessionStore::open(path)?;
    let key = gateway_session_state_key(session_scope, &config.server_url)?;
    Ok((store, key))
}

fn now_unix_seconds() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "local-whisper")]
    use super::load_local_whisper_provider;
    use super::{format_terminal_link, load_openai_compatible_provider, AgentProviderConfig};
    use serde_json::json;

    #[test]
    fn dashboard_links_are_clickable_in_supported_terminals() {
        let url = "https://dashboard.vifu.ai/pair#claim_token=redacted";

        assert_eq!(format_terminal_link(url, false), url);
        let linked = format_terminal_link(url, true);
        assert!(linked.starts_with("\u{1b}]8;;https://"));
        assert!(linked.contains(url));
        assert!(linked.ends_with("\u{1b}]8;;\u{1b}\\"));
    }

    #[test]
    fn openai_compatible_provider_is_exposed_through_gateway_runtime() {
        let provider = AgentProviderConfig {
            id: "cloudflare-ai-proxy".to_string(),
            name: Some("Cloudflare AI Proxy".to_string()),
            provider_type: "openai-compatible".to_string(),
            url: "https://provider.example.com/openai/v1".to_string(),
            token: None,
            config: json!({
                "chatModel": "gpt-5.5-mini",
                "embeddingModel": "text-embedding-ada-002",
                "inputModalities": ["text", "image"]
            }),
        };

        let (runtime_provider, agent) = load_openai_compatible_provider(provider).unwrap();

        assert_eq!(runtime_provider.id(), "cloudflare-ai-proxy");
        assert_eq!(runtime_provider.provider_type(), "vifu-runtime");
        assert_eq!(agent.id, "cloudflare-ai-proxy");
        assert_eq!(agent.name, "Cloudflare AI Proxy");
        assert_eq!(agent.metadata["providerKey"], "cloudflare-ai-proxy");
        assert_eq!(agent.metadata["providerType"], "vifu-runtime");
        assert_eq!(agent.metadata["localProviderType"], "openai-compatible");
        assert_eq!(agent.metadata["capabilities"], json!(["chat", "embedding"]));
        assert_eq!(agent.metadata["inputModalities"], json!(["text", "image"]));
    }

    #[cfg(feature = "local-whisper")]
    #[test]
    fn local_whisper_provider_is_exposed_through_gateway_runtime() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let home = std::env::temp_dir().join(format!("vifu-local-whisper-provider-{stamp}"));
        let models = home.join("models");
        std::fs::create_dir_all(&models).unwrap();
        std::fs::write(
            models.join("ggml-base.en.bin"),
            b"synthetic model placeholder",
        )
        .unwrap();
        let provider = AgentProviderConfig {
            id: "local-transcriber".to_string(),
            name: Some("Local Transcriber".to_string()),
            provider_type: "local-whisper".to_string(),
            url: String::new(),
            token: None,
            config: json!({
                "model": "ggml-base.en.bin",
                "language": "en"
            }),
        };

        let (runtime_provider, agent) = load_local_whisper_provider(provider, &home).unwrap();

        assert_eq!(runtime_provider.id(), "local-transcriber");
        assert_eq!(runtime_provider.provider_type(), "vifu-runtime");
        assert_eq!(agent.id, "local-transcriber");
        assert_eq!(agent.name, "Local Transcriber");
        assert_eq!(agent.metadata["providerKey"], "local-transcriber");
        assert_eq!(agent.metadata["providerType"], "vifu-runtime");
        assert_eq!(agent.metadata["localProviderType"], "local-whisper");
        assert_eq!(agent.metadata["capabilities"], json!(["transcription"]));
        assert_eq!(agent.metadata["inputModalities"], json!(["audio"]));
        std::fs::remove_dir_all(home).unwrap();
    }
}
