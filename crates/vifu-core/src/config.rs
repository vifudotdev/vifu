use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const DEFAULT_OPENCLAW_URL: &str = "http://127.0.0.1:18789";
pub const DEFAULT_SERVER_URL: &str = "http://127.0.0.1:6790";
pub const DEFAULT_AGENT_GATEWAY_BOOTSTRAP_TOKEN: &str = "vifu-local-agent-gateway-bootstrap-token";

#[derive(Debug, Clone)]
pub struct Config {
    pub home_dir: PathBuf,
    pub agent_providers_file: PathBuf,
    pub agent_providers: Vec<AgentProviderConfig>,
    pub server_url: String,
    pub agent_gateway_bootstrap_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentProviderConfig {
    pub id: String,
    pub provider_type: String,
    pub url: String,
    pub token: Option<String>,
}

impl Config {
    pub fn load(server_url: String) -> Result<Self, String> {
        Self::load_from_home_dir(default_home_dir()?, server_url)
    }

    fn load_from_home_dir(home_dir: PathBuf, server_url: String) -> Result<Self, String> {
        let agent_gateway_bootstrap_token = env_or_file("VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN")?
            .unwrap_or_else(|| DEFAULT_AGENT_GATEWAY_BOOTSTRAP_TOKEN.to_string());
        if agent_gateway_bootstrap_token.len() < 16 || agent_gateway_bootstrap_token.len() > 512 {
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
        })
    }

    pub fn session_file(&self) -> PathBuf {
        self.home_dir.join("agent-gateway-session")
    }

    pub fn openclaw_provider(&self) -> Option<&AgentProviderConfig> {
        self.agent_providers
            .iter()
            .find(|provider| provider.provider_type == "openclaw")
    }
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
}

impl AgentProviderAuthDefinition {
    pub fn is_empty(&self) -> bool {
        self.token.as_deref().is_none_or(str::is_empty)
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
    write_private_file(path, &format!("{json}\n"))
}

pub fn write_private_file(path: &Path, contents: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("{} could not be created: {error}", parent.display()))?;
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
    let file = read_provider_registry_file(&path)?;
    let mut providers = Vec::new();
    for provider in file.providers {
        if provider.enabled == Some(false) {
            continue;
        }
        providers.push(resolve_agent_provider(provider)?);
    }
    Ok((path, providers))
}

fn resolve_agent_provider(
    provider: AgentProviderDefinition,
) -> Result<AgentProviderConfig, String> {
    crate::protocol::validate_identifier("agent provider key", &provider.key)?;
    crate::protocol::validate_identifier("agent provider type", &provider.provider_type)?;
    let provider_type = provider.provider_type.trim().to_ascii_lowercase();
    let url = provider.url.trim().to_string();
    if url.is_empty() {
        return Err(format!("agent provider {} url is required", provider.key));
    }
    let token = resolve_provider_token(&provider.key, &provider.auth)?;
    Ok(AgentProviderConfig {
        id: provider.key,
        provider_type,
        url,
        token,
    })
}

pub fn resolve_provider_token(
    provider_id: &str,
    auth: &AgentProviderAuthDefinition,
) -> Result<Option<String>, String> {
    if let Some(token) = normalize_secret(auth.token.clone()) {
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
    use super::{Config, DEFAULT_OPENCLAW_URL, DEFAULT_SERVER_URL};
    use std::fs;
    use std::sync::{Mutex, OnceLock};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn default_urls_target_local_services() {
        assert_eq!(DEFAULT_OPENCLAW_URL, "http://127.0.0.1:18789");
        assert_eq!(DEFAULT_SERVER_URL, "http://127.0.0.1:6790");
    }

    #[test]
    fn reads_agent_gateway_bootstrap_token_from_file() {
        let _guard = env_lock().lock().unwrap();
        let dir = unique_directory("vifu-core-config-test");
        fs::create_dir_all(&dir).unwrap();
        let token_path = dir.join("agent_gateway_bootstrap_token");
        fs::write(&token_path, "agent-gateway-token-from-file\n").unwrap();

        let previous_token = std::env::var_os("VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN");
        let previous_token_file = std::env::var_os("VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN_FILE");
        std::env::remove_var("VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN");
        std::env::set_var("VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN_FILE", &token_path);

        let config =
            Config::load_from_home_dir(dir.join(".vifu"), DEFAULT_SERVER_URL.to_string()).unwrap();
        assert_eq!(
            config.agent_gateway_bootstrap_token,
            "agent-gateway-token-from-file"
        );

        restore_env("VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN", previous_token);
        restore_env(
            "VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN_FILE",
            previous_token_file,
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn loads_no_agent_providers_by_default() {
        let _guard = env_lock().lock().unwrap();
        let previous_token_file = std::env::var_os("VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN_FILE");
        let dir = unique_directory("vifu-core-no-providers");
        std::env::remove_var("VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN_FILE");

        let config =
            Config::load_from_home_dir(dir.clone(), DEFAULT_SERVER_URL.to_string()).unwrap();
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
        let dir = unique_directory("vifu-core-provider-file");
        fs::create_dir_all(&dir).unwrap();
        let providers_file = dir.join("providers.json");
        fs::write(
            &providers_file,
            r#"{"providers":[{"key":"openclaw-local","type":"openclaw","url":"http://127.0.0.1:18789","auth":{"token":"openclaw-provider-token"}}]}"#,
        )
        .unwrap();
        std::env::remove_var("VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN_FILE");

        let config =
            Config::load_from_home_dir(dir.clone(), DEFAULT_SERVER_URL.to_string()).unwrap();
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
    fn rejects_unknown_provider_auth_fields() {
        let _guard = env_lock().lock().unwrap();
        let previous_token_file = std::env::var_os("VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN_FILE");
        let dir = unique_directory("vifu-core-provider-auth");
        fs::create_dir_all(&dir).unwrap();
        let providers_file = dir.join("providers.json");
        fs::write(
            &providers_file,
            r#"{"providers":[{"key":"openclaw-local","type":"openclaw","url":"http://127.0.0.1:18789","auth":{"tokenSource":"OPENCLAW_GATEWAY_TOKEN"}}]}"#,
        )
        .unwrap();
        std::env::remove_var("VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN_FILE");

        let error =
            Config::load_from_home_dir(dir.clone(), DEFAULT_SERVER_URL.to_string()).unwrap_err();
        assert!(error.contains("unknown field `tokenSource`"));

        restore_env(
            "VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN_FILE",
            previous_token_file,
        );
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
