use tokio::sync::watch;
use tracing_subscriber::EnvFilter;

use crate::cli::{help_text, Command, Options};
use crate::gateway;
use crate::runtime_config::LoadedRuntimeConfig;

pub async fn execute(options: Options) -> Result<(), String> {
    init_tracing();
    let Options {
        command,
        config_profile,
        config_overrides,
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
        Command::Start => start(load_config()?).await,
        Command::Status => status(load_config()?).await,
        Command::Doctor => doctor(load_config()?).await,
        Command::Logout => gateway_logout(load_config()?),
        Command::Reset => gateway_reset(load_config()?),
    }
}

async fn start(config: LoadedRuntimeConfig) -> Result<(), String> {
    match (
        config.config.server.is_some(),
        config.config.gateway.is_some(),
    ) {
        (true, true) => run_combined(config).await,
        (true, false) => run_server_only(config).await,
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
        let gateway = config.gateway_options()?.load_config()?;
        gateway::status(&gateway).await?;
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
        let gateway = config.gateway_options()?.load_config()?;
        gateway::doctor(&gateway).await?;
    } else {
        println!("Agent Gateway: not configured");
    }
    Ok(())
}

fn gateway_logout(config: LoadedRuntimeConfig) -> Result<(), String> {
    let gateway = config.gateway_options()?.load_config()?;
    gateway::logout(&gateway)
}

fn gateway_reset(config: LoadedRuntimeConfig) -> Result<(), String> {
    let gateway = config.gateway_options()?.load_config()?;
    gateway::reset(&gateway)
}

fn role_status(enabled: bool) -> &'static str {
    if enabled {
        "configured"
    } else {
        "not configured"
    }
}

async fn run_combined(config: LoadedRuntimeConfig) -> Result<(), String> {
    let server_config = config.server_config()?;
    let gateway_options = config.gateway_options()?;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let mut server = tokio::spawn(vifu_server::serve(
        server_config,
        wait_for_shutdown(shutdown_rx.clone()),
    ));
    let mut gateway = tokio::spawn(gateway::run(gateway_options, shutdown_rx));
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
    }
}

async fn run_server_only(config: LoadedRuntimeConfig) -> Result<(), String> {
    let server_config = config.server_config()?;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let mut server = tokio::spawn(vifu_server::serve(
        server_config,
        wait_for_shutdown(shutdown_rx),
    ));
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
    }
}

async fn run_gateway_only(config: LoadedRuntimeConfig) -> Result<(), String> {
    let gateway_options = config.gateway_options()?;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let mut gateway = tokio::spawn(gateway::run(gateway_options, shutdown_rx));
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
    }
}

enum CombinedRuntimeOutcome {
    Shutdown,
    Server(Result<Result<(), String>, tokio::task::JoinError>),
    Gateway(Result<Result<(), String>, tokio::task::JoinError>),
}

enum SingleRuntimeOutcome {
    Shutdown,
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

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("vifu=info,vifu_server=info,tower_http=info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .json()
        .with_current_span(false)
        .try_init();
}
