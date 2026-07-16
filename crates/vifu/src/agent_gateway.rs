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
            agent_gateway_token: &config.agent_gateway_token,
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
    match fs::remove_file(config.session_file()) {
        Ok(()) => println!("Removed the local agent gateway session."),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            println!("No local agent gateway session found.");
        }
        Err(error) => return Err(error.to_string()),
    }
    Ok(())
}

fn reset(options: Options) -> Result<(), String> {
    let config = Config::load(options.server_url)?;
    ensure_safe_reset_dir(&config.home_dir)?;
    match fs::remove_dir_all(&config.home_dir) {
        Ok(()) => println!("Removed all local Vifu state."),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            println!("No local Vifu state found.");
        }
        Err(error) => return Err(error.to_string()),
    }
    Ok(())
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
        SessionStatus::Missing => {
            let summary = SessionSummary {
                gateway_id: format!("gateway-{}", Uuid::new_v4().simple()),
                resume_session_id: None,
                created_at_unix: now_unix_seconds()?,
            };
            session::write_session(&config.session_file(), &summary)?;
            Ok(summary)
        }
        SessionStatus::Invalid(reason) => Err(format!(
            "local agent gateway session is invalid: {reason}. Run `vifu --logout` to replace it."
        )),
    }
}

fn now_unix_seconds() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|error| error.to_string())
}

fn ensure_safe_reset_dir(path: &Path) -> Result<(), String> {
    if !path.is_absolute() {
        return Err(format!(
            "Refusing to reset '{}'. Vifu reset requires an absolute state directory path.",
            path.display()
        ));
    }
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "Refusing to reset an unnamed Vifu state directory.".to_string())?;
    if file_name != ".vifu" {
        return Err(format!(
            "Refusing to reset '{}'. Vifu reset only removes a directory named '.vifu'.",
            path.display()
        ));
    }
    let parent = path
        .parent()
        .ok_or_else(|| "Refusing to reset a root-level Vifu state directory.".to_string())?;
    if parent.parent().is_none() {
        return Err("Refusing to reset a root-level Vifu state directory.".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::ensure_safe_reset_dir;

    #[test]
    fn reset_allows_default_state_dir_name() {
        assert!(ensure_safe_reset_dir(&PathBuf::from("/Users/example/.vifu")).is_ok());
    }

    #[test]
    fn reset_rejects_unscoped_or_relative_directories() {
        assert!(ensure_safe_reset_dir(&PathBuf::from("/Users/example")).is_err());
        assert!(ensure_safe_reset_dir(&PathBuf::from(".vifu")).is_err());
        assert!(ensure_safe_reset_dir(&PathBuf::from("/.vifu")).is_err());
    }
}
