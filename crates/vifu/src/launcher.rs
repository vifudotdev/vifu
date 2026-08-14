use std::collections::HashMap;
use std::io::IsTerminal;
use std::process::Command as ProcessCommand;
use std::time::Duration;

use tokio::sync::watch;
use tracing_subscriber::EnvFilter;
use vifu_gateway::control::RuntimeControlClient;
use vifu_server::config::Config as ServerConfig;

use crate::cli::{help_text, Command, Options};
use crate::gateway;
use crate::monitor::{
    runtime_event_channel, RegisteredAgent, RuntimeEvent, RuntimeEventSender, RuntimeHealth,
    RuntimeStage, RuntimeTerminal, StageStatus,
};
use crate::runtime_config::LoadedRuntimeConfig;
use crate::tui;

pub async fn execute(options: Options) -> Result<(), String> {
    let interactive_start = matches!(&options.command, Command::Start) && tui::should_run();
    init_tracing(interactive_start);
    let Options {
        command,
        config_profile,
        config_overrides,
        open_browser,
        server_only,
    } = options;
    let load_config = || LoadedRuntimeConfig::load(config_profile.as_deref(), &config_overrides);
    match command {
        Command::Help => {
            print!("{}", help_text());
            Ok(())
        }
        Command::Version => {
            println!("vifu {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Command::Start => start(load_config()?, open_browser, server_only).await,
        Command::Status => status(load_config()?).await,
        Command::Doctor => doctor(load_config()?).await,
    }
}

async fn start(
    config: LoadedRuntimeConfig,
    open_browser: bool,
    server_only: bool,
) -> Result<(), String> {
    if server_only {
        if !config.server_is_local()? {
            return Err("--server-only requires a configured local Server".to_string());
        }
        return run_server_only(config, open_browser).await;
    }
    match start_plan(config.server_is_local()?, config.gateway_is_local()?) {
        StartPlan::Combined => run_combined(config, open_browser).await,
        StartPlan::ServerOnly => run_server_only(config, open_browser).await,
        StartPlan::GatewayOnly => run_gateway_only(config).await,
        StartPlan::RemoteServerOnly => run_remote_server_only(config, open_browser).await,
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum StartPlan {
    Combined,
    ServerOnly,
    GatewayOnly,
    RemoteServerOnly,
}

fn start_plan(server_is_local: bool, gateway_is_local: bool) -> StartPlan {
    match (server_is_local, gateway_is_local) {
        (true, true) => StartPlan::Combined,
        (true, false) => StartPlan::ServerOnly,
        (false, true) => StartPlan::GatewayOnly,
        (false, false) => StartPlan::RemoteServerOnly,
    }
}

async fn status(config: LoadedRuntimeConfig) -> Result<(), String> {
    println!("Vifu runtime");
    println!("Configuration: {}", config.path.display());
    if let Some(profile) = config.profile.as_ref() {
        println!("Profile: {profile}");
    }
    let gateway_is_local = config.gateway_is_local()?;
    println!("Server: {}", role_status(config.config.server.is_some()));
    println!(
        "Agent Gateway: {}",
        role_status(config.config.gateway.is_some())
    );
    if gateway_is_local {
        let gateway_options = config.gateway_options()?;
        gateway::migrate_legacy_session(&gateway_options)?;
        let session_scope = gateway_options.session_scope.clone();
        let gateway = gateway_options.load_config()?;
        gateway::status(&gateway, &session_scope).await?;
    } else if let Some(gateway) = config.config.gateway.as_ref() {
        println!("Agent Gateway location: remote ({})", gateway.address);
    }
    Ok(())
}

async fn doctor(config: LoadedRuntimeConfig) -> Result<(), String> {
    println!("Vifu runtime doctor");
    println!("Configuration: {}", config.path.display());
    if let Some(profile) = config.profile.as_ref() {
        println!("Profile: {profile}");
    }
    if config.gateway_is_local()? {
        let gateway_options = config.gateway_options()?;
        gateway::migrate_legacy_session(&gateway_options)?;
        let session_scope = gateway_options.session_scope.clone();
        let gateway = gateway_options.load_config()?;
        gateway::doctor(&gateway, &session_scope).await?;
    } else if let Some(gateway) = config.config.gateway.as_ref() {
        println!("Agent Gateway: remote ({})", gateway.address);
    } else {
        println!("Agent Gateway: not configured");
    }
    Ok(())
}

fn role_status(enabled: bool) -> &'static str {
    if enabled {
        "configured"
    } else {
        "not configured"
    }
}

async fn run_combined(config: LoadedRuntimeConfig, open_browser: bool) -> Result<(), String> {
    if tui::should_run() {
        return run_combined_tui(config).await;
    }
    let server_config = config.server_config()?;
    let console_url = local_console_url(&server_config);
    let mut gateway_options = config.gateway_options()?;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let server_state = vifu_server::connect(server_config)
        .await
        .map_err(|error| error.to_string())?;
    prepare_local_app(&config, &server_state, Some(&mut gateway_options)).await?;
    apply_local_server_certificate(
        &mut gateway_options,
        server_state.server_endpoint.as_deref(),
    )?;
    let mut server = tokio::spawn(vifu_server::serve_state(
        server_state,
        wait_for_shutdown(shutdown_rx.clone()),
    ));
    announce_console(console_url, open_browser);
    let gateway_control = gateway::GatewayControl::new();
    let mut gateway = tokio::spawn(gateway::run(
        gateway_options,
        shutdown_rx,
        None,
        gateway_control,
    ));
    let outcome = tokio::select! {
        () = shutdown_signal() => CombinedRuntimeOutcome::Shutdown,
        result = &mut server => CombinedRuntimeOutcome::Server(result),
        result = &mut gateway => CombinedRuntimeOutcome::Gateway(result),
    };
    let _ = shutdown_tx.send(true);
    match outcome {
        CombinedRuntimeOutcome::Shutdown => {
            let _ = server.await;
            let _ = gateway.await;
            Ok(())
        }
        CombinedRuntimeOutcome::Server(result) => {
            let _ = gateway.await;
            join_result("Vifu Server", result)
        }
        CombinedRuntimeOutcome::Gateway(result) => {
            let _ = server.await;
            join_result("Vifu Agent Gateway", result)
        }
        CombinedRuntimeOutcome::Tui(result) => result,
    }
}

async fn run_server_only(config: LoadedRuntimeConfig, open_browser: bool) -> Result<(), String> {
    if tui::should_run() {
        return run_server_only_tui(config).await;
    }
    let server_config = config.server_config()?;
    let console_url = local_console_url(&server_config);
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let server_state = vifu_server::connect(server_config)
        .await
        .map_err(|error| error.to_string())?;
    prepare_local_app(&config, &server_state, None).await?;
    let mut server = tokio::spawn(vifu_server::serve_state(
        server_state,
        wait_for_shutdown(shutdown_rx),
    ));
    announce_console(console_url, open_browser);
    let outcome = tokio::select! {
        () = shutdown_signal() => SingleRuntimeOutcome::Shutdown,
        result = &mut server => SingleRuntimeOutcome::Role(result),
    };
    let _ = shutdown_tx.send(true);
    match outcome {
        SingleRuntimeOutcome::Shutdown => {
            let _ = server.await;
            Ok(())
        }
        SingleRuntimeOutcome::Role(result) => join_result("Vifu Server", result),
        SingleRuntimeOutcome::Tui(result) => result,
    }
}

async fn run_gateway_only(config: LoadedRuntimeConfig) -> Result<(), String> {
    if tui::should_run() {
        return run_gateway_only_tui(config).await;
    }
    let gateway_options = config.gateway_options()?;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let gateway_control = gateway::GatewayControl::new();
    let mut gateway = tokio::spawn(gateway::run(
        gateway_options,
        shutdown_rx,
        None,
        gateway_control,
    ));
    let outcome = tokio::select! {
        () = shutdown_signal() => SingleRuntimeOutcome::Shutdown,
        result = &mut gateway => SingleRuntimeOutcome::Role(result),
    };
    let _ = shutdown_tx.send(true);
    match outcome {
        SingleRuntimeOutcome::Shutdown => {
            let _ = gateway.await;
            Ok(())
        }
        SingleRuntimeOutcome::Role(result) => join_result("Vifu Agent Gateway", result),
        SingleRuntimeOutcome::Tui(result) => result,
    }
}

async fn run_remote_server_only(
    config: LoadedRuntimeConfig,
    open_browser: bool,
) -> Result<(), String> {
    let server_address = config.server_address()?.to_string();
    let dashboard_url = remote_dashboard_url(&config).await;
    if tui::should_run() {
        let (monitor_tx, monitor_rx) = runtime_event_channel();
        let credential = RemoteMonitorCredential::Static(remote_monitor_credential()?);
        let remote_monitor = tokio::spawn(stream_remote_server_monitor(
            server_address.clone(),
            credential,
            monitor_tx,
        ));
        let result = tui::run(monitor_rx, dashboard_url, None, None)
            .await
            .map_err(|error| format!("Vifu TUI failed: {error}"));
        remote_monitor.abort();
        return result;
    }
    announce_console(dashboard_url, open_browser);
    shutdown_signal().await;
    Ok(())
}

async fn stream_remote_server_monitor(
    server_address: String,
    credential: RemoteMonitorCredential,
    monitor: RuntimeEventSender,
) {
    let (server_events, receiver) = tokio::sync::broadcast::channel(2_048);
    let bridge = tokio::spawn(bridge_server_monitor(receiver, monitor.clone()));
    loop {
        let credential = match credential.current() {
            Ok(Some(credential)) => credential,
            Ok(None) => {
                let _ = monitor.send(RuntimeEvent::HealthChanged {
                    health: RuntimeHealth::Starting,
                    message: Some("Waiting for Guest project monitor authorization".to_string()),
                });
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
            Err(error) => {
                let _ = monitor.send(RuntimeEvent::HealthChanged {
                    health: RuntimeHealth::Reconnecting,
                    message: Some(crate::monitor::safe_error_message(&error)),
                });
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
        };
        let _ = monitor.send(RuntimeEvent::HealthChanged {
            health: RuntimeHealth::Reconnecting,
            message: Some("Connecting to Vifu Server".to_string()),
        });
        match vifu_server::monitor::RemoteMonitorClient::connect(&server_address, &credential).await
        {
            Ok(mut client) => {
                let _ = monitor.send(RuntimeEvent::HealthChanged {
                    health: RuntimeHealth::Live,
                    message: None,
                });
                loop {
                    match client.next_event().await {
                        Ok(Some(event)) => {
                            let _ = server_events.send(event);
                        }
                        Ok(None) => break,
                        Err(error) => {
                            let _ = monitor.send(RuntimeEvent::HealthChanged {
                                health: RuntimeHealth::Reconnecting,
                                message: Some(crate::monitor::safe_error_message(&error)),
                            });
                            break;
                        }
                    }
                }
            }
            Err(error) => {
                let _ = monitor.send(RuntimeEvent::HealthChanged {
                    health: RuntimeHealth::Reconnecting,
                    message: Some(crate::monitor::safe_error_message(&error)),
                });
            }
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
        if bridge.is_finished() {
            break;
        }
    }
    bridge.abort();
}

async fn run_combined_tui(config: LoadedRuntimeConfig) -> Result<(), String> {
    let server_config = config.server_config()?;
    let console_url = local_console_url(&server_config);
    let mut gateway_options = config.gateway_options()?;
    let dashboard_url = console_url.clone();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let (monitor_tx, monitor_rx) = runtime_event_channel();
    let server_state = vifu_server::connect(server_config)
        .await
        .map_err(|error| error.to_string())?;
    prepare_local_app(&config, &server_state, Some(&mut gateway_options)).await?;
    apply_local_server_certificate(
        &mut gateway_options,
        server_state.server_endpoint.as_deref(),
    )?;
    let server_monitor = server_state.monitor.subscribe();
    let monitor_bridge = tokio::spawn(bridge_server_monitor(server_monitor, monitor_tx.clone()));
    let (local_monitor_tx, local_monitor_rx) = runtime_event_channel();
    let local_monitor_bridge =
        tokio::spawn(bridge_local_gateway_monitor(local_monitor_rx, monitor_tx));
    let gateway_control = gateway::GatewayControl::new();
    let optimization = gateway_control.optimization();
    let device_pairing = gateway_control.device_pairing();
    let mut server = tokio::spawn(vifu_server::serve_state(
        server_state,
        wait_for_shutdown(shutdown_rx.clone()),
    ));
    let mut gateway = tokio::spawn(gateway::run(
        gateway_options,
        shutdown_rx,
        Some(local_monitor_tx),
        gateway_control,
    ));
    let mut terminal = Box::pin(tui::run(
        monitor_rx,
        dashboard_url,
        Some(optimization),
        Some(device_pairing),
    ));
    let outcome = tokio::select! {
        () = shutdown_signal() => CombinedRuntimeOutcome::Shutdown,
        result = &mut terminal => CombinedRuntimeOutcome::Tui(result),
        result = &mut server => CombinedRuntimeOutcome::Server(result),
        result = &mut gateway => CombinedRuntimeOutcome::Gateway(result),
    };
    let _ = shutdown_tx.send(true);
    monitor_bridge.abort();
    local_monitor_bridge.abort();
    match outcome {
        CombinedRuntimeOutcome::Shutdown | CombinedRuntimeOutcome::Tui(Ok(())) => {
            let _ = server.await;
            let _ = gateway.await;
            Ok(())
        }
        CombinedRuntimeOutcome::Tui(Err(error)) => {
            let _ = server.await;
            let _ = gateway.await;
            Err(format!("Vifu TUI failed: {error}"))
        }
        CombinedRuntimeOutcome::Server(result) => {
            let _ = gateway.await;
            join_result("Vifu Server", result)
        }
        CombinedRuntimeOutcome::Gateway(result) => {
            let _ = server.await;
            join_result("Vifu Agent Gateway", result)
        }
    }
}

async fn run_server_only_tui(config: LoadedRuntimeConfig) -> Result<(), String> {
    let server_config = config.server_config()?;
    let console_url = local_console_url(&server_config);
    let readiness_addr = server_config.addr;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let (monitor_tx, monitor_rx) = runtime_event_channel();
    let _ = monitor_tx.send(RuntimeEvent::HealthChanged {
        health: RuntimeHealth::Starting,
        message: None,
    });
    let _ = monitor_tx.send(RuntimeEvent::LoadedModelsChanged(0));
    let readiness_monitor = monitor_tx.clone();
    std::mem::drop(tokio::spawn(async move {
        if wait_for_server(readiness_addr).await.is_ok() {
            let _ = readiness_monitor.send(RuntimeEvent::HealthChanged {
                health: RuntimeHealth::Live,
                message: None,
            });
        }
    }));
    let server_state = vifu_server::connect(server_config)
        .await
        .map_err(|error| error.to_string())?;
    prepare_local_app(&config, &server_state, None).await?;
    let server_monitor = server_state.monitor.subscribe();
    let monitor_bridge = tokio::spawn(bridge_server_monitor(server_monitor, monitor_tx));
    let mut server = tokio::spawn(vifu_server::serve_state(
        server_state,
        wait_for_shutdown(shutdown_rx),
    ));
    let mut terminal = Box::pin(tui::run(monitor_rx, console_url, None, None));
    let outcome = tokio::select! {
        () = shutdown_signal() => SingleRuntimeOutcome::Shutdown,
        result = &mut terminal => SingleRuntimeOutcome::Tui(result),
        result = &mut server => SingleRuntimeOutcome::Role(result),
    };
    let _ = shutdown_tx.send(true);
    monitor_bridge.abort();
    match outcome {
        SingleRuntimeOutcome::Shutdown | SingleRuntimeOutcome::Tui(Ok(())) => {
            let _ = server.await;
            Ok(())
        }
        SingleRuntimeOutcome::Tui(Err(error)) => {
            let _ = server.await;
            Err(format!("Vifu TUI failed: {error}"))
        }
        SingleRuntimeOutcome::Role(result) => join_result("Vifu Server", result),
    }
}

async fn bridge_server_monitor(
    mut receiver: tokio::sync::broadcast::Receiver<vifu_server::monitor::ServerMonitorEvent>,
    sender: RuntimeEventSender,
) {
    let mut invocations = HashMap::<(String, String), uuid::Uuid>::new();
    let mut gateway_agents = HashMap::<String, Vec<RegisteredAgent>>::new();
    loop {
        let event = match receiver.recv().await {
            Ok(event) => event,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(dropped)) => {
                let _ = sender.send(RuntimeEvent::MonitorEventsDropped {
                    dropped_events: usize::try_from(dropped).unwrap_or(usize::MAX),
                });
                continue;
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
        };
        match event {
            vifu_server::monitor::ServerMonitorEvent::Snapshot { gateways } => {
                gateway_agents.clear();
                for gateway in gateways {
                    let agents = gateway
                        .agents
                        .iter()
                        .flat_map(RegisteredAgent::from_descriptor)
                        .map(|mut agent| {
                            agent.id = monitor_agent_id(&gateway.gateway_id, &agent.id);
                            agent
                        })
                        .collect::<Vec<_>>();
                    gateway_agents.insert(gateway.gateway_id, agents);
                }
                let roster = gateway_agents.values().flatten().cloned().collect();
                let _ = sender.send(RuntimeEvent::AgentsRegistered(roster));
            }
            vifu_server::monitor::ServerMonitorEvent::GatewayConnected { gateway_id, agents } => {
                let agents = agents
                    .iter()
                    .flat_map(RegisteredAgent::from_descriptor)
                    .map(|mut agent| {
                        agent.id = monitor_agent_id(&gateway_id, &agent.id);
                        agent
                    })
                    .collect::<Vec<_>>();
                gateway_agents.insert(gateway_id, agents);
                let roster = gateway_agents.values().flatten().cloned().collect();
                let _ = sender.send(RuntimeEvent::AgentsRegistered(roster));
                let _ = sender.send(RuntimeEvent::HealthChanged {
                    health: RuntimeHealth::Live,
                    message: None,
                });
            }
            vifu_server::monitor::ServerMonitorEvent::GatewayDisconnected { gateway_id } => {
                gateway_agents.remove(&gateway_id);
                let roster = gateway_agents.values().flatten().cloned().collect();
                let _ = sender.send(RuntimeEvent::AgentsRegistered(roster));
                let _ = sender.send(RuntimeEvent::HealthChanged {
                    health: if gateway_agents.is_empty() {
                        RuntimeHealth::Reconnecting
                    } else {
                        RuntimeHealth::Live
                    },
                    message: Some(format!("Device Gateway {gateway_id} disconnected")),
                });
            }
            vifu_server::monitor::ServerMonitorEvent::GatewayEnrolled { enrollment_id, .. } => {
                let _ = sender.send(RuntimeEvent::GatewayEnrolled { enrollment_id });
            }
            vifu_server::monitor::ServerMonitorEvent::RuntimeTelemetry { gateway_id, batch } => {
                let invocation_key = (gateway_id.clone(), batch.trace_id.clone());
                let invocation_id = *invocations
                    .entry(invocation_key.clone())
                    .or_insert_with(uuid::Uuid::new_v4);
                let agent_id = monitor_agent_id(&gateway_id, &batch.agent_id);
                let agent_name = gateway_agents
                    .get(&gateway_id)
                    .and_then(|agents| agents.iter().find(|agent| agent.id == agent_id))
                    .map_or_else(|| batch.agent_id.clone(), |agent| agent.name.clone());
                let _ = sender.send(RuntimeEvent::IdentityChanged {
                    project: Some(batch.project_id.clone()),
                    deployment: Some(batch.deployment_id.to_string()),
                });
                if batch.dropped_events > 0 {
                    let _ = sender.send(RuntimeEvent::MonitorEventsDropped {
                        dropped_events: usize::try_from(batch.dropped_events).unwrap_or(usize::MAX),
                    });
                }
                if let Some(event) = runtime_telemetry_io_event(invocation_id, &batch) {
                    let _ = sender.send(event);
                }
                let host_metrics = batch.host_metrics;
                for telemetry in batch.events {
                    match telemetry {
                        vifu_gateway::protocol::TraceTelemetry::InvocationStarted {
                            provider_key,
                            capability,
                            model,
                        } => {
                            let _ = sender.send(RuntimeEvent::InvocationStarted {
                                invocation_id,
                                agent_id: agent_id.clone(),
                                agent_name: agent_name.clone(),
                                source_agent_id: agent_id.clone(),
                                capability,
                                provider: provider_key.clone(),
                                model: model.unwrap_or(provider_key),
                                started_unix_ms: batch.started_at_ms,
                            });
                        }
                        vifu_gateway::protocol::TraceTelemetry::ProviderStage {
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
                            let _ = sender.send(RuntimeEvent::StageChanged {
                                invocation_id,
                                observation_id,
                                stage: runtime_stage(stage),
                                status: match status {
                                    vifu_gateway::protocol::TraceStageStatus::Started => {
                                        StageStatus::Active
                                    }
                                    vifu_gateway::protocol::TraceStageStatus::Completed => {
                                        StageStatus::Passed
                                    }
                                    vifu_gateway::protocol::TraceStageStatus::Failed => {
                                        StageStatus::Failed
                                    }
                                },
                                start_offset: Duration::from_millis(start_offset_ms),
                                end_offset: end_offset_ms.map(Duration::from_millis),
                                elapsed: Duration::from_millis(elapsed_ms.unwrap_or(0)),
                                request_elapsed: request_elapsed_ms.map(Duration::from_millis),
                                input_tokens,
                                output_tokens,
                                resident,
                                error,
                            });
                        }
                        vifu_gateway::protocol::TraceTelemetry::Delivery { .. } => {}
                    }
                }
                if let Some(host_metrics) = host_metrics {
                    let _ = sender.send(RuntimeEvent::RuntimeHostMetrics {
                        invocation_id,
                        process_rss_bytes: host_metrics.process_rss_bytes,
                        total_memory_bytes: host_metrics.total_memory_bytes,
                    });
                }
                if let Some(terminal) = batch.terminal {
                    match terminal.status {
                        vifu_gateway::protocol::RuntimeTelemetryTerminalStatus::Cancelled => {
                            let _ =
                                sender.send(RuntimeEvent::InvocationCancelled { invocation_id });
                        }
                        vifu_gateway::protocol::RuntimeTelemetryTerminalStatus::Completed
                        | vifu_gateway::protocol::RuntimeTelemetryTerminalStatus::Error => {
                            let _ = sender.send(RuntimeEvent::InvocationFinished {
                                invocation_id,
                                elapsed: Duration::from_millis(terminal.duration_ms),
                                terminal: match terminal.status {
                                    vifu_gateway::protocol::RuntimeTelemetryTerminalStatus::Completed => {
                                        RuntimeTerminal::Delivered
                                    }
                                    _ => RuntimeTerminal::ProviderFailed,
                                },
                                error: terminal.error,
                            });
                        }
                    }
                    invocations.remove(&invocation_key);
                }
            }
        }
    }
}

fn runtime_telemetry_io_event(
    invocation_id: uuid::Uuid,
    batch: &vifu_gateway::protocol::RuntimeTelemetryBatch,
) -> Option<RuntimeEvent> {
    let input = batch.root_input_summary.as_ref();
    let output = batch.root_output_summary.as_ref();
    (input.is_some() || output.is_some()).then(|| RuntimeEvent::IoCaptured {
        invocation_id,
        input: input.map(|summary| summary.value.clone()),
        output: output.map(|summary| summary.value.clone()),
        truncated: input.is_some_and(|summary| summary.effective_truncated())
            || output.is_some_and(|summary| summary.effective_truncated()),
    })
}

async fn bridge_local_gateway_monitor(
    mut receiver: crate::monitor::RuntimeEventReceiver,
    sender: RuntimeEventSender,
) {
    while let Some(event) = receiver.recv().await {
        if let Some(event) = local_gateway_monitor_event(event) {
            let _ = sender.send(event);
        }
    }
}

fn local_gateway_monitor_event(event: RuntimeEvent) -> Option<RuntimeEvent> {
    match event {
        // The Server owns the gateway roster and namespaces its provider IDs.
        // Everything else is only available from the local Gateway for normal
        // OpenAI-compatible invocations; filtering those events makes the TUI
        // appear idle even while requests are running.
        RuntimeEvent::AgentsRegistered(_) => None,
        _ => Some(event),
    }
}

fn monitor_agent_id(gateway_id: &str, agent_id: &str) -> String {
    format!("{gateway_id}/{agent_id}")
}

fn runtime_stage(stage: vifu_gateway::relay::ProviderStage) -> RuntimeStage {
    match stage {
        vifu_gateway::relay::ProviderStage::Queue => RuntimeStage::Queue,
        vifu_gateway::relay::ProviderStage::Load => RuntimeStage::Load,
        vifu_gateway::relay::ProviderStage::Tokenize => RuntimeStage::Tokenize,
        vifu_gateway::relay::ProviderStage::Prefill => RuntimeStage::Prefill,
        vifu_gateway::relay::ProviderStage::FirstToken => RuntimeStage::FirstToken,
        vifu_gateway::relay::ProviderStage::Decode => RuntimeStage::Decode,
        vifu_gateway::relay::ProviderStage::Validate => RuntimeStage::Validate,
    }
}

async fn run_gateway_only_tui(config: LoadedRuntimeConfig) -> Result<(), String> {
    let gateway_options = config.gateway_options()?;
    let server_address = gateway_options.server_url.clone();
    let dashboard_url = remote_dashboard_url(&config).await;
    let credential = match configured_monitor_credential()? {
        Some(credential) => RemoteMonitorCredential::Static(credential),
        None => {
            RemoteMonitorCredential::GatewayGuest(gateway_guest_monitor_source(&gateway_options)?)
        }
    };
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let (monitor_tx, monitor_rx) = runtime_event_channel();
    let (local_monitor_tx, local_monitor_rx) = runtime_event_channel();
    let local_monitor_bridge = tokio::spawn(bridge_local_gateway_monitor(
        local_monitor_rx,
        monitor_tx.clone(),
    ));
    let remote_monitor = tokio::spawn(stream_remote_server_monitor(
        server_address,
        credential,
        monitor_tx,
    ));
    let gateway_control = gateway::GatewayControl::new();
    let optimization = gateway_control.optimization();
    let device_pairing = gateway_control.device_pairing();
    let mut gateway = tokio::spawn(gateway::run(
        gateway_options,
        shutdown_rx,
        Some(local_monitor_tx),
        gateway_control,
    ));
    let mut terminal = Box::pin(tui::run(
        monitor_rx,
        dashboard_url,
        Some(optimization),
        Some(device_pairing),
    ));
    let outcome = tokio::select! {
        () = shutdown_signal() => SingleRuntimeOutcome::Shutdown,
        result = &mut terminal => SingleRuntimeOutcome::Tui(result),
        result = &mut gateway => SingleRuntimeOutcome::Role(result),
    };
    let _ = shutdown_tx.send(true);
    remote_monitor.abort();
    local_monitor_bridge.abort();
    match outcome {
        SingleRuntimeOutcome::Shutdown | SingleRuntimeOutcome::Tui(Ok(())) => {
            let _ = gateway.await;
            Ok(())
        }
        SingleRuntimeOutcome::Tui(Err(error)) => {
            let _ = gateway.await;
            Err(format!("Vifu TUI failed: {error}"))
        }
        SingleRuntimeOutcome::Role(result) => join_result("Vifu Agent Gateway", result),
    }
}

async fn remote_dashboard_url(config: &LoadedRuntimeConfig) -> Option<String> {
    let fallback = config.dashboard_url();
    let server_address = config.server_address().ok()?;
    match RuntimeControlClient::discover_dashboard_url(server_address).await {
        Ok(Some(url)) => Some(url),
        Ok(None) | Err(_) => fallback,
    }
}

enum CombinedRuntimeOutcome {
    Shutdown,
    Tui(Result<(), String>),
    Server(Result<Result<(), String>, tokio::task::JoinError>),
    Gateway(Result<Result<(), String>, tokio::task::JoinError>),
}

enum SingleRuntimeOutcome {
    Shutdown,
    Tui(Result<(), String>),
    Role(Result<Result<(), String>, tokio::task::JoinError>),
}

fn join_result(
    role: &str,
    result: Result<Result<(), String>, tokio::task::JoinError>,
) -> Result<(), String> {
    match result {
        Ok(Ok(())) => Err(format!("{role} stopped unexpectedly")),
        Ok(Err(error)) => Err(format!("{role} failed: {error}")),
        Err(error) => Err(format!("{role} task failed: {error}")),
    }
}

fn local_console_url(config: &ServerConfig) -> Option<String> {
    config
        .server_url
        .clone()
        .or_else(|| Some(format!("http://{}", config.addr)))
}

async fn prepare_local_app(
    config: &LoadedRuntimeConfig,
    server: &vifu_server::AppState,
    gateway_options: Option<&mut crate::gateway::GatewayRuntimeOptions>,
) -> Result<(), String> {
    if !config.uses_local_app_bootstrap()? {
        return Ok(());
    }
    if let Some(options) = gateway_options.as_deref() {
        gateway::migrate_legacy_session(options)?;
    }
    let stored_guest_app_id = gateway_options
        .as_deref()
        .filter(|options| options.enrollment_token.is_none())
        .map(gateway::stored_guest_app_id)
        .transpose()?
        .flatten();
    let preferred_app_id = gateway_options
        .as_deref()
        .and_then(|options| options.enrollment_token.as_deref())
        .or_else(|| {
            gateway_options
                .as_deref()
                .and_then(|options| options.migrated_app_id.as_deref())
        })
        .or(stored_guest_app_id.as_deref());
    let app = vifu_server::ensure_local_bootstrap_app(server, preferred_app_id)
        .await
        .map_err(|error| format!("the local App could not be prepared: {error}"))?;
    if let Some(gateway_options) = gateway_options {
        gateway::clear_stored_guest_app(gateway_options)?;
        gateway::store_pending_app_id_if_unauthorized(gateway_options, &app.app_id)?;
        gateway_options.allow_guest_bootstrap = false;
        gateway_options.pairing_app = Some(crate::gateway::PairingAppTarget {
            project_id: app.project_id,
            deployment_id: app.deployment_id,
        });
    }
    Ok(())
}

fn apply_local_server_certificate(
    options: &mut crate::gateway::GatewayRuntimeOptions,
    endpoint: Option<&vifu_server::ServerEndpointIdentity>,
) -> Result<(), String> {
    options.server_certificate_der = endpoint
        .map(vifu_server::ServerEndpointIdentity::certificate_der)
        .transpose()?
        .flatten();
    Ok(())
}

fn remote_monitor_credential() -> Result<String, String> {
    configured_monitor_credential()?.ok_or_else(|| {
        "remote Server monitoring requires VIFU_MONITOR_KEY (a project API key with project read access) or VIFU_MONITOR_KEY_FILE"
            .to_string()
    })
}

fn configured_monitor_credential() -> Result<Option<String>, String> {
    optional_monitor_credential_from(
        std::env::var("VIFU_MONITOR_KEY").ok(),
        std::env::var_os("VIFU_MONITOR_KEY_FILE").map(std::path::PathBuf::from),
        std::env::var("VIFU_ADMIN_KEY").ok(),
        std::env::var_os("VIFU_ADMIN_KEY_FILE").map(std::path::PathBuf::from),
    )
}

#[cfg(test)]
fn monitor_credential_from(
    monitor_key: Option<String>,
    monitor_key_file: Option<std::path::PathBuf>,
    admin_key: Option<String>,
    admin_key_file: Option<std::path::PathBuf>,
) -> Result<String, String> {
    optional_monitor_credential_from(monitor_key, monitor_key_file, admin_key, admin_key_file)?
        .ok_or_else(|| {
            "remote Server monitoring requires VIFU_MONITOR_KEY (a project API key with project read access) or VIFU_MONITOR_KEY_FILE"
                .to_string()
        })
}

fn optional_monitor_credential_from(
    monitor_key: Option<String>,
    monitor_key_file: Option<std::path::PathBuf>,
    admin_key: Option<String>,
    admin_key_file: Option<std::path::PathBuf>,
) -> Result<Option<String>, String> {
    let (value, file) = if monitor_key.is_some() || monitor_key_file.is_some() {
        (monitor_key, monitor_key_file)
    } else {
        (admin_key, admin_key_file)
    };
    match (value, file) {
        (Some(value), None) if !value.trim().is_empty() => Ok(Some(value.trim().to_string())),
        (None, Some(path)) => {
            let value = std::fs::read_to_string(&path).map_err(|error| {
                format!(
                    "remote monitor credential {} could not be read: {error}",
                    path.display()
                )
            })?;
            let value = value.trim();
            if value.is_empty() {
                return Err(format!(
                    "remote monitor credential {} is empty",
                    path.display()
                ));
            }
            Ok(Some(value.to_string()))
        }
        (Some(_), Some(_)) => {
            Err("set either VIFU_MONITOR_KEY or VIFU_MONITOR_KEY_FILE, not both".to_string())
        }
        _ => Ok(None),
    }
}

#[derive(Clone)]
enum RemoteMonitorCredential {
    Static(String),
    GatewayGuest(GatewayGuestMonitorSource),
}

impl RemoteMonitorCredential {
    fn current(&self) -> Result<Option<String>, String> {
        match self {
            Self::Static(credential) => Ok(Some(credential.clone())),
            Self::GatewayGuest(source) => source.current(),
        }
    }
}

#[derive(Clone)]
struct GatewayGuestMonitorSource {
    store: vifu_gateway::session_store::GatewaySessionStore,
    state_key: String,
}

impl GatewayGuestMonitorSource {
    fn current(&self) -> Result<Option<String>, String> {
        Ok(self
            .store
            .load(&self.state_key, None, None)?
            .and_then(|session| session.guest_project)
            .map(|guest| guest.api_key))
    }
}

fn gateway_guest_monitor_source(
    options: &crate::gateway::GatewayRuntimeOptions,
) -> Result<GatewayGuestMonitorSource, String> {
    let config = options.load_config()?;
    let store =
        vifu_gateway::session_store::GatewaySessionStore::open(config.runtime_database_file())?;
    let state_key = vifu_gateway::session_store::gateway_session_state_key(
        &options.session_scope,
        &config.server_url,
    )?;
    Ok(GatewayGuestMonitorSource { store, state_key })
}

fn announce_console(console_url: Option<String>, open_browser: bool) {
    let Some(url) = console_url else {
        return;
    };
    println!("Vifu Console: {url}");
    if open_browser && should_open_browser() {
        tokio::spawn(open_console_when_ready(url));
    }
}

fn should_open_browser() -> bool {
    auto_browser_policy(
        std::io::stdin().is_terminal(),
        std::io::stdout().is_terminal(),
        std::env::var_os("CI").is_some(),
        std::env::var_os("TERM").is_some_and(|term| term == "dumb"),
    )
}

fn auto_browser_policy(
    stdin_is_terminal: bool,
    stdout_is_terminal: bool,
    is_ci: bool,
    term_is_dumb: bool,
) -> bool {
    stdin_is_terminal && stdout_is_terminal && !is_ci && !term_is_dumb
}

async fn open_console_when_ready(url: String) {
    if let Err(error) = open_browser_when_ready(url).await {
        eprintln!("Could not open Vifu Console automatically: {error}");
    }
}

async fn wait_for_console(url: &str) -> Result<(), String> {
    let address = console_socket_address(url)?;
    if !address.ip().is_loopback() {
        return Err(format!("unsupported non-loopback Console URL: {url}"));
    }

    for attempt in 0..50 {
        if tokio::net::TcpStream::connect(address).await.is_ok() {
            return Ok(());
        }
        if attempt < 49 {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    Err(format!("the local Server did not become ready at {url}"))
}

async fn wait_for_server(address: std::net::SocketAddr) -> Result<(), String> {
    let address = match address.ip() {
        std::net::IpAddr::V4(ip) if ip.is_unspecified() => {
            std::net::SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, address.port()))
        }
        std::net::IpAddr::V6(ip) if ip.is_unspecified() => {
            std::net::SocketAddr::from((std::net::Ipv6Addr::LOCALHOST, address.port()))
        }
        _ => address,
    };
    for attempt in 0..50 {
        if tokio::net::TcpStream::connect(address).await.is_ok() {
            return Ok(());
        }
        if attempt < 49 {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    Err(format!("the Vifu Server did not become ready at {address}"))
}

fn console_socket_address(url: &str) -> Result<std::net::SocketAddr, String> {
    let remainder = url
        .strip_prefix("http://")
        .ok_or_else(|| format!("unsupported local Console URL: {url}"))?;
    let authority = remainder
        .split(['/', '?', '#'])
        .next()
        .filter(|authority| !authority.is_empty())
        .ok_or_else(|| format!("invalid local Console URL: {url}"))?;
    let address = authority
        .parse::<std::net::SocketAddr>()
        .map_err(|error| format!("invalid local Console URL {url}: {error}"))?;
    Ok(address)
}

pub(crate) async fn open_browser_when_ready(url: String) -> Result<(), String> {
    if console_socket_address(&url).is_ok_and(|address| address.ip().is_loopback()) {
        wait_for_console(&url).await?;
    }
    open_browser(&url)
}

pub(crate) fn open_browser(url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let command = ProcessCommand::new("open").arg(url).spawn();

    #[cfg(target_os = "windows")]
    let command = ProcessCommand::new("cmd")
        .args(["/C", "start", "", url])
        .spawn();

    #[cfg(all(unix, not(target_os = "macos")))]
    let command = ProcessCommand::new("xdg-open").arg(url).spawn();

    command
        .map(|_| ())
        .map_err(|error| format!("failed to launch browser for {url}: {error}"))
}

async fn wait_for_shutdown(mut shutdown: watch::Receiver<bool>) {
    if *shutdown.borrow() {
        return;
    }
    let _ = shutdown.changed().await;
}

async fn shutdown_signal() {
    let control_c = async {
        if let Err(error) = tokio::signal::ctrl_c().await {
            tracing::error!(error = %error, "could not install Ctrl-C handler");
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => tracing::error!(error = %error, "could not install SIGTERM handler"),
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = control_c => {},
        () = terminate => {},
    }
}

fn init_tracing(suppress_terminal_output: bool) {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("vifu=info,vifu_server=info,tower_http=info"));
    if suppress_terminal_output {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .json()
            .with_current_span(false)
            .with_writer(std::io::sink)
            .try_init();
        return;
    }
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .json()
        .with_current_span(false)
        .try_init();
}

#[cfg(test)]
mod tests {
    use tokio::net::TcpListener;

    use super::{
        apply_local_server_certificate, auto_browser_policy, join_result, local_console_url,
        local_gateway_monitor_event, monitor_agent_id, monitor_credential_from,
        runtime_telemetry_io_event, start_plan, wait_for_console, RuntimeEvent, RuntimeHealth,
        StartPlan,
    };
    use vifu_server::config::{Config as ServerConfig, DeploymentMode};

    #[test]
    fn combined_gateway_uses_the_local_server_certificate() {
        let endpoint = vifu_server::ServerEndpointIdentity {
            server_url: "https://192.0.2.20:6790".to_string(),
            certificate_der_base64: Some("AQID".to_string()),
            certificate_sha256: Some("sha256:synthetic".to_string()),
        };
        let mut options = crate::gateway::GatewayRuntimeOptions {
            server_url: endpoint.server_url.clone(),
            server_certificate_der: None,
            allow_guest_bootstrap: true,
            enrollment_token: None,
            session_scope: "test".to_string(),
            legacy_session_scope: None,
            migrated_app_id: None,
            pairing_app: None,
        };

        apply_local_server_certificate(&mut options, Some(&endpoint)).unwrap();

        assert_eq!(options.server_certificate_der, Some(vec![1, 2, 3]));
    }

    #[test]
    fn remote_server_with_local_gateway_starts_only_the_gateway() {
        assert_eq!(start_plan(false, true), StartPlan::GatewayOnly);
    }

    #[test]
    fn remote_server_without_gateway_starts_no_local_component() {
        assert_eq!(start_plan(false, false), StartPlan::RemoteServerOnly);
    }

    #[test]
    fn local_server_with_local_gateway_starts_both_components() {
        assert_eq!(start_plan(true, true), StartPlan::Combined);
    }

    #[test]
    fn local_server_with_remote_gateway_starts_only_the_server() {
        assert_eq!(start_plan(true, false), StartPlan::ServerOnly);
    }

    #[test]
    fn remote_server_with_remote_gateway_starts_no_local_component() {
        assert_eq!(start_plan(false, false), StartPlan::RemoteServerOnly);
    }

    #[test]
    fn server_monitor_namespaces_the_same_agent_on_different_devices() {
        assert_eq!(
            monitor_agent_id("iphone-a", "companion-agent"),
            "iphone-a/companion-agent"
        );
        assert_ne!(
            monitor_agent_id("iphone-a", "companion-agent"),
            monitor_agent_id("iphone-b", "companion-agent")
        );
    }

    #[test]
    fn embedded_runtime_io_becomes_a_live_tui_capture_event() {
        let invocation_id = uuid::Uuid::new_v4();
        let input = vifu_gateway::protocol::canonical_trace_io_summary(&serde_json::json!({
            "messages": [{"role": "user", "content": "hello"}]
        }));
        let output = vifu_gateway::protocol::canonical_trace_io_summary(&serde_json::json!({
            "choices": [{"message": {"role": "assistant", "content": "hi"}}]
        }));
        let batch = vifu_gateway::protocol::RuntimeTelemetryBatch {
            deployment_id: uuid::Uuid::new_v4(),
            trace_id: "trace-io".to_string(),
            invocation_id: "invocation-io".to_string(),
            project_id: "android-demo".to_string(),
            endpoint: "chat".to_string(),
            agent_id: "android-chat".to_string(),
            started_at_ms: 1,
            events: Vec::new(),
            dropped_events: 0,
            root_input_summary: Some(input.clone()),
            root_output_summary: Some(output.clone()),
            host_metrics: None,
            terminal: None,
        };

        assert!(matches!(
            runtime_telemetry_io_event(invocation_id, &batch),
            Some(RuntimeEvent::IoCaptured {
                invocation_id: captured_id,
                input: Some(captured_input),
                output: Some(captured_output),
                truncated: false,
            }) if captured_id == invocation_id
                && captured_input == input.value
                && captured_output == output.value
        ));
    }

    #[test]
    fn automatic_browser_launch_requires_a_fully_interactive_terminal() {
        assert!(auto_browser_policy(true, true, false, false));
        assert!(!auto_browser_policy(false, true, false, false));
        assert!(!auto_browser_policy(true, false, false, false));
        assert!(!auto_browser_policy(true, true, true, false));
        assert!(!auto_browser_policy(true, true, false, true));
    }

    #[test]
    fn remote_monitor_prefers_a_project_scoped_monitor_key() {
        let credential = monitor_credential_from(
            Some(" vifu_pk_project ".to_string()),
            None,
            Some("deployment-admin".to_string()),
            None,
        )
        .unwrap();

        assert_eq!(credential, "vifu_pk_project");
    }

    #[test]
    fn remote_monitor_does_not_generate_an_implicit_admin_key() {
        let error = monitor_credential_from(None, None, None, None).unwrap_err();

        assert!(error.contains("VIFU_MONITOR_KEY"));
    }

    #[test]
    fn local_gateway_bridge_forwards_invocation_activity_but_not_the_raw_roster() {
        assert!(matches!(
            local_gateway_monitor_event(RuntimeEvent::LoadedModelsChanged(2)),
            Some(RuntimeEvent::LoadedModelsChanged(2))
        ));
        assert!(matches!(
            local_gateway_monitor_event(RuntimeEvent::HealthChanged {
                health: RuntimeHealth::Live,
                message: None,
            }),
            Some(RuntimeEvent::HealthChanged {
                health: RuntimeHealth::Live,
                message: None,
            })
        ));
        assert!(local_gateway_monitor_event(RuntimeEvent::AgentsRegistered(Vec::new())).is_none());
        assert!(matches!(
            local_gateway_monitor_event(RuntimeEvent::InvocationCancelled {
                invocation_id: uuid::Uuid::nil(),
            }),
            Some(RuntimeEvent::InvocationCancelled { invocation_id })
                if invocation_id == uuid::Uuid::nil()
        ));
        assert!(matches!(
            local_gateway_monitor_event(RuntimeEvent::MonitorEventsDropped { dropped_events: 3 }),
            Some(RuntimeEvent::MonitorEventsDropped { dropped_events: 3 })
        ));
    }

    #[test]
    fn self_hosted_server_presents_its_own_dashboard_origin() {
        let mut config = ServerConfig::from_env().unwrap();
        config.deployment_mode = DeploymentMode::SelfHosted;
        config.addr = "0.0.0.0:6790".parse().unwrap();
        config.server_url = Some("https://192.0.2.20:6790".to_string());
        config.dashboard_addr = None;

        assert_eq!(
            local_console_url(&config).as_deref(),
            Some("https://192.0.2.20:6790")
        );

        config.dashboard_addr = Some("dashboard:6791".to_string());
        assert_eq!(
            local_console_url(&config).as_deref(),
            Some("https://192.0.2.20:6790")
        );
    }

    #[test]
    fn local_lan_server_uses_its_own_console_address() {
        let mut config = ServerConfig::from_env().unwrap();
        config.deployment_mode = DeploymentMode::Local;
        config.addr = "192.0.2.20:6790".parse().unwrap();
        config.server_url = Some("https://192.0.2.20:6790".to_string());
        config.dashboard_addr = None;

        assert_eq!(
            local_console_url(&config).as_deref(),
            Some("https://192.0.2.20:6790")
        );
    }

    #[tokio::test]
    async fn runtime_errors_keep_their_original_cause() {
        let task = tokio::spawn(async { Err("could not bind loopback".to_string()) });

        let error = join_result("Vifu Server", task.await).unwrap_err();

        assert_eq!(error, "Vifu Server failed: could not bind loopback");
    }

    #[tokio::test]
    async fn console_readiness_succeeds_when_server_is_listening() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());

        assert!(wait_for_console(&url).await.is_ok());
    }

    #[tokio::test]
    async fn console_readiness_accepts_a_dashboard_deep_link() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "http://{}/project/demo/logs?invocationId=test#detail",
            listener.local_addr().unwrap()
        );

        assert!(wait_for_console(&url).await.is_ok());
    }

    #[tokio::test]
    async fn console_readiness_rejects_non_http_urls() {
        let error = wait_for_console("https://dashboard.example.com")
            .await
            .unwrap_err();

        assert!(error.contains("unsupported local Console URL"));
    }

    #[tokio::test]
    async fn console_readiness_rejects_non_loopback_urls() {
        let error = wait_for_console("http://192.0.2.1:6790").await.unwrap_err();

        assert!(error.contains("non-loopback"));
    }
}
