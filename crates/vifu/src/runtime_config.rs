use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::gateway::GatewayRuntimeOptions;

const CONFIG_FILE_NAME: &str = "config.json";
const CONFIG_VERSION: u32 = 1;
const DEFAULT_LOCAL_DATABASE_URL: &str = "postgres://vifu@127.0.0.1:5432/vifu";

#[derive(Debug, Clone)]
pub struct LoadedRuntimeConfig {
    pub path: PathBuf,
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
    pub listen: Option<String>,
    pub database_url: Option<String>,
    pub database_url_file: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runtime_extensions: Vec<RuntimeExtensionConfig>,
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

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct GatewayRuntimeConfig {
    pub server_url: Option<String>,
}

impl LoadedRuntimeConfig {
    pub fn load() -> Result<Self, String> {
        let home_dir = vifu_core::config::default_home_dir()?;
        vifu_core::config::ensure_provider_registry_file(&home_dir)?;
        let path = home_dir.join(CONFIG_FILE_NAME);
        let config = RuntimeConfig::load_or_create(&path)?;
        Ok(Self { path, config })
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
            .unwrap_or_else(|| vifu_core::config::DEFAULT_SERVER_URL.to_string());
        Ok(GatewayRuntimeOptions { server_url })
    }

    #[cfg(feature = "server")]
    pub fn server_config(&self) -> Result<vifu_server::config::Config, String> {
        let server =
            self.config.server.as_ref().ok_or_else(|| {
                format!("{} does not configure a Vifu Server", self.path.display())
            })?;
        let mut config = vifu_server::config::Config::from_env()?;
        if let Some(listen) = server.listen.as_deref() {
            let addr = listen
                .parse()
                .map_err(|error| format!("server listen address {listen:?} is invalid: {error}"))?;
            config.apply_listen_override(addr)?;
        }
        config.apply_database_url(server.database_url()?)?;
        config.apply_runtime_extensions(server.runtime_extensions(&self.path)?)?;
        Ok(config)
    }
}

impl RuntimeConfig {
    fn load_or_create(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(raw) => {
                let mut config = Self::parse(path, &raw)?;
                if config.add_missing_local_database_url() {
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
        let config = Self {
            version: CONFIG_VERSION,
            server: cfg!(feature = "server").then(|| ServerRuntimeConfig {
                listen: Some("127.0.0.1:6790".to_string()),
                database_url: Some(DEFAULT_LOCAL_DATABASE_URL.to_string()),
                database_url_file: None,
                runtime_extensions: Vec::new(),
            }),
            gateway: cfg!(feature = "gateway").then(|| GatewayRuntimeConfig {
                server_url: Some(vifu_core::config::DEFAULT_SERVER_URL.to_string()),
            }),
        };
        if config.server.is_none() && config.gateway.is_none() {
            return Err("this vifu build does not include a runtime role".to_string());
        }
        Ok(config)
    }

    fn write(&self, path: &Path) -> Result<(), String> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|error| format!("Vifu runtime configuration could not be encoded: {error}"))?;
        vifu_core::config::write_private_file(path, &format!("{json}\n"))
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

    fn add_missing_local_database_url(&mut self) -> bool {
        let Some(server) = self.server.as_mut() else {
            return false;
        };
        if server.database_url.is_some() || server.database_url_file.is_some() {
            return false;
        }
        server.database_url = Some(DEFAULT_LOCAL_DATABASE_URL.to_string());
        true
    }
}

impl ServerRuntimeConfig {
    fn validate(&self) -> Result<(), String> {
        if self.database_url.is_some() && self.database_url_file.is_some() {
            return Err(
                "server configuration can set databaseUrl or databaseUrlFile, not both".to_string(),
            );
        }
        Ok(())
    }

    fn database_url(&self) -> Result<String, String> {
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
        Err("server configuration must set databaseUrl or databaseUrlFile".to_string())
    }

    #[cfg(feature = "server")]
    fn runtime_extensions(
        &self,
        config_path: &Path,
    ) -> Result<Vec<vifu_core::runtime_extension::RuntimeExtensionDefinition>, String> {
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
                vifu_core::runtime_extension::RuntimeExtensionDefinition::new(
                    manifest,
                    extension.base_url.clone(),
                    extension.credential(base_dir)?,
                )
            })
            .collect()
    }
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

    use super::{RuntimeConfig, DEFAULT_LOCAL_DATABASE_URL};

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
            r#"{"version":1,"gateway":{"serverUrl":"https://runtime.example.com"}}"#,
        )
        .unwrap();
        assert!(gateway_only.server.is_none());
        assert!(gateway_only.gateway.is_some());
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
            config.server.unwrap().database_url().unwrap(),
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

        assert!(config.server.is_some());
        assert!(path.exists());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn adds_a_database_url_to_existing_server_configuration() {
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
            config.server.unwrap().database_url().unwrap(),
            DEFAULT_LOCAL_DATABASE_URL
        );
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("databaseUrl"));

        std::fs::remove_dir_all(directory).unwrap();
    }
}
