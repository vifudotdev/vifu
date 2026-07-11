use std::net::SocketAddr;
use std::time::Duration;

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeploymentMode {
    Local,
    SelfHosted,
    Cloud,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub addr: SocketAddr,
    pub deployment_mode: DeploymentMode,
    pub database_url: String,
    pub database_max_connections: u32,
    pub admin_key: String,
    pub connector_token: String,
    pub api_key_pepper: String,
    pub request_timeout: Duration,
    pub heartbeat_interval: Duration,
    pub queue_capacity: usize,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    fn from_lookup<F>(mut lookup: F) -> Result<Self, String>
    where
        F: FnMut(&str) -> Option<String>,
    {
        let addr = value(&mut lookup, "VIFU_SERVER_ADDR", "127.0.0.1:6790")
            .parse::<SocketAddr>()
            .map_err(|error| format!("VIFU_SERVER_ADDR is invalid: {error}"))?;
        let database_url = value(
            &mut lookup,
            "DATABASE_URL",
            "postgres://vifu@127.0.0.1:5432/vifu",
        );
        let deployment_mode = parse_deployment_mode(lookup("VIFU_DEPLOYMENT_MODE"))?;
        let admin_key = deployment_secret(
            &mut lookup,
            "VIFU_ADMIN_KEY",
            deployment_mode,
            "vifu-local-admin-key",
        )?;
        let connector_token = deployment_secret(
            &mut lookup,
            "VIFU_CONNECTOR_TOKEN",
            deployment_mode,
            "vifu-local-connector-token",
        )?;
        let api_key_pepper = deployment_secret(
            &mut lookup,
            "VIFU_API_KEY_PEPPER",
            deployment_mode,
            "vifu-local-api-key-pepper",
        )?;

        Ok(Self {
            addr,
            deployment_mode,
            database_url,
            database_max_connections: parse_u64(
                &mut lookup,
                "VIFU_DATABASE_MAX_CONNECTIONS",
                10,
                1,
                100,
            )? as u32,
            admin_key,
            connector_token,
            api_key_pepper,
            request_timeout: Duration::from_millis(parse_u64(
                &mut lookup,
                "VIFU_REQUEST_TIMEOUT_MS",
                30_000,
                500,
                120_000,
            )?),
            heartbeat_interval: Duration::from_millis(parse_u64(
                &mut lookup,
                "VIFU_HEARTBEAT_INTERVAL_MS",
                15_000,
                1_000,
                60_000,
            )?),
            queue_capacity: parse_u64(&mut lookup, "VIFU_CONNECTOR_QUEUE_CAPACITY", 256, 8, 4096)?
                as usize,
        })
    }
}

fn parse_deployment_mode(value: Option<String>) -> Result<DeploymentMode, String> {
    match value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("local")
    {
        "local" => Ok(DeploymentMode::Local),
        "self-hosted" => Ok(DeploymentMode::SelfHosted),
        "cloud" => Ok(DeploymentMode::Cloud),
        other => Err(format!(
            "VIFU_DEPLOYMENT_MODE must be local, self-hosted, or cloud, got {other}"
        )),
    }
}

fn value<F>(lookup: &mut F, key: &str, fallback: &str) -> String
where
    F: FnMut(&str) -> Option<String>,
{
    lookup(key)
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

fn deployment_secret<F>(
    lookup: &mut F,
    key: &str,
    deployment_mode: DeploymentMode,
    local_fallback: &str,
) -> Result<String, String>
where
    F: FnMut(&str) -> Option<String>,
{
    let configured = lookup(key)
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty());
    let secret = match (configured, deployment_mode) {
        (Some(secret), _) => secret,
        (None, DeploymentMode::Local) => local_fallback.to_string(),
        (None, mode) => {
            return Err(format!(
                "{key} is required for {} deployments",
                deployment_mode_name(mode)
            ));
        }
    };
    validate_secret(key, &secret)?;
    Ok(secret)
}

fn deployment_mode_name(mode: DeploymentMode) -> &'static str {
    match mode {
        DeploymentMode::Local => "local",
        DeploymentMode::SelfHosted => "self-hosted",
        DeploymentMode::Cloud => "cloud",
    }
}

fn parse_u64<F>(lookup: &mut F, key: &str, fallback: u64, min: u64, max: u64) -> Result<u64, String>
where
    F: FnMut(&str) -> Option<String>,
{
    let Some(raw) = lookup(key) else {
        return Ok(fallback);
    };
    let parsed = raw
        .trim()
        .parse::<u64>()
        .map_err(|_| format!("{key} must be an integer"))?;
    if !(min..=max).contains(&parsed) {
        return Err(format!("{key} must be between {min} and {max}"));
    }
    Ok(parsed)
}

fn validate_secret(name: &str, value: &str) -> Result<(), String> {
    if value.len() < 16 || value.len() > 512 || value.chars().any(char::is_control) {
        return Err(format!("{name} must contain 16-512 printable characters"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Config, DeploymentMode};

    #[test]
    fn defaults_to_local_postgres_and_loopback() {
        let config = Config::from_lookup(|_| None).unwrap();
        assert_eq!(config.addr.to_string(), "127.0.0.1:6790");
        assert!(config.database_url.contains("127.0.0.1:5432"));
        assert_eq!(config.deployment_mode, DeploymentMode::Local);
        assert_ne!(config.admin_key, config.connector_token);
        assert_ne!(config.admin_key, config.api_key_pepper);
        assert_ne!(config.connector_token, config.api_key_pepper);
    }

    #[test]
    fn rejects_short_admin_keys() {
        let error = Config::from_lookup(|key| match key {
            "VIFU_ADMIN_KEY" => Some("short".to_string()),
            _ => None,
        })
        .unwrap_err();
        assert!(error.contains("VIFU_ADMIN_KEY"));
    }

    #[test]
    fn preserves_cloud_deployment_mode_name() {
        let config = Config::from_lookup(|key| match key {
            "VIFU_DEPLOYMENT_MODE" => Some("cloud".to_string()),
            "VIFU_ADMIN_KEY" => Some("cloud-admin-key-value".to_string()),
            "VIFU_CONNECTOR_TOKEN" => Some("cloud-connector-token".to_string()),
            "VIFU_API_KEY_PEPPER" => Some("cloud-api-key-pepper".to_string()),
            _ => None,
        })
        .unwrap();
        assert_eq!(config.deployment_mode, DeploymentMode::Cloud);
    }

    #[test]
    fn requires_explicit_secrets_for_self_hosted_mode() {
        let error = Config::from_lookup(|key| {
            (key == "VIFU_DEPLOYMENT_MODE").then(|| "self-hosted".to_string())
        })
        .unwrap_err();
        assert!(error.contains("VIFU_ADMIN_KEY"));
    }
}
