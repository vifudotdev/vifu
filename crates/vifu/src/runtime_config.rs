use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::gateway::GatewayRuntimeOptions;

const CONFIG_FILE_NAME: &str = "config.toml";
const CONFIG_PROFILE_SUFFIX: &str = ".toml";
const DEFAULT_LOCAL_DATABASE_FILE: &str = "vifu.sqlite";

#[derive(Debug, Clone)]
pub struct LoadedRuntimeConfig {
    pub path: PathBuf,
    pub profile: Option<String>,
    pub config: RuntimeConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<ServerRuntimeConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway: Option<GatewayRuntimeConfig>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ServerRuntimeConfig {
    pub address: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dashboard: Option<ServerDashboardConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deployment_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database_url_file: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runtime_extensions: Vec<RuntimeExtensionConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authority: Option<AccessTokenAuthorityConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guest_bootstrap: Option<GuestBootstrapConfig>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ServerDashboardConfig {
    pub address: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GuestBootstrapConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_guest_ttl_hours")]
    pub ttl_hours: u64,
    #[serde(default = "default_guest_project_limit")]
    pub max_projects: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeExtensionConfig {
    pub manifest: PathBuf,
    pub base_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_file: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AccessTokenAuthorityConfig {
    pub url: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayRuntimeConfig {
    pub address: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guest_bootstrap: Option<bool>,
}

impl LoadedRuntimeConfig {
    pub fn load(config_profile: Option<&str>, config_overrides: &[String]) -> Result<Self, String> {
        let home_dir = vifu_gateway::config::default_home_dir()?;
        let (path, config) = match config_profile {
            Some(profile) => {
                let path = home_dir.join(format!("{profile}{CONFIG_PROFILE_SUFFIX}"));
                let config = RuntimeConfig::load_profile(&path)?;
                (path, config)
            }
            None => {
                let path = home_dir.join(CONFIG_FILE_NAME);
                let config = RuntimeConfig::load_or_create(&path)?;
                (path, config)
            }
        };
        let config = config.with_overrides(&path, config_overrides)?;
        if config.gateway.is_some() {
            vifu_gateway::config::ensure_provider_registry_file(&home_dir)?;
        }
        Ok(Self {
            path,
            profile: config_profile.map(str::to_string),
            config,
        })
    }

    pub fn gateway_options(&self) -> Result<GatewayRuntimeOptions, String> {
        let gateway = self.config.gateway.as_ref().ok_or_else(|| {
            format!(
                "{} does not configure an Agent Gateway",
                self.path.display()
            )
        })?;
        gateway.validate()?;
        let server = self
            .config
            .server
            .as_ref()
            .ok_or_else(|| "an Agent Gateway requires a configured Vifu Server".to_string())?;
        let server_url = server.address.clone();
        let local_server = server.local_socket_addr()?.is_some();
        let local_guest_enabled = server
            .guest_bootstrap
            .as_ref()
            .is_some_and(|guest| guest.enabled);
        let allow_guest_bootstrap = gateway.guest_bootstrap.unwrap_or(if local_server {
            local_guest_enabled
        } else {
            true
        });
        if local_server && allow_guest_bootstrap && !local_guest_enabled {
            return Err(
                "gateway.guest_bootstrap requires server.guest_bootstrap.enabled for a local Server"
                    .to_string(),
            );
        }
        Ok(GatewayRuntimeOptions {
            server_url,
            server_certificate_der: None,
            allow_guest_bootstrap,
            enrollment_token: None,
            session_scope: self
                .profile
                .clone()
                .unwrap_or_else(|| "default".to_string()),
        })
    }

    pub fn server_config(&self) -> Result<vifu_server::config::Config, String> {
        let server =
            self.config.server.as_ref().ok_or_else(|| {
                format!("{} does not configure a Vifu Server", self.path.display())
            })?;
        let local_address = server.local_socket_addr()?;
        let managed_lan = local_address.is_some_and(|address| !address.ip().is_loopback())
            && std::env::var_os("VIFU_DEPLOYMENT_MODE").is_none();
        let mut config = if managed_lan {
            vifu_server::config::Config::from_env_with_managed_self_hosted_secrets(
                &managed_server_secret_dir(&self.path),
            )?
        } else {
            vifu_server::config::Config::from_env()?
        };
        config.apply_service_version(env!("CARGO_PKG_VERSION"))?;
        let listen_address = server_listen_address(
            config.deployment_mode,
            config.addr,
            local_address,
            &server.address,
        )?;
        config.apply_listen_override(listen_address)?;
        config.apply_server_url(&server.address)?;
        if let Some(dashboard) = server.dashboard.as_ref() {
            config.apply_dashboard_addr(&dashboard.address)?;
        }
        if local_address.is_some() && server.address.trim_start().starts_with("https://") {
            config.apply_generated_tls()?;
        }
        config.apply_database_url(server.database_url(&self.path)?)?;
        config.apply_runtime_extensions(server.runtime_extensions(&self.path)?)?;
        if server.authority.is_some() {
            config.apply_access_token_authority(server.access_token_authority()?)?;
        }
        if let Some(guest) = server.guest_bootstrap.as_ref() {
            config.apply_guest_bootstrap(
                guest.enabled,
                std::time::Duration::from_secs(guest.ttl_hours.saturating_mul(60 * 60)),
                guest.max_projects,
            )?;
        }
        Ok(config)
    }

    pub fn server_is_local(&self) -> Result<bool, String> {
        let address_is_local = self
            .config
            .server
            .as_ref()
            .map(ServerRuntimeConfig::local_socket_addr)
            .transpose()
            .map(|address| address.flatten().is_some())?;
        let deployment_mode = std::env::var("VIFU_DEPLOYMENT_MODE").ok();
        Ok(address_is_local || deployment_owns_server(deployment_mode.as_deref())?)
    }

    pub fn server_address(&self) -> Result<&str, String> {
        self.config
            .server
            .as_ref()
            .map(|server| server.address.as_str())
            .ok_or_else(|| format!("{} does not configure a Vifu Server", self.path.display()))
    }

    /// Browser-visible Dashboard URL when the Server proxy is explicitly configured.
    pub fn dashboard_url(&self) -> Option<String> {
        self.config
            .server
            .as_ref()
            .and_then(|server| server.dashboard.as_ref().map(|_| server.address.clone()))
    }

    pub fn gateway_is_local(&self) -> Result<bool, String> {
        self.config
            .gateway
            .as_ref()
            .map(GatewayRuntimeConfig::local_socket_addr)
            .transpose()
            .map(|address| address.flatten().is_some())
    }
}

fn managed_server_secret_dir(config_path: &Path) -> PathBuf {
    let stem = config_path
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("config");
    config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!("{stem}-server-secrets"))
}

impl RuntimeConfig {
    fn load_or_create(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(raw) => {
                let (config, migrated) = Self::parse_document(path, &raw)?;
                if migrated {
                    config.write(path)?;
                }
                Ok(config)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let config = Self::local_defaults()?;
                config.write(path)?;
                Ok(config)
            }
            Err(error) => Err(format!(
                "Vifu runtime configuration {} could not be read: {error}",
                path.display()
            )),
        }
    }

    fn local_defaults() -> Result<Self, String> {
        Ok(Self {
            server: Some(ServerRuntimeConfig {
                address: vifu_gateway::config::DEFAULT_SERVER_URL.to_string(),
                dashboard: None,
                deployment_id: None,
                database_url: None,
                database_url_file: None,
                runtime_extensions: Vec::new(),
                authority: None,
                guest_bootstrap: Some(GuestBootstrapConfig {
                    enabled: true,
                    ttl_hours: default_guest_ttl_hours(),
                    max_projects: default_guest_project_limit(),
                }),
            }),
            gateway: Some(GatewayRuntimeConfig {
                address: vifu_gateway::config::DEFAULT_SERVER_URL.to_string(),
                guest_bootstrap: None,
            }),
        })
    }

    fn load_profile(path: &Path) -> Result<Self, String> {
        let raw = std::fs::read_to_string(path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                format!(
                    "Vifu configuration profile {} was not found",
                    path.display()
                )
            } else {
                format!(
                    "Vifu configuration profile {} could not be read: {error}",
                    path.display()
                )
            }
        })?;
        let (config, migrated) = Self::parse_document(path, &raw)?;
        if migrated {
            config.write(path)?;
        }
        Ok(config)
    }

    fn with_overrides(self, path: &Path, overrides: &[String]) -> Result<Self, String> {
        if overrides.is_empty() {
            return Ok(self);
        }

        let mut root = serde_json::to_value(self)
            .map_err(|error| format!("Vifu runtime configuration could not be encoded: {error}"))?;
        for raw_override in overrides {
            let (config_path, value) = parse_config_override(raw_override)?;
            let segments = config_path.split('.').collect::<Vec<_>>();
            apply_json_override(&mut root, &segments, value)?;
        }
        let config = serde_json::from_value(root).map_err(|error| {
            format!("Vifu runtime configuration is invalid after applying -c/--config: {error}")
        })?;
        Self::validate(path, config).map_err(|error| format!("{error} after applying -c/--config"))
    }

    fn write(&self, path: &Path) -> Result<(), String> {
        let toml = toml::to_string_pretty(self)
            .map_err(|error| format!("Vifu runtime configuration could not be encoded: {error}"))?;
        vifu_gateway::config::write_private_file(path, &toml)
    }

    #[cfg(test)]
    fn parse(path: &Path, raw: &str) -> Result<Self, String> {
        Self::parse_document(path, raw).map(|(config, _)| config)
    }

    fn parse_document(path: &Path, raw: &str) -> Result<(Self, bool), String> {
        let table = raw.parse::<toml::Table>().map_err(|error| {
            format!(
                "Vifu runtime configuration {} is invalid: {error}",
                path.display()
            )
        })?;
        match toml::Value::Table(table.clone()).try_into::<Self>() {
            Ok(config) => Self::validate(path, config).map(|config| (config, false)),
            Err(current_error) => {
                let Some(config) = migrate_pre_address_config(table)? else {
                    return Err(format!(
                        "Vifu runtime configuration {} is invalid: {current_error}",
                        path.display()
                    ));
                };
                Self::validate(path, config).map(|config| (config, true))
            }
        }
    }

    fn validate(path: &Path, config: Self) -> Result<Self, String> {
        if config.server.is_none() && config.gateway.is_none() {
            return Err(format!(
                "Vifu runtime configuration {} must configure server, gateway, or both",
                path.display()
            ));
        }
        if let Some(server) = config.server.as_ref() {
            server.validate()?;
        }
        if let Some(gateway) = config.gateway.as_ref() {
            if config.server.is_none() {
                return Err("an Agent Gateway requires a configured Vifu Server".to_string());
            }
            gateway.validate()?;
        }
        Ok(config)
    }
}

fn migrate_pre_address_config(mut root: toml::Table) -> Result<Option<RuntimeConfig>, String> {
    let server_has_legacy_fields = root
        .get("server")
        .and_then(toml::Value::as_table)
        .is_some_and(|server| {
            server.contains_key("api_addr")
                || server.contains_key("listener")
                || server.contains_key("tls")
        });
    if !root.contains_key("version") && !server_has_legacy_fields {
        return Ok(None);
    }

    if let Some(version) = root.remove("version") {
        if version.as_integer() != Some(1) {
            return Err("legacy Vifu runtime configuration version must be 1".to_string());
        }
    }

    if let Some(server) = root.get_mut("server").and_then(toml::Value::as_table_mut) {
        let legacy_api_address = server
            .remove("api_addr")
            .and_then(|value| value.as_str().map(str::to_string));
        let legacy_listener_address = server
            .remove("listener")
            .and_then(|value| value.as_table().cloned())
            .and_then(|mut listener| listener.remove("address"))
            .and_then(|value| value.as_str().map(str::to_string));
        server.remove("tls");

        if !server.contains_key("address") {
            let address = legacy_api_address
                .or_else(|| legacy_listener_address.as_deref().map(origin_from_listener))
                .unwrap_or_else(|| vifu_gateway::config::DEFAULT_SERVER_URL.to_string());
            server.insert("address".to_string(), toml::Value::String(address));
        }
    }

    if let Some(gateway) = root.get_mut("gateway").and_then(toml::Value::as_table_mut) {
        if !gateway.contains_key("address") {
            gateway.insert(
                "address".to_string(),
                toml::Value::String(vifu_gateway::config::DEFAULT_SERVER_URL.to_string()),
            );
        }
    }

    toml::Value::Table(root)
        .try_into::<RuntimeConfig>()
        .map(Some)
        .map_err(|error| format!("legacy Vifu runtime configuration is invalid: {error}"))
}

fn origin_from_listener(listener: &str) -> String {
    let listener = listener.trim();
    if let Ok(address) = listener.parse::<std::net::SocketAddr>() {
        let host = match address.ip() {
            std::net::IpAddr::V4(ip) if ip.is_unspecified() => "127.0.0.1".to_string(),
            std::net::IpAddr::V6(ip) if ip.is_unspecified() => "[::1]".to_string(),
            ip => ip.to_string(),
        };
        return format!("http://{host}:{}", address.port());
    }
    format!("http://{listener}")
}

fn parse_config_override(raw: &str) -> Result<(&str, serde_json::Value), String> {
    let (path, raw_value) = raw
        .split_once('=')
        .ok_or_else(|| "configuration override must use key=value".to_string())?;
    let path = path.trim();
    if path.is_empty() || path.split('.').any(|segment| segment.trim().is_empty()) {
        return Err("configuration override must use a non-empty dotted path".to_string());
    }

    let raw_value = raw_value.trim();
    let document = format!("value = {raw_value}");
    let value = document
        .parse::<toml::Table>()
        .ok()
        .and_then(|mut table| table.remove("value"))
        .and_then(|value| serde_json::to_value(value).ok())
        .unwrap_or_else(|| {
            serde_json::Value::String(
                raw_value
                    .trim_matches(|character| character == '"' || character == '\'')
                    .to_string(),
            )
        });
    Ok((path, value))
}

fn apply_json_override(
    root: &mut serde_json::Value,
    segments: &[&str],
    value: serde_json::Value,
) -> Result<(), String> {
    let Some((segment, remaining)) = segments.split_first() else {
        return Err("configuration override path must not be empty".to_string());
    };

    if !root.is_object() {
        *root = serde_json::Value::Object(serde_json::Map::new());
    }
    let serde_json::Value::Object(object) = root else {
        return Err("configuration override could not be applied".to_string());
    };

    if remaining.is_empty() {
        object.insert((*segment).to_string(), value);
        return Ok(());
    }

    let child = object
        .entry((*segment).to_string())
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    apply_json_override(child, remaining, value)
}

const fn default_guest_ttl_hours() -> u64 {
    7 * 24
}

const fn default_guest_project_limit() -> u32 {
    10_000
}

fn deployment_owns_server(value: Option<&str>) -> Result<bool, String> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        None | Some("local") => Ok(false),
        Some("self-hosted") => Ok(true),
        Some(other) => Err(format!(
            "VIFU_DEPLOYMENT_MODE must be local or self-hosted, got {other}"
        )),
    }
}

fn server_listen_address(
    deployment_mode: vifu_server::config::DeploymentMode,
    default_address: std::net::SocketAddr,
    local_address: Option<std::net::SocketAddr>,
    configured_address: &str,
) -> Result<std::net::SocketAddr, String> {
    if deployment_mode == vifu_server::config::DeploymentMode::SelfHosted {
        return Ok(std::net::SocketAddr::from((
            [0, 0, 0, 0],
            local_address.map_or(default_address.port(), |address| address.port()),
        )));
    }
    local_address.ok_or_else(|| {
        format!(
            "server.address {configured_address:?} is remote; only a self-hosted Vifu Server process can serve a remote public address"
        )
    })
}

impl ServerRuntimeConfig {
    fn validate(&self) -> Result<(), String> {
        if self.database_url.is_some() && self.database_url_file.is_some() {
            return Err(
                "server configuration can set database_url or database_url_file, not both"
                    .to_string(),
            );
        }
        match (self.deployment_id.as_deref(), self.authority.as_ref()) {
            (Some(deployment_id), Some(authority)) => {
                validate_deployment_id(deployment_id)?;
                authority.validate()?;
            }
            (None, None) => {}
            _ => {
                return Err(
                    "server deployment_id and authority must be configured together".to_string(),
                )
            }
        }
        if let Some(guest) = self.guest_bootstrap.as_ref() {
            let ttl = std::time::Duration::from_secs(guest.ttl_hours.saturating_mul(60 * 60));
            if !(std::time::Duration::from_secs(60 * 60)
                ..=std::time::Duration::from_secs(30 * 24 * 60 * 60))
                .contains(&ttl)
            {
                return Err("guest_bootstrap.ttl_hours must be between 1 and 720".to_string());
            }
            if !(1..=1_000_000).contains(&guest.max_projects) {
                return Err(
                    "guest_bootstrap.max_projects must be between 1 and 1000000".to_string()
                );
            }
        }
        vifu_gateway::relay::agent_gateway_websocket_url(&self.address)
            .map_err(|error| format!("server.address is invalid: {error}"))?;
        Ok(())
    }

    pub fn local_socket_addr(&self) -> Result<Option<std::net::SocketAddr>, String> {
        self.validate()?;
        vifu_gateway::config::local_component_socket_addr(&self.address)
    }

    fn database_url(&self, config_path: &Path) -> Result<String, String> {
        self.validate()?;
        if let Some(database_url) = self.database_url.as_deref() {
            let database_url = database_url.trim();
            if database_url.is_empty() {
                return Err("server database_url must not be empty".to_string());
            }
            return Ok(database_url.to_string());
        }
        if let Some(path) = self.database_url_file.as_ref() {
            let database_url = std::fs::read_to_string(path).map_err(|error| {
                format!(
                    "server database_url_file {} could not be read: {error}",
                    path.display()
                )
            })?;
            let database_url = database_url.trim();
            if database_url.is_empty() {
                return Err(format!(
                    "server database_url_file {} is empty",
                    path.display()
                ));
            }
            return Ok(database_url.to_string());
        }
        let base_dir = config_path.parent().unwrap_or_else(|| Path::new("."));
        Ok(format!(
            "sqlite://{}",
            base_dir.join(DEFAULT_LOCAL_DATABASE_FILE).display()
        ))
    }

    fn runtime_extensions(
        &self,
        config_path: &Path,
    ) -> Result<Vec<vifu_gateway::runtime_extension::RuntimeExtensionDefinition>, String> {
        let base_dir = config_path.parent().unwrap_or_else(|| Path::new("."));
        self.runtime_extensions
            .iter()
            .map(|extension| {
                let manifest_path = if extension.manifest.is_absolute() {
                    extension.manifest.clone()
                } else {
                    base_dir.join(&extension.manifest)
                };
                let raw = std::fs::read_to_string(&manifest_path).map_err(|error| {
                    format!(
                        "runtime extension manifest {} could not be read: {error}",
                        manifest_path.display()
                    )
                })?;
                let manifest = serde_json::from_str(&raw).map_err(|error| {
                    format!(
                        "runtime extension manifest {} is invalid: {error}",
                        manifest_path.display()
                    )
                })?;
                vifu_gateway::runtime_extension::RuntimeExtensionDefinition::new(
                    manifest,
                    extension.base_url.clone(),
                    extension.credential(base_dir)?,
                )
            })
            .collect()
    }

    fn access_token_authority(
        &self,
    ) -> Result<Option<vifu_server::config::AccessTokenAuthorityConfig>, String> {
        let (Some(deployment_id), Some(authority)) =
            (self.deployment_id.as_ref(), self.authority.as_ref())
        else {
            return Ok(None);
        };
        vifu_server::config::AccessTokenAuthorityConfig::new(
            authority.url.clone(),
            deployment_id.clone(),
        )
        .map(Some)
    }
}

impl GatewayRuntimeConfig {
    fn validate(&self) -> Result<(), String> {
        vifu_gateway::relay::agent_gateway_websocket_url(&self.address)
            .map_err(|error| format!("gateway.address is invalid: {error}"))?;
        Ok(())
    }

    fn local_socket_addr(&self) -> Result<Option<std::net::SocketAddr>, String> {
        self.validate()?;
        vifu_gateway::config::local_component_socket_addr(&self.address)
    }
}

impl AccessTokenAuthorityConfig {
    fn validate(&self) -> Result<(), String> {
        let url = self.url.trim();
        if !(url.starts_with("https://")
            || url.starts_with("http://localhost")
            || url.starts_with("http://127.0.0.1")
            || url.starts_with("http://[::1]"))
        {
            return Err("server authority URL must use HTTPS, except on loopback".to_string());
        }
        Ok(())
    }
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
        return Err("server deployment_id is invalid".to_string());
    }
    Ok(())
}

impl RuntimeExtensionConfig {
    fn credential(&self, base_dir: &Path) -> Result<String, String> {
        match (&self.credential, &self.credential_file) {
            (Some(credential), None) => Ok(credential.clone()),
            (None, Some(path)) => {
                let path = if path.is_absolute() {
                    path.clone()
                } else {
                    base_dir.join(path)
                };
                let credential = std::fs::read_to_string(&path).map_err(|error| {
                    format!(
                        "runtime extension credential_file {} could not be read: {error}",
                        path.display()
                    )
                })?;
                let credential = credential.trim().to_string();
                if credential.is_empty() {
                    return Err(format!(
                        "runtime extension credential_file {} is empty",
                        path.display()
                    ));
                }
                Ok(credential)
            }
            (Some(_), Some(_)) => {
                Err("runtime extension can set credential or credential_file, not both".to_string())
            }
            (None, None) => {
                Err("runtime extension must set credential or credential_file".to_string())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{
        deployment_owns_server, server_listen_address, LoadedRuntimeConfig, RuntimeConfig,
        DEFAULT_LOCAL_DATABASE_FILE,
    };

    #[test]
    fn self_hosted_deployment_owns_the_server_process() {
        assert!(!deployment_owns_server(None).unwrap());
        assert!(!deployment_owns_server(Some("local")).unwrap());
        assert!(deployment_owns_server(Some("self-hosted")).unwrap());
        assert!(deployment_owns_server(Some("unsupported")).is_err());
    }

    #[test]
    fn self_hosted_public_address_uses_the_deployment_internal_port() {
        let address = server_listen_address(
            vifu_server::config::DeploymentMode::SelfHosted,
            "127.0.0.1:6790".parse().unwrap(),
            None,
            "https://api.vifu.ai",
        )
        .unwrap();

        assert_eq!(address, "0.0.0.0:6790".parse().unwrap());
    }

    #[test]
    fn local_mode_does_not_serve_a_remote_public_address() {
        let error = server_listen_address(
            vifu_server::config::DeploymentMode::Local,
            "127.0.0.1:6790".parse().unwrap(),
            None,
            "https://api.vifu.ai",
        )
        .unwrap_err();

        assert!(error.contains("only a self-hosted Vifu Server process"));
    }

    #[test]
    fn accepts_versionless_component_addresses() {
        let config = RuntimeConfig::parse(
            Path::new("/tmp/config.toml"),
            r#"
[server]
address = "https://api.example.com"

[gateway]
address = "http://localhost:6790"
"#,
        )
        .unwrap();

        assert_eq!(config.server.unwrap().address, "https://api.example.com");
    }

    #[test]
    fn accepts_remote_gateway_addresses() {
        let config = RuntimeConfig::parse(
            Path::new("/tmp/config.toml"),
            r#"
[server]
address = "https://api.example.com"

[gateway]
address = "https://gateway.example.com"
"#,
        )
        .unwrap();

        assert_eq!(
            config.gateway.unwrap().address,
            "https://gateway.example.com"
        );
    }

    #[test]
    fn accepts_canonical_toml_server_configuration() {
        let config = RuntimeConfig::parse(
            Path::new("/tmp/config.toml"),
            r#"
[server]
address = "https://macbook.local:6790"
"#,
        )
        .unwrap();

        assert!(config.server.is_some());
    }

    #[test]
    fn dashboard_url_requires_an_explicit_server_dashboard_proxy() {
        let without_dashboard = LoadedRuntimeConfig {
            path: "/tmp/config.toml".into(),
            profile: None,
            config: RuntimeConfig::parse(
                Path::new("/tmp/config.toml"),
                "[server]\naddress = \"https://api.example.com\"\n",
            )
            .unwrap(),
        };
        assert_eq!(without_dashboard.dashboard_url(), None);

        let with_dashboard = LoadedRuntimeConfig {
            path: "/tmp/config.toml".into(),
            profile: None,
            config: RuntimeConfig::parse(
                Path::new("/tmp/config.toml"),
                "[server]\naddress = \"https://api.example.com\"\n\n[server.dashboard]\naddress = \"dashboard:6791\"\n",
            )
            .unwrap(),
        };
        assert_eq!(
            with_dashboard.dashboard_url().as_deref(),
            Some("https://api.example.com")
        );
    }

    #[test]
    fn accepts_combined_runtime_configuration() {
        let config = RuntimeConfig::parse(
            Path::new("/tmp/config.toml"),
            r#"
[server]
address = "http://localhost:6790"
database_url = "postgres://vifu@127.0.0.1:5432/vifu"

[gateway]
address = "http://localhost:6790"
"#,
        )
        .unwrap();
        assert!(config.server.is_some());
        assert!(config.gateway.is_some());
    }

    #[test]
    fn accepts_server_without_a_gateway() {
        let server_only = RuntimeConfig::parse(
            Path::new("/tmp/server.toml"),
            r#"
[server]
address = "http://localhost:6790"
database_url = "postgres://vifu@127.0.0.1:5432/vifu"
"#,
        )
        .unwrap();
        assert!(server_only.server.is_some());
        assert!(server_only.gateway.is_none());
    }

    #[test]
    fn rejects_gateway_without_a_server() {
        let error = RuntimeConfig::parse(
            Path::new("/tmp/gateway.toml"),
            r#"
[gateway]
address = "http://localhost:6790"
"#,
        )
        .unwrap_err();

        assert!(error.contains("requires a configured Vifu Server"));
    }

    #[test]
    fn rejects_configuration_without_a_runtime_role() {
        let error = RuntimeConfig::parse(Path::new("/tmp/config.toml"), "").unwrap_err();
        assert!(error.contains("server, gateway, or both"));
    }

    #[test]
    fn rejects_unknown_fields() {
        let error = RuntimeConfig::parse(
            Path::new("/tmp/config.toml"),
            "unknown = true\n\n[server]\naddress = \"http://localhost:6790\"\n",
        )
        .unwrap_err();
        assert!(error.contains("unknown field"));
    }

    #[test]
    fn rejects_persisted_gateway_enrollment_tokens() {
        let error = RuntimeConfig::parse(
            Path::new("/tmp/config.toml"),
            r#"
[server]
address = "https://api.example.com"

[gateway]
address = "http://localhost:6790"
enrollment_token = "vifu_ge_not-persisted"
"#,
        )
        .unwrap_err();

        assert!(error.contains("unknown field `enrollment_token`"));
    }

    #[test]
    fn gateway_can_disable_guest_bootstrap() {
        let config = RuntimeConfig::parse(
            Path::new("/tmp/config.toml"),
            r#"
[server]
address = "https://api.example.com"

[gateway]
address = "http://localhost:6790"
guest_bootstrap = false
"#,
        )
        .unwrap();
        let loaded = super::LoadedRuntimeConfig {
            path: Path::new("/tmp/config.toml").to_path_buf(),
            profile: None,
            config,
        };

        let options = loaded.gateway_options().unwrap();
        assert!(!options.allow_guest_bootstrap);
    }

    #[test]
    fn server_config_reports_the_vifu_product_version() {
        let config = RuntimeConfig::parse(
            Path::new("/tmp/config.toml"),
            r#"
[server]
address = "http://localhost:6790"
database_url = "postgres://vifu@127.0.0.1:5432/vifu"
"#,
        )
        .unwrap();
        let loaded = super::LoadedRuntimeConfig {
            path: Path::new("/tmp/config.toml").to_path_buf(),
            profile: None,
            config,
        };

        let server = loaded.server_config().unwrap();

        assert_eq!(server.service_version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn all_interface_server_address_enables_managed_tls_and_guest_device_enrollment() {
        let directory =
            std::env::temp_dir().join(format!("vifu-lan-config-{}", uuid::Uuid::new_v4()));
        let path = directory.join("config.toml");
        let config = RuntimeConfig::parse(
            &path,
            r#"
[server]
address = "https://0.0.0.0:6790"

[server.guest_bootstrap]
enabled = true

[gateway]
address = "http://127.0.0.1:6790"
"#,
        )
        .unwrap();
        let loaded = super::LoadedRuntimeConfig {
            path,
            profile: None,
            config,
        };

        let server = loaded.server_config().unwrap();

        assert_eq!(
            server.deployment_mode,
            vifu_server::config::DeploymentMode::SelfHosted
        );
        assert_eq!(server.addr, "0.0.0.0:6790".parse().unwrap());
        assert_eq!(server.server_url.as_deref(), Some("https://0.0.0.0:6790"));
        assert!(server.tls.is_some());
        assert!(server.guest_bootstrap_enabled);
        assert!(loaded.gateway_options().unwrap().allow_guest_bootstrap);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn rejects_ambiguous_database_configuration() {
        let error = RuntimeConfig::parse(
            Path::new("/tmp/config.toml"),
            r#"
[server]
address = "http://localhost:6790"
database_url = "postgres://vifu@127.0.0.1:5432/vifu"
database_url_file = "/run/secrets/database_url"
"#,
        )
        .unwrap_err();

        assert!(error.contains("database_url or database_url_file"));
    }

    #[test]
    fn loads_database_url_from_a_configured_file() {
        let directory =
            std::env::temp_dir().join(format!("vifu-runtime-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let database_url_file = directory.join("database_url");
        std::fs::write(&database_url_file, "postgres://vifu@postgres:5432/vifu\n").unwrap();
        let raw = format!(
            "[server]\naddress = \"http://localhost:6790\"\ndatabase_url_file = {:?}\n",
            database_url_file.display()
        );

        let config = RuntimeConfig::parse(Path::new("/tmp/config.toml"), &raw).unwrap();
        assert_eq!(
            config
                .server
                .unwrap()
                .database_url(Path::new("/tmp/config.toml"))
                .unwrap(),
            "postgres://vifu@postgres:5432/vifu"
        );

        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn loads_runtime_extension_credential_from_a_file() {
        let directory =
            std::env::temp_dir().join(format!("vifu-runtime-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let credential_file = directory.join("content_runtime_key");
        std::fs::write(&credential_file, "content-runtime-extension-key\n").unwrap();
        let raw = format!(
            r#"[server]
address = "http://localhost:6790"
database_url = "postgres://vifu@127.0.0.1:5432/vifu"

[[server.runtime_extensions]]
manifest = "runtime-extension.json"
base_url = "http://content-runtime:6792"
credential_file = {:?}
"#,
            credential_file.display()
        );

        let config = RuntimeConfig::parse(&directory.join("config.toml"), &raw).unwrap();
        let extension = &config.server.unwrap().runtime_extensions[0];
        assert_eq!(
            extension.credential(&directory).unwrap(),
            "content-runtime-extension-key"
        );

        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn loads_access_token_authority() {
        let raw = r#"
[server]
address = "http://localhost:6790"
deployment_id = "dep_01JTESTDEPLOYMENT"

[server.authority]
url = "https://auth.vifu.test/v1/deployments/authorize"
"#;

        let config = RuntimeConfig::parse(Path::new("/tmp/config.toml"), raw).unwrap();
        let authority = config
            .server
            .unwrap()
            .access_token_authority()
            .unwrap()
            .unwrap();
        assert_eq!(
            authority.url,
            "https://auth.vifu.test/v1/deployments/authorize"
        );
        assert_eq!(authority.deployment_id, "dep_01JTESTDEPLOYMENT");
    }

    #[test]
    fn rejects_ambiguous_runtime_extension_credentials() {
        let raw = r#"
[server]
address = "http://localhost:6790"
database_url = "postgres://vifu@127.0.0.1:5432/vifu"

[[server.runtime_extensions]]
manifest = "runtime-extension.json"
base_url = "http://content-runtime:6792"
credential = "inline-key-with-safe-length"
credential_file = "extension_key"
"#;
        let config = RuntimeConfig::parse(Path::new("/tmp/config.toml"), raw).unwrap();
        let extension = &config.server.unwrap().runtime_extensions[0];

        assert!(extension
            .credential(Path::new("/tmp"))
            .unwrap_err()
            .contains("credential or credential_file"));
    }

    #[test]
    fn creates_local_defaults_when_runtime_configuration_is_missing() {
        let directory =
            std::env::temp_dir().join(format!("vifu-runtime-config-{}", uuid::Uuid::new_v4()));
        let path = directory.join("config.toml");

        let config = RuntimeConfig::load_or_create(&path).unwrap();

        let server = config.server.as_ref().expect("first run enables Server");
        assert_eq!(server.address, vifu_gateway::config::DEFAULT_SERVER_URL);
        assert!(server
            .guest_bootstrap
            .as_ref()
            .is_some_and(|guest| guest.enabled));
        assert_eq!(
            server.database_url(&path).unwrap(),
            format!(
                "sqlite://{}",
                directory.join(DEFAULT_LOCAL_DATABASE_FILE).display()
            )
        );
        let gateway = config.gateway.as_ref().expect("first run enables Gateway");
        assert_eq!(gateway.address, vifu_gateway::config::DEFAULT_SERVER_URL);
        let loaded = super::LoadedRuntimeConfig {
            path: path.clone(),
            profile: None,
            config: config.clone(),
        };
        assert!(loaded.gateway_options().unwrap().allow_guest_bootstrap);
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("[gateway]\naddress = \"http://127.0.0.1:6790\""));
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("[server.guest_bootstrap]"));
        assert!(!std::fs::read_to_string(&path).unwrap().contains("version"));
        assert!(path.exists());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn migrates_pre_address_toml_to_explicit_component_addresses() {
        let directory =
            std::env::temp_dir().join(format!("vifu-runtime-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("config.toml");
        std::fs::write(
            &path,
            r#"version = 1

[server]
api_addr = "http://127.0.0.1:6790"

[server.listener]
address = "127.0.0.1:6790"

[gateway]
"#,
        )
        .unwrap();

        let config = RuntimeConfig::load_or_create(&path).unwrap();

        assert_eq!(
            config.server.unwrap().address,
            vifu_gateway::config::DEFAULT_SERVER_URL
        );
        assert_eq!(
            config.gateway.unwrap().address,
            vifu_gateway::config::DEFAULT_SERVER_URL
        );
        let migrated = std::fs::read_to_string(&path).unwrap();
        assert!(!migrated.contains("version"));
        assert!(!migrated.contains("api_addr"));
        assert!(!migrated.contains("listener"));
        assert_eq!(migrated.matches("address =").count(), 2);

        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn rejects_unknown_pre_address_config_versions() {
        let error = RuntimeConfig::parse(
            Path::new("/tmp/config.toml"),
            "version = 2\n\n[server]\napi_addr = \"http://localhost:6790\"\n",
        )
        .unwrap_err();

        assert!(error.contains("version must be 1"));
    }

    #[test]
    fn uses_the_embedded_database_for_existing_server_configuration() {
        let directory =
            std::env::temp_dir().join(format!("vifu-runtime-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("config.toml");
        std::fs::write(&path, "[server]\naddress = \"http://localhost:6790\"\n").unwrap();

        let config = RuntimeConfig::load_or_create(&path).unwrap();
        assert_eq!(
            config.server.unwrap().database_url(&path).unwrap(),
            format!(
                "sqlite://{}",
                directory.join(DEFAULT_LOCAL_DATABASE_FILE).display()
            )
        );
        assert!(!std::fs::read_to_string(&path)
            .unwrap()
            .contains("database_url"));

        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn applies_dotted_config_overrides_in_order() {
        let config = RuntimeConfig::parse(
            Path::new("/tmp/config.toml"),
            "[server]\naddress = \"https://api.example.com\"\n\n[gateway]\naddress = \"http://localhost:6790\"\n",
        )
        .unwrap()
        .with_overrides(
            Path::new("/tmp/config.toml"),
            &[
                "gateway.address=http://localhost:6791".to_string(),
                "gateway.address=http://localhost:6792".to_string(),
            ],
        )
        .unwrap();

        assert_eq!(config.gateway.unwrap().address, "http://localhost:6792");
    }

    #[test]
    fn parses_toml_config_override_values() {
        let config = RuntimeConfig::parse(
            Path::new("/tmp/config.toml"),
            "[server]\naddress = \"http://localhost:6790\"\n\n[gateway]\naddress = \"http://localhost:6790\"\n",
        )
        .unwrap()
        .with_overrides(
            Path::new("/tmp/config.toml"),
            &[
                "server.guest_bootstrap={ enabled = true, ttl_hours = 24, max_projects = 50 }"
                    .to_string(),
            ],
        )
        .unwrap();

        let guest = config.server.unwrap().guest_bootstrap.unwrap();
        assert!(guest.enabled);
        assert_eq!(guest.ttl_hours, 24);
        assert_eq!(guest.max_projects, 50);
    }

    #[test]
    fn validates_one_server_address_without_a_listener() {
        let config = RuntimeConfig::parse(
            Path::new("/tmp/config.toml"),
            r#"
[server]
address = "https://macbook.local:6790"
"#,
        )
        .unwrap();

        let server = config.server.unwrap();
        assert_eq!(server.address, "https://macbook.local:6790");
        assert!(RuntimeConfig::parse(
            Path::new("/tmp/config.toml"),
            "[server]\naddress = \"http://macbook.local:6790\"\n",
        )
        .is_err());
    }

    #[test]
    fn combined_gateway_connects_to_the_configured_server_address() {
        let config = RuntimeConfig::parse(
            Path::new("/tmp/config.toml"),
            "[server]\naddress = \"https://runtime.example.com\"\n\n[gateway]\naddress = \"http://localhost:6790\"\n",
        )
        .unwrap();
        let loaded = super::LoadedRuntimeConfig {
            path: Path::new("/tmp/config.toml").to_path_buf(),
            profile: None,
            config,
        };

        assert_eq!(
            loaded.gateway_options().unwrap().server_url,
            "https://runtime.example.com"
        );
    }

    #[test]
    fn rejects_unknown_config_override_paths() {
        let error = RuntimeConfig::parse(
            Path::new("/tmp/config.toml"),
            "[server]\naddress = \"https://runtime.example.com\"\n\n[gateway]\naddress = \"http://localhost:6790\"\n",
        )
        .unwrap()
        .with_overrides(
            Path::new("/tmp/config.toml"),
            &["gateway.unknown=true".to_string()],
        )
        .unwrap_err();

        assert!(error.contains("unknown field `unknown`"));
    }

    #[test]
    fn rejects_config_override_without_a_value_separator() {
        let error = RuntimeConfig::local_defaults()
            .unwrap()
            .with_overrides(
                Path::new("/tmp/config.toml"),
                &["gateway.address".to_string()],
            )
            .unwrap_err();

        assert_eq!(error, "configuration override must use key=value");
    }

    #[test]
    fn profile_is_standalone_before_cli_overrides() {
        let directory =
            std::env::temp_dir().join(format!("vifu-config-profile-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let profile_path = directory.join("cloud.toml");
        std::fs::write(
            &profile_path,
            "[server]\naddress = \"https://profile.example.com\"\n\n[gateway]\naddress = \"http://localhost:6790\"\n",
        )
        .unwrap();

        let config = RuntimeConfig::load_profile(&profile_path)
            .unwrap()
            .with_overrides(
                &profile_path,
                &["server.address=https://override.example.com".to_string()],
            )
            .unwrap();

        assert_eq!(
            config.server.unwrap().address,
            "https://override.example.com"
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn server_profile_uses_the_shared_default_database_path() {
        let directory =
            std::env::temp_dir().join(format!("vifu-config-profile-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let profile_path = directory.join("self-hosted.toml");
        std::fs::write(
            &profile_path,
            "[server]\naddress = \"http://localhost:6790\"\n",
        )
        .unwrap();

        let config = RuntimeConfig::load_profile(&profile_path).unwrap();

        assert_eq!(
            config.server.unwrap().database_url(&profile_path).unwrap(),
            format!(
                "sqlite://{}",
                directory.join(DEFAULT_LOCAL_DATABASE_FILE).display()
            )
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn missing_profile_is_reported_without_creating_it() {
        let directory =
            std::env::temp_dir().join(format!("vifu-config-profile-{}", uuid::Uuid::new_v4()));
        let profile_path = directory.join("missing.toml");

        let error = RuntimeConfig::load_profile(&profile_path).unwrap_err();

        assert!(error.contains("was not found"));
        assert!(!profile_path.exists());
    }
}
