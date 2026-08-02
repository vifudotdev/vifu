use std::io::IsTerminal;
use std::process::Command as ProcessCommand;
use std::time::Duration;

use tokio::sync::watch;
use tracing_subscriber::EnvFilter;
use vifu_server::config::{Config as ServerConfig, DeploymentMode};

use crate::cli::{help_text, Command, Options};
use crate::gateway;
use crate::monitor::{runtime_event_channel, RuntimeEvent, RuntimeHealth};
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
        Command::Start => start(load_config()?, open_browser).await,
        Command::Status => status(load_config()?).await,
        Command::Doctor => doctor(load_config()?).await,
    }
}

async fn start(config: LoadedRuntimeConfig, open_browser: bool) -> Result<(), String> {
    match (
        config.config.server.is_some(),
        config.config.gateway.is_some(),
    ) {
        (true, true) => run_combined(config, open_browser).await,
        (true, false) => run_server_only(config, open_browser).await,
        (false, true) => run_gateway_only(config).await,
        (false, false) => unreachable!("runtime configuration validates roles"),
    }
}

async fn status(config: LoadedRuntimeConfig) -> Result<(), String> {
    println!("Vifu runtime");
    println!("Configuration: {}", config.path.display());
    if let Some(profile) = config.profile.as_ref() {
        println!("Profile: {profile}");
    }
    println!("Server: {}", role_status(config.config.server.is_some()));
    println!(
        "Agent Gateway: {}",
        role_status(config.config.gateway.is_some())
    );
    if config.config.gateway.is_some() {
        let gateway_options = config.gateway_options()?;
        let dashboard_url = gateway_options.dashboard_url.clone();
        let session_scope = gateway_options.session_scope.clone();
        let gateway = gateway_options.load_config()?;
        gateway::status(&gateway, dashboard_url.as_deref(), &session_scope).await?;
    }
    Ok(())
}

async fn doctor(config: LoadedRuntimeConfig) -> Result<(), String> {
    println!("Vifu runtime doctor");
    println!("Configuration: {}", config.path.display());
    if let Some(profile) = config.profile.as_ref() {
        println!("Profile: {profile}");
    }
    if config.config.gateway.is_some() {
        let gateway_options = config.gateway_options()?;
        let dashboard_url = gateway_options.dashboard_url.clone();
        let session_scope = gateway_options.session_scope.clone();
        let gateway = gateway_options.load_config()?;
        gateway::doctor(&gateway, dashboard_url.as_deref(), &session_scope).await?;
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
    let gateway_options = config.gateway_options()?;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let mut server = tokio::spawn(vifu_server::serve(
        server_config,
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
    let mut server = tokio::spawn(vifu_server::serve(
        server_config,
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

async fn run_combined_tui(config: LoadedRuntimeConfig) -> Result<(), String> {
    let server_config = config.server_config()?;
    let console_url = local_console_url(&server_config);
    let gateway_options = config.gateway_options()?;
    let dashboard_url = console_url
        .clone()
        .or_else(|| gateway_options.dashboard_url.clone());
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let (monitor_tx, monitor_rx) = runtime_event_channel();
    let gateway_control = gateway::GatewayControl::new();
    let optimization = gateway_control.optimization();
    let mut server = tokio::spawn(vifu_server::serve(
        server_config,
        wait_for_shutdown(shutdown_rx.clone()),
    ));
    let mut gateway = tokio::spawn(gateway::run(
        gateway_options,
        shutdown_rx,
        Some(monitor_tx),
        gateway_control,
    ));
    let mut terminal = Box::pin(tui::run(monitor_rx, dashboard_url, Some(optimization)));
    let outcome = tokio::select! {
        () = shutdown_signal() => CombinedRuntimeOutcome::Shutdown,
        result = &mut terminal => CombinedRuntimeOutcome::Tui(result),
        result = &mut server => CombinedRuntimeOutcome::Server(result),
        result = &mut gateway => CombinedRuntimeOutcome::Gateway(result),
    };
    let _ = shutdown_tx.send(true);
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
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let (monitor_tx, monitor_rx) = runtime_event_channel();
    let _ = monitor_tx.send(RuntimeEvent::HealthChanged {
        health: RuntimeHealth::Starting,
        message: None,
    });
    let _ = monitor_tx.send(RuntimeEvent::LoadedModelsChanged(0));
    if let Some(readiness_url) = console_url.clone() {
        let readiness_monitor = monitor_tx.clone();
        std::mem::drop(tokio::spawn(async move {
            if wait_for_console(&readiness_url).await.is_ok() {
                let _ = readiness_monitor.send(RuntimeEvent::HealthChanged {
                    health: RuntimeHealth::Live,
                    message: None,
                });
            }
        }));
    }
    drop(monitor_tx);
    let mut server = tokio::spawn(vifu_server::serve(
        server_config,
        wait_for_shutdown(shutdown_rx),
    ));
    let mut terminal = Box::pin(tui::run(monitor_rx, console_url, None));
    let outcome = tokio::select! {
        () = shutdown_signal() => SingleRuntimeOutcome::Shutdown,
        result = &mut terminal => SingleRuntimeOutcome::Tui(result),
        result = &mut server => SingleRuntimeOutcome::Role(result),
    };
    let _ = shutdown_tx.send(true);
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

async fn run_gateway_only_tui(config: LoadedRuntimeConfig) -> Result<(), String> {
    let gateway_options = config.gateway_options()?;
    let dashboard_url = gateway_options.dashboard_url.clone();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let (monitor_tx, monitor_rx) = runtime_event_channel();
    let gateway_control = gateway::GatewayControl::new();
    let optimization = gateway_control.optimization();
    let mut gateway = tokio::spawn(gateway::run(
        gateway_options,
        shutdown_rx,
        Some(monitor_tx),
        gateway_control,
    ));
    let mut terminal = Box::pin(tui::run(monitor_rx, dashboard_url, Some(optimization)));
    let outcome = tokio::select! {
        () = shutdown_signal() => SingleRuntimeOutcome::Shutdown,
        result = &mut terminal => SingleRuntimeOutcome::Tui(result),
        result = &mut gateway => SingleRuntimeOutcome::Role(result),
    };
    let _ = shutdown_tx.send(true);
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
    (config.deployment_mode == DeploymentMode::Local && config.addr.ip().is_loopback())
        .then(|| format!("http://{}", config.addr))
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

    use super::{auto_browser_policy, join_result, wait_for_console};

    #[test]
    fn automatic_browser_launch_requires_a_fully_interactive_terminal() {
        assert!(auto_browser_policy(true, true, false, false));
        assert!(!auto_browser_policy(false, true, false, false));
        assert!(!auto_browser_policy(true, false, false, false));
        assert!(!auto_browser_policy(true, true, true, false));
        assert!(!auto_browser_policy(true, true, false, true));
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
