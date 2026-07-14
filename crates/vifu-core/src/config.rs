use std::path::PathBuf;

pub const DEFAULT_OPENCLAW_URL: &str = "http://127.0.0.1:18789";
pub const DEFAULT_SERVER_URL: &str = "http://127.0.0.1:6790";
pub const DEFAULT_AGENT_GATEWAY_TOKEN: &str = "vifu-local-agent-gateway-token";

#[derive(Debug, Clone)]
pub struct Config {
    pub home_dir: PathBuf,
    pub openclaw_url: String,
    pub openclaw_token: Option<String>,
    pub server_url: String,
    pub agent_gateway_token: String,
}

impl Config {
    pub fn load(openclaw_url: String, server_url: String) -> Result<Self, String> {
        let home_dir = match std::env::var_os("VIFU_HOME") {
            Some(value) if !value.is_empty() => PathBuf::from(value),
            _ => default_home_dir()?,
        };
        let agent_gateway_token = std::env::var("VIFU_AGENT_GATEWAY_TOKEN")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_AGENT_GATEWAY_TOKEN.to_string());
        if agent_gateway_token.len() < 16 || agent_gateway_token.len() > 512 {
            return Err("VIFU_AGENT_GATEWAY_TOKEN must contain 16-512 characters".to_string());
        }
        let openclaw_token = std::env::var("VIFU_OPENCLAW_TOKEN")
            .ok()
            .or_else(|| std::env::var("OPENCLAW_GATEWAY_TOKEN").ok())
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        if openclaw_token
            .as_ref()
            .is_some_and(|token| token.len() > 4096 || token.chars().any(char::is_control))
        {
            return Err("VIFU_OPENCLAW_TOKEN contains invalid characters".to_string());
        }
        Ok(Self {
            home_dir,
            openclaw_url,
            openclaw_token,
            server_url,
            agent_gateway_token,
        })
    }

    pub fn session_file(&self) -> PathBuf {
        self.home_dir.join("agent-gateway-session")
    }
}

fn default_home_dir() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME").ok_or_else(|| {
        "HOME is not set. Set VIFU_HOME to choose where vifu stores local state.".to_string()
    })?;
    Ok(PathBuf::from(home).join(".vifu"))
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_OPENCLAW_URL, DEFAULT_SERVER_URL};

    #[test]
    fn default_urls_target_local_services() {
        assert_eq!(DEFAULT_OPENCLAW_URL, "http://127.0.0.1:18789");
        assert_eq!(DEFAULT_SERVER_URL, "http://127.0.0.1:6790");
    }
}
