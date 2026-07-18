use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use uuid::Uuid;

use crate::cli::{help_text, Command, Options};
use crate::config::{AgentProviderConfig, Config};
use crate::openclaw::{self, ProbeStatus};
use crate::relay;
use crate::session::{self, SessionStatus, SessionSummary};

const PROVIDER_RETRY_DELAY: Duration = Duration::from_secs(10);

pub async fn execute(options: Options) -> Result<(), String> {
    match options.command {
        Command::Help => {
            print!("{}", help_text());
            Ok(())
        }
        Command::Version => {
            println!("vifu {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Command::Connect => connect(options).await,
        Command::Status => status(options).await,
        Command::Doctor => doctor(options).await,
        Command::Logout => logout(options),
        Command::Reset => reset(options),
    }
}

async fn connect(options: Options) -> Result<(), String> {
    println!("Vifu Agent Gateway");
    loop {
        let config = Config::load(options.server_url.clone())?;
        ensure_home_dir(&config)?;
        print_server_config(&config)?;
        let Some(provider) = config.openclaw_provider().cloned() else {
            print_agent_provider_config(&config);
            if wait_for_provider_retry("No agent provider is configured.").await {
                continue;
            }
            return Ok(());
        };

        let report = openclaw::probe(&provider.url).await;
        print_openclaw_report(&provider, &report);

        if !matches!(report.status, ProbeStatus::Online) {
            if wait_for_provider_retry("OpenClaw Gateway is not reachable.").await {
                continue;
            }
            return Ok(());
        }

        let agents =
            match openclaw::discover_agents(&report.endpoint, provider.token.as_deref()).await {
                Ok(agents) => agents,
                Err(error) => {
                    if wait_for_provider_retry(&format!(
                        "OpenClaw agent discovery is unavailable ({error})."
                    ))
                    .await
                    {
                        continue;
                    }
                    return Ok(());
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
        match relay::run_agent_gateway(runtime, &mut session).await {
            Ok(()) => return Ok(()),
            Err(error) => {
                if wait_for_provider_retry(&format!("Agent Gateway stopped ({error}).")).await {
                    continue;
                }
                return Ok(());
            }
        }
    }
}

async fn wait_for_provider_retry(message: &str) -> bool {
    eprintln!(
        "{message} Waiting {}s before retrying.",
        PROVIDER_RETRY_DELAY.as_secs()
    );
    tokio::select! {
        _ = tokio::time::sleep(PROVIDER_RETRY_DELAY) => true,
        _ = tokio::signal::ctrl_c() => false,
    }
}

async fn status(options: Options) -> Result<(), String> {
    let config = Config::load(options.server_url)?;
    println!("Vifu status");
    println!("State: {}", config.home_dir.display());
    print_agent_provider_status(&config).await;
    print_server_config(&config)?;
    print_stored_session(&config);
    Ok(())
}

async fn doctor(options: Options) -> Result<(), String> {
    let config = Config::load(options.server_url)?;
    println!("Vifu doctor");
    println!("State directory: {}", config.home_dir.display());
    let Some((provider, report)) = print_agent_provider_status(&config).await else {
        print_server_config(&config)?;
        print_stored_session(&config);
        return Ok(());
    };
    print_server_config(&config)?;
    print_stored_session(&config);
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

fn logout(options: Options) -> Result<(), String> {
    let config = Config::load(options.server_url)?;
    match session::read_session(&config.session_file()) {
        SessionStatus::Ready(mut summary) | SessionStatus::UpgradeRequired(mut summary) => {
            summary.resume_session_id = None;
            session::write_session(&config.session_file(), &summary)?;
            println!("Cleared the resumable Agent Gateway session.");
        }
        SessionStatus::Missing => {
            println!("No local agent gateway session found.");
        }
        SessionStatus::Invalid(reason) => {
            return Err(format!(
                "local agent gateway state is invalid: {reason}. Run `vifu --reset` to replace it."
            ));
        }
    }
    Ok(())
}

fn reset(options: Options) -> Result<(), String> {
    let config = Config::load(options.server_url)?;
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
        ProbeStatus::Unsupported(reason) => {
            println!(
                "OpenClaw provider {}: unsupported configuration ({reason})",
                provider.id
            );
        }
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
