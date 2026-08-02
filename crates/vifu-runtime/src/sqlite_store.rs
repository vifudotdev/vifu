use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use rusqlite::{params, Connection, OptionalExtension};

use crate::{
    LocalProviderBinding, RuntimeError, RuntimeRelease, RuntimeSnapshot, RuntimeStore,
    RuntimeTraceRecord,
};

const MAX_TRACE_OUTBOX_RECORDS: i64 = 1_000;
const MAX_HOST_STATE_BYTES: usize = 64 * 1024;

/// SQLite-backed application state for an embedded Vifu runtime.
///
/// The database contains portable releases, session state, local provider
/// references, and a bounded trace upload queue. Credentials remain the host's
/// responsibility and should be represented by Keychain or credential-store
/// references rather than copied into a release manifest.
pub struct SqliteRuntimeStore {
    connection: Mutex<Connection>,
}

impl SqliteRuntimeStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, RuntimeError> {
        let path = path.as_ref();
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)
                .map_err(|error| RuntimeError::store(error.to_string()))?;
        }
        let connection =
            Connection::open(path).map_err(|error| RuntimeError::store(error.to_string()))?;
        protect_state_file(path)?;
        Self::from_connection(connection)
    }

    pub fn in_memory() -> Result<Self, RuntimeError> {
        let connection =
            Connection::open_in_memory().map_err(|error| RuntimeError::store(error.to_string()))?;
        Self::from_connection(connection)
    }

    fn from_connection(connection: Connection) -> Result<Self, RuntimeError> {
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(|error| RuntimeError::store(error.to_string()))?;
        connection
            .execute_batch(
                "PRAGMA foreign_keys = ON;
                 PRAGMA journal_mode = WAL;
                 CREATE TABLE IF NOT EXISTS runtime_sessions (
                    project_id TEXT NOT NULL,
                    session_id TEXT NOT NULL,
                    revision INTEGER NOT NULL,
                    state_json TEXT NOT NULL,
                    PRIMARY KEY (project_id, session_id)
                 );
                 CREATE TABLE IF NOT EXISTS runtime_releases (
                    project_id TEXT NOT NULL,
                    version INTEGER NOT NULL,
                    content_hash TEXT NOT NULL,
                    manifest_json TEXT NOT NULL,
                    installed_at_ms INTEGER NOT NULL,
                    PRIMARY KEY (project_id, version),
                    UNIQUE (project_id, content_hash)
                 );
                 CREATE TABLE IF NOT EXISTS runtime_active_releases (
                    project_id TEXT PRIMARY KEY,
                    version INTEGER NOT NULL,
                    FOREIGN KEY (project_id, version)
                      REFERENCES runtime_releases(project_id, version)
                 );
                 CREATE TABLE IF NOT EXISTS runtime_provider_bindings (
                    project_id TEXT NOT NULL,
                    provider_id TEXT NOT NULL,
                    binding_json TEXT NOT NULL,
                    PRIMARY KEY (project_id, provider_id)
                 );
                 CREATE TABLE IF NOT EXISTS runtime_trace_outbox (
                    trace_id TEXT PRIMARY KEY,
                    project_id TEXT NOT NULL,
                    payload_json TEXT NOT NULL,
                    created_at_ms INTEGER NOT NULL
                 );
                 CREATE INDEX IF NOT EXISTS runtime_trace_outbox_created_idx
                   ON runtime_trace_outbox(created_at_ms, trace_id);
                 CREATE TABLE IF NOT EXISTS runtime_host_state (
                    namespace TEXT NOT NULL,
                    state_key TEXT NOT NULL,
                    value_json TEXT NOT NULL,
                    updated_at_ms INTEGER NOT NULL,
                    PRIMARY KEY (namespace, state_key)
                 );",
            )
            .map_err(|error| RuntimeError::store(error.to_string()))?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    fn connection(&self) -> Result<std::sync::MutexGuard<'_, Connection>, RuntimeError> {
        self.connection.lock().map_err(|_| RuntimeError::Internal)
    }

    /// Loads host-owned local state kept beside the runtime database.
    ///
    /// This is intended for adapters such as Agent Gateway. Portable runtime
    /// releases must not depend on these records.
    pub fn load_host_state(
        &self,
        namespace: &str,
        state_key: &str,
    ) -> Result<Option<serde_json::Value>, RuntimeError> {
        validate_host_state_key(namespace, "namespace")?;
        validate_host_state_key(state_key, "state key")?;
        self.connection()?
            .query_row(
                "SELECT value_json
                 FROM runtime_host_state
                 WHERE namespace = ?1 AND state_key = ?2",
                params![namespace, state_key],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| RuntimeError::store(error.to_string()))?
            .map(|value_json| {
                serde_json::from_str(&value_json)
                    .map_err(|error| RuntimeError::store(error.to_string()))
            })
            .transpose()
    }

    /// Saves bounded host-owned local state atomically.
    pub fn save_host_state(
        &self,
        namespace: &str,
        state_key: &str,
        value: &serde_json::Value,
    ) -> Result<(), RuntimeError> {
        validate_host_state_key(namespace, "namespace")?;
        validate_host_state_key(state_key, "state key")?;
        let value_json =
            serde_json::to_string(value).map_err(|error| RuntimeError::store(error.to_string()))?;
        if value_json.len() > MAX_HOST_STATE_BYTES {
            return Err(RuntimeError::store("host state is too large".to_string()));
        }
        let updated_at_ms = i64::try_from(crate::unix_time_ms())
            .map_err(|error| RuntimeError::store(error.to_string()))?;
        self.connection()?
            .execute(
                "INSERT INTO runtime_host_state(namespace, state_key, value_json, updated_at_ms)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(namespace, state_key) DO UPDATE SET
                   value_json = excluded.value_json,
                   updated_at_ms = excluded.updated_at_ms",
                params![namespace, state_key, value_json, updated_at_ms],
            )
            .map_err(|error| RuntimeError::store(error.to_string()))?;
        Ok(())
    }
}

fn validate_host_state_key(value: &str, label: &str) -> Result<(), RuntimeError> {
    if value.is_empty() || value.len() > 512 || value.chars().any(char::is_control) {
        return Err(RuntimeError::store(format!("invalid host state {label}")));
    }
    Ok(())
}

#[cfg(unix)]
fn protect_state_file(path: &Path) -> Result<(), RuntimeError> {
    let mut permissions = std::fs::metadata(path)
        .map_err(|error| RuntimeError::store(error.to_string()))?
        .permissions();
    permissions.set_mode(0o600);
    std::fs::set_permissions(path, permissions)
        .map_err(|error| RuntimeError::store(error.to_string()))
}

#[cfg(not(unix))]
fn protect_state_file(_path: &Path) -> Result<(), RuntimeError> {
    Ok(())
}

impl RuntimeStore for SqliteRuntimeStore {
    fn load(
        &self,
        project_id: &str,
        session_id: &str,
    ) -> Result<Option<RuntimeSnapshot>, RuntimeError> {
        let connection = self.connection()?;
        connection
            .query_row(
                "SELECT revision, state_json
                 FROM runtime_sessions
                 WHERE project_id = ?1 AND session_id = ?2",
                params![project_id, session_id],
                |row| {
                    let revision = row.get::<_, i64>(0)?;
                    let state_json = row.get::<_, String>(1)?;
                    Ok((revision, state_json))
                },
            )
            .optional()
            .map_err(|error| RuntimeError::store(error.to_string()))?
            .map(|(revision, state_json)| {
                Ok(RuntimeSnapshot {
                    revision: u64::try_from(revision)
                        .map_err(|error| RuntimeError::store(error.to_string()))?,
                    state: serde_json::from_str(&state_json)
                        .map_err(|error| RuntimeError::store(error.to_string()))?,
                })
            })
            .transpose()
    }

    fn save(
        &self,
        project_id: &str,
        session_id: &str,
        snapshot: &RuntimeSnapshot,
    ) -> Result<(), RuntimeError> {
        let state_json = serde_json::to_string(&snapshot.state)
            .map_err(|error| RuntimeError::store(error.to_string()))?;
        let revision = i64::try_from(snapshot.revision)
            .map_err(|error| RuntimeError::store(error.to_string()))?;
        self.connection()?
            .execute(
                "INSERT INTO runtime_sessions(project_id, session_id, revision, state_json)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(project_id, session_id) DO UPDATE SET
                   revision = excluded.revision,
                   state_json = excluded.state_json",
                params![project_id, session_id, revision, state_json],
            )
            .map_err(|error| RuntimeError::store(error.to_string()))?;
        Ok(())
    }

    fn save_release(&self, release: &RuntimeRelease) -> Result<(), RuntimeError> {
        release.validate()?;
        let version = i64::try_from(release.version)
            .map_err(|error| RuntimeError::store(error.to_string()))?;
        let manifest_json = serde_json::to_string(&release.manifest)
            .map_err(|error| RuntimeError::store(error.to_string()))?;
        let installed_at_ms = i64::try_from(crate::unix_time_ms())
            .map_err(|error| RuntimeError::store(error.to_string()))?;
        let connection = self.connection()?;
        let inserted = connection
            .execute(
                "INSERT OR IGNORE INTO runtime_releases(
                    project_id, version, content_hash, manifest_json, installed_at_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    release.manifest.project_id,
                    version,
                    release.content_hash,
                    manifest_json,
                    installed_at_ms
                ],
            )
            .map_err(|error| RuntimeError::store(error.to_string()))?;
        if inserted == 0 {
            let existing =
                load_release(&connection, &release.manifest.project_id, release.version)?;
            if existing.as_ref() != Some(release) {
                return Err(RuntimeError::store(
                    "runtime release versions are immutable".to_string(),
                ));
            }
        }
        Ok(())
    }

    fn load_release(
        &self,
        project_id: &str,
        version: u64,
    ) -> Result<Option<RuntimeRelease>, RuntimeError> {
        let connection = self.connection()?;
        load_release(&connection, project_id, version)
    }

    fn list_releases(&self, project_id: &str) -> Result<Vec<RuntimeRelease>, RuntimeError> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT version, content_hash, manifest_json
                 FROM runtime_releases
                 WHERE project_id = ?1
                 ORDER BY version DESC",
            )
            .map_err(|error| RuntimeError::store(error.to_string()))?;
        let rows = statement
            .query_map(params![project_id], release_from_row)
            .map_err(|error| RuntimeError::store(error.to_string()))?;
        rows.map(|row| row.map_err(|error| RuntimeError::store(error.to_string())))
            .collect()
    }

    fn active_release(&self, project_id: &str) -> Result<Option<u64>, RuntimeError> {
        self.connection()?
            .query_row(
                "SELECT version FROM runtime_active_releases WHERE project_id = ?1",
                params![project_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|error| RuntimeError::store(error.to_string()))?
            .map(|version| {
                u64::try_from(version).map_err(|error| RuntimeError::store(error.to_string()))
            })
            .transpose()
    }

    fn set_active_release(&self, project_id: &str, version: u64) -> Result<(), RuntimeError> {
        let version =
            i64::try_from(version).map_err(|error| RuntimeError::store(error.to_string()))?;
        self.connection()?
            .execute(
                "INSERT INTO runtime_active_releases(project_id, version)
                 VALUES (?1, ?2)
                 ON CONFLICT(project_id) DO UPDATE SET version = excluded.version",
                params![project_id, version],
            )
            .map_err(|error| RuntimeError::store(error.to_string()))?;
        Ok(())
    }

    fn save_local_provider_binding(
        &self,
        project_id: &str,
        binding: &LocalProviderBinding,
    ) -> Result<(), RuntimeError> {
        let binding_json = serde_json::to_string(binding)
            .map_err(|error| RuntimeError::store(error.to_string()))?;
        self.connection()?
            .execute(
                "INSERT INTO runtime_provider_bindings(project_id, provider_id, binding_json)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(project_id, provider_id) DO UPDATE SET
                   binding_json = excluded.binding_json",
                params![project_id, binding.provider_id, binding_json],
            )
            .map_err(|error| RuntimeError::store(error.to_string()))?;
        Ok(())
    }

    fn local_provider_bindings(
        &self,
        project_id: &str,
    ) -> Result<Vec<LocalProviderBinding>, RuntimeError> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT binding_json
                 FROM runtime_provider_bindings
                 WHERE project_id = ?1
                 ORDER BY provider_id",
            )
            .map_err(|error| RuntimeError::store(error.to_string()))?;
        let rows = statement
            .query_map(params![project_id], |row| row.get::<_, String>(0))
            .map_err(|error| RuntimeError::store(error.to_string()))?;
        rows.map(|row| {
            let json = row.map_err(|error| RuntimeError::store(error.to_string()))?;
            serde_json::from_str(&json).map_err(|error| RuntimeError::store(error.to_string()))
        })
        .collect()
    }

    fn enqueue_trace(&self, trace: &RuntimeTraceRecord) -> Result<(), RuntimeError> {
        let payload_json =
            serde_json::to_string(trace).map_err(|error| RuntimeError::store(error.to_string()))?;
        let created_at_ms = i64::try_from(trace.created_at_ms)
            .map_err(|error| RuntimeError::store(error.to_string()))?;
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction()
            .map_err(|error| RuntimeError::store(error.to_string()))?;
        transaction
            .execute(
                "INSERT OR IGNORE INTO runtime_trace_outbox(
                    trace_id, project_id, payload_json, created_at_ms
                 ) VALUES (?1, ?2, ?3, ?4)",
                params![trace.id, trace.project_id, payload_json, created_at_ms],
            )
            .map_err(|error| RuntimeError::store(error.to_string()))?;
        transaction
            .execute(
                "DELETE FROM runtime_trace_outbox
                 WHERE trace_id IN (
                   SELECT trace_id FROM runtime_trace_outbox
                   ORDER BY created_at_ms DESC, trace_id DESC
                   LIMIT -1 OFFSET ?1
                 )",
                params![MAX_TRACE_OUTBOX_RECORDS],
            )
            .map_err(|error| RuntimeError::store(error.to_string()))?;
        transaction
            .commit()
            .map_err(|error| RuntimeError::store(error.to_string()))?;
        Ok(())
    }

    fn pending_traces(&self, limit: usize) -> Result<Vec<RuntimeTraceRecord>, RuntimeError> {
        let limit = i64::try_from(limit.min(MAX_TRACE_OUTBOX_RECORDS as usize))
            .map_err(|error| RuntimeError::store(error.to_string()))?;
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT payload_json
                 FROM runtime_trace_outbox
                 ORDER BY created_at_ms, trace_id
                 LIMIT ?1",
            )
            .map_err(|error| RuntimeError::store(error.to_string()))?;
        let rows = statement
            .query_map(params![limit], |row| row.get::<_, String>(0))
            .map_err(|error| RuntimeError::store(error.to_string()))?;
        rows.map(|row| {
            let json = row.map_err(|error| RuntimeError::store(error.to_string()))?;
            serde_json::from_str(&json).map_err(|error| RuntimeError::store(error.to_string()))
        })
        .collect()
    }

    fn acknowledge_traces(&self, trace_ids: &[String]) -> Result<(), RuntimeError> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction()
            .map_err(|error| RuntimeError::store(error.to_string()))?;
        for trace_id in trace_ids {
            transaction
                .execute(
                    "DELETE FROM runtime_trace_outbox WHERE trace_id = ?1",
                    params![trace_id],
                )
                .map_err(|error| RuntimeError::store(error.to_string()))?;
        }
        transaction
            .commit()
            .map_err(|error| RuntimeError::store(error.to_string()))?;
        Ok(())
    }
}

fn load_release(
    connection: &Connection,
    project_id: &str,
    version: u64,
) -> Result<Option<RuntimeRelease>, RuntimeError> {
    let version = i64::try_from(version).map_err(|error| RuntimeError::store(error.to_string()))?;
    connection
        .query_row(
            "SELECT version, content_hash, manifest_json
             FROM runtime_releases
             WHERE project_id = ?1 AND version = ?2",
            params![project_id, version],
            release_from_row,
        )
        .optional()
        .map_err(|error| RuntimeError::store(error.to_string()))
}

fn release_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RuntimeRelease> {
    let version = row.get::<_, i64>(0)?;
    let content_hash = row.get::<_, String>(1)?;
    let manifest_json = row.get::<_, String>(2)?;
    let manifest = serde_json::from_str(&manifest_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            manifest_json.len(),
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })?;
    let version = u64::try_from(version).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            8,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })?;
    Ok(RuntimeRelease {
        version,
        content_hash,
        manifest,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use super::*;
    use crate::{
        AgentDefinition, EndpointDefinition, ProviderRequirement, RuntimeManifest,
        RUNTIME_MANIFEST_SCHEMA_VERSION,
    };

    fn release(version: u64) -> RuntimeRelease {
        RuntimeRelease::new(
            version,
            RuntimeManifest {
                schema_version: RUNTIME_MANIFEST_SCHEMA_VERSION,
                project_id: "test-project".to_string(),
                providers: vec![ProviderRequirement {
                    id: "native".to_string(),
                    provider_type: "native".to_string(),
                    capabilities: vec!["chat".to_string()],
                    settings: json!({}),
                    resources: BTreeMap::new(),
                }],
                agents: vec![AgentDefinition {
                    id: "guide".to_string(),
                    name: "Guide".to_string(),
                    provider: "native".to_string(),
                    capabilities: vec!["chat".to_string()],
                    metadata: json!({}),
                }],
                endpoints: vec![EndpointDefinition {
                    name: "guide".to_string(),
                    agent: "guide".to_string(),
                    capability: "chat".to_string(),
                    timeout_ms: 30_000,
                }],
                metadata: json!({}),
            },
        )
        .unwrap()
    }

    #[test]
    fn sqlite_store_persists_release_session_binding_and_trace() {
        let store = SqliteRuntimeStore::in_memory().unwrap();
        let release = release(1);
        store.save_release(&release).unwrap();
        store.set_active_release("test-project", 1).unwrap();
        store
            .save(
                "test-project",
                "player",
                &RuntimeSnapshot {
                    revision: 2,
                    state: json!({ "chapter": 3 }),
                },
            )
            .unwrap();
        store
            .save_local_provider_binding(
                "test-project",
                &LocalProviderBinding {
                    provider_id: "native".to_string(),
                    configuration: json!({ "credentialRef": "keychain:vifu/native" }),
                },
            )
            .unwrap();
        store
            .enqueue_trace(&RuntimeTraceRecord {
                id: "trace-1".to_string(),
                project_id: "test-project".to_string(),
                invocation_id: "invocation-1".to_string(),
                endpoint: "guide".to_string(),
                agent: Some("guide".to_string()),
                provider: Some("native".to_string()),
                capability: Some("chat".to_string()),
                status: "completed".to_string(),
                duration_ms: 4,
                created_at_ms: 10,
            })
            .unwrap();

        assert_eq!(store.active_release("test-project").unwrap(), Some(1));
        assert_eq!(
            store
                .load("test-project", "player")
                .unwrap()
                .unwrap()
                .revision,
            2
        );
        assert_eq!(
            store.local_provider_bindings("test-project").unwrap().len(),
            1
        );
        assert_eq!(store.pending_traces(10).unwrap().len(), 1);

        store.acknowledge_traces(&["trace-1".to_string()]).unwrap();
        assert!(store.pending_traces(10).unwrap().is_empty());
    }

    #[test]
    fn sqlite_store_rejects_mutating_an_existing_release_version() {
        let store = SqliteRuntimeStore::in_memory().unwrap();
        store.save_release(&release(1)).unwrap();
        let mut changed = release(1);
        changed.manifest.metadata = json!({ "changed": true });
        changed.content_hash = changed.manifest.content_hash().unwrap();
        assert!(store.save_release(&changed).is_err());
    }

    #[test]
    fn sqlite_store_persists_and_updates_bounded_host_state() {
        let store = SqliteRuntimeStore::in_memory().unwrap();
        let value = json!({ "gatewayId": "gateway-test", "resume": true });

        store
            .save_host_state(
                "agent-gateway-session",
                "cloud|https://api.example.com/",
                &value,
            )
            .unwrap();

        assert_eq!(
            store
                .load_host_state("agent-gateway-session", "cloud|https://api.example.com/")
                .unwrap(),
            Some(value)
        );
        let updated = json!({ "gatewayId": "gateway-test", "resume": false });
        store
            .save_host_state(
                "agent-gateway-session",
                "cloud|https://api.example.com/",
                &updated,
            )
            .unwrap();
        assert_eq!(
            store
                .load_host_state("agent-gateway-session", "cloud|https://api.example.com/")
                .unwrap(),
            Some(updated)
        );
    }
}
