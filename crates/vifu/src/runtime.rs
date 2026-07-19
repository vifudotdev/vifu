use tokio::sync::watch;
use tracing_subscriber::EnvFilter;

use crate::agent_gateway;
use crate::cli::{help_text, Command, Options};
use crate::runtime_config::LoadedRuntimeConfig;

pub async fn execute(options: Options) -> Result<(), String> {
    init_tracing();
    match options.command {
        Command::Help => {
            print!("{}", help_text());
            Ok(())
        }
        Command::Version => {
            println!("vifu {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Command::Start => start(LoadedRuntimeConfig::load()?).await,
        Command::Status => status(LoadedRuntimeConfig::load()?).await,
        Command::Doctor => doctor(LoadedRuntimeConfig::load()?).await,
        Command::Logout => gateway_logout(LoadedRuntimeConfig::load()?),
        Command::Reset => gateway_reset(LoadedRuntimeConfig::load()?),
    }
}

async fn start(config: LoadedRuntimeConfig) -> Result<(), String> {
    if config.config.server.is_some() && !cfg!(feature = "server") {
        return Err("this vifu build does not include the Server role".to_string());
    }
    if config.config.gateway.is_some() && !cfg!(feature = "gateway") {
        return Err("this vifu build does not include the Agent Gateway role".to_string());
    }

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
    println!("Server: {}", role_status(config.config.server.is_some()));
    println!(
        "Agent Gateway: {}",
        role_status(config.config.gateway.is_some())
    );
    if config.config.gateway.is_some() {
        let gateway = config.gateway_options()?.load_config()?;
        agent_gateway::status(&gateway).await?;
    }
    Ok(())
}

async fn doctor(config: LoadedRuntimeConfig) -> Result<(), String> {
    println!("Vifu runtime doctor");
    println!("Configuration: {}", config.path.display());
    if config.config.gateway.is_some() {
        let gateway = config.gateway_options()?.load_config()?;
        agent_gateway::doctor(&gateway).await?;
    } else {
        println!("Agent Gateway: not configured");
    }
    Ok(())
}

fn gateway_logout(config: LoadedRuntimeConfig) -> Result<(), String> {
    let gateway = config.gateway_options()?.load_config()?;
    agent_gateway::logout(&gateway)
}

fn gateway_reset(config: LoadedRuntimeConfig) -> Result<(), String> {
    let gateway = config.gateway_options()?.load_config()?;
    agent_gateway::reset(&gateway)
}

fn role_status(enabled: bool) -> &'static str {
    if enabled {
        "configured"
    } else {
        "not configured"
    }
}

#[cfg(feature = "server")]
async fn run_combined(config: LoadedRuntimeConfig) -> Result<(), String> {
    let server_config = config.server_config()?;
    let gateway_options = config.gateway_options()?;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let mut server = tokio::spawn(vifu_server::serve(
        server_config,
        wait_for_shutdown(shutdown_rx.clone()),
    ));
    let mut gateway = tokio::spawn(agent_gateway::run(gateway_options, shutdown_rx));
    let outcome = tokio::select! {
        () = shutdown_signal() => Ok(()),
        result = &mut server => join_result("Vifu Server", result),
        result = &mut gateway => join_result("Vifu Agent Gateway", result),
    };
    let _ = shutdown_tx.send(true);
    let _ = server.await;
    let _ = gateway.await;
    outcome
}

#[cfg(not(feature = "server"))]
async fn run_combined(_config: LoadedRuntimeConfig) -> Result<(), String> {
    Err("this vifu build does not include the Server role".to_string())
}

#[cfg(feature = "server")]
async fn run_server_only(config: LoadedRuntimeConfig) -> Result<(), String> {
    let server_config = config.server_config()?;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let mut server = tokio::spawn(vifu_server::serve(
        server_config,
        wait_for_shutdown(shutdown_rx),
    ));
    let outcome = tokio::select! {
        () = shutdown_signal() => Ok(()),
        result = &mut server => join_result("Vifu Server", result),
    };
    let _ = shutdown_tx.send(true);
    let _ = server.await;
    outcome
}

#[cfg(not(feature = "server"))]
async fn run_server_only(_config: LoadedRuntimeConfig) -> Result<(), String> {
    Err("this vifu build does not include the Server role".to_string())
}

async fn run_gateway_only(config: LoadedRuntimeConfig) -> Result<(), String> {
    let gateway_options = config.gateway_options()?;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let mut gateway = tokio::spawn(agent_gateway::run(gateway_options, shutdown_rx));
    let outcome = tokio::select! {
        () = shutdown_signal() => Ok(()),
        result = &mut gateway => join_result("Vifu Agent Gateway", result),
    };
    let _ = shutdown_tx.send(true);
    let _ = gateway.await;
    outcome
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

#[cfg(feature = "server")]
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
