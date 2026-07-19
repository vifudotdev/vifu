use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::sync::watch;
use uuid::Uuid;

use crate::config::{AgentProviderConfig, Config};
use crate::openclaw::{self, ProbeStatus};
use crate::relay;
use crate::session::{self, SessionStatus, SessionSummary};

const PROVIDER_RETRY_DELAY: Duration = Duration::from_secs(10);

#[derive(Debug, Clone)]
pub struct GatewayRuntimeOptions {
    pub server_url: String,
}

impl GatewayRuntimeOptions {
    pub fn load_config(&self) -> Result<Config, String> {
        Config::load(self.server_url.clone())
    }
}

pub async fn run(
    options: GatewayRuntimeOptions,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), String> {
    println!("Vifu Agent Gateway");
    loop {
        if *shutdown.borrow() {
            return Ok(());
        }
        let config = options.load_config()?;
        ensure_home_dir(&config)?;
        print_server_config(&config)?;
        let Some(provider) = config.openclaw_provider().cloned() else {
            print_agent_provider_config(&config);
            wait_for_provider_retry("No agent provider is configured.", &mut shutdown).await;
            continue;
        };

        let report = openclaw::probe(&provider.url).await;
        print_openclaw_report(&provider, &report);
        if !matches!(report.status, ProbeStatus::Online) {
            wait_for_provider_retry("OpenClaw Gateway is not reachable.", &mut shutdown).await;
            continue;
        }

        let agents =
            match openclaw::discover_agents(&report.endpoint, provider.token.as_deref()).await {
                Ok(agents) => agents,
                Err(error) => {
                    wait_for_provider_retry(
                        &format!("OpenClaw agent discovery is unavailable ({error})."),
                        &mut shutdown,
                    )
                    .await;
                    continue;
                }
            };
        println!("Agents: {} discovered", agents.len());
        let mut session = load_or_create_session(&config)?;
        print_session(&session);
        let session_file = config.session_file();
        let runtime = relay::AgentGatewayRuntime {
            server_url: &config.server_url,
            agent_gateway_bootstrap_token: &config.agent_gateway_bootstrap_token,
            provider_id: &provider.id,
            endpoint: &report.endpoint,
            openclaw_token: provider.token.as_deref(),
            agents: &agents,
            session_path: &session_file,
        };
        let result = tokio::select! {
            result = relay::run_agent_gateway(runtime, &mut session) => result,
            () = wait_for_shutdown(&mut shutdown) => return Ok(()),
        };
        if let Err(error) = result {
            wait_for_provider_retry(&format!("Agent Gateway stopped ({error})."), &mut shutdown)
                .await;
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

pub async fn status(config: &Config) -> Result<(), String> {
    println!("Vifu Agent Gateway status");
    println!("State: {}", config.home_dir.display());
    print_agent_provider_status(config).await;
    print_server_config(config)?;
    print_stored_session(config);
    Ok(())
}

pub async fn doctor(config: &Config) -> Result<(), String> {
    println!("Vifu Agent Gateway doctor");
    println!("State directory: {}", config.home_dir.display());
    let Some((provider, report)) = print_agent_provider_status(config).await else {
        print_server_config(config)?;
        print_stored_session(config);
        return Ok(());
    };
    print_server_config(config)?;
    print_stored_session(config);
    match report.status {
        ProbeStatus::Online => {
            match openclaw::discover_agents(&report.endpoint, provider.token.as_deref()).await {
                Ok(agents) => println!("OpenClaw API: ready ({} agents)", agents.len()),
                Err(error) => println!("OpenClaw API: unavailable ({error})"),
            }
        }
        ProbeStatus::Offline(_) => {
            println!("OpenClaw: start the Gateway on loopback, for example:");
            println!("  openclaw gateway --port 18789");
        }
        ProbeStatus::Unsupported(_) => {
            println!("OpenClaw: use a loopback URL such as http://127.0.0.1:18789");
        }
    }
    Ok(())
}

async fn print_agent_provider_status(
    config: &Config,
) -> Option<(AgentProviderConfig, openclaw::ProbeReport)> {
    let Some(provider) = config.openclaw_provider().cloned() else {
        print_agent_provider_config(config);
        return None;
    };
    let report = openclaw::probe(&provider.url).await;
    print_openclaw_report(&provider, &report);
    Some((provider, report))
}

pub fn logout(config: &Config) -> Result<(), String> {
    match session::read_session(&config.session_file()) {
        SessionStatus::Ready(mut summary) | SessionStatus::UpgradeRequired(mut summary) => {
            summary.resume_session_id = None;
            session::write_session(&config.session_file(), &summary)?;
            println!("Cleared the resumable Agent Gateway session.");
        }
        SessionStatus::Missing => println!("No local Agent Gateway session found."),
        SessionStatus::Invalid(reason) => {
            return Err(format!(
                "local agent gateway state is invalid: {reason}. Run `vifu --reset` to replace it."
            ));
        }
    }
    Ok(())
}

pub fn reset(config: &Config) -> Result<(), String> {
    if remove_gateway_identity(&config.session_file())? {
        println!("Removed the local Agent Gateway identity.");
    } else {
        println!("No local Agent Gateway identity found.");
    }
    Ok(())
}

fn remove_gateway_identity(path: &Path) -> Result<bool, String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.to_string()),
    }
}

fn ensure_home_dir(config: &Config) -> Result<(), String> {
    fs::create_dir_all(&config.home_dir).map_err(|error| error.to_string())
}

fn print_agent_provider_config(config: &Config) {
    if config.agent_providers.is_empty() {
        println!(
            "Agent providers: none configured ({})",
            config.agent_providers_file.display()
        );
    } else {
        println!(
            "Agent providers: {} configured; no supported provider is available yet",
            config.agent_providers.len()
        );
    }
}

fn print_openclaw_report(provider: &AgentProviderConfig, report: &openclaw::ProbeReport) {
    match &report.status {
        ProbeStatus::Online => println!(
            "OpenClaw provider {}: online at {}:{}",
            provider.id, report.endpoint.host, report.endpoint.port
        ),
        ProbeStatus::Offline(reason) => println!(
            "OpenClaw provider {}: offline at {}:{} ({reason})",
            provider.id, report.endpoint.host, report.endpoint.port
        ),
        ProbeStatus::Unsupported(reason) => println!(
            "OpenClaw provider {}: unsupported configuration ({reason})",
            provider.id
        ),
    }
}

fn print_server_config(config: &Config) -> Result<(), String> {
    println!("Server: {}", config.server_url);
    println!(
        "WebSocket: {}",
        relay::agent_gateway_websocket_url(&config.server_url)?
    );
    Ok(())
}

fn print_stored_session(config: &Config) {
    match session::read_session(&config.session_file()) {
        SessionStatus::Ready(summary) => print_session(&summary),
        SessionStatus::UpgradeRequired(summary) => {
            println!("Session: upgrade pending ({})", summary.gateway_id);
        }
        SessionStatus::Missing => println!("Session: not established"),
        SessionStatus::Invalid(reason) => println!("Session: invalid ({reason})"),
    }
}

fn print_session(session: &SessionSummary) {
    match session.resume_session_id {
        Some(session_id) => println!(
            "Session: resumable ({}, {})",
            session.gateway_id, session_id
        ),
        None => println!("Session: new ({})", session.gateway_id),
    }
}

fn load_or_create_session(config: &Config) -> Result<SessionSummary, String> {
    match session::read_session(&config.session_file()) {
        SessionStatus::Ready(summary) => Ok(summary),
        SessionStatus::UpgradeRequired(summary) => {
            session::write_session(&config.session_file(), &summary)?;
            println!("Upgraded the local Agent Gateway session.");
            Ok(summary)
        }
        SessionStatus::Missing => {
            let summary = SessionSummary {
                gateway_id: format!("gateway-{}", Uuid::new_v4().simple()),
                gateway_credential: session::generate_gateway_credential(),
                resume_session_id: None,
                created_at_unix: now_unix_seconds()?,
            };
            session::write_session(&config.session_file(), &summary)?;
            Ok(summary)
        }
        SessionStatus::Invalid(reason) => Err(format!(
            "local agent gateway session is invalid: {reason}. Run `vifu --reset` to replace it."
        )),
    }
}

fn now_unix_seconds() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use uuid::Uuid;

    use super::remove_gateway_identity;

    #[test]
    fn gateway_identity_reset_does_not_remove_provider_config() {
        let directory = std::env::temp_dir().join(format!("vifu-reset-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let session = directory.join("agent-gateway-session");
        let providers = directory.join("providers.json");
        fs::write(&session, "session").unwrap();
        fs::write(&providers, "{}").unwrap();

        assert!(remove_gateway_identity(&session).unwrap());

        assert!(!session.exists());
        assert!(providers.exists());
        fs::remove_dir_all(directory).unwrap();
    }
}
