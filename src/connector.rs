use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::cli::{help_text, Command, Options};
use crate::config::Config;
use crate::deployment;
use crate::openclaw::{self, ProbeStatus};
use crate::relay;
use crate::session::{self, SessionStatus, SessionSummary};

pub fn execute(options: Options) -> Result<(), String> {
    match options.command {
        Command::Help => {
            print!("{}", help_text());
            Ok(())
        }
        Command::Version => {
            println!("vifu {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Command::Connect => connect(options),
        Command::Deploy | Command::Server => deploy(options),
        Command::Status => status(options),
        Command::Doctor => doctor(options),
        Command::Logout => logout(options),
        Command::Reset => reset(options),
    }
}

fn connect(options: Options) -> Result<(), String> {
    let config = Config::load(
        options.openclaw_url,
        options.relay_addr,
        options.listen_addr,
    )?;
    ensure_home_dir(&config)?;
    let report = openclaw::probe(&config.openclaw_url);
    let relay_session =
        if matches!(report.status, ProbeStatus::Online) && config.relay_addr.is_some() {
            Some(load_or_create_session(&config)?)
        } else {
            None
        };

    println!("Vifu connector");
    print_openclaw_report(&report);
    print_relay_config(&config);
    print_session_status(&config, relay_session.as_ref());

    match &report.status {
        ProbeStatus::Online => {
            if let Some(relay_addr) = config.relay_addr.as_deref() {
                let session = relay_session
                    .as_ref()
                    .ok_or_else(|| "relay session was not initialized".to_string())?;
                relay::run_client(relay_addr, &session.device_id, &report.endpoint)
            } else {
                Ok(())
            }
        }
        ProbeStatus::Offline(_) | ProbeStatus::Unsupported(_) => Err(
            "OpenClaw Gateway is not reachable. Run `vifu --doctor` for setup checks.".to_string(),
        ),
    }
}

fn deploy(options: Options) -> Result<(), String> {
    let config = Config::load(
        options.openclaw_url,
        options.relay_addr,
        options.listen_addr,
    )?;
    print_deployment_config(&config);
    let _deployment = deployment::start(&config.deployment)?;
    relay::run_server(&config.listen_addr)
}

fn status(options: Options) -> Result<(), String> {
    let config = Config::load(
        options.openclaw_url,
        options.relay_addr,
        options.listen_addr,
    )?;
    let report = openclaw::probe(&config.openclaw_url);

    println!("Vifu status");
    println!("State: {}", config.home_dir.display());
    print_deployment_config(&config);
    print_openclaw_report(&report);
    print_relay_config(&config);
    print_session_status(&config, None);
    Ok(())
}

fn doctor(options: Options) -> Result<(), String> {
    let config = Config::load(
        options.openclaw_url,
        options.relay_addr,
        options.listen_addr,
    )?;
    let report = openclaw::probe(&config.openclaw_url);

    println!("Vifu doctor");
    println!("State directory: {}", config.home_dir.display());
    println!("Server listen: {}", config.listen_addr);
    print_deployment_config(&config);
    print_openclaw_report(&report);
    print_relay_config(&config);
    print_session_status(&config, None);

    match report.status {
        ProbeStatus::Online => {
            println!("OpenClaw: ready");
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
    let config = Config::load(
        options.openclaw_url,
        options.relay_addr,
        options.listen_addr,
    )?;
    match fs::remove_file(config.session_file()) {
        Ok(()) => println!("Removed local Vifu session state."),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            println!("No local Vifu session state found.");
        }
        Err(error) => return Err(error.to_string()),
    }
    Ok(())
}

fn reset(options: Options) -> Result<(), String> {
    let config = Config::load(
        options.openclaw_url,
        options.relay_addr,
        options.listen_addr,
    )?;
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
        ProbeStatus::Online => {
            println!(
                "OpenClaw: online at {}:{}",
                report.endpoint.host, report.endpoint.port
            );
        }
        ProbeStatus::Offline(reason) => {
            println!(
                "OpenClaw: offline at {}:{} ({reason})",
                report.endpoint.host, report.endpoint.port
            );
        }
        ProbeStatus::Unsupported(reason) => {
            println!("OpenClaw: unsupported configuration ({reason})");
        }
    }
}

fn print_deployment_config(config: &Config) {
    println!("Deployment: {}", config.deployment.target.label());
}

fn print_relay_config(config: &Config) {
    match config.relay_addr.as_deref() {
        Some(addr) => println!("Relay: {addr}"),
        None => println!("Relay: not configured"),
    }
}

fn print_session_status(config: &Config, current: Option<&SessionSummary>) {
    if current.is_some() {
        println!("Session: paired");
        return;
    }

    match session::read_session(&config.session_file()) {
        SessionStatus::Ready(_) => println!("Session: paired"),
        SessionStatus::Missing => println!("Session: not paired"),
        SessionStatus::Invalid(reason) => println!("Session: invalid ({reason})"),
    }
}

fn load_or_create_session(config: &Config) -> Result<SessionSummary, String> {
    match session::read_session(&config.session_file()) {
        SessionStatus::Ready(summary) => Ok(summary),
        SessionStatus::Missing => {
            let summary = SessionSummary {
                device_id: generate_device_id()?,
                created_at_unix: now_unix_seconds()?,
            };
            session::write_session(&config.session_file(), &summary)?;
            Ok(summary)
        }
        SessionStatus::Invalid(reason) => Err(format!("local session is invalid: {reason}")),
    }
}

fn generate_device_id() -> Result<String, String> {
    Ok(format!(
        "local-{}-{}",
        now_unix_seconds()?,
        std::process::id()
    ))
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
    fn reset_rejects_unscoped_state_dir() {
        let error = ensure_safe_reset_dir(&PathBuf::from("/Users/example")).unwrap_err();
        assert!(error.contains("Refusing to reset"));
    }

    #[test]
    fn reset_rejects_relative_state_dir() {
        let error = ensure_safe_reset_dir(&PathBuf::from(".vifu")).unwrap_err();
        assert!(error.contains("absolute"));
    }

    #[test]
    fn reset_rejects_root_level_state_dir() {
        let error = ensure_safe_reset_dir(&PathBuf::from("/.vifu")).unwrap_err();
        assert!(error.contains("root-level"));
    }
}
