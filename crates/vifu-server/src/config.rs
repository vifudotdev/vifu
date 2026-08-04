use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use serde::Serialize;
use sha2::Digest;
use vifu_gateway::runtime_extension::RuntimeExtensionDefinition;

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
    pub service_version: String,
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
    pub access_token_authority: Option<AccessTokenAuthorityConfig>,
    pub guest_bootstrap_enabled: bool,
    pub guest_project_ttl: Duration,
    pub guest_project_limit: u32,
    pub server_url: Option<String>,
    pub dashboard_addr: Option<String>,
    pub tls: Option<ServerTlsConfig>,
}

#[derive(Debug, Clone)]
pub struct ServerTlsConfig {
    pub certificate_path: PathBuf,
    pub private_key_path: PathBuf,
    pub certificate_der_path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct AccessTokenAuthorityConfig {
    pub url: String,
    pub deployment_id: String,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    pub fn apply_service_version(
        &mut self,
        service_version: impl AsRef<str>,
    ) -> Result<(), String> {
        let service_version = service_version.as_ref().trim();
        if service_version.is_empty() {
            return Err("server service version must not be empty".to_string());
        }
        self.service_version = service_version.to_string();
        Ok(())
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

    pub fn apply_access_token_authority(
        &mut self,
        access_token_authority: Option<AccessTokenAuthorityConfig>,
    ) -> Result<(), String> {
        if let Some(authority) = access_token_authority.as_ref() {
            authority.validate()?;
        }
        self.access_token_authority = access_token_authority;
        Ok(())
    }

    pub fn apply_guest_bootstrap(
        &mut self,
        enabled: bool,
        ttl: Duration,
        project_limit: u32,
    ) -> Result<(), String> {
        if !(Duration::from_secs(60 * 60)..=Duration::from_secs(30 * 24 * 60 * 60)).contains(&ttl) {
            return Err("guest project TTL must be between 1 hour and 30 days".to_string());
        }
        if !(1..=1_000_000).contains(&project_limit) {
            return Err("guest project limit must be between 1 and 1000000".to_string());
        }
        self.guest_bootstrap_enabled = enabled;
        self.guest_project_ttl = ttl;
        self.guest_project_limit = project_limit;
        Ok(())
    }

    pub fn apply_server_url(&mut self, server_url: impl Into<String>) -> Result<(), String> {
        let server_url = server_url.into();
        let url = reqwest::Url::parse(server_url.trim())
            .map_err(|error| format!("server URL is invalid: {error}"))?;
        let Some(host) = url.host_str() else {
            return Err("server API address must include a host".to_string());
        };
        let is_loopback = host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|address| address.is_loopback());
        if url.scheme() != "https" && !(url.scheme() == "http" && is_loopback) {
            return Err(
                "server API address must use HTTPS except on loopback and include a host"
                    .to_string(),
            );
        }
        if !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
        {
            return Err(
                "server API address must be an origin without credentials, a path, query, or fragment"
                    .to_string(),
            );
        }
        self.server_url = Some(server_url.trim_end_matches('/').to_string());
        Ok(())
    }

    pub fn apply_dashboard_addr(&mut self, address: impl Into<String>) -> Result<(), String> {
        let address = address.into();
        let address = address.trim();
        let url = reqwest::Url::parse(&format!("http://{address}"))
            .map_err(|error| format!("dashboard address is invalid: {error}"))?;
        if url.host_str().is_none()
            || url.port().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err("dashboard address must be a host and port".to_string());
        }
        self.dashboard_addr = Some(address.to_string());
        Ok(())
    }

    pub fn apply_generated_tls(&mut self) -> Result<(), String> {
        let server_url = self
            .server_url
            .as_deref()
            .ok_or_else(|| "generated server TLS requires server.address".to_string())?;
        let certificate_id = sha2::Sha256::digest(server_url.as_bytes())[..8]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        self.tls = Some(ServerTlsConfig {
            certificate_path: self
                .provider_home_dir
                .join(format!("server-{certificate_id}-cert.pem")),
            private_key_path: self
                .provider_home_dir
                .join(format!("server-{certificate_id}-key.pem")),
            certificate_der_path: self
                .provider_home_dir
                .join(format!("server-{certificate_id}-cert.der.b64")),
        });
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
        let provider_home_dir = vifu_gateway::config::default_home_dir()?;
        let provider_registry_file =
            vifu_gateway::config::discover_provider_registry_file(&provider_home_dir);
        let database_url = format!(
            "sqlite://{}",
            provider_home_dir.join("vifu.sqlite").display()
        );
        let access_token_authority = access_token_authority(&mut lookup)?;

        Ok(Self {
            addr,
            deployment_mode,
            service_version: env!("CARGO_PKG_VERSION").to_string(),
            database_url,
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
            access_token_authority,
            guest_bootstrap_enabled: false,
            guest_project_ttl: Duration::from_secs(7 * 24 * 60 * 60),
            guest_project_limit: 10_000,
            server_url: None,
            dashboard_addr: None,
            tls: None,
        })
    }
}

impl AccessTokenAuthorityConfig {
    pub fn new(url: impl Into<String>, deployment_id: impl Into<String>) -> Result<Self, String> {
        let config = Self {
            url: url.into(),
            deployment_id: deployment_id.into(),
        };
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), String> {
        let url = reqwest::Url::parse(self.url.trim())
            .map_err(|error| format!("access-token authority URL is invalid: {error}"))?;
        let host = url
            .host_str()
            .ok_or_else(|| "access-token authority URL must include a host".to_string())?;
        let loopback = host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback());
        if url.scheme() != "https" && !(url.scheme() == "http" && loopback) {
            return Err(
                "access-token authority URL must use HTTPS, except on loopback".to_string(),
            );
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err("access-token authority URL must not contain credentials".to_string());
        }
        validate_deployment_id(&self.deployment_id)
    }
}

fn access_token_authority<F>(lookup: &mut F) -> Result<Option<AccessTokenAuthorityConfig>, String>
where
    F: FnMut(&str) -> Option<String>,
{
    let url = configured_value(lookup, "VIFU_ACCESS_TOKEN_AUTHORITY_URL")?;
    let deployment_id = configured_value(lookup, "VIFU_DEPLOYMENT_ID")?;
    match (url, deployment_id) {
        (None, None) => Ok(None),
        (Some(url), Some(deployment_id)) => {
            AccessTokenAuthorityConfig::new(url, deployment_id).map(Some)
        }
        _ => Err(
            "VIFU_ACCESS_TOKEN_AUTHORITY_URL and VIFU_DEPLOYMENT_ID must be configured together"
                .to_string(),
        ),
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

fn validate_deployment_id(value: &str) -> Result<(), String> {
    let value = value.trim();
    let suffix = value.strip_prefix("dep_").unwrap_or_default();
    let mut bytes = suffix.bytes();
    if !(12..=128).contains(&value.len())
        || !bytes
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        || !bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return Err(
            "VIFU_DEPLOYMENT_ID must start with dep_ and contain 12-128 ASCII letters, numbers, underscores, or hyphens"
                .to_string(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Config, DeploymentMode};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn defaults_to_local_sqlite_and_loopback() {
        let config = Config::from_lookup(|_| None).unwrap();
        assert_eq!(config.addr.to_string(), "127.0.0.1:6790");
        assert!(config.database_url.starts_with("sqlite://"));
        assert!(config.database_url.ends_with("/.vifu/vifu.sqlite"));
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

        assert!(config.database_url.starts_with("sqlite://"));
        assert!(config.database_url.ends_with("/.vifu/vifu.sqlite"));
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
    fn server_endpoint_reuses_the_primary_listener() {
        let mut config = Config::from_lookup(|_| None).unwrap();
        config
            .apply_server_url("https://macbook.local:6790")
            .unwrap();
        config.apply_generated_tls().unwrap();

        assert_eq!(config.addr.to_string(), "127.0.0.1:6790");
        assert_eq!(
            config.server_url.as_deref(),
            Some("https://macbook.local:6790")
        );
        assert!(config
            .tls
            .unwrap()
            .certificate_der_path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .ends_with(".der.b64"));
    }

    #[test]
    fn server_endpoint_must_be_an_https_origin() {
        let mut config = Config::from_lookup(|_| None).unwrap();
        assert!(config
            .apply_server_url("https://macbook.local:6790/not-an-origin")
            .is_err());
        assert!(config
            .apply_server_url("http://macbook.local:6790")
            .is_err());
    }

    #[test]
    fn server_endpoint_allows_plaintext_only_on_loopback() {
        let mut config = Config::from_lookup(|_| None).unwrap();

        config.apply_server_url("http://127.0.0.1:6790").unwrap();

        assert_eq!(config.server_url.as_deref(), Some("http://127.0.0.1:6790"));
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
    fn accepts_a_complete_access_token_authority_configuration() {
        let config = Config::from_lookup(|key| match key {
            "VIFU_ACCESS_TOKEN_AUTHORITY_URL" => {
                Some("https://auth.vifu.test/v1/deployments/authorize".to_string())
            }
            "VIFU_DEPLOYMENT_ID" => Some("dep_01JTESTDEPLOYMENT".to_string()),
            _ => None,
        })
        .unwrap();
        let authority = config.access_token_authority.unwrap();
        assert_eq!(
            authority.url,
            "https://auth.vifu.test/v1/deployments/authorize"
        );
        assert_eq!(authority.deployment_id, "dep_01JTESTDEPLOYMENT");
    }

    #[test]
    fn rejects_a_partial_access_token_authority_configuration() {
        let error = Config::from_lookup(|key| {
            (key == "VIFU_ACCESS_TOKEN_AUTHORITY_URL")
                .then(|| "https://auth.vifu.test/v1/deployments/authorize".to_string())
        })
        .unwrap_err();
        assert!(error.contains("must be configured together"));
    }

    #[test]
    fn rejects_account_derived_deployment_identifiers() {
        let error = Config::from_lookup(|key| match key {
            "VIFU_ACCESS_TOKEN_AUTHORITY_URL" => {
                Some("https://auth.vifu.test/v1/deployments/authorize".to_string())
            }
            "VIFU_DEPLOYMENT_ID" => Some("account:user-test".to_string()),
            _ => None,
        })
        .unwrap_err();
        assert!(error.contains("VIFU_DEPLOYMENT_ID"));
    }

    #[test]
    fn rejects_plaintext_remote_access_token_authorities() {
        let error = Config::from_lookup(|key| match key {
            "VIFU_ACCESS_TOKEN_AUTHORITY_URL" => {
                Some("http://auth.vifu.test/v1/deployments/authorize".to_string())
            }
            "VIFU_DEPLOYMENT_ID" => Some("dep_01JTESTDEPLOYMENT".to_string()),
            _ => None,
        })
        .unwrap_err();
        assert!(error.contains("must use HTTPS"));
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

    #[test]
    fn dashboard_proxy_uses_an_internal_host_and_port_address() {
        let mut config = Config::from_lookup(|_| None).unwrap();

        config.apply_dashboard_addr("dashboard:6791").unwrap();

        assert_eq!(config.dashboard_addr.as_deref(), Some("dashboard:6791"));
        assert!(config
            .apply_dashboard_addr("https://dashboard:6791/path")
            .is_err());
        assert!(config.apply_dashboard_addr("dashboard").is_err());
    }
}
