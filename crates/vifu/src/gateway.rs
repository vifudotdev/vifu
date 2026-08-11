use std::collections::HashMap;
use std::fs;
use std::io::{self, IsTerminal};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::sync::watch;
use uuid::Uuid;

use vifu_gateway::control::RuntimeControlClient;
use vifu_gateway::optimization::SessionRouteOverrides;
use vifu_gateway::session_store::{
    gateway_session_state_key, GatewaySecretStorage, GatewaySessionStore,
};
use vifu_gateway::{config, openclaw, providers, relay, session};

#[cfg(feature = "local-llama")]
use vifu_provider_llama::LlamaProviderConfig;

#[cfg(feature = "local-llama")]
use crate::local_models::{llama_input_modalities, LazyLlamaGatewayProvider, LocalModelPool};

use config::{AgentProviderConfig, Config};
use openclaw::ProbeStatus;
use session::SessionSummary;

use crate::benchmark::OptimizationController;
use crate::monitor::{
    redacted_io_summary, safe_error_message, FeedbackEvent, FeedbackOutcome,
    ProjectProfileRegistration, RegisteredAgent, RuntimeEvent, RuntimeEventSender, RuntimeHealth,
    RuntimeStage, RuntimeTerminal, StageStatus,
};

const PROVIDER_RETRY_DELAY: Duration = Duration::from_secs(10);
const RUNTIME_ROSTER_REFRESH_DELAY: Duration = Duration::from_secs(5);
const CAPTURE_QUEUE_CAPACITY: usize = 4;

type RuntimeRosterTask = Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>;

#[derive(Clone)]
pub(crate) struct GatewayControl {
    optimization: OptimizationController,
    device_pairing: DevicePairingController,
    #[cfg(feature = "local-llama")]
    local_model_pool: LocalModelPool,
}

impl GatewayControl {
    pub(crate) fn new() -> Self {
        let route_overrides = Arc::new(SessionRouteOverrides::default());
        #[cfg(feature = "local-llama")]
        let local_model_pool = LocalModelPool::for_device();
        let optimization = OptimizationController::new(
            route_overrides,
            #[cfg(feature = "local-llama")]
            local_model_pool.clone(),
        );
        Self {
            optimization,
            device_pairing: DevicePairingController::default(),
            #[cfg(feature = "local-llama")]
            local_model_pool,
        }
    }

    pub(crate) fn optimization(&self) -> OptimizationController {
        self.optimization.clone()
    }

    pub(crate) fn device_pairing(&self) -> DevicePairingController {
        self.device_pairing.clone()
    }
}

#[derive(Clone, Default)]
pub(crate) struct DevicePairingController {
    current_guest: Arc<Mutex<Option<GuestPairingContext>>>,
    gateway_credential: Arc<Mutex<Option<String>>>,
}

#[derive(Clone)]
struct GuestPairingContext {
    server_url: String,
    server_certificate_der: Option<Vec<u8>>,
    project_api_key: String,
    claim_token: String,
    project_id: Uuid,
    deployment_id: Uuid,
    claimed: bool,
}

impl DevicePairingController {
    fn clear(&self) {
        if let Ok(mut current) = self.current_guest.lock() {
            *current = None;
        }
        if let Ok(mut credential) = self.gateway_credential.lock() {
            *credential = None;
        }
    }

    fn set_guest(
        &self,
        server_url: &str,
        server_certificate_der: Option<&[u8]>,
        guest: &session::GuestProjectSummary,
    ) {
        if let Ok(mut current) = self.current_guest.lock() {
            *current = Some(GuestPairingContext {
                server_url: server_url.to_string(),
                server_certificate_der: server_certificate_der.map(<[u8]>::to_vec),
                project_api_key: guest.api_key.clone(),
                claim_token: guest.claim_token.clone(),
                project_id: guest.project_id,
                deployment_id: guest.deployment_id,
                claimed: false,
            });
        }
    }

    fn set_project_claimed(&self, project_id: Uuid, claimed: bool) {
        if let Ok(mut current) = self.current_guest.lock() {
            if let Some(context) = current
                .as_mut()
                .filter(|context| context.project_id == project_id)
            {
                context.claimed = claimed;
            }
        }
    }

    fn set_gateway_credential(&self, credential: &str) {
        if let Ok(mut current) = self.gateway_credential.lock() {
            *current = Some(credential.to_string());
        }
    }

    pub(crate) fn external_guest_claim_url(
        &self,
        dashboard_url: &str,
    ) -> Result<Option<String>, String> {
        let context = self
            .current_guest
            .lock()
            .map_err(|_| "Guest project state is unavailable".to_string())?
            .clone();
        let Some(context) = context else {
            return Ok(None);
        };
        if context.claimed {
            return Ok(None);
        }
        if same_web_origin(&context.server_url, dashboard_url) {
            return Ok(None);
        }
        relay::guest_claim_url(dashboard_url, &context.claim_token).map(Some)
    }

    pub(crate) async fn create_enrollment(
        &self,
    ) -> Result<vifu_gateway::control::GuestGatewayEnrollment, String> {
        let context = self
            .current_guest
            .lock()
            .map_err(|_| "Guest device pairing state is unavailable".to_string())?
            .clone()
            .ok_or_else(|| {
                "Waiting for the Agent Gateway to create its Guest project".to_string()
            })?;
        let gateway_credential = self
            .gateway_credential
            .lock()
            .map_err(|_| "Agent Gateway authorization state is unavailable".to_string())?
            .clone();
        if let Some(gateway_credential) = gateway_credential {
            return RuntimeControlClient::create_peer_gateway_enrollment_with_server_certificate(
                &context.server_url,
                &gateway_credential,
                context.deployment_id,
                context.server_certificate_der.as_deref(),
            )
            .await;
        }
        RuntimeControlClient::create_guest_gateway_enrollment_with_server_certificate(
            &context.server_url,
            &context.project_api_key,
            context.server_certificate_der.as_deref(),
        )
        .await
    }
}

fn same_web_origin(left: &str, right: &str) -> bool {
    fn origin(value: &str) -> Option<&str> {
        let value = value.trim();
        let scheme_end = value.find("://")?;
        if !matches!(&value[..scheme_end], "http" | "https") {
            return None;
        }
        let authority_start = scheme_end + 3;
        let authority_end = value[authority_start..]
            .find(['/', '?', '#'])
            .map_or(value.len(), |offset| authority_start + offset);
        (authority_end > authority_start).then_some(&value[..authority_end])
    }

    origin(left)
        .zip(origin(right))
        .is_some_and(|(left, right)| left.eq_ignore_ascii_case(right))
}

fn restore_device_pairing(
    controller: &DevicePairingController,
    server_url: &str,
    server_certificate_der: Option<&[u8]>,
    guest: Option<&session::GuestProjectSummary>,
) {
    if let Some(guest) = guest {
        controller.set_guest(server_url, server_certificate_der, guest);
    }
}

#[derive(Debug, Clone)]
pub struct GatewayRuntimeOptions {
    pub server_url: String,
    pub server_certificate_der: Option<Vec<u8>>,
    pub allow_guest_bootstrap: bool,
    pub enrollment_token: Option<String>,
    pub session_scope: String,
}

impl GatewayRuntimeOptions {
    pub fn load_config(&self) -> Result<Config, String> {
        Config::load_with_implicit_local_bootstrap(
            self.server_url.clone(),
            self.enrollment_token.clone(),
            !self.allow_guest_bootstrap,
        )
    }
}

type LoadedModelNotifier = Arc<dyn Fn() + Send + Sync>;
type ProviderModels = Arc<HashMap<(String, String), String>>;

fn provider_models(agents: &[vifu_gateway::protocol::AgentDescriptor]) -> ProviderModels {
    let mut models = HashMap::new();
    for agent in agents {
        let provider_key = agent
            .metadata
            .get("providerKey")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(&agent.id);
        let Some(configured_models) = agent
            .metadata
            .get("models")
            .and_then(serde_json::Value::as_object)
        else {
            continue;
        };
        for (capability, model) in configured_models {
            let Some(model) = model
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            models
                .entry((provider_key.to_string(), capability.clone()))
                .or_insert_with(|| model.to_string());
        }
    }
    Arc::new(models)
}

fn runtime_backends(agents: &[vifu_gateway::protocol::AgentDescriptor]) -> Vec<String> {
    let mut backends = Vec::new();
    for agent in agents {
        let metadata = &agent.metadata;
        let local_type = metadata
            .get("localProviderType")
            .and_then(serde_json::Value::as_str);
        let provider_type = metadata
            .get("providerType")
            .and_then(serde_json::Value::as_str);
        let execution = metadata
            .get("executionLocation")
            .and_then(serde_json::Value::as_str);
        let label = match local_type {
            Some("llama") => "llama.cpp",
            Some("local-whisper") => "whisper.cpp",
            Some("openai-compatible") if execution == Some("local") => "OpenAI-compatible (local)",
            Some("openai-compatible") => "OpenAI-compatible (remote)",
            Some("openclaw") => "OpenClaw",
            _ if provider_type == Some("openclaw") => "OpenClaw",
            _ => provider_type.unwrap_or("unknown"),
        };
        let label = label.trim();
        if !label.is_empty()
            && label.len() <= 48
            && !label.chars().any(char::is_control)
            && !backends.iter().any(|existing| existing == label)
        {
            backends.push(label.to_string());
        }
    }
    backends
}

fn project_profile_registrations(
    deployment: &vifu_gateway::control::RuntimeDeploymentAgents,
) -> Vec<ProjectProfileRegistration> {
    let provider = format!("{}/{}", deployment.project_slug, deployment.deployment);
    deployment
        .agents
        .iter()
        .map(|agent| {
            let mut capabilities = agent.capabilities.clone();
            capabilities.sort();
            capabilities.dedup();
            ProjectProfileRegistration {
                id: agent.id.to_string(),
                name: if agent.name.trim().is_empty() {
                    agent.slug.clone()
                } else {
                    agent.name.clone()
                },
                provider: provider.clone(),
                capabilities,
                model: agent.slug.clone(),
            }
        })
        .collect()
}

fn gateway_runtime_observer(
    monitor: Option<RuntimeEventSender>,
    loaded_model_notifier: Option<LoadedModelNotifier>,
    optimization: OptimizationController,
) -> relay::GatewayRuntimeObserver {
    Arc::new(move |event| match event {
        relay::GatewayRuntimeEvent::ConnectionStatus { state, message } => {
            send_monitor(
                monitor.as_ref(),
                RuntimeEvent::HealthChanged {
                    health: match state {
                        relay::GatewayConnectionState::Connected
                        | relay::GatewayConnectionState::Degraded => RuntimeHealth::Live,
                        relay::GatewayConnectionState::Reconnecting => RuntimeHealth::Reconnecting,
                        relay::GatewayConnectionState::AuthorizationRequired => {
                            RuntimeHealth::Starting
                        }
                    },
                    message,
                },
            );
        }
        relay::GatewayRuntimeEvent::InvocationStarted {
            request_id,
            agent_id,
            profile_name,
            profile_id,
            binding_id,
            provider_key,
            capability,
            model,
            model_parameters,
            started_unix_ms,
            ..
        } => {
            #[cfg(feature = "local-llama")]
            optimization.note_active_route(&binding_id.to_string(), &provider_key);
            #[cfg(not(feature = "local-llama"))]
            let _ = binding_id;
            send_monitor(
                monitor.as_ref(),
                RuntimeEvent::InvocationStarted {
                    invocation_id: request_id,
                    agent_id: profile_id.to_string(),
                    agent_name: profile_name,
                    source_agent_id: agent_id,
                    capability,
                    provider: provider_key,
                    model: model.unwrap_or_else(|| "unknown".to_string()),
                    started_unix_ms,
                },
            );
            send_monitor(
                monitor.as_ref(),
                RuntimeEvent::InvocationMetadata {
                    invocation_id: request_id,
                    model_parameters,
                },
            );
        }
        relay::GatewayRuntimeEvent::ProviderStage {
            request_id,
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
            ..
        } => {
            let status = match status {
                vifu_gateway::protocol::TraceStageStatus::Started => StageStatus::Active,
                vifu_gateway::protocol::TraceStageStatus::Completed => StageStatus::Passed,
                vifu_gateway::protocol::TraceStageStatus::Failed => StageStatus::Failed,
            };
            send_monitor(
                monitor.as_ref(),
                RuntimeEvent::StageChanged {
                    invocation_id: request_id,
                    observation_id,
                    stage: provider_stage(stage),
                    status,
                    start_offset: Duration::from_millis(start_offset_ms),
                    end_offset: end_offset_ms.map(Duration::from_millis),
                    elapsed: Duration::from_millis(elapsed_ms.unwrap_or_default()),
                    request_elapsed: request_elapsed_ms.map(Duration::from_millis),
                    input_tokens,
                    output_tokens,
                    resident,
                    error,
                },
            );
        }
        relay::GatewayRuntimeEvent::InvocationFinished {
            request_id,
            elapsed_ms,
            terminal,
            error,
            delivery_observation,
        } => {
            if let Some(delivery) = delivery_observation {
                send_monitor(
                    monitor.as_ref(),
                    RuntimeEvent::StageChanged {
                        invocation_id: request_id,
                        observation_id: delivery.observation_id,
                        stage: RuntimeStage::Deliver,
                        status: match delivery.status {
                            vifu_gateway::protocol::TraceDeliveryStatus::Delivered => {
                                StageStatus::Passed
                            }
                            vifu_gateway::protocol::TraceDeliveryStatus::Failed => {
                                StageStatus::Failed
                            }
                        },
                        start_offset: Duration::from_millis(delivery.start_offset_ms),
                        end_offset: Some(Duration::from_millis(delivery.end_offset_ms)),
                        elapsed: Duration::from_millis(delivery.elapsed_ms),
                        request_elapsed: Some(Duration::from_millis(delivery.end_offset_ms)),
                        input_tokens: None,
                        output_tokens: None,
                        resident: None,
                        error: delivery.error.as_deref().map(safe_error_message),
                    },
                );
            }
            send_monitor(
                monitor.as_ref(),
                RuntimeEvent::InvocationFinished {
                    invocation_id: request_id,
                    elapsed: Duration::from_millis(elapsed_ms),
                    terminal: runtime_terminal(terminal),
                    error: error.as_deref().map(safe_error_message),
                },
            );
            if let Some(notify_loaded_models) = loaded_model_notifier.as_ref() {
                notify_loaded_models();
            }
        }
        relay::GatewayRuntimeEvent::ApplicationFeedback {
            request_id,
            observation_id,
            start_offset_ms,
            end_offset_ms,
            feedback,
        } => {
            send_monitor(
                monitor.as_ref(),
                RuntimeEvent::ApplicationFeedback {
                    invocation_id: request_id,
                    observation_id,
                    start_offset: Duration::from_millis(start_offset_ms),
                    end_offset: Duration::from_millis(end_offset_ms),
                    event: match feedback.event {
                        vifu_gateway::protocol::ApplicationFeedbackEvent::OutputAccepted => {
                            FeedbackEvent::OutputAccepted
                        }
                        vifu_gateway::protocol::ApplicationFeedbackEvent::ActionApplied => {
                            FeedbackEvent::ActionApplied
                        }
                        vifu_gateway::protocol::ApplicationFeedbackEvent::FramePresented => {
                            FeedbackEvent::FramePresented
                        }
                    },
                    outcome: match feedback.outcome {
                        vifu_gateway::protocol::ApplicationFeedbackOutcome::Pass => {
                            FeedbackOutcome::Pass
                        }
                        vifu_gateway::protocol::ApplicationFeedbackOutcome::Fail => {
                            FeedbackOutcome::Fail
                        }
                        vifu_gateway::protocol::ApplicationFeedbackOutcome::Unknown => {
                            FeedbackOutcome::Unknown
                        }
                        vifu_gateway::protocol::ApplicationFeedbackOutcome::NotApplicable => {
                            FeedbackOutcome::NotApplicable
                        }
                    },
                    message: feedback.message.map(|message| safe_error_message(&message)),
                    path: feedback.path,
                },
            );
        }
        relay::GatewayRuntimeEvent::CaptureDropped {
            config_epoch,
            request_id,
            binding_id,
            capability,
            provider_key,
        } => {
            optimization.note_capture_dropped(
                config_epoch,
                request_id,
                Some(binding_id.to_string()),
                Some(capability),
                Some(provider_key),
            );
            send_monitor(
                monitor.as_ref(),
                RuntimeEvent::IoDropped {
                    invocation_id: request_id,
                },
            );
        }
        relay::GatewayRuntimeEvent::InvocationCancelled {
            config_epoch,
            request_id,
        } => {
            optimization.discard_capture(config_epoch, request_id);
            send_monitor(
                monitor.as_ref(),
                RuntimeEvent::InvocationCancelled {
                    invocation_id: request_id,
                },
            );
        }
    })
}

fn runtime_terminal(terminal: relay::GatewayInvocationTerminal) -> RuntimeTerminal {
    match terminal {
        relay::GatewayInvocationTerminal::Delivered => RuntimeTerminal::Delivered,
        relay::GatewayInvocationTerminal::ProviderFailed => RuntimeTerminal::ProviderFailed,
        relay::GatewayInvocationTerminal::TimedOut => RuntimeTerminal::TimedOut,
        relay::GatewayInvocationTerminal::DeliveryFailed => RuntimeTerminal::DeliveryFailed,
        relay::GatewayInvocationTerminal::PreflightFailed => RuntimeTerminal::PreflightFailed,
    }
}

async fn run_capture_worker(
    optimization: OptimizationController,
    monitor: Option<RuntimeEventSender>,
    mut captures: tokio::sync::mpsc::Receiver<relay::GatewayCaptureEvent>,
) {
    while let Some(capture) = captures.recv().await {
        let monitor_event = monitor.as_ref().and_then(|_| match &capture {
            relay::GatewayCaptureEvent::InvocationStarted {
                request_id, input, ..
            } => {
                let (input, truncated) = redacted_io_summary(input);
                Some(RuntimeEvent::IoCaptured {
                    invocation_id: *request_id,
                    input: Some(input),
                    output: None,
                    truncated,
                })
            }
            relay::GatewayCaptureEvent::InvocationFinished {
                request_id,
                output: Some(output),
                ..
            } => {
                let (output, truncated) = redacted_io_summary(output);
                Some(RuntimeEvent::IoCaptured {
                    invocation_id: *request_id,
                    input: None,
                    output: Some(output),
                    truncated,
                })
            }
            relay::GatewayCaptureEvent::InvocationFinished { .. }
            | relay::GatewayCaptureEvent::InvocationCancelled { .. } => None,
        });
        if let (Some(monitor), Some(event)) = (monitor.as_ref(), monitor_event) {
            let _ = monitor.send(event);
        }
        let _ = optimization.capture(capture);
    }
}

fn provider_stage(stage: relay::ProviderStage) -> RuntimeStage {
    match stage {
        relay::ProviderStage::Queue => RuntimeStage::Queue,
        relay::ProviderStage::Load => RuntimeStage::Load,
        relay::ProviderStage::Tokenize => RuntimeStage::Tokenize,
        relay::ProviderStage::Prefill => RuntimeStage::Prefill,
        relay::ProviderStage::FirstToken => RuntimeStage::FirstToken,
        relay::ProviderStage::Decode => RuntimeStage::Decode,
        relay::ProviderStage::Validate => RuntimeStage::Validate,
    }
}

fn send_monitor(monitor: Option<&RuntimeEventSender>, event: RuntimeEvent) {
    if let Some(monitor) = monitor {
        let _ = monitor.send(event);
    }
}

fn mark_gateway_authorized(monitor: Option<&RuntimeEventSender>) {
    send_monitor(
        monitor,
        RuntimeEvent::HealthChanged {
            health: RuntimeHealth::Live,
            message: None,
        },
    );
}

pub async fn run(
    options: GatewayRuntimeOptions,
    mut shutdown: watch::Receiver<bool>,
    monitor: Option<RuntimeEventSender>,
    control: GatewayControl,
) -> Result<(), String> {
    let verbose = monitor.is_none();
    if verbose {
        println!("Vifu");
    }
    send_monitor(
        monitor.as_ref(),
        RuntimeEvent::HealthChanged {
            health: RuntimeHealth::Starting,
            message: None,
        },
    );
    let mut previous_provider_snapshot: Option<Vec<u8>> = None;
    loop {
        if *shutdown.borrow() {
            return Ok(());
        }
        control.optimization.clear_runtime_control();
        control.device_pairing.clear();
        let mut config = options.load_config()?;
        let provider_file = config.agent_providers_file.clone();
        let provider_snapshot = fs::read(&provider_file).unwrap_or_default();
        let provider_configuration_changed = previous_provider_snapshot
            .as_deref()
            .is_none_or(|previous| previous != provider_snapshot.as_slice());
        ensure_home_dir(&config)?;
        if verbose {
            print_server_config(&config)?;
        }
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
        if verbose {
            println!();
            println!("Providers");
            if config.agent_providers.is_empty() {
                print_agent_provider_config(&config);
            }
        }
        let has_configured_providers = !openclaw_providers.is_empty()
            || !llama_providers.is_empty()
            || !local_whisper_providers.is_empty()
            || !openai_compatible_providers.is_empty();
        let mut runtime_providers: Vec<Arc<dyn relay::AgentGatewayProvider>> = Vec::new();
        let mut agents = Vec::new();
        for provider in llama_providers {
            let base_dir = config
                .agent_providers_file
                .parent()
                .unwrap_or_else(|| Path::new("."));
            #[cfg(feature = "local-llama")]
            let registered = register_llama_provider(
                provider,
                base_dir,
                control.local_model_pool.clone(),
                verbose,
            );
            #[cfg(not(feature = "local-llama"))]
            let registered = register_llama_provider(provider, base_dir, verbose);
            let (runtime_provider, agent) = registered?;
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
            if verbose {
                print_openai_compatible_report(&provider, &probe);
            }
            let Some((runtime_provider, agent)) =
                load_available_openai_compatible_provider(provider, &probe)
            else {
                continue;
            };
            runtime_providers.push(runtime_provider);
            agents.push(agent);
        }
        for provider in openclaw_providers {
            let report = openclaw::probe(&provider.url).await;
            if verbose {
                print_openclaw_report(&provider, &report);
            }
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
                    tracing::warn!(
                        provider_id = %provider.id,
                        error = %safe_error_message(&error),
                        "OpenClaw provider agent discovery is unavailable"
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
            send_monitor(
                monitor.as_ref(),
                RuntimeEvent::HealthChanged {
                    health: RuntimeHealth::Reconnecting,
                    message: Some("No configured Agent Provider is reachable".to_string()),
                },
            );
            wait_for_provider_retry(
                "No configured Agent Provider is reachable.",
                &mut shutdown,
                verbose,
            )
            .await;
            continue;
        }
        if verbose {
            println!(
                "  Ready: {} providers, {} agents",
                runtime_providers.len(),
                agents.len()
            );
        }
        let config_epoch = if provider_configuration_changed {
            control.optimization.configure(&runtime_providers, &agents)
        } else {
            control
                .optimization
                .refresh_providers(&runtime_providers, &agents)
        };
        previous_provider_snapshot = Some(provider_snapshot.clone());
        if let Some(monitor) = monitor.as_ref() {
            let registrations = agents
                .iter()
                .flat_map(RegisteredAgent::from_descriptor)
                .collect::<Vec<_>>();
            let _ = monitor.send(RuntimeEvent::AgentsRegistered(registrations));
            let _ = monitor.send(RuntimeEvent::BackendsChanged(runtime_backends(&agents)));
            #[cfg(feature = "local-llama")]
            {
                let _ = monitor.send(RuntimeEvent::LoadedModelsChanged(
                    control.local_model_pool.loaded_count(),
                ));
            }
        }
        let runtime_database_file = config.runtime_database_file();
        let session_store = GatewaySessionStore::open(&runtime_database_file)?;
        let session_key = gateway_session_state_key(&options.session_scope, &config.server_url)?;
        let mut session = load_or_create_session(&session_store, &session_key)?;
        restore_device_pairing(
            &control.device_pairing,
            &config.server_url,
            options.server_certificate_der.as_deref(),
            session.guest_project.as_ref(),
        );
        send_monitor(
            monitor.as_ref(),
            RuntimeEvent::IdentityChanged {
                project: session
                    .guest_project
                    .as_ref()
                    .map(|project| project.project_slug.clone()),
                deployment: session
                    .guest_project
                    .as_ref()
                    .map(|project| project.deployment.clone()),
            },
        );
        if verbose {
            print_session(&session, &config.server_url);
        }
        let session_persistence =
            session_store.persistence(session_key, GatewaySecretStorage::Persisted);
        #[cfg(feature = "local-llama")]
        let loaded_model_notifier: Option<LoadedModelNotifier> = monitor.as_ref().map(|monitor| {
            let monitor = monitor.clone();
            let pool = control.local_model_pool.clone();
            Arc::new(move || {
                let _ = monitor.send(RuntimeEvent::LoadedModelsChanged(pool.loaded_count()));
            }) as LoadedModelNotifier
        });
        #[cfg(not(feature = "local-llama"))]
        let loaded_model_notifier: Option<LoadedModelNotifier> = None;
        let models = provider_models(&agents);
        let runtime_observer = Some(gateway_runtime_observer(
            monitor.clone(),
            loaded_model_notifier.clone(),
            control.optimization.clone(),
        ));
        let (capture_sender, capture_receiver) = tokio::sync::mpsc::channel(CAPTURE_QUEUE_CAPACITY);
        let capture_worker = tokio::spawn(run_capture_worker(
            control.optimization.clone(),
            monitor.clone(),
            capture_receiver,
        ));
        let runtime = relay::AgentGatewayRuntime {
            server_url: &config.server_url,
            server_certificate_der: options.server_certificate_der.as_deref(),
            agent_gateway_bootstrap_token: config.agent_gateway_bootstrap_token.as_deref(),
            enrollment_token: config.enrollment_token.take(),
            allow_guest_bootstrap: options.allow_guest_bootstrap,
            providers: &runtime_providers,
            agents: &agents,
            route_overrides: Some(control.optimization.route_overrides()),
            runtime_observer,
            capture_sender: Some(capture_sender.clone()),
            config_epoch,
            provider_models: Some(models),
            session_path: None,
            runtime_database_path: &runtime_database_file,
            embedded_runtime: None,
            embedded_monitor: None,
            output_policy: if verbose {
                relay::GatewayOutputPolicy::Terminal
            } else {
                relay::GatewayOutputPolicy::Observer
            },
        };
        let guest_project_observer = match monitor.clone() {
            None => {
                let server_url = config.server_url.clone();
                let server_certificate_der = options.server_certificate_der.clone();
                let device_pairing = control.device_pairing.clone();
                Some(Arc::new(move |guest: &session::GuestProjectSummary| {
                    device_pairing.set_guest(&server_url, server_certificate_der.as_deref(), guest);
                    print_guest_management_link(&server_url, guest);
                }) as relay::GuestProjectObserver)
            }
            Some(monitor) => {
                let server_url = config.server_url.clone();
                let server_certificate_der = options.server_certificate_der.clone();
                let device_pairing = control.device_pairing.clone();
                Some(Arc::new(move |guest: &session::GuestProjectSummary| {
                    device_pairing.set_guest(&server_url, server_certificate_der.as_deref(), guest);
                    send_monitor(
                        Some(&monitor),
                        RuntimeEvent::IdentityChanged {
                            project: Some(guest.project_slug.clone()),
                            deployment: Some(guest.deployment.clone()),
                        },
                    );
                }) as relay::GuestProjectObserver)
            }
        };
        let runtime_roster_task = RuntimeRosterTask::default();
        let authorization_observer = {
            let server_url = config.server_url.clone();
            let optimization = control.optimization.clone();
            let device_pairing = control.device_pairing.clone();
            let monitor = monitor.clone();
            let runtime_roster_task = Arc::clone(&runtime_roster_task);
            Some(
                Arc::new(move |summary: &relay::GatewayAuthorizationSummary| {
                    stop_runtime_roster_task(&runtime_roster_task);
                    mark_gateway_authorized(monitor.as_ref());
                    device_pairing.set_gateway_credential(&summary.device_token);
                    if optimization
                        .configure_runtime_control(
                            &server_url,
                            summary.gateway_id.clone(),
                            summary.device_token.clone(),
                        )
                        .is_err()
                    {
                        optimization.clear_runtime_control();
                        return;
                    }
                    let Some(monitor) = monitor.clone() else {
                        return;
                    };
                    let Ok(client) =
                        RuntimeControlClient::new(&server_url, summary.device_token.clone())
                    else {
                        return;
                    };
                    let gateway_id = summary.gateway_id.clone();
                    let roster_device_pairing = device_pairing.clone();
                    let task = tokio::spawn(async move {
                        if !publish_runtime_project_roster(
                            &client,
                            &gateway_id,
                            &monitor,
                            &roster_device_pairing,
                        )
                        .await
                        {
                            return;
                        }
                        loop {
                            tokio::time::sleep(RUNTIME_ROSTER_REFRESH_DELAY).await;
                            if !publish_runtime_project_roster(
                                &client,
                                &gateway_id,
                                &monitor,
                                &roster_device_pairing,
                            )
                            .await
                            {
                                return;
                            }
                        }
                    });
                    replace_runtime_roster_task(&runtime_roster_task, task);
                }) as relay::GatewayAuthorizationObserver,
            )
        };
        let result = tokio::select! {
            result = relay::run_agent_gateway_with_session_persistence(
                runtime,
                &mut session,
                session_persistence,
                guest_project_observer,
                authorization_observer,
                None,
            ) => Some(result),
            () = wait_for_provider_config_change(&provider_file, &provider_snapshot) => None,
            () = wait_for_shutdown(&mut shutdown) => {
                stop_runtime_roster_task(&runtime_roster_task);
                return Ok(());
            },
        };
        stop_runtime_roster_task(&runtime_roster_task);
        drop(capture_sender);
        if result.is_none() {
            capture_worker.abort();
        }
        let _ = capture_worker.await;
        let Some(result) = result else {
            if verbose {
                println!("Agent provider configuration changed; reconnecting.");
            }
            send_monitor(
                monitor.as_ref(),
                RuntimeEvent::HealthChanged {
                    health: RuntimeHealth::Reconnecting,
                    message: Some("Agent provider configuration changed".to_string()),
                },
            );
            continue;
        };
        if let Err(error) = result {
            send_monitor(
                monitor.as_ref(),
                RuntimeEvent::HealthChanged {
                    health: RuntimeHealth::Reconnecting,
                    message: Some(safe_error_message(&error)),
                },
            );
            wait_for_provider_retry(
                &format!("Agent Gateway stopped ({error})."),
                &mut shutdown,
                verbose,
            )
            .await;
        }
    }
}

async fn publish_runtime_project_roster(
    client: &RuntimeControlClient,
    gateway_id: &str,
    monitor: &RuntimeEventSender,
    device_pairing: &DevicePairingController,
) -> bool {
    let Ok(Ok(configuration)) =
        tokio::time::timeout(Duration::from_secs(10), client.configuration()).await
    else {
        return true;
    };
    if configuration.gateway_id != gateway_id {
        return false;
    }
    let mut primary = configuration
        .deployments
        .iter()
        .filter(|deployment| deployment.is_primary);
    let Some(selected) = primary.next() else {
        return true;
    };
    if primary.next().is_some() {
        return true;
    }
    device_pairing.set_project_claimed(selected.project_id, selected.project_claimed);
    let _ = monitor.send(RuntimeEvent::IdentityChanged {
        project: Some(selected.project_slug.clone()),
        deployment: Some(selected.deployment.clone()),
    });

    let Ok(Ok(roster)) =
        tokio::time::timeout(Duration::from_secs(10), client.runtime_agents()).await
    else {
        return true;
    };
    if roster.gateway_id != gateway_id {
        return false;
    }
    let profiles = roster
        .deployments
        .iter()
        .find(|deployment| deployment.deployment_id == selected.deployment_id)
        .map(project_profile_registrations)
        .unwrap_or_default();
    let _ = monitor.send(RuntimeEvent::ProjectProfilesRegistered(profiles));
    true
}

fn replace_runtime_roster_task(
    slot: &Mutex<Option<tokio::task::JoinHandle<()>>>,
    task: tokio::task::JoinHandle<()>,
) {
    let mut slot = slot.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(previous) = slot.replace(task) {
        previous.abort();
    }
}

fn stop_runtime_roster_task(slot: &Mutex<Option<tokio::task::JoinHandle<()>>>) {
    let mut slot = slot.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(task) = slot.take() {
        task.abort();
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

async fn wait_for_provider_retry(
    message: &str,
    shutdown: &mut watch::Receiver<bool>,
    verbose: bool,
) {
    if verbose {
        eprintln!(
            "{message} Waiting {}s before retrying.",
            PROVIDER_RETRY_DELAY.as_secs()
        );
    } else {
        tracing::warn!(
            message = %safe_error_message(message),
            retry_seconds = PROVIDER_RETRY_DELAY.as_secs(),
            "Agent Gateway will retry"
        );
    }
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

pub async fn status(config: &Config, session_scope: &str) -> Result<(), String> {
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
    print_stored_session(config, session_scope)?;
    Ok(())
}

pub async fn doctor(config: &Config, session_scope: &str) -> Result<(), String> {
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
    print_stored_session(config, session_scope)?;
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
    let chat_model_metadata = chat_model.clone();
    let embedding_model_metadata = embedding_model.clone();
    // Replay eligibility is a security boundary: a user-provided label must
    // never turn a network endpoint into a local optimization target. A
    // loopback proxy may still opt out when it forwards requests elsewhere.
    let configured_location = provider_config_text(&provider.config, "executionLocation")
        .filter(|location| matches!(location.as_str(), "local" | "remote"));
    let execution_location = if config::is_local_provider_url(&provider.url)
        && configured_location.as_deref() != Some("remote")
    {
        "local"
    } else {
        "remote"
    };

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
        name: name.clone(),
        metadata: serde_json::json!({
            "providerKey": provider.id,
            "providerName": name,
            "providerType": "vifu-runtime",
            "localProviderType": "openai-compatible",
            "executionLocation": execution_location,
            "capabilities": capabilities,
            "inputModalities": input_modalities,
            "models": {
                "chat": chat_model_metadata,
                "embedding": embedding_model_metadata,
            },
            "modelLoaded": false,
        }),
    };
    Ok((Arc::new(runtime_provider), agent))
}

fn should_register_openai_compatible(
    provider: &AgentProviderConfig,
    probe: &Result<(), String>,
) -> bool {
    let explicitly_remote =
        provider_config_text(&provider.config, "executionLocation").as_deref() == Some("remote");
    probe.is_ok() || (!explicitly_remote && config::is_local_provider_url(&provider.url))
}

fn load_available_openai_compatible_provider(
    provider: AgentProviderConfig,
    probe: &Result<(), String>,
) -> Option<(
    Arc<dyn relay::AgentGatewayProvider>,
    vifu_gateway::protocol::AgentDescriptor,
)> {
    if !should_register_openai_compatible(&provider, probe) {
        return None;
    }
    let provider_id = provider.id.clone();
    match load_openai_compatible_provider(provider) {
        Ok(provider) => Some(provider),
        Err(error) => {
            tracing::warn!(
                provider_id = %provider_id,
                error = %safe_error_message(&error),
                "OpenAI-compatible provider configuration is unavailable"
            );
            None
        }
    }
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
fn register_llama_provider(
    provider: AgentProviderConfig,
    base_dir: &Path,
    pool: LocalModelPool,
    verbose: bool,
) -> Result<
    (
        Arc<dyn relay::AgentGatewayProvider>,
        vifu_gateway::protocol::AgentDescriptor,
    ),
    String,
> {
    let model_name = provider_config_text(&provider.config, "modelPath")
        .and_then(|path| {
            Path::new(&path)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| provider.id.clone());
    let input_modalities = llama_input_modalities(&provider.config);
    let runtime_provider =
        LazyLlamaGatewayProvider::new(provider.id.clone(), provider.config, base_dir, pool)?;
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
            "models": {"chat": model_name.clone(), "embedding": model_name},
            "modelLoaded": false,
        }),
    };
    if verbose {
        println!(
            "  {} (Llama): ready; model loads on first request",
            agent.id
        );
    } else {
        tracing::debug!(provider_id = %agent.id, "Llama provider is ready; model loads on first request");
    }
    Ok((Arc::new(runtime_provider), agent))
}

#[cfg(not(feature = "local-llama"))]
fn register_llama_provider(
    provider: AgentProviderConfig,
    _base_dir: &Path,
    _verbose: bool,
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
            "models": {"transcription": model},
            "modelLoaded": false,
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

fn print_stored_session(config: &Config, session_scope: &str) -> Result<(), String> {
    let (store, key) = gateway_session_store(config, session_scope)?;
    match store.load(&key, None, None)? {
        Some(summary) => print_session(&summary, &config.server_url),
        None => println!("Session: not established"),
    }
    Ok(())
}

fn print_session(session: &SessionSummary, server_url: &str) {
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
        print_guest_management_link(server_url, guest);
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
    use std::sync::Arc;
    use std::time::Duration;

    #[cfg(feature = "local-whisper")]
    use super::load_local_whisper_provider;
    use super::{
        format_terminal_link, gateway_runtime_observer, load_available_openai_compatible_provider,
        load_openai_compatible_provider, mark_gateway_authorized, project_profile_registrations,
        restore_device_pairing, run_capture_worker, runtime_backends,
        should_register_openai_compatible, AgentProviderConfig, DevicePairingController,
    };
    use crate::monitor::{RuntimeEvent, RuntimeHealth, RuntimeStage, StageStatus};
    use serde_json::json;
    use uuid::Uuid;

    fn optimization() -> crate::benchmark::OptimizationController {
        crate::benchmark::OptimizationController::new(
            Arc::new(vifu_gateway::optimization::SessionRouteOverrides::default()),
            #[cfg(feature = "local-llama")]
            crate::local_models::LocalModelPool::for_device(),
        )
    }

    fn guest_project() -> vifu_gateway::session::GuestProjectSummary {
        vifu_gateway::session::GuestProjectSummary {
            project_id: Uuid::nil(),
            app_id: String::new(),
            project_slug: "guest-test".to_string(),
            deployment_id: Uuid::nil(),
            deployment: "development".to_string(),
            endpoint_path: "/guest-test/v1".to_string(),
            api_key: "synthetic-project-key".to_string(),
            claim_token: "synthetic-claim-token".to_string(),
            expires_at: "2099-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn persisted_guest_restores_device_pairing_context() {
        let controller = DevicePairingController::default();
        let guest = guest_project();

        restore_device_pairing(
            &controller,
            "https://192.0.2.20:6790",
            Some(&[1, 2, 3]),
            Some(&guest),
        );

        let current = controller.current_guest.lock().unwrap().clone().unwrap();
        assert_eq!(
            (
                current.server_url,
                current.server_certificate_der,
                current.project_api_key,
                current.claim_token,
            ),
            (
                "https://192.0.2.20:6790".to_string(),
                Some(vec![1, 2, 3]),
                "synthetic-project-key".to_string(),
                "synthetic-claim-token".to_string(),
            )
        );
    }

    #[test]
    fn external_dashboard_uses_the_guest_claim_link() {
        let controller = DevicePairingController::default();
        let guest = guest_project();
        restore_device_pairing(&controller, "https://api.vifu.dev", None, Some(&guest));

        assert_eq!(
            controller
                .external_guest_claim_url("https://dashboard.vifu.dev")
                .unwrap()
                .as_deref(),
            Some("https://dashboard.vifu.dev/pair#claim_token=synthetic-claim-token")
        );
    }

    #[test]
    fn claimed_guest_opens_the_project_instead_of_reusing_the_claim_link() {
        let controller = DevicePairingController::default();
        let guest = guest_project();
        restore_device_pairing(&controller, "https://api.vifu.dev", None, Some(&guest));

        controller.set_project_claimed(guest.project_id, true);

        assert!(controller
            .external_guest_claim_url("https://dashboard.vifu.dev")
            .unwrap()
            .is_none());
    }

    #[test]
    fn same_origin_dashboard_keeps_the_local_project_link() {
        let controller = DevicePairingController::default();
        let guest = guest_project();
        restore_device_pairing(&controller, "https://192.0.2.20:6790", None, Some(&guest));

        assert!(controller
            .external_guest_claim_url("https://192.0.2.20:6790/console")
            .unwrap()
            .is_none());
    }

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
    fn authorized_gateway_transition_marks_the_runtime_live() {
        let (sender, mut receiver) = crate::monitor::runtime_event_channel();

        mark_gateway_authorized(Some(&sender));

        assert!(matches!(
            receiver.try_recv().unwrap(),
            RuntimeEvent::HealthChanged {
                health: RuntimeHealth::Live,
                message: None,
            }
        ));
    }

    #[test]
    fn project_roster_should_register_one_lane_per_agent() {
        let deployment = vifu_gateway::control::RuntimeDeploymentAgents {
            deployment_id: Uuid::nil(),
            deployment: "development".to_string(),
            project_id: Uuid::nil(),
            project_slug: "stardew-valley".to_string(),
            project_name: "Stardew Valley".to_string(),
            is_primary: true,
            agents: vec![vifu_gateway::control::RuntimeProjectAgent {
                id: Uuid::new_v4(),
                slug: "farmhand".to_string(),
                name: "Farmhand".to_string(),
                capabilities: vec!["embedding".to_string(), "chat".to_string()],
            }],
        };

        let registrations = project_profile_registrations(&deployment);

        assert_eq!(registrations.len(), 1);
        assert_eq!(
            registrations[0].capabilities,
            vec!["chat".to_string(), "embedding".to_string()]
        );
    }

    #[test]
    fn gateway_connection_status_reaches_the_tui_health_channel() {
        let (sender, mut receiver) = crate::monitor::runtime_event_channel();
        let observer = gateway_runtime_observer(Some(sender), None, optimization());

        observer(vifu_gateway::relay::GatewayRuntimeEvent::ConnectionStatus {
            state: vifu_gateway::relay::GatewayConnectionState::Reconnecting,
            message: Some("connection lost; retrying".to_string()),
        });

        assert!(matches!(
            receiver.try_recv().unwrap(),
            RuntimeEvent::HealthChanged {
                health: RuntimeHealth::Reconnecting,
                message: Some(message),
            } if message == "connection lost; retrying"
        ));
    }

    #[test]
    fn gateway_observer_should_preserve_canonical_id_and_profile_lane_identity() {
        let (sender, mut receiver) = crate::monitor::runtime_event_channel();
        let observer = gateway_runtime_observer(Some(sender), None, optimization());
        let request_id = Uuid::new_v4();
        let profile_id = Uuid::new_v4();
        observer(
            vifu_gateway::relay::GatewayRuntimeEvent::InvocationStarted {
                request_id,
                endpoint_id: Uuid::new_v4(),
                profile_id,
                binding_id: Uuid::new_v4(),
                agent_id: "local-qwen".to_string(),
                profile_name: "Stardew Valley combat/0".to_string(),
                provider_key: "llama-local".to_string(),
                capability: "chat".to_string(),
                model: Some("qwen-new:2b".to_string()),
                model_parameters: serde_json::json!({"temperature": 0.2}),
                timeout_ms: 5_000,
                started_unix_ms: 42,
            },
        );

        let RuntimeEvent::InvocationStarted {
            invocation_id,
            agent_id,
            agent_name,
            source_agent_id,
            model,
            ..
        } = receiver.try_recv().unwrap()
        else {
            panic!("expected invocation start event");
        };

        assert_eq!(invocation_id, request_id);
        assert_eq!(agent_id, profile_id.to_string());
        assert_eq!(agent_name, "Stardew Valley combat/0");
        assert_eq!(source_agent_id, "local-qwen");
        assert_eq!(model, "qwen-new:2b");
        assert!(matches!(
            receiver.try_recv().unwrap(),
            RuntimeEvent::InvocationMetadata {
                invocation_id,
                model_parameters,
            } if invocation_id == request_id
                && model_parameters == serde_json::json!({"temperature": 0.2})
        ));
    }

    #[test]
    fn gateway_observer_preserves_the_delivery_observation_id_and_timing() {
        let (sender, mut receiver) = crate::monitor::runtime_event_channel();
        let observer = gateway_runtime_observer(Some(sender), None, optimization());
        let request_id = Uuid::new_v4();
        let observation_id = Uuid::new_v4();

        observer(
            vifu_gateway::relay::GatewayRuntimeEvent::InvocationFinished {
                request_id,
                elapsed_ms: 31,
                terminal: vifu_gateway::relay::GatewayInvocationTerminal::Delivered,
                error: None,
                delivery_observation: Some(vifu_gateway::relay::GatewayDeliveryObservation {
                    observation_id,
                    status: vifu_gateway::protocol::TraceDeliveryStatus::Delivered,
                    start_offset_ms: 20,
                    end_offset_ms: 31,
                    elapsed_ms: 11,
                    error: None,
                }),
            },
        );

        assert!(matches!(
            receiver.try_recv().unwrap(),
            RuntimeEvent::StageChanged {
                invocation_id,
                observation_id: delivered_id,
                stage: RuntimeStage::Deliver,
                status: StageStatus::Passed,
                start_offset,
                end_offset,
                elapsed,
                ..
            } if invocation_id == request_id
                && delivered_id == observation_id
                && start_offset == Duration::from_millis(20)
                && end_offset == Some(Duration::from_millis(31))
                && elapsed == Duration::from_millis(11)
        ));
        assert!(matches!(
            receiver.try_recv().unwrap(),
            RuntimeEvent::InvocationFinished { invocation_id, .. } if invocation_id == request_id
        ));
    }

    #[test]
    fn gateway_observer_preserves_server_feedback_offsets() {
        let (sender, mut receiver) = crate::monitor::runtime_event_channel();
        let observer = gateway_runtime_observer(Some(sender), None, optimization());
        let request_id = Uuid::new_v4();
        let observation_id = Uuid::new_v4();

        observer(
            vifu_gateway::relay::GatewayRuntimeEvent::ApplicationFeedback {
                request_id,
                observation_id,
                start_offset_ms: 83,
                end_offset_ms: 83,
                feedback: vifu_gateway::protocol::ApplicationFeedback {
                    event: vifu_gateway::protocol::ApplicationFeedbackEvent::FramePresented,
                    outcome: vifu_gateway::protocol::ApplicationFeedbackOutcome::Pass,
                    message: None,
                    path: Some("$.frame".to_string()),
                },
            },
        );

        assert!(matches!(
            receiver.try_recv().unwrap(),
            RuntimeEvent::ApplicationFeedback {
                invocation_id,
                observation_id: received_id,
                start_offset,
                end_offset,
                ..
            } if invocation_id == request_id
                && received_id == observation_id
                && start_offset == Duration::from_millis(83)
                && end_offset == Duration::from_millis(83)
        ));
    }

    #[tokio::test]
    async fn monitor_io_does_not_depend_on_optimization_capture_acceptance() {
        let (monitor, mut events) = crate::monitor::runtime_event_channel();
        let (captures, receiver) = tokio::sync::mpsc::channel(1);
        let request_id = Uuid::new_v4();
        captures
            .send(
                vifu_gateway::relay::GatewayCaptureEvent::InvocationStarted {
                    config_epoch: u64::MAX,
                    request_id,
                    binding_id: Uuid::new_v4(),
                    agent_id: "unsupported-agent".to_string(),
                    provider_key: "unsupported-provider".to_string(),
                    capability: "chat".to_string(),
                    binding: Arc::new(json!({})),
                    input: Arc::new(json!({
                        "message": "hello",
                        "apiKey": "private",
                    })),
                    timeout_ms: 1_000,
                },
            )
            .await
            .unwrap();
        drop(captures);

        run_capture_worker(optimization(), Some(monitor), receiver).await;

        assert!(matches!(
            events.try_recv().unwrap(),
            RuntimeEvent::IoCaptured {
                invocation_id,
                input: Some(input),
                ..
            } if invocation_id == request_id
                && input["message"] == "hello"
                && input["apiKey"] == "[REDACTED]"
        ));
    }

    #[test]
    fn runtime_backend_inventory_names_the_actual_local_engines() {
        let agents = vec![
            vifu_gateway::protocol::AgentDescriptor {
                id: "chat".to_string(),
                name: "Chat".to_string(),
                metadata: serde_json::json!({"localProviderType": "llama"}),
            },
            vifu_gateway::protocol::AgentDescriptor {
                id: "speech".to_string(),
                name: "Speech".to_string(),
                metadata: serde_json::json!({"localProviderType": "local-whisper"}),
            },
        ];

        assert_eq!(runtime_backends(&agents), vec!["llama.cpp", "whisper.cpp"]);
    }

    #[test]
    fn gateway_observer_should_translate_typed_provider_stages() {
        let (sender, mut receiver) = crate::monitor::runtime_event_channel();
        let observer = gateway_runtime_observer(Some(sender), None, optimization());
        let request_id = Uuid::new_v4();
        let observation_id = Uuid::new_v4();
        observer(vifu_gateway::relay::GatewayRuntimeEvent::ProviderStage {
            request_id,
            observation_id,
            stage: vifu_gateway::relay::ProviderStage::Prefill,
            status: vifu_gateway::protocol::TraceStageStatus::Completed,
            start_offset_ms: 0,
            end_offset_ms: Some(120),
            elapsed_ms: Some(120),
            request_elapsed_ms: None,
            input_tokens: Some(12),
            output_tokens: None,
            resident: None,
            error: None,
        });

        let RuntimeEvent::StageChanged {
            invocation_id,
            observation_id: received_observation_id,
            stage,
            status,
            start_offset,
            end_offset,
            elapsed,
            input_tokens,
            ..
        } = receiver.try_recv().unwrap()
        else {
            panic!("expected provider stage event");
        };

        assert_eq!(invocation_id, request_id);
        assert_eq!(received_observation_id, observation_id);
        assert_eq!(stage, RuntimeStage::Prefill);
        assert_eq!(status, StageStatus::Passed);
        assert_eq!(start_offset, Duration::ZERO);
        assert_eq!(end_offset, Some(Duration::from_millis(120)));
        assert_eq!(elapsed, Duration::from_millis(120));
        assert_eq!(input_tokens, Some(12));
    }

    #[test]
    fn openai_compatible_provider_is_exposed_through_gateway_runtime() {
        let provider = AgentProviderConfig {
            id: "openai-compatible-test-proxy".to_string(),
            name: Some("OpenAI Compatible Test Proxy".to_string()),
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

        assert_eq!(runtime_provider.id(), "openai-compatible-test-proxy");
        assert_eq!(runtime_provider.provider_type(), "vifu-runtime");
        assert_eq!(agent.id, "openai-compatible-test-proxy");
        assert_eq!(agent.name, "OpenAI Compatible Test Proxy");
        assert_eq!(
            agent.metadata["providerKey"],
            "openai-compatible-test-proxy"
        );
        assert_eq!(
            agent.metadata["providerName"],
            "OpenAI Compatible Test Proxy"
        );
        assert_eq!(agent.metadata["providerType"], "vifu-runtime");
        assert_eq!(agent.metadata["localProviderType"], "openai-compatible");
        assert_eq!(agent.metadata["executionLocation"], "remote");
        assert_eq!(agent.metadata["capabilities"], json!(["chat", "embedding"]));
        assert_eq!(agent.metadata["inputModalities"], json!(["text", "image"]));
    }

    #[test]
    fn invalid_openai_compatible_provider_does_not_stop_the_gateway() {
        let provider = AgentProviderConfig {
            id: "incomplete-openai-compatible".to_string(),
            name: None,
            provider_type: "openai-compatible".to_string(),
            url: "https://provider.example.com/v1".to_string(),
            token: None,
            config: json!({}),
        };

        assert!(load_available_openai_compatible_provider(provider, &Ok(())).is_none());
    }

    #[test]
    fn loopback_openai_compatible_provider_is_marked_local_for_safe_replay() {
        let provider = AgentProviderConfig {
            id: "loopback-openai".to_string(),
            name: None,
            provider_type: "openai-compatible".to_string(),
            url: "http://127.0.0.1:11434/v1".to_string(),
            token: None,
            config: json!({"chatModel": "qwen2.5:2b", "embeddingModel": "nomic-embed"}),
        };

        let (_, agent) = load_openai_compatible_provider(provider).unwrap();

        assert_eq!(agent.metadata["localProviderType"], "openai-compatible");
        assert_eq!(agent.metadata["executionLocation"], "local");
        assert_eq!(agent.metadata["capabilities"], json!(["chat", "embedding"]));
    }

    #[test]
    fn remote_openai_compatible_provider_cannot_claim_local_replay() {
        let provider = AgentProviderConfig {
            id: "claimed-local".to_string(),
            name: None,
            provider_type: "openai-compatible".to_string(),
            url: "https://provider.example.com/v1".to_string(),
            token: None,
            config: json!({
                "chatModel": "hosted-model",
                "executionLocation": "local"
            }),
        };

        let (_, agent) = load_openai_compatible_provider(provider.clone()).unwrap();

        assert_eq!(agent.metadata["executionLocation"], "remote");
        assert!(!should_register_openai_compatible(
            &provider,
            &Err("offline".to_string())
        ));
    }

    #[test]
    fn loopback_openai_compatible_proxy_can_opt_out_of_local_replay() {
        let provider = AgentProviderConfig {
            id: "loopback-cloud-proxy".to_string(),
            name: None,
            provider_type: "openai-compatible".to_string(),
            url: "http://127.0.0.1:11434/v1".to_string(),
            token: None,
            config: json!({
                "chatModel": "hosted-model",
                "executionLocation": "remote"
            }),
        };

        let (_, agent) = load_openai_compatible_provider(provider.clone()).unwrap();

        assert_eq!(agent.metadata["executionLocation"], "remote");
        assert!(!should_register_openai_compatible(
            &provider,
            &Err("offline".to_string())
        ));
    }

    #[test]
    fn offline_local_openai_compatible_provider_remains_available_for_benchmarking() {
        let local = AgentProviderConfig {
            id: "offline-local".to_string(),
            name: None,
            provider_type: "openai-compatible".to_string(),
            url: "http://127.0.0.1:11434/v1".to_string(),
            token: None,
            config: json!({"chatModel": "qwen2.5:2b"}),
        };
        let remote = AgentProviderConfig {
            id: "offline-remote".to_string(),
            name: None,
            provider_type: "openai-compatible".to_string(),
            url: "https://provider.example.com/v1".to_string(),
            token: None,
            config: json!({"chatModel": "remote-model"}),
        };
        let offline = Err("connection refused".to_string());

        assert!(should_register_openai_compatible(&local, &offline));
        assert!(!should_register_openai_compatible(&remote, &offline));
        assert!(should_register_openai_compatible(&remote, &Ok(())));
    }

    #[cfg(feature = "local-whisper")]
    #[test]
    fn missing_local_whisper_model_is_registered_for_explicit_load_failure() {
        let provider = AgentProviderConfig {
            id: "missing-local-transcriber".to_string(),
            name: None,
            provider_type: "local-whisper".to_string(),
            url: String::new(),
            token: None,
            config: json!({"model": "missing.bin"}),
        };
        let home = std::env::temp_dir().join("vifu-missing-local-whisper-provider");

        let (runtime_provider, agent) = load_local_whisper_provider(provider, &home).unwrap();

        assert_eq!(runtime_provider.id(), "missing-local-transcriber");
        assert_eq!(agent.metadata["models"]["transcription"], "missing.bin");
        assert_eq!(agent.metadata["modelLoaded"], false);
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
