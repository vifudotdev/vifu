use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use uuid::Uuid;
use vifu_protocol_alias::validate_identifier;

use crate::protocol as vifu_protocol_alias;

const SESSION_VERSION: &str = "3";
const LEGACY_SESSION_VERSION: &str = "2";
const MAX_SESSION_BYTES: u64 = 8 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionStatus {
    Missing,
    Ready(SessionSummary),
    UpgradeRequired(SessionSummary),
    Invalid(String),
}

#[derive(Clone, PartialEq, Eq)]
pub struct SessionSummary {
    pub gateway_id: String,
    pub gateway_credential: String,
    pub resume_session_id: Option<Uuid>,
    pub created_at_unix: u64,
}

impl fmt::Debug for SessionSummary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SessionSummary")
            .field("gateway_id", &self.gateway_id)
            .field("gateway_credential", &"[REDACTED]")
            .field("resume_session_id", &self.resume_session_id)
            .field("created_at_unix", &self.created_at_unix)
            .finish()
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
    if !metadata.is_file() {
        return SessionStatus::Invalid("session path is not a file".to_string());
    }
    if metadata.len() > MAX_SESSION_BYTES {
        return SessionStatus::Invalid("session file is too large".to_string());
    }
    #[cfg(unix)]
    if metadata.mode() & 0o077 != 0 {
        return SessionStatus::Invalid("session file permissions are too broad".to_string());
    }

    match fs::read_to_string(path) {
        Ok(contents) => match parse_session(&contents) {
            Ok((session, false)) => SessionStatus::Ready(session),
            Ok((session, true)) => SessionStatus::UpgradeRequired(session),
            Err(error) => SessionStatus::Invalid(error),
        },
        Err(error) => SessionStatus::Invalid(error.to_string()),
    }
}

pub fn write_session(path: &Path, session: &SessionSummary) -> Result<(), String> {
    validate_identifier("agent gateway id", &session.gateway_id)?;
    validate_gateway_credential(&session.gateway_credential)?;
    if session.created_at_unix == 0 {
        return Err("created_at_unix must be greater than zero".to_string());
    }
    let parent = path
        .parent()
        .ok_or_else(|| "session path must have a parent directory".to_string())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let tmp_path = tmp_session_path(path);
    let mut file = private_open_options()
        .open(&tmp_path)
        .map_err(|error| error.to_string())?;
    write!(
        file,
        "version={SESSION_VERSION}\ngateway_id={}\ngateway_credential={}\nresume_session_id={}\ncreated_at_unix={}\n",
        session.gateway_id,
        session.gateway_credential,
        session
            .resume_session_id
            .map(|value| value.to_string())
            .unwrap_or_default(),
        session.created_at_unix
    )
    .map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    drop(file);
    fs::rename(&tmp_path, path).map_err(|error| error.to_string())
}

fn parse_session(contents: &str) -> Result<(SessionSummary, bool), String> {
    let mut version = None;
    let mut gateway_id = None;
    let mut gateway_credential = None;
    let mut resume_session_id = None;
    let mut created_at_unix = None;
    for raw_line in contents.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| "invalid session line".to_string())?;
        match key {
            "version" => version = Some(value.to_string()),
            "gateway_id" => gateway_id = Some(value.to_string()),
            "gateway_credential" => gateway_credential = Some(value.to_string()),
            "resume_session_id" if value.is_empty() => resume_session_id = Some(None),
            "resume_session_id" => {
                resume_session_id = Some(Some(
                    Uuid::parse_str(value).map_err(|_| "invalid resume_session_id".to_string())?,
                ));
            }
            "created_at_unix" => {
                created_at_unix = Some(
                    value
                        .parse::<u64>()
                        .map_err(|_| "invalid created_at_unix".to_string())?,
                );
            }
            _ => return Err(format!("unknown session key: {key}")),
        }
    }
    let upgrade_required = match version.as_deref() {
        Some(SESSION_VERSION) => false,
        Some(LEGACY_SESSION_VERSION) => true,
        _ => return Err("unsupported session version".to_string()),
    };
    let gateway_id = gateway_id.ok_or_else(|| "session is missing gateway_id".to_string())?;
    validate_identifier("agent gateway id", &gateway_id)?;
    let gateway_credential = if upgrade_required {
        if gateway_credential.is_some() {
            return Err("legacy session contains a gateway credential".to_string());
        }
        generate_gateway_credential()
    } else {
        gateway_credential.ok_or_else(|| "session is missing gateway_credential".to_string())?
    };
    validate_gateway_credential(&gateway_credential)?;
    let created_at_unix =
        created_at_unix.ok_or_else(|| "session is missing created_at_unix".to_string())?;
    if created_at_unix == 0 {
        return Err("invalid created_at_unix".to_string());
    }
    Ok((
        SessionSummary {
            gateway_id,
            gateway_credential,
            resume_session_id: resume_session_id.unwrap_or(None),
            created_at_unix,
        },
        upgrade_required,
    ))
}

pub fn generate_gateway_credential() -> String {
    format!(
        "vifu_gw_{}{}",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    )
}

pub fn validate_gateway_credential(value: &str) -> Result<(), String> {
    let secret = value
        .strip_prefix("vifu_gw_")
        .ok_or_else(|| "agent gateway credential must start with vifu_gw_".to_string())?;
    if !(48..=256).contains(&value.len())
        || !secret
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err("invalid agent gateway credential".to_string());
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
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;
    use uuid::Uuid;

    use super::{parse_session, read_session, write_session, SessionStatus, SessionSummary};

    #[test]
    fn parses_resumable_session() {
        let session_id = Uuid::new_v4();
        let session = parse_session(&format!(
            "version=3\ngateway_id=gateway-local\ngateway_credential=vifu_gw_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\nresume_session_id={session_id}\ncreated_at_unix=42\n"
        ))
        .unwrap()
        .0;
        assert_eq!(session.resume_session_id, Some(session_id));
    }

    #[test]
    fn upgrades_v2_session_without_replacing_gateway_identity() {
        let session_id = Uuid::new_v4();
        let (session, upgrade_required) = parse_session(&format!(
            "version=2\ngateway_id=gateway-local\nresume_session_id={session_id}\ncreated_at_unix=42\n"
        ))
        .unwrap();

        assert!(upgrade_required);
        assert_eq!(session.gateway_id, "gateway-local");
        assert_eq!(session.resume_session_id, Some(session_id));
        assert!(session.gateway_credential.starts_with("vifu_gw_"));
    }

    #[test]
    fn rejects_unknown_session_keys() {
        let error = parse_session(
            "version=3\ngateway_id=gateway-local\ngateway_credential=vifu_gw_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\nresume_session_id=\ncreated_at_unix=42\nsecret=x\n",
        )
        .unwrap_err();
        assert!(error.contains("unknown session key"));
    }

    #[test]
    fn missing_session_is_not_an_error() {
        assert_eq!(
            read_session(&PathBuf::from(
                "/tmp/vifu-missing-agent-gateway-session-for-test"
            )),
            SessionStatus::Missing
        );
    }

    #[test]
    fn writes_and_reads_private_session() {
        let dir = unique_temp_dir("vifu-session-read");
        let path = dir.join("agent-gateway-session");
        let summary = SessionSummary {
            gateway_id: "gateway-local".to_string(),
            gateway_credential:
                "vifu_gw_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_string(),
            resume_session_id: Some(Uuid::new_v4()),
            created_at_unix: 42,
        };
        write_session(&path, &summary).unwrap();
        assert_eq!(read_session(&path), SessionStatus::Ready(summary));
        #[cfg(unix)]
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
        let _ = fs::remove_dir_all(dir);
    }

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{}-{nanos}", std::process::id()))
    }
}
