use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use serde::Serialize;
use vifu_core::runtime_extension::RuntimeExtensionDefinition;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeploymentMode {
    Local,
    SelfHosted,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub addr: SocketAddr,
    pub deployment_mode: DeploymentMode,
    pub database_url: String,
    pub database_max_connections: u32,
    pub admin_key: String,
    pub agent_gateway_bootstrap_token: String,
    pub api_key_pepper: String,
    pub provider_secret_key: String,
    pub request_timeout: Duration,
    pub heartbeat_interval: Duration,
    pub queue_capacity: usize,
    pub provider_home_dir: PathBuf,
    pub provider_registry_file: Option<PathBuf>,
    pub runtime_extensions: Vec<RuntimeExtensionDefinition>,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    pub fn apply_listen_override(&mut self, addr: SocketAddr) -> Result<(), String> {
        if self.deployment_mode == DeploymentMode::Local && !addr.ip().is_loopback() {
            return Err("local deployments require a loopback server listen address".to_string());
        }
        self.addr = addr;
        Ok(())
    }

    pub fn apply_database_url(&mut self, database_url: String) -> Result<(), String> {
        let database_url = database_url.trim();
        if database_url.is_empty() {
            return Err("server database URL must not be empty".to_string());
        }
        self.database_url = database_url.to_string();
        Ok(())
    }

    pub fn apply_runtime_extensions(
        &mut self,
        runtime_extensions: Vec<RuntimeExtensionDefinition>,
    ) -> Result<(), String> {
        let mut ids = std::collections::HashSet::new();
        for extension in &runtime_extensions {
            if !ids.insert(extension.manifest.id.as_str()) {
                return Err(format!(
                    "runtime extension {} is configured more than once",
                    extension.manifest.id
                ));
            }
        }
        self.runtime_extensions = runtime_extensions;
        Ok(())
    }

    fn from_lookup<F>(mut lookup: F) -> Result<Self, String>
    where
        F: FnMut(&str) -> Option<String>,
    {
        let addr = SocketAddr::from(([127, 0, 0, 1], 6790));
        let deployment_mode =
            parse_deployment_mode(configured_value(&mut lookup, "VIFU_DEPLOYMENT_MODE")?)?;
        let admin_key = deployment_secret(
            &mut lookup,
            "VIFU_ADMIN_KEY",
            deployment_mode,
            "vifu-local-admin-key",
        )?;
        let agent_gateway_bootstrap_token = deployment_secret(
            &mut lookup,
            "VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN",
            deployment_mode,
            "vifu-local-agent-gateway-bootstrap-token",
        )?;
        let api_key_pepper = deployment_secret(
            &mut lookup,
            "VIFU_API_KEY_PEPPER",
            deployment_mode,
            "vifu-local-api-key-pepper",
        )?;
        let provider_secret_key = deployment_secret(
            &mut lookup,
            "VIFU_PROVIDER_SECRET_KEY",
            deployment_mode,
            "vifu-local-provider-secret-key",
        )?;
        let provider_home_dir = vifu_core::config::default_home_dir()?;
        let provider_registry_file =
            vifu_core::config::discover_provider_registry_file(&provider_home_dir);

        Ok(Self {
            addr,
            deployment_mode,
            database_url: "postgres://vifu@127.0.0.1:5432/vifu".to_string(),
            database_max_connections: parse_u64(
                &mut lookup,
                "VIFU_DATABASE_MAX_CONNECTIONS",
                10,
                1,
                100,
            )? as u32,
            admin_key,
            agent_gateway_bootstrap_token,
            api_key_pepper,
            provider_secret_key,
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
            queue_capacity: parse_u64(
                &mut lookup,
                "VIFU_AGENT_GATEWAY_QUEUE_CAPACITY",
                256,
                8,
                4096,
            )? as usize,
            provider_home_dir,
            provider_registry_file,
            runtime_extensions: Vec::new(),
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
        other => Err(format!(
            "VIFU_DEPLOYMENT_MODE must be local or self-hosted, got {other}"
        )),
    }
}

fn configured_value<F>(lookup: &mut F, key: &str) -> Result<Option<String>, String>
where
    F: FnMut(&str) -> Option<String>,
{
    if let Some(value) = lookup(key)
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
    {
        return Ok(Some(value));
    }
    let file_key = format!("{key}_FILE");
    let Some(file_path) = lookup(&file_key)
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
    else {
        return Ok(None);
    };
    let value = std::fs::read_to_string(&file_path)
        .map_err(|error| format!("{file_key} could not be read: {error}"))?
        .trim()
        .to_string();
    if value.is_empty() {
        return Err(format!("{file_key} is empty"));
    }
    Ok(Some(value))
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
    let configured = configured_value(lookup, key)?;
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
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn defaults_to_local_postgres_and_loopback() {
        let config = Config::from_lookup(|_| None).unwrap();
        assert_eq!(config.addr.to_string(), "127.0.0.1:6790");
        assert!(config.database_url.contains("127.0.0.1:5432"));
        assert_eq!(config.deployment_mode, DeploymentMode::Local);
        assert_ne!(config.admin_key, config.agent_gateway_bootstrap_token);
        assert_ne!(config.admin_key, config.api_key_pepper);
        assert_ne!(config.admin_key, config.provider_secret_key);
        assert_ne!(config.agent_gateway_bootstrap_token, config.api_key_pepper);
        assert_ne!(
            config.agent_gateway_bootstrap_token,
            config.provider_secret_key
        );
        assert_ne!(config.api_key_pepper, config.provider_secret_key);
    }

    #[test]
    fn database_url_is_not_read_from_the_environment() {
        let config = Config::from_lookup(|key| match key {
            "DATABASE_URL" => Some("postgres://remote.example/vifu".to_string()),
            _ => None,
        })
        .unwrap();

        assert_eq!(config.database_url, "postgres://vifu@127.0.0.1:5432/vifu");
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
    fn rejects_unknown_deployment_mode() {
        let error = Config::from_lookup(|key| match key {
            "VIFU_DEPLOYMENT_MODE" => Some("unsupported".to_string()),
            _ => None,
        })
        .unwrap_err();
        assert!(error.contains("local or self-hosted"));
    }

    #[test]
    fn local_listen_override_must_remain_loopback() {
        let mut config = Config::from_lookup(|_| None).unwrap();
        let error = config
            .apply_listen_override("0.0.0.0:6790".parse().unwrap())
            .unwrap_err();
        assert!(error.contains("loopback"));
    }

    #[test]
    fn requires_explicit_secrets_for_self_hosted_mode() {
        let error = Config::from_lookup(|key| {
            (key == "VIFU_DEPLOYMENT_MODE").then(|| "self-hosted".to_string())
        })
        .unwrap_err();
        assert!(error.contains("VIFU_ADMIN_KEY"));
    }

    #[test]
    fn self_hosted_accepts_runtime_config_without_auth() {
        let config = Config::from_lookup(|key| match key {
            "VIFU_DEPLOYMENT_MODE" => Some("self-hosted".to_string()),
            "VIFU_ADMIN_KEY" => Some("self-host-admin-key".to_string()),
            "VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN" => {
                Some("self-host-agent-gateway-bootstrap-token".to_string())
            }
            "VIFU_API_KEY_PEPPER" => Some("self-host-api-key-pepper".to_string()),
            "VIFU_PROVIDER_SECRET_KEY" => Some("self-host-provider-secret-key".to_string()),
            _ => None,
        })
        .unwrap();
        assert_eq!(config.deployment_mode, DeploymentMode::SelfHosted);
    }

    #[test]
    fn self_hosted_accepts_secrets_from_files() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("vifu-config-test-{stamp}"));
        fs::create_dir_all(&dir).unwrap();
        let admin_key = dir.join("admin_key");
        let agent_gateway_bootstrap_token = dir.join("agent_gateway_bootstrap_token");
        let api_key_pepper = dir.join("api_key_pepper");
        let provider_secret_key = dir.join("provider_secret_key");
        fs::write(&admin_key, "self-host-admin-key-from-file\n").unwrap();
        fs::write(
            &agent_gateway_bootstrap_token,
            "self-host-agent-gateway-token-from-file\n",
        )
        .unwrap();
        fs::write(&api_key_pepper, "self-host-api-key-pepper-from-file\n").unwrap();
        fs::write(
            &provider_secret_key,
            "self-host-provider-secret-key-from-file\n",
        )
        .unwrap();

        let config = Config::from_lookup(|key| match key {
            "VIFU_DEPLOYMENT_MODE" => Some("self-hosted".to_string()),
            "VIFU_ADMIN_KEY_FILE" => Some(admin_key.display().to_string()),
            "VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN_FILE" => {
                Some(agent_gateway_bootstrap_token.display().to_string())
            }
            "VIFU_API_KEY_PEPPER_FILE" => Some(api_key_pepper.display().to_string()),
            "VIFU_PROVIDER_SECRET_KEY_FILE" => Some(provider_secret_key.display().to_string()),
            _ => None,
        })
        .unwrap();

        assert_eq!(config.deployment_mode, DeploymentMode::SelfHosted);
        assert_eq!(config.admin_key, "self-host-admin-key-from-file");
        fs::remove_dir_all(dir).unwrap();
    }
}
