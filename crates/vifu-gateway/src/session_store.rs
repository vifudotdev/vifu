use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use url::Url;
use vifu_runtime::SqliteRuntimeStore;

use crate::identity::MachineIdentity;
use crate::session::{self, GuestProjectSummary, PairingSummary, SessionSummary};

const SESSION_NAMESPACE: &str = "agent-gateway-session";
const SESSION_SCHEMA_VERSION: u32 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GatewaySecretStorage {
    Persisted,
    External,
}

#[derive(Clone)]
pub struct GatewaySessionStore {
    store: Arc<SqliteRuntimeStore>,
}

#[derive(Clone)]
pub struct GatewaySessionPersistence {
    store: GatewaySessionStore,
    state_key: String,
    secret_storage: GatewaySecretStorage,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredGatewaySession {
    version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    identity: Option<MachineIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    gateway_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    device_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    token_generation: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    token_expires_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    resume_session_id: Option<uuid::Uuid>,
    created_at_unix: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    guest_project: Option<GuestProjectSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pairing: Option<PairingSummary>,
}

impl GatewaySessionStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, String> {
        Ok(Self {
            store: Arc::new(SqliteRuntimeStore::open(path).map_err(|error| error.to_string())?),
        })
    }

    pub fn load(
        &self,
        state_key: &str,
        external_identity: Option<&MachineIdentity>,
        external_device_token: Option<&str>,
    ) -> Result<Option<SessionSummary>, String> {
        let Some(value) = self
            .store
            .load_host_state(SESSION_NAMESPACE, state_key)
            .map_err(|error| error.to_string())?
        else {
            return Ok(None);
        };
        let stored = serde_json::from_value::<StoredGatewaySession>(value)
            .map_err(|error| format!("stored Agent Gateway session is invalid: {error}"))?;
        if stored.version != SESSION_SCHEMA_VERSION {
            return Ok(None);
        }
        let identity = match (external_identity, stored.identity) {
            (Some(external), Some(stored)) if external != &stored => {
                return Err(
                    "stored Gateway session does not match the supplied Machine identity"
                        .to_string(),
                );
            }
            (Some(external), _) => external.clone(),
            (None, Some(stored)) => stored,
            (None, None) => {
                return Err(
                    "stored Gateway session requires an external Machine identity".to_string(),
                );
            }
        };
        let device_token = match (external_device_token, stored.device_token) {
            (Some(external), Some(stored)) if external != stored => {
                return Err(
                    "stored Gateway session does not match the supplied Device Token".to_string(),
                );
            }
            (Some(external), _) => Some(external.to_string()),
            (None, stored) => stored,
        };
        let session = SessionSummary {
            identity,
            gateway_id: stored.gateway_id,
            device_token,
            token_generation: stored.token_generation,
            token_expires_at: stored.token_expires_at,
            resume_session_id: stored.resume_session_id,
            created_at_unix: stored.created_at_unix,
            guest_project: stored.guest_project,
            pairing: stored.pairing,
        };
        session::validate_session(&session)?;
        Ok(Some(session))
    }

    pub fn persistence(
        &self,
        state_key: impl Into<String>,
        secret_storage: GatewaySecretStorage,
    ) -> GatewaySessionPersistence {
        GatewaySessionPersistence {
            store: self.clone(),
            state_key: state_key.into(),
            secret_storage,
        }
    }
}

impl GatewaySessionPersistence {
    pub fn save(&self, session: &SessionSummary) -> Result<(), String> {
        session::validate_session(session)?;
        let stored = StoredGatewaySession {
            version: SESSION_SCHEMA_VERSION,
            identity: match self.secret_storage {
                GatewaySecretStorage::Persisted => Some(session.identity.clone()),
                GatewaySecretStorage::External => None,
            },
            gateway_id: session.gateway_id.clone(),
            device_token: match self.secret_storage {
                GatewaySecretStorage::Persisted => session.device_token.clone(),
                GatewaySecretStorage::External => None,
            },
            token_generation: session.token_generation,
            token_expires_at: session.token_expires_at.clone(),
            resume_session_id: session.resume_session_id,
            created_at_unix: session.created_at_unix,
            guest_project: session.guest_project.clone(),
            pairing: session.pairing.clone(),
        };
        let value = serde_json::to_value(stored).map_err(|error| error.to_string())?;
        self.store
            .store
            .save_host_state(SESSION_NAMESPACE, &self.state_key, &value)
            .map_err(|error| error.to_string())
    }
}

pub fn gateway_session_state_key(scope: &str, server_url: &str) -> Result<String, String> {
    if scope.is_empty()
        || scope.len() > 128
        || !scope
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b':'))
    {
        return Err("Agent Gateway session scope is invalid".to_string());
    }
    crate::relay::agent_gateway_websocket_url(server_url)?;
    let mut url = Url::parse(server_url.trim())
        .map_err(|_| "Agent Gateway server URL is invalid".to_string())?;
    let path = url.path().trim_end_matches('/').to_string();
    url.set_path(if path.is_empty() { "/" } else { &path });
    let key = format!("{scope}|{url}");
    if key.len() > 512 {
        return Err("Agent Gateway session key is too long".to_string());
    }
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::MachineIdentity;

    #[test]
    fn persisted_session_round_trips_machine_identity() {
        let path = std::env::temp_dir().join(format!(
            "vifu-session-store-{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = GatewaySessionStore::open(&path).unwrap();
        let session = SessionSummary::new(MachineIdentity::generate().unwrap(), 42).unwrap();
        store
            .persistence("cloud", GatewaySecretStorage::Persisted)
            .save(&session)
            .unwrap();
        assert_eq!(store.load("cloud", None, None).unwrap(), Some(session));
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;

            assert_eq!(std::fs::metadata(&path).unwrap().mode() & 0o077, 0);
            let wal_path = std::path::PathBuf::from(format!("{}-wal", path.display()));
            if wal_path.exists() {
                assert_eq!(std::fs::metadata(wal_path).unwrap().mode() & 0o077, 0);
            }
        }
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn external_storage_does_not_copy_secrets_into_sqlite() {
        let path = std::env::temp_dir().join(format!(
            "vifu-session-store-{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = GatewaySessionStore::open(&path).unwrap();
        let identity = MachineIdentity::generate().unwrap();
        let session = SessionSummary::new(identity.clone(), 42).unwrap();
        store
            .persistence("ios", GatewaySecretStorage::External)
            .save(&session)
            .unwrap();
        assert!(store.load("ios", None, None).is_err());
        assert_eq!(
            store.load("ios", Some(&identity), None).unwrap(),
            Some(session)
        );
        let _ = std::fs::remove_file(path);
    }
}
