use std::time::Duration;

use reqwest::redirect::Policy;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::header::AUTHORIZATION;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use url::Url;
use uuid::Uuid;

use crate::protocol::validate_identifier;

pub const RUNTIME_EXTENSION_MANIFEST_VERSION: u32 = 1;
pub const RUNTIME_EXTENSION_PROTOCOL_VERSION: &str = "vifu.runtime-extension/1";
pub const MAX_RUNTIME_RPC_BYTES: usize = 4 * 1024 * 1024;
pub type RuntimeExtensionWebSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeExtensionManifest {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub protocol: String,
    pub capabilities: Vec<String>,
    pub rpc_methods: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeProfileInvocation {
    pub profile_id: Uuid,
    pub profile_version_id: Uuid,
    pub capability: String,
    pub operation_id: String,
    pub input: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
}

impl RuntimeExtensionManifest {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != RUNTIME_EXTENSION_MANIFEST_VERSION {
            return Err(format!(
                "runtime extension manifest schemaVersion must be {RUNTIME_EXTENSION_MANIFEST_VERSION}"
            ));
        }
        validate_identifier("runtime extension id", &self.id)?;
        if self.name.trim().is_empty() || self.name.len() > 128 {
            return Err("runtime extension name must contain 1-128 characters".to_string());
        }
        if self.protocol != RUNTIME_EXTENSION_PROTOCOL_VERSION {
            return Err(format!(
                "runtime extension protocol must be {RUNTIME_EXTENSION_PROTOCOL_VERSION}"
            ));
        }
        if !self
            .capabilities
            .iter()
            .any(|capability| capability == "runtime.rpc")
        {
            return Err("runtime extension must declare the runtime.rpc capability".to_string());
        }
        if self.rpc_methods.is_empty() {
            return Err("runtime extension must declare at least one RPC method".to_string());
        }
        for method in &self.rpc_methods {
            validate_identifier("runtime RPC method", method)?;
        }
        Ok(())
    }

    pub fn allows_method(&self, method: &str) -> bool {
        self.rpc_methods.iter().any(|candidate| candidate == method)
    }
}

#[derive(Clone)]
pub struct RuntimeExtensionDefinition {
    pub manifest: RuntimeExtensionManifest,
    pub base_url: String,
    credential: String,
}

impl std::fmt::Debug for RuntimeExtensionDefinition {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RuntimeExtensionDefinition")
            .field("manifest", &self.manifest)
            .field("base_url", &self.base_url)
            .field("credential", &"[REDACTED]")
            .finish()
    }
}

impl RuntimeExtensionDefinition {
    pub fn new(
        manifest: RuntimeExtensionManifest,
        base_url: impl Into<String>,
        credential: impl Into<String>,
    ) -> Result<Self, String> {
        manifest.validate()?;
        let base_url = normalize_base_url(&base_url.into())?;
        let credential = credential.into();
        if credential.len() < 16
            || credential.len() > 512
            || credential.chars().any(char::is_control)
        {
            return Err(
                "runtime extension credential must contain 16-512 printable characters".to_string(),
            );
        }
        Ok(Self {
            manifest,
            base_url,
            credential,
        })
    }

    pub async fn call_rpc(
        &self,
        project_id: Uuid,
        project_slug: &str,
        release_ref: &str,
        request_id: Uuid,
        request: &Value,
        timeout: Duration,
    ) -> Result<Value, String> {
        let response = reqwest::Client::builder()
            .redirect(Policy::none())
            .timeout(timeout)
            .build()
            .map_err(|error| format!("runtime extension client could not start: {error}"))?
            .post(self.rpc_http_url()?)
            .bearer_auth(&self.credential)
            .header("x-vifu-project-id", project_id.to_string())
            .header("x-vifu-project-slug", project_slug)
            .header("x-vifu-runtime-release", release_ref)
            .header("x-vifu-request-id", request_id.to_string())
            .json(request)
            .send()
            .await
            .map_err(|error| format!("runtime extension request failed: {error}"))?;
        if !response.status().is_success() {
            return Err(format!(
                "runtime extension returned HTTP {}",
                response.status().as_u16()
            ));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_RUNTIME_RPC_BYTES as u64)
        {
            return Err("runtime extension response is too large".to_string());
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|error| format!("runtime extension response could not be read: {error}"))?;
        if bytes.len() > MAX_RUNTIME_RPC_BYTES {
            return Err("runtime extension response is too large".to_string());
        }
        serde_json::from_slice(&bytes)
            .map_err(|error| format!("runtime extension returned invalid JSON: {error}"))
    }

    pub fn authenticates(&self, candidate: &str) -> bool {
        constant_time_eq(self.credential.as_bytes(), candidate.as_bytes())
    }

    pub async fn connect_rpc_websocket(
        &self,
        project_id: Uuid,
        project_slug: &str,
        release_ref: &str,
        request_id: Uuid,
    ) -> Result<RuntimeExtensionWebSocket, String> {
        let mut request = self
            .rpc_websocket_url()?
            .to_string()
            .into_client_request()
            .map_err(|error| format!("runtime extension WebSocket URL is invalid: {error}"))?;
        request.headers_mut().insert(
            AUTHORIZATION,
            format!("Bearer {}", self.credential)
                .parse()
                .map_err(|_| "runtime extension credential is invalid".to_string())?,
        );
        for (name, value) in [
            ("x-vifu-project-id", project_id.to_string()),
            ("x-vifu-project-slug", project_slug.to_string()),
            ("x-vifu-runtime-release", release_ref.to_string()),
            ("x-vifu-request-id", request_id.to_string()),
        ] {
            request.headers_mut().insert(
                name,
                value
                    .parse()
                    .map_err(|_| format!("runtime extension {name} header is invalid"))?,
            );
        }
        connect_async(request)
            .await
            .map(|(socket, _)| socket)
            .map_err(|error| format!("runtime extension WebSocket connection failed: {error}"))
    }

    fn rpc_http_url(&self) -> Result<Url, String> {
        Url::parse(&self.base_url)
            .map_err(|error| format!("runtime extension base URL is invalid: {error}"))?
            .join("v1/rpc")
            .map_err(|error| format!("runtime extension RPC URL is invalid: {error}"))
    }

    fn rpc_websocket_url(&self) -> Result<Url, String> {
        let mut url = self.rpc_http_url()?;
        let scheme = match url.scheme() {
            "http" => "ws",
            "https" => "wss",
            _ => return Err("runtime extension base URL must use HTTP or HTTPS".to_string()),
        };
        url.set_scheme(scheme)
            .map_err(|_| "runtime extension WebSocket URL is invalid".to_string())?;
        Ok(url)
    }
}

#[derive(Clone, Debug)]
pub struct RuntimeCoreClient {
    base_url: String,
    extension_id: String,
    credential: String,
    client: reqwest::Client,
}

impl RuntimeCoreClient {
    pub fn new(
        base_url: impl Into<String>,
        extension_id: impl Into<String>,
        credential: impl Into<String>,
        timeout: Duration,
    ) -> Result<Self, String> {
        let base_url = normalize_base_url(&base_url.into())?;
        let extension_id = extension_id.into();
        validate_identifier("runtime extension id", &extension_id)?;
        let credential = credential.into();
        if credential.len() < 16
            || credential.len() > 512
            || credential.chars().any(char::is_control)
        {
            return Err(
                "runtime extension credential must contain 16-512 printable characters".to_string(),
            );
        }
        let client = reqwest::Client::builder()
            .redirect(Policy::none())
            .timeout(timeout)
            .build()
            .map_err(|error| format!("Vifu core client could not start: {error}"))?;
        Ok(Self {
            base_url,
            extension_id,
            credential,
            client,
        })
    }

    pub async fn invoke_profile(
        &self,
        project_id: Uuid,
        invocation: &RuntimeProfileInvocation,
    ) -> Result<Value, String> {
        let path = format!(
            "v1/runtime-extensions/{}/projects/{project_id}/invoke",
            self.extension_id
        );
        let url = Url::parse(&self.base_url)
            .map_err(|error| format!("Vifu core URL is invalid: {error}"))?
            .join(&path)
            .map_err(|error| format!("Vifu core callback URL is invalid: {error}"))?;
        let response = self
            .client
            .post(url)
            .bearer_auth(&self.credential)
            .json(invocation)
            .send()
            .await
            .map_err(|error| format!("Vifu core callback failed: {error}"))?;
        if !response.status().is_success() {
            return Err(format!(
                "Vifu core callback returned HTTP {}",
                response.status().as_u16()
            ));
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|error| format!("Vifu core callback could not be read: {error}"))?;
        if bytes.len() > MAX_RUNTIME_RPC_BYTES {
            return Err("Vifu core callback response is too large".to_string());
        }
        let response: Value = serde_json::from_slice(&bytes)
            .map_err(|error| format!("Vifu core callback returned invalid JSON: {error}"))?;
        response
            .get("output")
            .cloned()
            .ok_or_else(|| "Vifu core callback response is missing output".to_string())
    }
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

fn normalize_base_url(value: &str) -> Result<String, String> {
    let mut url = Url::parse(value.trim())
        .map_err(|error| format!("runtime extension base URL is invalid: {error}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("runtime extension base URL must use HTTP or HTTPS".to_string());
    }
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(
            "runtime extension base URL cannot contain credentials, a query, or a fragment"
                .to_string(),
        );
    }
    if !url.path().ends_with('/') {
        url.set_path(&format!("{}/", url.path()));
    }
    Ok(url.to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        RuntimeExtensionDefinition, RuntimeExtensionManifest, RUNTIME_EXTENSION_MANIFEST_VERSION,
        RUNTIME_EXTENSION_PROTOCOL_VERSION,
    };

    fn manifest() -> RuntimeExtensionManifest {
        RuntimeExtensionManifest {
            schema_version: RUNTIME_EXTENSION_MANIFEST_VERSION,
            id: "example.runtime".to_string(),
            name: "Example Runtime".to_string(),
            protocol: RUNTIME_EXTENSION_PROTOCOL_VERSION.to_string(),
            capabilities: vec!["runtime.rpc".to_string()],
            rpc_methods: vec!["runtime.describe".to_string(), "session.create".to_string()],
        }
    }

    #[test]
    fn validates_metadata_without_loading_runtime_code() {
        let manifest = manifest();
        manifest.validate().unwrap();
        assert!(manifest.allows_method("runtime.describe"));
        assert!(!manifest.allows_method("unknown.method"));
    }

    #[test]
    fn rejects_unregistered_runtime_capability() {
        let mut manifest = manifest();
        manifest.capabilities.clear();
        assert!(manifest.validate().unwrap_err().contains("runtime.rpc"));
    }

    #[test]
    fn rejects_unsafe_service_urls() {
        let error = RuntimeExtensionDefinition::new(
            manifest(),
            "https://user@example.com",
            "example-runtime-credential",
        )
        .unwrap_err();
        assert!(error.contains("credentials"));
    }

    #[test]
    fn authenticates_without_exposing_the_credential() {
        let definition = RuntimeExtensionDefinition::new(
            manifest(),
            "https://runtime.example.com",
            "example-runtime-credential",
        )
        .unwrap();
        assert!(definition.authenticates("example-runtime-credential"));
        assert!(!definition.authenticates("example-runtime-credential-wrong"));
    }
}
