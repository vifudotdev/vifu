use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::gateway::GatewayRuntimeOptions;

const CONFIG_FILE_NAME: &str = "config.json";
const CONFIG_PROFILE_SUFFIX: &str = ".config.json";
const CONFIG_VERSION: u32 = 1;
const DEFAULT_LOCAL_DATABASE_FILE: &str = "vifu.sqlite";

#[derive(Debug, Clone)]
pub struct LoadedRuntimeConfig {
    pub path: PathBuf,
    pub profile: Option<String>,
    pub config: RuntimeConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RuntimeConfig {
    version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<ServerRuntimeConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway: Option<GatewayRuntimeConfig>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ServerRuntimeConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub listen: Option<String>,
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

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct GuestBootstrapConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_guest_ttl_hours")]
    pub ttl_hours: u64,
    #[serde(default = "default_guest_project_limit")]
    pub max_projects: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RuntimeExtensionConfig {
    pub manifest: PathBuf,
    pub base_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_file: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AccessTokenAuthorityConfig {
    pub url: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct GatewayRuntimeConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dashboard_url: Option<String>,
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
        let server_url = gateway
            .server_url
            .clone()
            .unwrap_or_else(|| vifu_gateway::config::DEFAULT_SERVER_URL.to_string());
        Ok(GatewayRuntimeOptions {
            server_url,
            dashboard_url: gateway.dashboard_url.clone(),
            allow_guest_bootstrap: gateway.guest_bootstrap.unwrap_or(true),
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
        let mut config = vifu_server::config::Config::from_env()?;
        config.apply_service_version(env!("CARGO_PKG_VERSION"))?;
        if let Some(listen) = server.listen.as_deref() {
            let addr = listen
                .parse()
                .map_err(|error| format!("server listen address {listen:?} is invalid: {error}"))?;
            config.apply_listen_override(addr)?;
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
}

impl RuntimeConfig {
    fn load_or_create(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(raw) => Self::parse(path, &raw),
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
            version: CONFIG_VERSION,
            server: Some(ServerRuntimeConfig {
                listen: Some("127.0.0.1:6790".to_string()),
                deployment_id: None,
                database_url: None,
                database_url_file: None,
                runtime_extensions: Vec::new(),
                authority: None,
                guest_bootstrap: None,
            }),
            gateway: Some(GatewayRuntimeConfig {
                server_url: Some(vifu_gateway::config::DEFAULT_SERVER_URL.to_string()),
                dashboard_url: None,
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
        let mut profile = serde_json::from_str::<serde_json::Value>(&raw).map_err(|error| {
            format!(
                "Vifu configuration profile {} is invalid: {error}",
                path.display()
            )
        })?;
        let serde_json::Value::Object(profile) = &mut profile else {
            return Err(format!(
                "Vifu configuration profile {} must contain a JSON object",
                path.display()
            ));
        };
        profile
            .entry("version")
            .or_insert(serde_json::Value::from(CONFIG_VERSION));
        let raw = serde_json::to_string(&profile)
            .map_err(|error| format!("Vifu configuration profile could not be encoded: {error}"))?;
        Self::parse(path, &raw)
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
        let raw = serde_json::to_string(&root)
            .map_err(|error| format!("Vifu runtime configuration could not be encoded: {error}"))?;
        Self::parse(path, &raw).map_err(|error| format!("{error} after applying -c/--config"))
    }

    fn write(&self, path: &Path) -> Result<(), String> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|error| format!("Vifu runtime configuration could not be encoded: {error}"))?;
        vifu_gateway::config::write_private_file(path, &format!("{json}\n"))
    }

    fn parse(path: &Path, raw: &str) -> Result<Self, String> {
        let config = serde_json::from_str::<Self>(raw).map_err(|error| {
            format!(
                "Vifu runtime configuration {} is invalid: {error}",
                path.display()
            )
        })?;
        if config.version != CONFIG_VERSION {
            return Err(format!(
                "Vifu runtime configuration {} must use version {CONFIG_VERSION}",
                path.display()
            ));
        }
        if config.server.is_none() && config.gateway.is_none() {
            return Err(format!(
                "Vifu runtime configuration {} must configure server, gateway, or both",
                path.display()
            ));
        }
        if let Some(server) = config.server.as_ref() {
            server.validate()?;
        }
        Ok(config)
    }
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
    let value = serde_json::from_str(raw_value).unwrap_or_else(|_| {
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

impl ServerRuntimeConfig {
    fn validate(&self) -> Result<(), String> {
        if self.database_url.is_some() && self.database_url_file.is_some() {
            return Err(
                "server configuration can set databaseUrl or databaseUrlFile, not both".to_string(),
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
                    "server deploymentId and authority must be configured together".to_string(),
                )
            }
        }
        if let Some(guest) = self.guest_bootstrap.as_ref() {
            let ttl = std::time::Duration::from_secs(guest.ttl_hours.saturating_mul(60 * 60));
            if !(std::time::Duration::from_secs(60 * 60)
                ..=std::time::Duration::from_secs(30 * 24 * 60 * 60))
                .contains(&ttl)
            {
                return Err("guestBootstrap.ttlHours must be between 1 and 720".to_string());
            }
            if !(1..=1_000_000).contains(&guest.max_projects) {
                return Err("guestBootstrap.maxProjects must be between 1 and 1000000".to_string());
            }
        }
        Ok(())
    }

    fn database_url(&self, config_path: &Path) -> Result<String, String> {
        self.validate()?;
        if let Some(database_url) = self.database_url.as_deref() {
            let database_url = database_url.trim();
            if database_url.is_empty() {
                return Err("server databaseUrl must not be empty".to_string());
            }
            return Ok(database_url.to_string());
        }
        if let Some(path) = self.database_url_file.as_ref() {
            let database_url = std::fs::read_to_string(path).map_err(|error| {
                format!(
                    "server databaseUrlFile {} could not be read: {error}",
                    path.display()
                )
            })?;
            let database_url = database_url.trim();
            if database_url.is_empty() {
                return Err(format!(
                    "server databaseUrlFile {} is empty",
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
        return Err("server deploymentId is invalid".to_string());
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
                        "runtime extension credentialFile {} could not be read: {error}",
                        path.display()
                    )
                })?;
                let credential = credential.trim().to_string();
                if credential.is_empty() {
                    return Err(format!(
                        "runtime extension credentialFile {} is empty",
                        path.display()
                    ));
                }
                Ok(credential)
            }
            (Some(_), Some(_)) => {
                Err("runtime extension can set credential or credentialFile, not both".to_string())
            }
            (None, None) => {
                Err("runtime extension must set credential or credentialFile".to_string())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{RuntimeConfig, DEFAULT_LOCAL_DATABASE_FILE};

    #[test]
    fn accepts_combined_runtime_configuration() {
        let config = RuntimeConfig::parse(
            Path::new("/tmp/config.json"),
            r#"{"version":1,"server":{"listen":"127.0.0.1:6790","databaseUrl":"postgres://vifu@127.0.0.1:5432/vifu"},"gateway":{"serverUrl":"http://127.0.0.1:6790"}}"#,
        )
        .unwrap();
        assert!(config.server.is_some());
        assert!(config.gateway.is_some());
    }

    #[test]
    fn accepts_independent_runtime_roles() {
        let server_only = RuntimeConfig::parse(
            Path::new("/tmp/server.json"),
            r#"{"version":1,"server":{"databaseUrl":"postgres://vifu@127.0.0.1:5432/vifu"}}"#,
        )
        .unwrap();
        assert!(server_only.server.is_some());
        assert!(server_only.gateway.is_none());

        let gateway_only = RuntimeConfig::parse(
            Path::new("/tmp/gateway.json"),
            r#"{"version":1,"gateway":{"serverUrl":"https://runtime.example.com","dashboardUrl":"https://dashboard.example.com"}}"#,
        )
        .unwrap();
        assert!(gateway_only.server.is_none());
        assert_eq!(
            gateway_only.gateway.unwrap().dashboard_url.as_deref(),
            Some("https://dashboard.example.com")
        );
    }

    #[test]
    fn rejects_configuration_without_a_runtime_role() {
        let error =
            RuntimeConfig::parse(Path::new("/tmp/config.json"), r#"{"version":1}"#).unwrap_err();
        assert!(error.contains("server, gateway, or both"));
    }

    #[test]
    fn rejects_unknown_fields() {
        let error = RuntimeConfig::parse(
            Path::new("/tmp/config.json"),
            r#"{"version":1,"server":{},"unknown":true}"#,
        )
        .unwrap_err();
        assert!(error.contains("unknown field"));
    }

    #[test]
    fn rejects_persisted_gateway_enrollment_tokens() {
        let error = RuntimeConfig::parse(
            Path::new("/tmp/config.json"),
            r#"{"version":1,"gateway":{"serverUrl":"https://runtime.example.com","enrollmentToken":"vifu_ge_not-persisted"}}"#,
        )
        .unwrap_err();

        assert!(error.contains("unknown field `enrollmentToken`"));
    }

    #[test]
    fn gateway_can_disable_guest_bootstrap() {
        let config = RuntimeConfig::parse(
            Path::new("/tmp/config.json"),
            r#"{"version":1,"gateway":{"serverUrl":"https://runtime.example.com","guestBootstrap":false}}"#,
        )
        .unwrap();
        let loaded = super::LoadedRuntimeConfig {
            path: Path::new("/tmp/config.json").to_path_buf(),
            profile: None,
            config,
        };

        let options = loaded.gateway_options().unwrap();
        assert!(!options.allow_guest_bootstrap);
    }

    #[test]
    fn server_config_reports_the_vifu_product_version() {
        let config = RuntimeConfig::parse(
            Path::new("/tmp/config.json"),
            r#"{"version":1,"server":{"databaseUrl":"postgres://vifu@127.0.0.1:5432/vifu"}}"#,
        )
        .unwrap();
        let loaded = super::LoadedRuntimeConfig {
            path: Path::new("/tmp/config.json").to_path_buf(),
            profile: None,
            config,
        };

        let server = loaded.server_config().unwrap();

        assert_eq!(server.service_version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn rejects_ambiguous_database_configuration() {
        let error = RuntimeConfig::parse(
            Path::new("/tmp/config.json"),
            r#"{"version":1,"server":{"databaseUrl":"postgres://vifu@127.0.0.1:5432/vifu","databaseUrlFile":"/run/secrets/database_url"}}"#,
        )
        .unwrap_err();

        assert!(error.contains("databaseUrl or databaseUrlFile"));
    }

    #[test]
    fn loads_database_url_from_a_configured_file() {
        let directory =
            std::env::temp_dir().join(format!("vifu-runtime-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let database_url_file = directory.join("database_url");
        std::fs::write(&database_url_file, "postgres://vifu@postgres:5432/vifu\n").unwrap();
        let raw = format!(
            r#"{{"version":1,"server":{{"databaseUrlFile":"{}"}}}}"#,
            database_url_file.display()
        );

        let config = RuntimeConfig::parse(Path::new("/tmp/config.json"), &raw).unwrap();
        assert_eq!(
            config
                .server
                .unwrap()
                .database_url(Path::new("/tmp/config.json"))
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
            r#"{{"version":1,"server":{{"databaseUrl":"postgres://vifu@127.0.0.1:5432/vifu","runtimeExtensions":[{{"manifest":"runtime-extension.json","baseUrl":"http://content-runtime:6792","credentialFile":"{}"}}]}}}}"#,
            credential_file.display()
        );

        let config = RuntimeConfig::parse(&directory.join("config.json"), &raw).unwrap();
        let extension = &config.server.unwrap().runtime_extensions[0];
        assert_eq!(
            extension.credential(&directory).unwrap(),
            "content-runtime-extension-key"
        );

        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn loads_access_token_authority() {
        let raw = r#"{"version":1,"server":{"deploymentId":"dep_01JTESTDEPLOYMENT","authority":{"url":"https://auth.vifu.test/v1/deployments/authorize"}}}"#;

        let config = RuntimeConfig::parse(Path::new("/tmp/config.json"), raw).unwrap();
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
        let raw = r#"{"version":1,"server":{"databaseUrl":"postgres://vifu@127.0.0.1:5432/vifu","runtimeExtensions":[{"manifest":"runtime-extension.json","baseUrl":"http://content-runtime:6792","credential":"inline-key-with-safe-length","credentialFile":"extension_key"}]}}"#;
        let config = RuntimeConfig::parse(Path::new("/tmp/config.json"), raw).unwrap();
        let extension = &config.server.unwrap().runtime_extensions[0];

        assert!(extension
            .credential(Path::new("/tmp"))
            .unwrap_err()
            .contains("credential or credentialFile"));
    }

    #[test]
    fn creates_local_defaults_when_runtime_configuration_is_missing() {
        let directory =
            std::env::temp_dir().join(format!("vifu-runtime-config-{}", uuid::Uuid::new_v4()));
        let path = directory.join("config.json");

        let config = RuntimeConfig::load_or_create(&path).unwrap();

        let server = config.server.as_ref().expect("first run enables Server");
        assert_eq!(server.listen.as_deref(), Some("127.0.0.1:6790"));
        assert_eq!(
            server.database_url(&path).unwrap(),
            format!(
                "sqlite://{}",
                directory.join(DEFAULT_LOCAL_DATABASE_FILE).display()
            )
        );
        let gateway = config.gateway.as_ref().expect("first run enables Gateway");
        assert_eq!(
            gateway.server_url.as_deref(),
            Some(vifu_gateway::config::DEFAULT_SERVER_URL)
        );
        assert!(path.exists());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn uses_the_embedded_database_for_existing_server_configuration() {
        let directory =
            std::env::temp_dir().join(format!("vifu-runtime-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("config.json");
        std::fs::write(
            &path,
            r#"{"version":1,"server":{"listen":"127.0.0.1:6790"}}"#,
        )
        .unwrap();

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
            .contains("databaseUrl"));

        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn applies_dotted_config_overrides_in_order() {
        let config = RuntimeConfig::parse(
            Path::new("/tmp/config.json"),
            r#"{"version":1,"gateway":{"serverUrl":"https://old.example.com"}}"#,
        )
        .unwrap()
        .with_overrides(
            Path::new("/tmp/config.json"),
            &[
                "gateway.serverUrl=https://first.example.com".to_string(),
                "gateway.serverUrl=https://second.example.com/v1?region=jp".to_string(),
            ],
        )
        .unwrap();

        assert_eq!(
            config.gateway.unwrap().server_url.as_deref(),
            Some("https://second.example.com/v1?region=jp")
        );
    }

    #[test]
    fn parses_json_config_override_values() {
        let config = RuntimeConfig::parse(
            Path::new("/tmp/config.json"),
            r#"{"version":1,"server":{},"gateway":{"serverUrl":"https://runtime.example.com"}}"#,
        )
        .unwrap()
        .with_overrides(
            Path::new("/tmp/config.json"),
            &[
                "server.guestBootstrap={\"enabled\":true,\"ttlHours\":24,\"maxProjects\":50}"
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
    fn rejects_unknown_config_override_paths() {
        let error = RuntimeConfig::parse(
            Path::new("/tmp/config.json"),
            r#"{"version":1,"gateway":{"serverUrl":"https://runtime.example.com"}}"#,
        )
        .unwrap()
        .with_overrides(
            Path::new("/tmp/config.json"),
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
                Path::new("/tmp/config.json"),
                &["gateway.serverUrl".to_string()],
            )
            .unwrap_err();

        assert_eq!(error, "configuration override must use key=value");
    }

    #[test]
    fn profile_is_standalone_before_cli_overrides() {
        let directory =
            std::env::temp_dir().join(format!("vifu-config-profile-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let profile_path = directory.join("cloud.config.json");
        std::fs::write(
            &profile_path,
            r#"{"gateway":{"serverUrl":"https://profile.example.com"}}"#,
        )
        .unwrap();

        let config = RuntimeConfig::load_profile(&profile_path)
            .unwrap()
            .with_overrides(
                &profile_path,
                &["gateway.serverUrl=https://override.example.com".to_string()],
            )
            .unwrap();

        assert!(config.server.is_none());
        assert_eq!(
            config.gateway.unwrap().server_url.as_deref(),
            Some("https://override.example.com")
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn server_profile_uses_the_shared_default_database_path() {
        let directory =
            std::env::temp_dir().join(format!("vifu-config-profile-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let profile_path = directory.join("self-hosted.config.json");
        std::fs::write(&profile_path, r#"{"server":{}}"#).unwrap();

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
        let profile_path = directory.join("missing.config.json");

        let error = RuntimeConfig::load_profile(&profile_path).unwrap_err();

        assert!(error.contains("was not found"));
        assert!(!profile_path.exists());
    }
}
