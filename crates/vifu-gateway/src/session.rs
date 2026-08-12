use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use uuid::Uuid;

use crate::identity::MachineIdentity;
use crate::protocol::validate_identifier;

const SESSION_VERSION: u32 = 5;
const MAX_SESSION_BYTES: u64 = 32 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionStatus {
    Missing,
    Ready(Box<SessionSummary>),
    Invalid(String),
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionSummary {
    pub identity: MachineIdentity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_generation: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_expires_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_session_id: Option<Uuid>,
    pub created_at_unix: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(rename = "guestApp", alias = "guestProject")]
    pub guest_project: Option<GuestProjectSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pairing: Option<PairingSummary>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PairingSummary {
    pub request_id: Uuid,
    pub auth_url: String,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GuestProjectSummary {
    #[serde(rename = "id", alias = "projectId")]
    pub project_id: Uuid,
    #[serde(default)]
    pub app_id: String,
    #[serde(rename = "appSlug", alias = "projectSlug")]
    pub project_slug: String,
    pub deployment_id: Uuid,
    pub deployment: String,
    pub endpoint_path: String,
    pub api_key: String,
    pub claim_token: String,
    pub expires_at: String,
}

impl fmt::Debug for SessionSummary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SessionSummary")
            .field("identity", &self.identity)
            .field("gateway_id", &self.gateway_id)
            .field(
                "device_token",
                &self.device_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("token_generation", &self.token_generation)
            .field("token_expires_at", &self.token_expires_at)
            .field("resume_session_id", &self.resume_session_id)
            .field("created_at_unix", &self.created_at_unix)
            .field(
                "guest_project",
                &self
                    .guest_project
                    .as_ref()
                    .map(|guest| format!("{} ({})", guest.project_slug, guest.deployment)),
            )
            .field(
                "pairing",
                &self.pairing.as_ref().map(|pairing| pairing.request_id),
            )
            .finish()
    }
}

impl SessionSummary {
    pub fn new(identity: MachineIdentity, created_at_unix: u64) -> Result<Self, String> {
        let session = Self {
            identity,
            gateway_id: None,
            device_token: None,
            token_generation: None,
            token_expires_at: None,
            resume_session_id: None,
            created_at_unix,
            guest_project: None,
            pairing: None,
        };
        validate_session(&session)?;
        Ok(session)
    }

    pub fn authorized_gateway_id(&self) -> Result<&str, String> {
        self.gateway_id
            .as_deref()
            .ok_or_else(|| "Agent Gateway has not been authorized".to_string())
    }

    pub fn device_token(&self) -> Result<&str, String> {
        self.device_token
            .as_deref()
            .ok_or_else(|| "Agent Gateway does not have a Device Token".to_string())
    }
}

pub fn read_session(path: &Path) -> SessionStatus {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return SessionStatus::Missing;
        }
        Err(error) => return SessionStatus::Invalid(error.to_string()),
    };
    if !metadata.is_file() || metadata.len() > MAX_SESSION_BYTES {
        return SessionStatus::Invalid("session file is invalid".to_string());
    }
    #[cfg(unix)]
    if metadata.mode() & 0o077 != 0 {
        return SessionStatus::Invalid("session file permissions are too broad".to_string());
    }
    let contents = match fs::read(path) {
        Ok(contents) => contents,
        Err(error) => return SessionStatus::Invalid(error.to_string()),
    };
    let stored = match serde_json::from_slice::<StoredSession>(&contents) {
        Ok(stored) if stored.version == SESSION_VERSION => stored,
        Ok(_) => return SessionStatus::Missing,
        Err(error) => return SessionStatus::Invalid(error.to_string()),
    };
    match validate_session(&stored.session) {
        Ok(()) => SessionStatus::Ready(Box::new(stored.session)),
        Err(error) => SessionStatus::Invalid(error),
    }
}

pub fn write_session(path: &Path, session: &SessionSummary) -> Result<(), String> {
    validate_session(session)?;
    let parent = path
        .parent()
        .ok_or_else(|| "session path must have a parent directory".to_string())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let tmp_path = tmp_session_path(path);
    let mut file = private_open_options()
        .open(&tmp_path)
        .map_err(|error| error.to_string())?;
    let encoded = serde_json::to_vec(&StoredSession {
        version: SESSION_VERSION,
        session: session.clone(),
    })
    .map_err(|error| error.to_string())?;
    file.write_all(&encoded)
        .map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    drop(file);
    fs::rename(&tmp_path, path).map_err(|error| error.to_string())
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredSession {
    version: u32,
    session: SessionSummary,
}

pub(crate) fn validate_session(session: &SessionSummary) -> Result<(), String> {
    session.identity.validate()?;
    if let Some(gateway_id) = session.gateway_id.as_deref() {
        validate_identifier("Agent Gateway id", gateway_id)?;
    }
    if let Some(device_token) = session.device_token.as_deref() {
        validate_device_token(device_token)?;
        let has_authorization_metadata =
            session.gateway_id.is_some() || session.token_generation.is_some();
        if has_authorization_metadata
            && (session.gateway_id.is_none() || session.token_generation.is_none())
        {
            return Err("Device Token is missing Gateway authorization metadata".to_string());
        }
    } else if session.token_generation.is_some() || session.token_expires_at.is_some() {
        return Err("Gateway authorization metadata is missing its Device Token".to_string());
    }
    if session.token_generation == Some(0) {
        return Err("Gateway token generation must be greater than zero".to_string());
    }
    if session.created_at_unix == 0 {
        return Err("created_at_unix must be greater than zero".to_string());
    }
    if let Some(guest) = session.guest_project.as_ref() {
        validate_guest_project(guest)?;
    }
    if let Some(pairing) = session.pairing.as_ref() {
        if pairing.auth_url.is_empty()
            || pairing.auth_url.len() > 2048
            || pairing.auth_url.chars().any(char::is_control)
        {
            return Err("Gateway authorization URL is invalid".to_string());
        }
    }
    Ok(())
}

fn validate_guest_project(guest: &GuestProjectSummary) -> Result<(), String> {
    if !guest.app_id.is_empty() {
        let app_id = guest
            .app_id
            .strip_prefix("vifu_app_")
            .ok_or_else(|| "invalid guest App ID".to_string())?;
        if app_id.len() != 64 || !app_id.chars().all(|value| value.is_ascii_hexdigit()) {
            return Err("invalid guest App ID".to_string());
        }
    }
    validate_identifier("guest app slug", &guest.project_slug)?;
    validate_identifier("guest deployment", &guest.deployment)?;
    if !guest.endpoint_path.starts_with('/')
        || guest.endpoint_path.len() > 256
        || guest.endpoint_path.chars().any(char::is_control)
    {
        return Err("invalid guest App endpoint path".to_string());
    }
    validate_prefixed_secret("guest API key", &guest.api_key, "vifu_pk_")?;
    validate_prefixed_secret("guest claim token", &guest.claim_token, "vifu_gc_")?;
    if guest.expires_at.is_empty()
        || guest.expires_at.len() > 64
        || guest.expires_at.chars().any(char::is_control)
    {
        return Err("invalid guest expiration".to_string());
    }
    Ok(())
}

fn validate_prefixed_secret(name: &str, value: &str, prefix: &str) -> Result<(), String> {
    let secret = value
        .strip_prefix(prefix)
        .ok_or_else(|| format!("invalid {name}"))?;
    if value.len() > 256
        || secret.len() < 48
        || !secret
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(format!("invalid {name}"));
    }
    Ok(())
}

pub fn validate_device_token(value: &str) -> Result<(), String> {
    let secret = value
        .strip_prefix("vifu_gw_")
        .ok_or_else(|| "Gateway Device Token must start with vifu_gw_".to_string())?;
    if !(48..=256).contains(&value.len())
        || !secret
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err("invalid Gateway Device Token".to_string());
    }
    Ok(())
}

fn private_open_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.create(true).write(true).truncate(true);
    #[cfg(unix)]
    options.mode(0o600);
    options
}

fn tmp_session_path(path: &Path) -> PathBuf {
    let mut tmp = path.to_path_buf();
    tmp.set_extension("tmp");
    tmp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_machine_identity_without_logging_secrets() {
        let directory = std::env::temp_dir().join(format!("vifu-session-{}", Uuid::new_v4()));
        let path = directory.join("session.json");
        let session = SessionSummary::new(MachineIdentity::generate().unwrap(), 42).unwrap();
        write_session(&path, &session).unwrap();
        assert_eq!(
            read_session(&path),
            SessionStatus::Ready(Box::new(session.clone()))
        );
        let debug = format!("{session:?}");
        assert!(debug.contains("[REDACTED]") || session.device_token.is_none());
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn old_session_formats_are_not_migrated() {
        let directory = std::env::temp_dir().join(format!("vifu-session-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("session.json");
        fs::write(&path, b"version=4\ngateway_id=gateway-old\n").unwrap();
        #[cfg(unix)]
        fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o600)).unwrap();
        assert!(matches!(read_session(&path), SessionStatus::Invalid(_)));
        let _ = fs::remove_dir_all(directory);
    }
}
