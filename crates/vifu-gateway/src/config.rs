use std::fmt;
use std::fs::OpenOptions;
use std::io::Write;
use std::net::{IpAddr, SocketAddr, TcpListener, ToSocketAddrs};
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const DEFAULT_OPENCLAW_URL: &str = "http://127.0.0.1:18789";
pub const DEFAULT_SERVER_URL: &str = "http://127.0.0.1:6790";
pub const DEFAULT_AGENT_GATEWAY_BOOTSTRAP_TOKEN: &str = "vifu-local-agent-gateway-bootstrap-token";

#[derive(Clone)]
pub struct Config {
    pub home_dir: PathBuf,
    pub agent_providers_file: PathBuf,
    pub agent_providers: Vec<AgentProviderConfig>,
    pub server_url: String,
    pub agent_gateway_bootstrap_token: Option<String>,
    pub enrollment_token: Option<String>,
}

impl fmt::Debug for Config {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Config")
            .field("home_dir", &self.home_dir)
            .field("agent_providers_file", &self.agent_providers_file)
            .field("agent_provider_count", &self.agent_providers.len())
            .field("server_url", &self.server_url)
            .field(
                "agent_gateway_bootstrap_token",
                &self
                    .agent_gateway_bootstrap_token
                    .as_ref()
                    .map(|_| "[REDACTED]"),
            )
            .field(
                "enrollment_token",
                &self.enrollment_token.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct AgentProviderConfig {
    pub id: String,
    pub name: Option<String>,
    pub provider_type: String,
    pub url: String,
    pub token: Option<String>,
    pub config: Value,
}

impl fmt::Debug for AgentProviderConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AgentProviderConfig")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("provider_type", &self.provider_type)
            .field("url", &self.url)
            .field("token", &self.token.as_ref().map(|_| "[REDACTED]"))
            .field("config", &"[REDACTED]")
            .finish()
    }
}

impl Config {
    pub fn load(server_url: String, enrollment_token: Option<String>) -> Result<Self, String> {
        Self::load_with_implicit_local_bootstrap(server_url, enrollment_token, true)
    }

    pub fn load_with_implicit_local_bootstrap(
        server_url: String,
        enrollment_token: Option<String>,
        enabled: bool,
    ) -> Result<Self, String> {
        Self::load_from_home_dir_with_implicit_local_bootstrap(
            default_home_dir()?,
            server_url,
            enrollment_token,
            enabled,
        )
    }

    #[cfg(test)]
    fn load_from_home_dir(
        home_dir: PathBuf,
        server_url: String,
        enrollment_token: Option<String>,
    ) -> Result<Self, String> {
        Self::load_from_home_dir_with_implicit_local_bootstrap(
            home_dir,
            server_url,
            enrollment_token,
            true,
        )
    }

    fn load_from_home_dir_with_implicit_local_bootstrap(
        home_dir: PathBuf,
        server_url: String,
        enrollment_token: Option<String>,
        implicit_local_bootstrap: bool,
    ) -> Result<Self, String> {
        let enrollment_token = enrollment_token
            .map(|token| token.trim().to_string())
            .filter(|token| !token.is_empty())
            .or(env_or_file("VIFU_AGENT_GATEWAY_ENROLLMENT_TOKEN")?);
        if let Some(token) = enrollment_token.as_deref() {
            validate_enrollment_token(token)?;
        }
        let agent_gateway_bootstrap_token = match env_or_file("VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN")?
        {
            Some(token) => Some(token),
            None if implicit_local_bootstrap && is_local_server_url(&server_url) => {
                Some(DEFAULT_AGENT_GATEWAY_BOOTSTRAP_TOKEN.to_string())
            }
            None => None,
        };
        if agent_gateway_bootstrap_token
            .as_ref()
            .is_some_and(|token| token.len() < 16 || token.len() > 512)
        {
            return Err(
                "VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN must contain 16-512 characters".to_string(),
            );
        }
        let (agent_providers_file, agent_providers) = load_agent_providers(&home_dir)?;
        Ok(Self {
            home_dir,
            agent_providers_file,
            agent_providers,
            server_url,
            agent_gateway_bootstrap_token,
            enrollment_token,
        })
    }

    pub fn runtime_database_file(&self) -> PathBuf {
        self.home_dir.join("runtime.sqlite")
    }

    pub fn openclaw_provider(&self) -> Option<&AgentProviderConfig> {
        self.agent_providers
            .iter()
            .find(|provider| provider.provider_type == "openclaw")
    }

    pub fn openclaw_providers(&self) -> impl Iterator<Item = &AgentProviderConfig> {
        self.agent_providers
            .iter()
            .filter(|provider| provider.provider_type == "openclaw")
    }

    pub fn llama_providers(&self) -> impl Iterator<Item = &AgentProviderConfig> {
        self.agent_providers
            .iter()
            .filter(|provider| provider.provider_type == "llama")
    }

    pub fn local_whisper_providers(&self) -> impl Iterator<Item = &AgentProviderConfig> {
        self.agent_providers
            .iter()
            .filter(|provider| provider.provider_type == "local-whisper")
    }

    pub fn openai_compatible_providers(&self) -> impl Iterator<Item = &AgentProviderConfig> {
        self.agent_providers
            .iter()
            .filter(|provider| provider.provider_type == "openai-compatible")
    }
}

fn validate_enrollment_token(token: &str) -> Result<(), String> {
    let secret = token
        .strip_prefix("vifu_ge_")
        .or_else(|| token.strip_prefix("vifu_app_"))
        .ok_or_else(|| "Agent Gateway enrollment token or App ID is invalid".to_string())?;
    if secret.len() != 64 || !secret.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Agent Gateway enrollment token or App ID is invalid".to_string());
    }
    Ok(())
}

fn is_local_server_url(server_url: &str) -> bool {
    let Ok(url) = url::Url::parse(server_url.trim()) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    is_loopback_host(host) || (!host.is_empty() && !host.contains('.'))
}

/// Returns true when a Vifu Server URL resolves explicitly to loopback.
pub fn is_loopback_server_url(server_url: &str) -> bool {
    let Ok(url) = url::Url::parse(server_url.trim()) else {
        return false;
    };
    url.host_str().is_some_and(is_loopback_host)
}

/// Resolves a component origin to a socket on this machine.
///
/// Fully qualified Internet hosts are treated as remote without DNS lookup.
/// Loopback, single-label, `.local`, and literal IP hosts are checked against
/// interfaces owned by the current machine.
pub fn local_component_socket_addr(address: &str) -> Result<Option<SocketAddr>, String> {
    let url = url::Url::parse(address.trim())
        .map_err(|error| format!("component address is invalid: {error}"))?;
    let host = url
        .host_str()
        .ok_or_else(|| "component address must include a host".to_string())?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return Err(
            "component address must be an origin without credentials, a path, query, or fragment"
                .to_string(),
        );
    }
    let port = url
        .port_or_known_default()
        .ok_or_else(|| "component address must include a port for its URL scheme".to_string())?;
    if host.eq_ignore_ascii_case("localhost") {
        return Ok(Some(SocketAddr::from(([127, 0, 0, 1], port))));
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(is_local_interface(ip).then_some(SocketAddr::new(ip, port)));
    }
    if host.contains('.') && !host.ends_with(".local") {
        return Ok(None);
    }
    let mut addresses = (host, port)
        .to_socket_addrs()
        .map_err(|error| {
            format!("local component address {address:?} could not be resolved: {error}")
        })?
        .collect::<Vec<_>>();
    addresses.sort_by_key(|candidate| candidate.is_ipv6());
    Ok(addresses
        .into_iter()
        .find(|candidate| is_local_interface(candidate.ip())))
}

fn is_local_interface(ip: IpAddr) -> bool {
    if ip.is_loopback() {
        return true;
    }
    TcpListener::bind(SocketAddr::new(ip, 0)).is_ok()
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

/// Returns true when a configured HTTP provider resolves to this machine.
pub fn is_local_provider_url(provider_url: &str) -> bool {
    let Ok(url) = url::Url::parse(provider_url.trim()) else {
        return false;
    };
    url.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|address| address.is_loopback())
    })
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AgentProvidersFile {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub providers: Vec<AgentProviderDefinition>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AgentProviderDefinition {
    #[serde(alias = "id")]
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub provider_type: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "AgentProviderAuthDefinition::is_empty")]
    pub auth: AgentProviderAuthDefinition,
    #[serde(default = "empty_object", skip_serializing_if = "is_empty_object")]
    pub config: Value,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AgentProviderAuthDefinition {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_env: Option<String>,
}

impl AgentProviderAuthDefinition {
    pub fn is_empty(&self) -> bool {
        self.token.as_deref().is_none_or(str::is_empty)
            && self.token_env.as_deref().is_none_or(str::is_empty)
    }
}

pub fn provider_registry_candidates(home_dir: &Path) -> Vec<PathBuf> {
    vec![home_dir.join("providers.json")]
}

pub fn discover_provider_registry_file(home_dir: &Path) -> Option<PathBuf> {
    provider_registry_candidates(home_dir)
        .into_iter()
        .find(|path| path.exists())
}

pub fn default_provider_registry_file(home_dir: &Path) -> PathBuf {
    home_dir.join("providers.json")
}

pub fn ensure_provider_registry_file(home_dir: &Path) -> Result<PathBuf, String> {
    let path = default_provider_registry_file(home_dir);
    if !path.exists() {
        write_provider_registry_file(&path, &AgentProvidersFile::default())?;
    }
    Ok(path)
}

pub fn read_provider_registry_file(path: &Path) -> Result<AgentProvidersFile, String> {
    if !path.exists() {
        return Ok(AgentProvidersFile::default());
    }
    let raw = std::fs::read_to_string(path)
        .map_err(|error| format!("{} could not be read: {error}", path.display()))?;
    if raw.trim().is_empty() {
        return Ok(AgentProvidersFile::default());
    }
    serde_json::from_str::<AgentProvidersFile>(&raw).map_err(|error| {
        format!(
            "{} is not a valid agent providers file: {error}",
            path.display()
        )
    })
}

pub fn write_provider_registry_file(path: &Path, file: &AgentProvidersFile) -> Result<(), String> {
    let json = serde_json::to_string_pretty(file)
        .map_err(|error| format!("provider registry could not be encoded: {error}"))?;
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("providers.json");
    let temporary = path.with_file_name(format!(".{file_name}.tmp-{}", std::process::id()));
    write_private_file(&temporary, &format!("{json}\n"))?;
    if let Err(error) = std::fs::rename(&temporary, path) {
        let _ = std::fs::remove_file(&temporary);
        return Err(format!("{} could not be replaced: {error}", path.display()));
    }
    Ok(())
}

pub fn write_private_file(path: &Path, contents: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("{} could not be created: {error}", parent.display()))?;
        #[cfg(unix)]
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700)).map_err(
            |error| {
                format!(
                    "{} permissions could not be updated: {error}",
                    parent.display()
                )
            },
        )?;
    }

    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    options.mode(0o600);

    let mut file = options
        .open(path)
        .map_err(|error| format!("{} could not be opened: {error}", path.display()))?;
    file.write_all(contents.as_bytes())
        .map_err(|error| format!("{} could not be written: {error}", path.display()))?;

    #[cfg(unix)]
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|error| {
        format!(
            "{} permissions could not be updated: {error}",
            path.display()
        )
    })?;

    Ok(())
}

fn load_agent_providers(home_dir: &Path) -> Result<(PathBuf, Vec<AgentProviderConfig>), String> {
    let path = ensure_provider_registry_file(home_dir)?;
    let providers = load_provider_registry_file(&path)?;
    Ok((path, providers))
}

pub fn load_provider_registry_file(path: &Path) -> Result<Vec<AgentProviderConfig>, String> {
    let file = read_provider_registry_file(path)?;
    let mut providers = Vec::new();
    for provider in file.providers {
        if provider.enabled == Some(false) {
            continue;
        }
        providers.push(resolve_agent_provider(provider)?);
    }
    Ok(providers)
}

fn resolve_agent_provider(
    provider: AgentProviderDefinition,
) -> Result<AgentProviderConfig, String> {
    crate::protocol::validate_identifier("agent provider key", &provider.key)?;
    crate::protocol::validate_identifier("agent provider type", &provider.provider_type)?;
    let provider_type = provider.provider_type.trim().to_ascii_lowercase();
    let url = provider.url.trim().to_string();
    if provider_type_requires_url(&provider_type) && url.is_empty() {
        return Err(format!("agent provider {} url is required", provider.key));
    }
    if !provider.config.is_object() {
        return Err(format!(
            "agent provider {} config must be an object",
            provider.key
        ));
    }
    let token = resolve_provider_token(&provider.key, &provider.auth)?;
    Ok(AgentProviderConfig {
        id: provider.key,
        name: provider
            .name
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty()),
        provider_type,
        url,
        token,
        config: provider.config,
    })
}

fn provider_type_requires_url(provider_type: &str) -> bool {
    !matches!(provider_type, "llama" | "local-whisper")
}

pub fn resolve_provider_token(
    provider_id: &str,
    auth: &AgentProviderAuthDefinition,
) -> Result<Option<String>, String> {
    let inline_token = normalize_secret(auth.token.clone());
    let token_env = normalize_secret(auth.token_env.clone());
    if inline_token.is_some() && token_env.is_some() {
        return Err(format!(
            "agent provider {provider_id} auth cannot set both token and tokenEnv"
        ));
    }
    if let Some(token) = inline_token {
        validate_provider_token(provider_id, &token)?;
        return Ok(Some(token));
    }
    if let Some(env_name) = token_env {
        validate_env_var_name(provider_id, "tokenEnv", &env_name)?;
        let token = std::env::var(&env_name).map_err(|_| {
            format!("agent provider {provider_id} auth.tokenEnv {env_name} is not set")
        })?;
        let token = normalize_secret(Some(token)).ok_or_else(|| {
            format!("agent provider {provider_id} auth.tokenEnv {env_name} is empty")
        })?;
        validate_provider_token(provider_id, &token)?;
        return Ok(Some(token));
    }
    Ok(None)
}

fn empty_object() -> Value {
    Value::Object(serde_json::Map::new())
}

fn is_empty_object(value: &Value) -> bool {
    value.as_object().is_some_and(serde_json::Map::is_empty)
}

fn normalize_secret(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn validate_provider_token(provider_id: &str, token: &str) -> Result<(), String> {
    if token.len() > 4096 || token.chars().any(char::is_control) {
        return Err(format!(
            "agent provider {provider_id} token contains invalid characters"
        ));
    }
    Ok(())
}

fn validate_env_var_name(provider_id: &str, field: &str, name: &str) -> Result<(), String> {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return Err(format!(
            "agent provider {provider_id} auth.{field} is empty"
        ));
    };
    if !(first == '_' || first.is_ascii_alphabetic())
        || chars.any(|character| !(character == '_' || character.is_ascii_alphanumeric()))
    {
        return Err(format!(
            "agent provider {provider_id} auth.{field} must be an environment variable name"
        ));
    }
    Ok(())
}

fn env_or_file(name: &str) -> Result<Option<String>, String> {
    if let Ok(value) = std::env::var(name) {
        let value = value.trim().to_string();
        if !value.is_empty() {
            return Ok(Some(value));
        }
    }
    let file_name = format!("{name}_FILE");
    let Ok(file_path) = std::env::var(&file_name) else {
        return Ok(None);
    };
    let file_path = file_path.trim();
    if file_path.is_empty() {
        return Ok(None);
    }
    let value = std::fs::read_to_string(file_path)
        .map_err(|error| format!("{file_name} could not be read: {error}"))?
        .trim()
        .to_string();
    if value.is_empty() {
        return Err(format!("{file_name} is empty"));
    }
    Ok(Some(value))
}

pub fn default_home_dir() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .ok_or_else(|| "HOME is not set, so Vifu cannot locate ~/.vifu.".to_string())?;
    Ok(PathBuf::from(home).join(".vifu"))
}

#[cfg(test)]
mod tests {
    use super::{
        local_component_socket_addr, read_provider_registry_file, write_private_file,
        write_provider_registry_file, AgentProviderAuthDefinition, AgentProviderDefinition,
        AgentProvidersFile, Config, DEFAULT_OPENCLAW_URL, DEFAULT_SERVER_URL,
    };
    use serde_json::json;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::{Mutex, OnceLock};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn default_urls_target_local_services() {
        assert_eq!(DEFAULT_OPENCLAW_URL, "http://127.0.0.1:18789");
        assert_eq!(DEFAULT_SERVER_URL, "http://127.0.0.1:6790");
    }

    #[test]
    fn local_component_address_resolves_loopback_without_a_listener() {
        assert_eq!(
            local_component_socket_addr("http://localhost:6790").unwrap(),
            Some("127.0.0.1:6790".parse().unwrap())
        );
    }

    #[test]
    fn public_component_address_is_remote_without_dns_lookup() {
        assert_eq!(
            local_component_socket_addr("https://api.example.com").unwrap(),
            None
        );
    }

    #[cfg(unix)]
    #[test]
    fn private_file_writer_secures_its_parent_directory() {
        use std::os::unix::fs::PermissionsExt;

        let root = unique_directory("vifu-private-directory");
        let directory = root.join(".vifu");
        write_private_file(&directory.join("config.json"), "{}\n").unwrap();

        let mode = fs::metadata(&directory).unwrap().permissions().mode() & 0o777;

        assert_eq!(mode, 0o700);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn explicit_enrollment_token_is_kept_separate_from_bootstrap() {
        let _guard = env_lock().lock().unwrap();
        let dir = unique_directory("vifu-gateway-enrollment-token");
        let previous_token = std::env::var_os("VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN");
        let previous_token_file = std::env::var_os("VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN_FILE");
        std::env::remove_var("VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN");
        std::env::remove_var("VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN_FILE");
        let config = Config::load_from_home_dir(
            dir.clone(),
            "https://api.example.com".to_string(),
            Some(
                "vifu_ge_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_string(),
            ),
        )
        .unwrap();

        assert!(config.enrollment_token.unwrap().starts_with("vifu_ge_"));
        assert!(config.agent_gateway_bootstrap_token.is_none());
        restore_env("VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN", previous_token);
        restore_env(
            "VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN_FILE",
            previous_token_file,
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn app_id_is_accepted_as_a_gateway_enrollment_selector() {
        let _guard = env_lock().lock().unwrap();
        let dir = unique_directory("vifu-gateway-app-id");
        let previous_token = std::env::var_os("VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN");
        let previous_token_file = std::env::var_os("VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN_FILE");
        std::env::remove_var("VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN");
        std::env::remove_var("VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN_FILE");
        let app_id = format!("vifu_app_{}", "a".repeat(64));

        let config = Config::load_from_home_dir(
            dir.clone(),
            "https://api.example.com".to_string(),
            Some(app_id.clone()),
        )
        .unwrap();

        assert_eq!(config.enrollment_token.as_deref(), Some(app_id.as_str()));
        restore_env("VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN", previous_token);
        restore_env(
            "VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN_FILE",
            previous_token_file,
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn guest_mode_does_not_inject_the_legacy_local_bootstrap_token() {
        let _guard = env_lock().lock().unwrap();
        let dir = unique_directory("vifu-gateway-guest-mode");
        let previous_token = std::env::var_os("VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN");
        let previous_token_file = std::env::var_os("VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN_FILE");
        std::env::remove_var("VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN");
        std::env::remove_var("VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN_FILE");

        let config = Config::load_from_home_dir_with_implicit_local_bootstrap(
            dir.clone(),
            DEFAULT_SERVER_URL.to_string(),
            None,
            false,
        )
        .unwrap();

        assert!(config.agent_gateway_bootstrap_token.is_none());
        restore_env("VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN", previous_token);
        restore_env(
            "VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN_FILE",
            previous_token_file,
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn remote_config_debug_redacts_transient_enrollment_token() {
        let token = "vifu_ge_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let config = Config {
            home_dir: PathBuf::from("/tmp/vifu-redaction"),
            agent_providers_file: PathBuf::from("/tmp/vifu-redaction/providers.json"),
            agent_providers: Vec::new(),
            server_url: "https://api.example.com".to_string(),
            agent_gateway_bootstrap_token: None,
            enrollment_token: Some(token.to_string()),
        };

        assert!(!format!("{config:?}").contains(token));
    }

    #[test]
    fn reads_agent_gateway_bootstrap_token_from_file() {
        let _guard = env_lock().lock().unwrap();
        let dir = unique_directory("vifu-gateway-config-test");
        fs::create_dir_all(&dir).unwrap();
        let token_path = dir.join("agent_gateway_bootstrap_token");
        fs::write(&token_path, "agent-gateway-token-from-file\n").unwrap();

        let previous_token = std::env::var_os("VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN");
        let previous_token_file = std::env::var_os("VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN_FILE");
        std::env::remove_var("VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN");
        std::env::set_var("VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN_FILE", &token_path);

        let config =
            Config::load_from_home_dir(dir.join(".vifu"), DEFAULT_SERVER_URL.to_string(), None)
                .unwrap();
        assert_eq!(
            config.agent_gateway_bootstrap_token.as_deref(),
            Some("agent-gateway-token-from-file")
        );

        restore_env("VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN", previous_token);
        restore_env(
            "VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN_FILE",
            previous_token_file,
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn reads_transient_enrollment_token_from_file_without_exposing_it_in_debug() {
        let _guard = env_lock().lock().unwrap();
        let dir = unique_directory("vifu-gateway-enrollment-file");
        fs::create_dir_all(&dir).unwrap();
        let token_path = dir.join("agent_gateway_enrollment_token");
        let token = "vifu_ge_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        fs::write(&token_path, format!("{token}\n")).unwrap();

        let previous_token = std::env::var_os("VIFU_AGENT_GATEWAY_ENROLLMENT_TOKEN");
        let previous_token_file = std::env::var_os("VIFU_AGENT_GATEWAY_ENROLLMENT_TOKEN_FILE");
        std::env::remove_var("VIFU_AGENT_GATEWAY_ENROLLMENT_TOKEN");
        std::env::set_var("VIFU_AGENT_GATEWAY_ENROLLMENT_TOKEN_FILE", &token_path);

        let config = Config::load_from_home_dir(
            dir.join(".vifu"),
            "https://api.example.com".to_string(),
            None,
        )
        .unwrap();
        assert_eq!(config.enrollment_token.as_deref(), Some(token));
        assert!(!format!("{config:?}").contains(token));

        restore_env("VIFU_AGENT_GATEWAY_ENROLLMENT_TOKEN", previous_token);
        restore_env(
            "VIFU_AGENT_GATEWAY_ENROLLMENT_TOKEN_FILE",
            previous_token_file,
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn loads_no_agent_providers_by_default() {
        let _guard = env_lock().lock().unwrap();
        let previous_token_file = std::env::var_os("VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN_FILE");
        let dir = unique_directory("vifu-gateway-no-providers");
        std::env::remove_var("VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN_FILE");

        let config =
            Config::load_from_home_dir(dir.clone(), DEFAULT_SERVER_URL.to_string(), None).unwrap();
        assert!(config.agent_providers.is_empty());
        assert!(config.agent_providers_file.exists());

        restore_env(
            "VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN_FILE",
            previous_token_file,
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn loads_openclaw_provider_from_generic_file() {
        let _guard = env_lock().lock().unwrap();
        let previous_token_file = std::env::var_os("VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN_FILE");
        let dir = unique_directory("vifu-gateway-provider-file");
        fs::create_dir_all(&dir).unwrap();
        let providers_file = dir.join("providers.json");
        fs::write(
            &providers_file,
            r#"{"providers":[{"key":"openclaw-local","type":"openclaw","url":"http://127.0.0.1:18789","auth":{"token":"openclaw-provider-token"}}]}"#,
        )
        .unwrap();
        std::env::remove_var("VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN_FILE");

        let config =
            Config::load_from_home_dir(dir.clone(), DEFAULT_SERVER_URL.to_string(), None).unwrap();
        assert_eq!(config.agent_providers_file, providers_file);
        assert_eq!(config.agent_providers.len(), 1);
        let provider = config.openclaw_provider().unwrap();
        assert_eq!(provider.id, "openclaw-local");
        assert_eq!(provider.url, DEFAULT_OPENCLAW_URL);
        assert_eq!(provider.token.as_deref(), Some("openclaw-provider-token"));

        restore_env(
            "VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN_FILE",
            previous_token_file,
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn loads_in_process_provider_without_a_url() {
        let _guard = env_lock().lock().unwrap();
        let previous_token_file = std::env::var_os("VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN_FILE");
        let dir = unique_directory("vifu-gateway-in-process-provider");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("providers.json"),
            r#"{"providers":[{"key":"local-qwen","type":"llama","config":{"modelPath":"models/qwen.gguf","contextSize":4096}}]}"#,
        )
        .unwrap();
        std::env::remove_var("VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN_FILE");

        let config =
            Config::load_from_home_dir(dir.clone(), DEFAULT_SERVER_URL.to_string(), None).unwrap();

        assert_eq!(config.agent_providers[0].id, "local-qwen");
        assert!(config.agent_providers[0].url.is_empty());
        assert_eq!(
            config.agent_providers[0].config,
            json!({ "modelPath": "models/qwen.gguf", "contextSize": 4096 })
        );
        restore_env(
            "VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN_FILE",
            previous_token_file,
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn loads_local_whisper_provider_without_a_url() {
        let _guard = env_lock().lock().unwrap();
        let previous_token_file = std::env::var_os("VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN_FILE");
        let dir = unique_directory("vifu-gateway-local-whisper-provider");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("providers.json"),
            r#"{"providers":[{"key":"local-transcriber","type":"local-whisper","config":{"model":"ggml-base.en.bin","language":"en"}}]}"#,
        )
        .unwrap();
        std::env::remove_var("VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN_FILE");

        let config =
            Config::load_from_home_dir(dir.clone(), DEFAULT_SERVER_URL.to_string(), None).unwrap();

        let provider = config.local_whisper_providers().next().unwrap();
        assert_eq!(provider.id, "local-transcriber");
        assert!(provider.url.is_empty());
        assert_eq!(provider.config["model"], "ggml-base.en.bin");
        assert_eq!(provider.config["language"], "en");

        restore_env(
            "VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN_FILE",
            previous_token_file,
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn loads_openai_compatible_provider_with_env_token() {
        let _guard = env_lock().lock().unwrap();
        let previous_provider_token = std::env::var_os("VIFU_TEST_OPENAI_PROVIDER_TOKEN");
        let previous_token_file = std::env::var_os("VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN_FILE");
        let dir = unique_directory("vifu-gateway-openai-compatible-provider");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("providers.json"),
            r#"{"providers":[{"key":"cloudflare-proxy","type":"openai-compatible","url":"https://provider.example.com/openai/v1","auth":{"tokenEnv":"VIFU_TEST_OPENAI_PROVIDER_TOKEN"},"config":{"chatModel":"gpt-5.5-mini","embeddingModel":"text-embedding-ada-002"}}]}"#,
        )
        .unwrap();
        std::env::set_var(
            "VIFU_TEST_OPENAI_PROVIDER_TOKEN",
            "synthetic-provider-token",
        );
        std::env::remove_var("VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN_FILE");

        let config =
            Config::load_from_home_dir(dir.clone(), DEFAULT_SERVER_URL.to_string(), None).unwrap();

        let provider = config.openai_compatible_providers().next().unwrap();
        assert_eq!(provider.id, "cloudflare-proxy");
        assert_eq!(provider.provider_type, "openai-compatible");
        assert_eq!(provider.token.as_deref(), Some("synthetic-provider-token"));
        assert_eq!(provider.config["chatModel"], "gpt-5.5-mini");
        assert_eq!(provider.config["embeddingModel"], "text-embedding-ada-002");

        restore_env("VIFU_TEST_OPENAI_PROVIDER_TOKEN", previous_provider_token);
        restore_env(
            "VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN_FILE",
            previous_token_file,
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn external_provider_still_requires_a_url() {
        let _guard = env_lock().lock().unwrap();
        let previous_token_file = std::env::var_os("VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN_FILE");
        let dir = unique_directory("vifu-gateway-external-provider-url");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("providers.json"),
            r#"{"providers":[{"key":"openclaw-local","type":"openclaw"}]}"#,
        )
        .unwrap();
        std::env::remove_var("VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN_FILE");

        let error = Config::load_from_home_dir(dir.clone(), DEFAULT_SERVER_URL.to_string(), None)
            .unwrap_err();

        assert!(error.contains("agent provider openclaw-local url is required"));
        restore_env(
            "VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN_FILE",
            previous_token_file,
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_unknown_provider_auth_fields() {
        let _guard = env_lock().lock().unwrap();
        let previous_token_file = std::env::var_os("VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN_FILE");
        let dir = unique_directory("vifu-gateway-provider-auth");
        fs::create_dir_all(&dir).unwrap();
        let providers_file = dir.join("providers.json");
        fs::write(
            &providers_file,
            r#"{"providers":[{"key":"openclaw-local","type":"openclaw","url":"http://127.0.0.1:18789","auth":{"tokenSource":"OPENCLAW_GATEWAY_TOKEN"}}]}"#,
        )
        .unwrap();
        std::env::remove_var("VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN_FILE");

        let error = Config::load_from_home_dir(dir.clone(), DEFAULT_SERVER_URL.to_string(), None)
            .unwrap_err();
        assert!(error.contains("unknown field `tokenSource`"));

        restore_env(
            "VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN_FILE",
            previous_token_file,
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn writes_and_reloads_multiple_provider_definitions() {
        let dir = unique_directory("vifu-gateway-provider-write");
        let path = dir.join("providers.json");
        let providers = AgentProvidersFile {
            providers: vec![
                AgentProviderDefinition {
                    key: "openclaw-primary".to_string(),
                    name: Some("Primary Gateway".to_string()),
                    provider_type: "openclaw".to_string(),
                    url: "http://127.0.0.1:18789".to_string(),
                    enabled: Some(true),
                    auth: AgentProviderAuthDefinition {
                        token: Some("synthetic-provider-token".to_string()),
                        token_env: None,
                    },
                    config: json!({}),
                },
                AgentProviderDefinition {
                    key: "openclaw-story".to_string(),
                    name: Some("Story Gateway".to_string()),
                    provider_type: "openclaw".to_string(),
                    url: "http://127.0.0.1:18790".to_string(),
                    enabled: Some(true),
                    auth: AgentProviderAuthDefinition::default(),
                    config: json!({ "channel": "story" }),
                },
            ],
        };

        write_provider_registry_file(&path, &providers).unwrap();
        let loaded = read_provider_registry_file(&path).unwrap();

        assert_eq!(loaded.providers.len(), 2);
        assert_eq!(loaded.providers[1].key, "openclaw-story");
        assert_eq!(loaded.providers[1].config["channel"], "story");
        fs::remove_dir_all(dir).unwrap();
    }

    fn restore_env(name: &str, value: Option<std::ffi::OsString>) {
        match value {
            Some(value) => std::env::set_var(name, value),
            None => std::env::remove_var(name),
        }
    }

    fn unique_directory(prefix: &str) -> std::path::PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{stamp}"))
    }

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }
}
