use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use uuid::Uuid;

use crate::cli::{help_text, Command, Options};
use crate::config::Config;
use crate::openclaw::{self, ProbeStatus};
use crate::relay;
use crate::session::{self, SessionStatus, SessionSummary};

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
    let config = Config::load(options.openclaw_url, options.server_url)?;
    ensure_home_dir(&config)?;
    let report = openclaw::probe(&config.openclaw_url).await;

    println!("Vifu connector");
    print_openclaw_report(&report);
    print_server_config(&config)?;

    if !matches!(report.status, ProbeStatus::Online) {
        return Err(
            "OpenClaw Gateway is not reachable. Run `vifu --doctor` for setup checks.".to_string(),
        );
    }
    let agents =
        openclaw::discover_agents(&report.endpoint, config.openclaw_token.as_deref()).await?;
    println!("Agents: {} discovered", agents.len());
    let mut session = load_or_create_session(&config)?;
    print_session(&session);
    relay::run_connector(
        &config.server_url,
        &config.connector_token,
        &report.endpoint,
        config.openclaw_token.as_deref(),
        &agents,
        &config.session_file(),
        &mut session,
    )
    .await
}

async fn status(options: Options) -> Result<(), String> {
    let config = Config::load(options.openclaw_url, options.server_url)?;
    let report = openclaw::probe(&config.openclaw_url).await;
    println!("Vifu status");
    println!("State: {}", config.home_dir.display());
    print_openclaw_report(&report);
    print_server_config(&config)?;
    print_stored_session(&config);
    Ok(())
}

async fn doctor(options: Options) -> Result<(), String> {
    let config = Config::load(options.openclaw_url, options.server_url)?;
    let report = openclaw::probe(&config.openclaw_url).await;
    println!("Vifu doctor");
    println!("State directory: {}", config.home_dir.display());
    print_openclaw_report(&report);
    print_server_config(&config)?;
    print_stored_session(&config);
    match report.status {
        ProbeStatus::Online => {
            match openclaw::discover_agents(&report.endpoint, config.openclaw_token.as_deref())
                .await
            {
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

fn logout(options: Options) -> Result<(), String> {
    let config = Config::load(options.openclaw_url, options.server_url)?;
    match fs::remove_file(config.session_file()) {
        Ok(()) => println!("Removed the local connector session."),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            println!("No local connector session found.");
        }
        Err(error) => return Err(error.to_string()),
    }
    Ok(())
}

fn reset(options: Options) -> Result<(), String> {
    let config = Config::load(options.openclaw_url, options.server_url)?;
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

fn print_openclaw_report(report: &openclaw::ProbeReport) {
    match &report.status {
        ProbeStatus::Online => println!(
            "OpenClaw: online at {}:{}",
            report.endpoint.host, report.endpoint.port
        ),
        ProbeStatus::Offline(reason) => println!(
            "OpenClaw: offline at {}:{} ({reason})",
            report.endpoint.host, report.endpoint.port
        ),
        ProbeStatus::Unsupported(reason) => {
            println!("OpenClaw: unsupported configuration ({reason})");
        }
    }
}

fn print_server_config(config: &Config) -> Result<(), String> {
    println!("Server: {}", config.server_url);
    println!(
        "WebSocket: {}",
        relay::connector_websocket_url(&config.server_url)?
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
            session.connector_id, session_id
        ),
        None => println!("Session: new ({})", session.connector_id),
    }
}

fn load_or_create_session(config: &Config) -> Result<SessionSummary, String> {
    match session::read_session(&config.session_file()) {
        SessionStatus::Ready(summary) => Ok(summary),
        SessionStatus::Missing => {
            let summary = SessionSummary {
                connector_id: format!("connector-{}", Uuid::new_v4().simple()),
                resume_session_id: None,
                created_at_unix: now_unix_seconds()?,
            };
            session::write_session(&config.session_file(), &summary)?;
            Ok(summary)
        }
        SessionStatus::Invalid(reason) => Err(format!(
            "local connector session is invalid: {reason}. Run `vifu --logout` to replace it."
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
