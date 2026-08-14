mod postgres;
mod sqlite;
mod types;

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{PgPool, SqlitePool};
use std::str::FromStr;
use std::time::Duration;
use uuid::Uuid;

use crate::error::ApiError;
use crate::models::*;

pub use types::*;

#[derive(Clone)]
pub enum Storage {
    Postgres(PgPool),
    Sqlite(SqlitePool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageKind {
    Postgres,
    Sqlite,
}

impl Storage {
    pub fn postgres(pool: PgPool) -> Self {
        Self::Postgres(pool)
    }

    pub fn sqlite(pool: SqlitePool) -> Self {
        Self::Sqlite(pool)
    }

    pub fn kind(&self) -> StorageKind {
        match self {
            Self::Postgres(_) => StorageKind::Postgres,
            Self::Sqlite(_) => StorageKind::Sqlite,
        }
    }
}

pub async fn connect(database_url: &str, max_connections: u32) -> Result<Storage, ApiError> {
    if database_url.starts_with("postgres://") || database_url.starts_with("postgresql://") {
        let pool = PgPoolOptions::new()
            .max_connections(max_connections)
            .connect(database_url)
            .await?;
        return Ok(Storage::postgres(pool));
    }
    if database_url.starts_with("sqlite:") {
        let options = SqliteConnectOptions::from_str(database_url)?
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(max_connections.min(5))
            .connect_with(options)
            .await?;
        return Ok(Storage::sqlite(pool));
    }
    Err(ApiError::Invalid(
        "database URL must use postgres://, postgresql://, or sqlite:".to_string(),
    ))
}

#[cfg(unix)]
pub fn protect_sqlite_files(database_url: &str) -> Result<(), ApiError> {
    use std::os::unix::fs::PermissionsExt;

    if !database_url.starts_with("sqlite:") {
        return Ok(());
    }

    let options = SqliteConnectOptions::from_str(database_url)?;
    let path = options.get_filename();
    if path == std::path::Path::new(":memory:") {
        return Ok(());
    }

    for candidate in [
        path.to_path_buf(),
        sqlite_sidecar_path(path, "-wal"),
        sqlite_sidecar_path(path, "-shm"),
    ] {
        let metadata = match std::fs::metadata(&candidate) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(ApiError::Invalid(format!(
                    "{} metadata could not be read: {error}",
                    candidate.display()
                )))
            }
        };
        let mut permissions = metadata.permissions();
        permissions.set_mode(0o600);
        std::fs::set_permissions(&candidate, permissions).map_err(|error| {
            ApiError::Invalid(format!(
                "{} permissions could not be updated: {error}",
                candidate.display()
            ))
        })?;
    }

    Ok(())
}

#[cfg(not(unix))]
pub fn protect_sqlite_files(_database_url: &str) -> Result<(), ApiError> {
    Ok(())
}

#[cfg(unix)]
fn sqlite_sidecar_path(path: &std::path::Path, suffix: &str) -> std::path::PathBuf {
    let mut value = path.as_os_str().to_owned();
    value.push(suffix);
    value.into()
}

macro_rules! dispatch {
    (
        $(
            pub async fn $name:ident(
                $storage:ident: &Storage
                $(, $arg:ident: $arg_ty:ty)*
                $(,)?
            ) -> $output:ty;
        )+
    ) => {
        $(
            pub async fn $name(
                $storage: &Storage,
                $($arg: $arg_ty),*
            ) -> $output {
                match $storage {
                    Storage::Postgres(pool) => postgres::$name(pool, $($arg),*).await,
                    Storage::Sqlite(pool) => sqlite::$name(pool, $($arg),*).await,
                }
            }
        )+
    };
}

dispatch! {
    pub async fn migrate(storage: &Storage) -> Result<(), ApiError>;
    pub async fn ready(storage: &Storage) -> Result<(), ApiError>;
    pub async fn mark_agent_gateway_sessions_disconnected(storage: &Storage) -> Result<(), ApiError>;
    pub async fn list_projects(storage: &Storage) -> Result<Vec<ProjectWithBindings>, ApiError>;
    pub async fn list_projects_for_owner_user_id(storage: &Storage, owner_user_id: &str) -> Result<Vec<ProjectWithBindings>, ApiError>;
    pub async fn get_project(storage: &Storage, id: Uuid) -> Result<ProjectWithBindings, ApiError>;
    pub async fn get_project_by_slug(storage: &Storage, slug: &str) -> Result<ProjectWithBindings, ApiError>;
    pub async fn set_project_owner_user_id(storage: &Storage, id: Uuid, owner_user_id: &str) -> Result<ProjectWithBindings, ApiError>;
    pub async fn list_runtime_deployments(storage: &Storage, project_id: Uuid) -> Result<Vec<RuntimeDeployment>, ApiError>;
    pub async fn get_runtime_deployment(storage: &Storage, project_id: Uuid, name: &str) -> Result<RuntimeDeployment, ApiError>;
    pub async fn create_runtime_deployment(storage: &Storage, input: NewRuntimeDeployment<'_>) -> Result<RuntimeDeployment, ApiError>;
    pub async fn update_runtime_deployment(storage: &Storage, project_id: Uuid, name: &str, patch: RuntimeDeploymentPatch<'_>) -> Result<RuntimeDeployment, ApiError>;
    pub async fn promote_runtime_deployment(storage: &Storage, project_id: Uuid, name: &str) -> Result<RuntimeDeployment, ApiError>;
    pub async fn delete_runtime_deployment(storage: &Storage, project_id: Uuid, name: &str) -> Result<(), ApiError>;
    pub async fn list_runtime_deployment_gateway_ids(storage: &Storage, deployment_id: Uuid) -> Result<Vec<String>, ApiError>;
    pub async fn list_runtime_deployment_apply_states(storage: &Storage, deployment_id: Uuid) -> Result<Vec<RuntimeDeploymentApplyState>, ApiError>;
    pub async fn record_runtime_deployment_apply_state(storage: &Storage, deployment_id: Uuid, gateway_id: &str, release_version: i64, content_hash: &str) -> Result<(), ApiError>;
    pub async fn assign_runtime_deployment_gateway(storage: &Storage, project_id: Uuid, deployment_id: Uuid, gateway_id: &str) -> Result<(), ApiError>;
    pub async fn unassign_runtime_deployment_gateway(storage: &Storage, project_id: Uuid, deployment_id: Uuid, gateway_id: &str) -> Result<(), ApiError>;
    pub async fn list_runtime_deployments_for_gateway(storage: &Storage, gateway_id: &str) -> Result<Vec<RuntimeDeployment>, ApiError>;
    pub async fn runtime_deployment_allows_remote_invocation(storage: &Storage, project_id: Uuid, gateway_id: &str) -> Result<bool, ApiError>;
    pub async fn create_runtime_distribution(storage: &Storage, input: NewRuntimeDistribution<'_>) -> Result<RuntimeDistribution, ApiError>;
    pub async fn list_runtime_distributions(storage: &Storage, project_id: Uuid) -> Result<Vec<RuntimeDistribution>, ApiError>;
    pub async fn revoke_runtime_distribution(storage: &Storage, project_id: Uuid, distribution_id: Uuid) -> Result<RuntimeDistribution, ApiError>;
    pub async fn authorize_runtime_distribution_gateway(storage: &Storage, public_id: &str, machine_id: &str, suggested_gateway_id: &str) -> Result<RuntimeDistributionGatewayAssignment, ApiError>;
    pub async fn create_project_runtime_release(storage: &Storage, input: NewProjectRuntimeRelease<'_>) -> Result<ProjectRuntimeRelease, ApiError>;
    pub async fn list_project_runtime_releases(storage: &Storage, project_id: Uuid) -> Result<Vec<ProjectRuntimeRelease>, ApiError>;
    pub async fn get_project_runtime_release(storage: &Storage, project_id: Uuid, version: i64) -> Result<ProjectRuntimeRelease, ApiError>;
    pub async fn activate_runtime_deployment_release(storage: &Storage, project_id: Uuid, deployment_name: &str, version: i64) -> Result<RuntimeDeployment, ApiError>;
    pub async fn activate_runtime_configuration_release(storage: &Storage, deployment_id: Uuid, release: NewProjectRuntimeRelease<'_>) -> Result<(), ApiError>;
    pub async fn activate_profile_runtime_release(storage: &Storage, profile_id: Uuid, profile_version_id: Uuid, deployment_id: Uuid, release: NewProjectRuntimeRelease<'_>) -> Result<i64, ApiError>;
    pub async fn create_guest_project(storage: &Storage, input: NewGuestProject<'_>) -> Result<(), ApiError>;
    pub async fn promote_guest_project(storage: &Storage, project_id: Uuid) -> Result<bool, ApiError>;
    pub async fn get_active_guest_project_for_gateway(storage: &Storage, gateway_id: &str) -> Result<Option<(ProjectWithBindings, DateTime<Utc>)>, ApiError>;
    pub async fn get_active_guest_project_by_project_id(storage: &Storage, project_id: Uuid) -> Result<Option<DateTime<Utc>>, ApiError>;
    pub async fn count_active_guest_projects(storage: &Storage) -> Result<i64, ApiError>;
    pub async fn prune_expired_guest_projects(storage: &Storage) -> Result<u64, ApiError>;
    pub async fn claim_guest_project(storage: &Storage, claim_token_hash: &[u8], owner_user_id: &str) -> Result<ProjectWithBindings, ApiError>;
    pub async fn repair_guest_project_gateway_ownership(storage: &Storage, project_id: Uuid, owner_user_id: &str) -> Result<(), ApiError>;
    pub async fn get_project_runtime_extension(storage: &Storage, project_id: Uuid) -> Result<Option<ProjectRuntimeExtension>, ApiError>;
    pub async fn set_project_runtime_extension(storage: &Storage, project_id: Uuid, extension_id: &str, enabled: bool, active_release_ref: Option<&str>, metadata: &Value) -> Result<ProjectRuntimeExtension, ApiError>;
    pub async fn delete_project_runtime_extension(storage: &Storage, project_id: Uuid) -> Result<(), ApiError>;
    pub async fn create_project_runtime_channel(storage: &Storage, input: NewProjectRuntimeChannel<'_>) -> Result<ProjectRuntimeChannel, ApiError>;
    pub async fn list_project_runtime_channels(storage: &Storage, project_id: Uuid) -> Result<Vec<ProjectRuntimeChannel>, ApiError>;
    pub async fn delete_project_runtime_channel(storage: &Storage, project_id: Uuid, channel_id: Uuid) -> Result<(), ApiError>;
    pub async fn runtime_channel_for_launch(storage: &Storage, project_id: Uuid, public_id: Uuid, launch_key_hash: &[u8]) -> Result<ProjectRuntimeChannel, ApiError>;
    pub async fn create_runtime_launch_session(storage: &Storage, id: Uuid, project_id: Uuid, channel_id: Uuid, token_hash: &[u8], expires_at: DateTime<Utc>) -> Result<(), ApiError>;
    pub async fn active_runtime_launch_project(storage: &Storage, token_hash: &[u8]) -> Result<Option<Uuid>, ApiError>;
    pub async fn create_project(storage: &Storage, project: NewProject<'_>) -> Result<ProjectWithBindings, ApiError>;
    pub async fn update_project(storage: &Storage, id: Uuid, patch: ProjectPatch<'_>) -> Result<ProjectWithBindings, ApiError>;
    pub async fn delete_project(storage: &Storage, id: Uuid) -> Result<(), ApiError>;
    pub async fn list_provider_connections(storage: &Storage, project_slug: &str) -> Result<Vec<ProviderConnection>, ApiError>;
    pub async fn upsert_provider_connection(storage: &Storage, project_slug: &str, connection: NewProviderConnection<'_>) -> Result<ProviderConnection, ApiError>;
    pub async fn get_provider_connection_secret(storage: &Storage, id: Uuid) -> Result<ProviderConnectionSecret, ApiError>;
    pub async fn get_provider_connection_secret_by_key(storage: &Storage, project_slug: &str, provider_key: &str) -> Result<ProviderConnectionSecret, ApiError>;
    pub async fn update_provider_connection_status(storage: &Storage, id: Uuid, status: &str) -> Result<ProviderConnection, ApiError>;
    pub async fn project_provider_is_assigned(storage: &Storage, project_id: Uuid, provider_key: &str) -> Result<bool, ApiError>;
    pub async fn list_projects_for_gateway(storage: &Storage, gateway_id: &str) -> Result<Vec<(Uuid, String)>, ApiError>;
    pub async fn list_projects_for_provider_key(storage: &Storage, provider_key: &str) -> Result<Vec<(Uuid, String)>, ApiError>;
    pub async fn list_project_profile_provider_resources(storage: &Storage, project_id: Uuid) -> Result<Vec<(String, String)>, ApiError>;
    pub async fn list_archived_project_agent_sources(storage: &Storage, project_id: Uuid) -> Result<Vec<ArchivedProjectAgentSource>, ApiError>;
    pub async fn archive_legacy_discovered_provider(storage: &Storage, project_id: Uuid, runtime_provider_key: &str) -> Result<u64, ApiError>;
    pub async fn restore_project_profile(storage: &Storage, project_id: Uuid, profile_id: Uuid) -> Result<AgentProfile, ApiError>;
    pub async fn find_project_profile_by_provider_resource(storage: &Storage, project_id: Uuid, gateway_id: &str, provider_key: &str, agent_id: &str) -> Result<Option<(Uuid, bool, Uuid)>, ApiError>;
    pub async fn refresh_discovered_binding_record(storage: &Storage, binding_id: Uuid, gateway_id: &str, agent_name: &str) -> Result<(), ApiError>;
    pub async fn unassign_project_provider(storage: &Storage, project_id: Uuid, provider_key: &str) -> Result<(), ApiError>;
    pub async fn assign_project_binding(storage: &Storage, project_id: Uuid, binding_id: Uuid) -> Result<(), ApiError>;
    pub async fn attach_project_binding(storage: &Storage, project_id: Uuid, binding_id: Uuid) -> Result<(), ApiError>;
    pub async fn list_profiles(storage: &Storage) -> Result<Vec<AgentProfile>, ApiError>;
    pub async fn list_project_profiles(storage: &Storage, project_id: Uuid) -> Result<Vec<AgentProfile>, ApiError>;
    pub async fn get_profile(storage: &Storage, id: Uuid) -> Result<AgentProfile, ApiError>;
    pub async fn get_project_profile(storage: &Storage, project_id: Uuid, id: Uuid) -> Result<AgentProfile, ApiError>;
    pub async fn create_profile(storage: &Storage, id: Uuid, project_id: Uuid, slug: &str, name: &str, description: Option<&str>) -> Result<AgentProfile, ApiError>;
    pub async fn update_profile(storage: &Storage, id: Uuid, patch: ProfilePatch<'_>) -> Result<AgentProfile, ApiError>;
    pub async fn delete_profile(storage: &Storage, id: Uuid) -> Result<(), ApiError>;
    pub async fn archive_project_profile(storage: &Storage, project_id: Uuid, profile_id: Uuid) -> Result<(), ApiError>;
    pub async fn create_profile_version(storage: &Storage, profile_id: Uuid, input: NewProfileVersion<'_>) -> Result<AgentProfileVersion, ApiError>;
    pub async fn list_profile_versions(storage: &Storage, profile_id: Uuid) -> Result<Vec<AgentProfileVersion>, ApiError>;
    pub async fn get_profile_version(storage: &Storage, profile_id: Uuid, version_id: Uuid) -> Result<AgentProfileVersion, ApiError>;
    pub async fn list_profile_capabilities(storage: &Storage, version_id: Uuid) -> Result<Vec<AgentProfileCapability>, ApiError>;
    pub async fn list_profile_rollout(storage: &Storage, profile_id: Uuid) -> Result<Vec<AgentProfileRollout>, ApiError>;
    pub async fn set_profile_rollout(storage: &Storage, profile_id: Uuid, allocations: &[(Uuid, i32)]) -> Result<Vec<AgentProfileRollout>, ApiError>;
    pub async fn archive_profile_version(storage: &Storage, profile_id: Uuid, version_id: Uuid) -> Result<AgentProfileVersion, ApiError>;
    pub async fn resolve_profile_route(storage: &Storage, project_id: Uuid, model: &str, capability_kind: &str, selection_key: Option<&str>, version_id: Option<Uuid>) -> Result<ProfileRoute, ApiError>;
    pub async fn list_public_agents(storage: &Storage, project_id: Uuid, allowed_profile_ids: Option<&[Uuid]>) -> Result<Vec<PublicAgent>, ApiError>;
    pub async fn list_bindings(storage: &Storage) -> Result<Vec<AgentBinding>, ApiError>;
    pub async fn get_binding(storage: &Storage, id: Uuid) -> Result<AgentBinding, ApiError>;
    pub async fn create_binding(storage: &Storage, id: Uuid, profile_id: Uuid, provider: &str, gateway_id: &str, agent_id: &str, config: &Value) -> Result<AgentBinding, ApiError>;
    pub async fn ensure_discovered_binding(storage: &Storage, input: NewDiscoveredBinding<'_>) -> Result<Uuid, ApiError>;
    pub async fn update_binding(storage: &Storage, id: Uuid, gateway_id: Option<&str>, agent_id: Option<&str>, config: Option<&Value>) -> Result<AgentBinding, ApiError>;
    pub async fn delete_binding(storage: &Storage, id: Uuid) -> Result<(), ApiError>;
    pub async fn list_endpoints(storage: &Storage) -> Result<Vec<AgentEndpoint>, ApiError>;
    pub async fn list_enabled_endpoints(storage: &Storage) -> Result<Vec<AgentEndpoint>, ApiError>;
    pub async fn list_enabled_endpoints_for_project(storage: &Storage, project_slug: &str) -> Result<Vec<AgentEndpoint>, ApiError>;
    pub async fn list_enabled_endpoints_for_project_id(storage: &Storage, project_id: Uuid) -> Result<Vec<AgentEndpoint>, ApiError>;
    pub async fn get_endpoint(storage: &Storage, id: Uuid) -> Result<AgentEndpoint, ApiError>;
    pub async fn create_endpoint(storage: &Storage, endpoint: NewEndpoint<'_>) -> Result<AgentEndpoint, ApiError>;
    pub async fn update_endpoint(storage: &Storage, id: Uuid, patch: EndpointPatch<'_>) -> Result<AgentEndpoint, ApiError>;
    pub async fn delete_endpoint(storage: &Storage, id: Uuid) -> Result<(), ApiError>;
    pub async fn list_api_keys(storage: &Storage) -> Result<Vec<ApiKeyRecord>, ApiError>;
    pub async fn create_api_key(storage: &Storage, input: NewApiKey<'_>) -> Result<ApiKeyRecord, ApiError>;
    pub async fn get_api_key(storage: &Storage, id: Uuid) -> Result<ApiKeyRecord, ApiError>;
    pub async fn update_api_key(storage: &Storage, id: Uuid, patch: ApiKeyPatch<'_>) -> Result<ApiKeyRecord, ApiError>;
    pub async fn revoke_api_key(storage: &Storage, id: Uuid) -> Result<ApiKeyRecord, ApiError>;
    pub async fn delete_api_key(storage: &Storage, id: Uuid) -> Result<(), ApiError>;
    pub async fn create_realtime_session(storage: &Storage, id: Uuid, project_id: Uuid, profile_id: Uuid, api_key_id: Option<Uuid>, token_hash: &[u8], expires_at: DateTime<Utc>) -> Result<RealtimeSession, ApiError>;
    pub async fn active_realtime_session_by_hash(storage: &Storage, project_id: Uuid, token_hash: &[u8]) -> Result<RealtimeSession, ApiError>;
    pub async fn active_api_key_by_hash(storage: &Storage, key_hash: &[u8]) -> Result<ApiKeyRecord, ApiError>;
    pub async fn active_api_key_by_hash_optional(storage: &Storage, key_hash: &[u8]) -> Result<Option<ApiKeyRecord>, ApiError>;
    pub async fn resolve_endpoint_route(storage: &Storage, id_or_slug: &str) -> Result<EndpointRoute, ApiError>;
    pub async fn resolve_project_endpoint_route(storage: &Storage, project_slug: &str, id_or_slug: &str) -> Result<EndpointRoute, ApiError>;
    pub async fn resolve_project_model_route(storage: &Storage, project_id: Uuid, model: &str) -> Result<EndpointRoute, ApiError>;
    pub async fn upsert_agent_gateway_machine(storage: &Storage, machine_id: &str, public_key: &str) -> Result<(), ApiError>;
    pub async fn get_agent_gateway_authorization_for_machine(storage: &Storage, machine_id: &str) -> Result<Option<AgentGatewayAuthorization>, ApiError>;
    pub async fn get_agent_gateway_authorization(storage: &Storage, gateway_id: &str) -> Result<AgentGatewayAuthorization, ApiError>;
    pub async fn create_agent_gateway_authorization(storage: &Storage, input: NewAgentGatewayAuthorization<'_>) -> Result<AgentGatewayAuthorization, ApiError>;
    pub async fn rotate_agent_gateway_authorization(storage: &Storage, input: RotatedAgentGatewayAuthorization<'_>) -> Result<AgentGatewayAuthorization, ApiError>;
    pub async fn claim_agent_gateway_authorization_owner(storage: &Storage, gateway_id: &str, owner_user_id: &str) -> Result<AgentGatewayAuthorization, ApiError>;
    pub async fn authenticate_agent_gateway_device_token(storage: &Storage, token_hash: &[u8]) -> Result<String, ApiError>;
    pub async fn revoke_agent_gateway_authorization(storage: &Storage, gateway_id: &str) -> Result<AgentGatewayAuthorization, ApiError>;
    pub async fn consume_agent_gateway_machine_enrollment(storage: &Storage, token_hash: &[u8], gateway_id: &str) -> Result<AgentGatewayEnrollmentAssignment, ApiError>;
    pub async fn create_or_get_agent_gateway_pairing(storage: &Storage, machine_id: &str, expires_at: DateTime<Utc>) -> Result<AgentGatewayPairingRequest, ApiError>;
    pub async fn get_agent_gateway_pairing(storage: &Storage, id: Uuid) -> Result<AgentGatewayPairingRequest, ApiError>;
    pub async fn consume_agent_gateway_pairing(storage: &Storage, id: Uuid, machine_id: &str) -> Result<AgentGatewayPairingRequest, ApiError>;
    pub async fn consume_approved_agent_gateway_pairing_for_machine(storage: &Storage, machine_id: &str) -> Result<Option<AgentGatewayPairingRequest>, ApiError>;
    pub async fn list_agent_gateway_pairings(storage: &Storage) -> Result<Vec<AgentGatewayPairingRequest>, ApiError>;
    pub async fn resolve_agent_gateway_pairing(storage: &Storage, id: Uuid, status: &str, owner_user_id: Option<&str>) -> Result<AgentGatewayPairingRequest, ApiError>;
    pub async fn register_agent_gateway_credential(storage: &Storage, gateway_id: &str, owner_user_id: Option<&str>, credential_prefix: &str, credential_hash: &[u8]) -> Result<AgentGatewayRegistration, ApiError>;
    pub async fn create_agent_gateway_enrollment(storage: &Storage, input: NewAgentGatewayEnrollment<'_>) -> Result<(), ApiError>;
    pub async fn consume_agent_gateway_enrollment(storage: &Storage, token_hash: &[u8], gateway_id: &str, credential_prefix: &str, credential_hash: &[u8]) -> Result<AgentGatewayRegistration, ApiError>;
    pub async fn authenticate_agent_gateway_credential(storage: &Storage, credential_hash: &[u8]) -> Result<String, ApiError>;
    pub async fn revoke_agent_gateway_credential(storage: &Storage, gateway_id: &str) -> Result<AgentGatewayCredential, ApiError>;
    pub async fn open_agent_gateway_session(storage: &Storage, gateway_id: &str, resume_session_id: Option<Uuid>, agents: &Value, metadata: &Value) -> Result<(Uuid, bool), ApiError>;
    pub async fn touch_agent_gateway_session(storage: &Storage, session_id: Uuid) -> Result<(), ApiError>;
    pub async fn close_agent_gateway_session(storage: &Storage, session_id: Uuid) -> Result<(), ApiError>;
    pub async fn list_agent_gateway_sessions(storage: &Storage) -> Result<Vec<AgentGatewaySession>, ApiError>;
    pub async fn list_available_agents(storage: &Storage) -> Result<Vec<AvailableAgent>, ApiError>;
    pub async fn create_trace(storage: &Storage, trace: NewTrace<'_>) -> Result<Uuid, ApiError>;
    pub async fn create_uploaded_runtime_trace(storage: &Storage, trace: NewUploadedRuntimeTrace<'_>) -> Result<bool, ApiError>;
    pub async fn create_trace_span(storage: &Storage, span: NewTraceSpan<'_>) -> Result<Uuid, ApiError>;
    pub async fn create_trace_span_with_id(storage: &Storage, span_id: Uuid, span: NewTraceSpan<'_>) -> Result<Uuid, ApiError>;
    pub async fn upsert_runtime_trace_observation(storage: &Storage, observation: RuntimeTraceObservation<'_>) -> Result<(), ApiError>;
    pub async fn update_trace_generation(storage: &Storage, span_id: Uuid, completion_start_ms: Option<i64>, usage: Option<&Value>) -> Result<(), ApiError>;
    pub async fn get_runtime_trace_target(storage: &Storage, request_id: Uuid) -> Result<Option<RuntimeTraceTarget>, ApiError>;
    pub async fn get_runtime_trace_gateway_id(storage: &Storage, request_id: Uuid) -> Result<Option<String>, ApiError>;
    pub async fn update_trace_runtime_identity(storage: &Storage, request_id: Uuid, provider_key: &str, capability_kind: &str, model: Option<&str>) -> Result<(), ApiError>;
    pub async fn merge_trace_runtime_generation(storage: &Storage, request_id: Uuid, completion_start_ms: Option<i64>, input_tokens: Option<i64>, output_tokens: Option<i64>) -> Result<(), ApiError>;
    pub async fn update_trace_runtime_io_summaries(storage: &Storage, request_id: Uuid, input_summary: Option<&Value>, input_truncated: bool, output_summary: Option<&Value>, output_truncated: bool) -> Result<(), ApiError>;
    pub async fn complete_trace_span(storage: &Storage, span_id: Uuid, status: &str, duration_ms: i64, output_summary: Option<&Value>, error: Option<&str>) -> Result<(), ApiError>;
    pub async fn upsert_trace_score(storage: &Storage, score: NewTraceScore<'_>) -> Result<TraceScore, ApiError>;
    pub async fn complete_trace(storage: &Storage, request_id: Uuid, status: &str, latency_ms: i64, response: Option<&Value>, error: Option<&str>) -> Result<(), ApiError>;
    pub async fn list_traces(storage: &Storage, options: TraceListOptions<'_>) -> Result<Vec<EndpointTrace>, ApiError>;
    pub async fn get_trace_project_id(storage: &Storage, trace_id: Uuid) -> Result<Option<Uuid>, ApiError>;
    pub async fn get_trace_identity(storage: &Storage, trace_id: Uuid) -> Result<TraceIdentity, ApiError>;
    pub async fn list_trace_spans(storage: &Storage, trace_id: Uuid) -> Result<Vec<TraceSpan>, ApiError>;
    pub async fn list_trace_scores(storage: &Storage, trace_id: Uuid) -> Result<Vec<TraceScore>, ApiError>;
    pub async fn trace_feedback_target(storage: &Storage, project_id: Uuid, request_id: Uuid) -> Result<TraceFeedbackTarget, ApiError>;
}

pub async fn refresh_discovered_binding(
    storage: &Storage,
    binding_id: Uuid,
    gateway_id: &str,
    agent_name: &str,
    discovered_persona: Option<&Value>,
) -> Result<(), ApiError> {
    refresh_discovered_binding_record(storage, binding_id, gateway_id, agent_name).await?;

    let binding = get_binding(storage, binding_id).await?;
    let discovered_source = binding
        .config
        .get("source")
        .and_then(Value::as_str)
        .is_some_and(|source| source.ends_with("-discovery"));
    if !discovered_source {
        return Ok(());
    }

    let profile = get_profile(storage, binding.profile_id).await?;
    let Some(active_version_id) = profile.active_version_id else {
        return Ok(());
    };
    let active_version = get_profile_version(storage, profile.id, active_version_id).await?;
    let provider_key = binding
        .config
        .get("providerKey")
        .and_then(Value::as_str)
        .unwrap_or(&binding.provider);
    let managed_source_matches = active_version
        .source
        .get("managed")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        && active_version
            .source
            .get("providerKey")
            .and_then(Value::as_str)
            == Some(provider_key)
        && active_version
            .source
            .get("resourceId")
            .and_then(Value::as_str)
            == Some(binding.agent_id.as_str());
    if !managed_source_matches {
        return Ok(());
    }

    let mut source = active_version.source.clone();
    let mut changed = source.get("gatewayId").and_then(Value::as_str) != Some(gateway_id);
    if changed {
        source.as_object_mut().ok_or(ApiError::Internal)?.insert(
            "gatewayId".to_string(),
            Value::String(gateway_id.to_string()),
        );
    }
    let mut persona = active_version.persona.clone();
    let active_prompt = persona
        .get("systemPrompt")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|prompt| !prompt.is_empty());
    let discovered_prompt = discovered_persona
        .and_then(|value| value.get("systemPrompt"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|prompt| !prompt.is_empty());
    if active_prompt.is_none() && discovered_prompt.is_some() {
        persona = discovered_persona.cloned().ok_or(ApiError::Internal)?;
        changed = true;
    }

    let capabilities = list_profile_capabilities(storage, active_version_id).await?;
    let capability_drafts = capabilities
        .into_iter()
        .map(|capability| {
            let mut config = capability.config;
            let matches_binding = capability.provider_key == provider_key
                && capability.resource_id.as_deref() == Some(binding.agent_id.as_str());
            if matches_binding
                && config.get("gatewayId").and_then(Value::as_str) != Some(gateway_id)
            {
                config.as_object_mut().ok_or(ApiError::Internal)?.insert(
                    "gatewayId".to_string(),
                    Value::String(gateway_id.to_string()),
                );
                changed = true;
            }
            Ok(ProfileCapabilityDraft {
                kind: capability.kind,
                provider_type: capability.provider_type,
                provider_key: capability.provider_key,
                resource_id: capability.resource_id,
                config,
                input_schema: capability.input_schema,
                output_schema: capability.output_schema,
            })
        })
        .collect::<Result<Vec<_>, ApiError>>()?;
    if !changed {
        return Ok(());
    }

    let version = create_profile_version(
        storage,
        profile.id,
        NewProfileVersion {
            persona: &persona,
            runtime: &active_version.runtime,
            presentation: &active_version.presentation,
            source: &source,
            capabilities: &capability_drafts,
            change_summary: Some("Reconnected discovered agent"),
        },
    )
    .await?;
    set_profile_rollout(storage, profile.id, &[(version.id, 10_000)]).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::Duration as ChronoDuration;
    use serde_json::json;

    use super::*;

    async fn sqlite_storage() -> (Storage, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!("vifu-storage-{}.sqlite", Uuid::new_v4()));
        let storage = connect(&format!("sqlite://{}", path.display()), 5)
            .await
            .expect("SQLite should connect");
        migrate(&storage).await.expect("SQLite should migrate");
        (storage, path)
    }

    async fn close_and_remove(storage: Storage, path: &std::path::Path) {
        match storage {
            Storage::Postgres(pool) => pool.close().await,
            Storage::Sqlite(pool) => pool.close().await,
        }
        std::fs::remove_file(path).expect("SQLite database should be removable");
    }

    #[cfg(unix)]
    #[test]
    fn sqlite_state_files_are_private() {
        use std::os::unix::fs::PermissionsExt;

        let directory = std::env::temp_dir().join(format!("vifu-private-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&directory).expect("temporary directory should be created");
        let path = directory.join("vifu.sqlite");
        let files = [
            path.clone(),
            sqlite_sidecar_path(&path, "-wal"),
            sqlite_sidecar_path(&path, "-shm"),
        ];
        for file in &files {
            std::fs::write(file, b"test").expect("SQLite state file should be created");
        }

        protect_sqlite_files(&format!("sqlite://{}", path.display()))
            .expect("SQLite state files should be protected");

        let modes = files.map(|file| {
            std::fs::metadata(file)
                .expect("SQLite state file should exist")
                .permissions()
                .mode()
                & 0o777
        });
        assert_eq!(modes, [0o600; 3]);
        std::fs::remove_dir_all(directory).expect("temporary directory should be removable");
    }

    async fn primary_deployment_id(storage: &Storage, project_id: Uuid) -> Uuid {
        list_runtime_deployments(storage, project_id)
            .await
            .expect("runtime deployments should list")
            .into_iter()
            .find(|deployment| deployment.is_primary)
            .expect("project should have a primary deployment")
            .id
    }

    #[tokio::test]
    async fn sqlite_profile_rollback_reuses_the_existing_runtime_release() {
        let (storage, path) = sqlite_storage().await;
        let project_id = Uuid::new_v4();
        let profile_id = Uuid::new_v4();
        create_project(
            &storage,
            NewProject {
                id: project_id,
                owner_user_id: None,
                slug: "profile-rollback",
                name: "Profile rollback",
                description: None,
                gateway_id: "gateway-rollback",
                binding_ids: &[],
            },
        )
        .await
        .expect("project should be created");
        create_profile(
            &storage,
            profile_id,
            project_id,
            "researcher",
            "Researcher",
            None,
        )
        .await
        .expect("profile should be created");
        let empty = json!({});
        let first = create_profile_version(
            &storage,
            profile_id,
            NewProfileVersion {
                persona: &json!({ "systemPrompt": "First prompt" }),
                runtime: &empty,
                presentation: &empty,
                source: &empty,
                capabilities: &[],
                change_summary: None,
            },
        )
        .await
        .expect("first profile version should be created");
        let second = create_profile_version(
            &storage,
            profile_id,
            NewProfileVersion {
                persona: &json!({ "systemPrompt": "Second prompt" }),
                runtime: &empty,
                presentation: &empty,
                source: &empty,
                capabilities: &[],
                change_summary: None,
            },
        )
        .await
        .expect("second profile version should be created");
        let deployment_id = primary_deployment_id(&storage, project_id).await;
        let first_manifest =
            json!({ "schemaVersion": 1, "projectId": "profile-rollback", "prompt": "first" });
        let second_manifest =
            json!({ "schemaVersion": 1, "projectId": "profile-rollback", "prompt": "second" });

        let first_release = activate_profile_runtime_release(
            &storage,
            profile_id,
            first.id,
            deployment_id,
            NewProjectRuntimeRelease {
                id: Uuid::new_v4(),
                project_id,
                version: 1,
                content_hash: "sha256:first",
                manifest: &first_manifest,
                created_by: None,
            },
        )
        .await
        .expect("first version should be activated");
        assert_eq!(first_release, 1);
        let second_release = activate_profile_runtime_release(
            &storage,
            profile_id,
            second.id,
            deployment_id,
            NewProjectRuntimeRelease {
                id: Uuid::new_v4(),
                project_id,
                version: 2,
                content_hash: "sha256:second",
                manifest: &second_manifest,
                created_by: None,
            },
        )
        .await
        .expect("second version should be activated");
        assert_eq!(second_release, 2);
        let restored_release = activate_profile_runtime_release(
            &storage,
            profile_id,
            first.id,
            deployment_id,
            NewProjectRuntimeRelease {
                id: Uuid::new_v4(),
                project_id,
                version: 3,
                content_hash: "sha256:first",
                manifest: &first_manifest,
                created_by: None,
            },
        )
        .await
        .expect("the existing first release should be reusable");

        assert_eq!(restored_release, 1);
        assert_eq!(
            get_profile(&storage, profile_id)
                .await
                .expect("profile should exist")
                .active_version_id,
            Some(first.id)
        );
        assert_eq!(
            get_runtime_deployment(&storage, project_id, "development")
                .await
                .expect("deployment should exist")
                .active_release_version,
            Some(1)
        );
        assert_eq!(
            list_project_runtime_releases(&storage, project_id)
                .await
                .expect("releases should list")
                .len(),
            2
        );

        close_and_remove(storage, &path).await;
    }

    #[tokio::test]
    async fn sqlite_refresh_discovered_binding_moves_profile_route_to_new_gateway() {
        let (storage, path) = sqlite_storage().await;
        let project_id = Uuid::new_v4();
        create_project(
            &storage,
            NewProject {
                id: project_id,
                owner_user_id: None,
                slug: "discovered-agent-reconnect",
                name: "Discovered agent reconnect",
                description: None,
                gateway_id: "gateway-old",
                binding_ids: &[],
            },
        )
        .await
        .expect("project should be created");
        let binding_id = ensure_discovered_binding(
            &storage,
            NewDiscoveredBinding {
                project_id,
                gateway_id: "gateway-old",
                agent_id: "companion-demo",
                agent_name: "Android Local Companion",
                provider_key: "android-local-model",
                runtime_provider_key: "android-local-model",
                provider_type: "vifu-runtime",
                persona: serde_json::json!({
                    "systemPrompt": "Help the player understand the garden."
                }),
            },
        )
        .await
        .expect("discovered binding should be created");
        let profile = get_profile(
            &storage,
            get_binding(&storage, binding_id)
                .await
                .expect("binding should exist")
                .profile_id,
        )
        .await
        .expect("profile should exist");
        let version = get_profile_version(
            &storage,
            profile.id,
            profile
                .active_version_id
                .expect("profile should have a live version"),
        )
        .await
        .expect("profile version should exist");
        assert_eq!(
            version.persona["systemPrompt"],
            "Help the player understand the garden."
        );

        refresh_discovered_binding(
            &storage,
            binding_id,
            "gateway-new",
            "Android Local Companion",
            None,
        )
        .await
        .expect("discovered binding should refresh");
        refresh_discovered_binding(
            &storage,
            binding_id,
            "gateway-new",
            "Android Local Companion",
            None,
        )
        .await
        .expect("repeated refresh should be idempotent");

        let route = resolve_profile_route(&storage, project_id, &profile.slug, "chat", None, None)
            .await
            .expect("profile route should resolve");
        assert_eq!(
            (
                route.source.get("gatewayId").and_then(Value::as_str),
                route
                    .capability_config
                    .get("gatewayId")
                    .and_then(Value::as_str),
            ),
            (Some("gateway-new"), Some("gateway-new"))
        );
        assert_eq!(
            list_profile_versions(&storage, profile.id)
                .await
                .expect("profile versions should list")
                .len(),
            2
        );

        let legacy_binding_id = ensure_discovered_binding(
            &storage,
            NewDiscoveredBinding {
                project_id,
                gateway_id: "gateway-legacy",
                agent_id: "legacy-researcher",
                agent_name: "Legacy Researcher",
                provider_key: "legacy-local-model",
                runtime_provider_key: "legacy-local-model",
                provider_type: "vifu-runtime",
                persona: json!({ "files": {} }),
            },
        )
        .await
        .expect("legacy discovered binding should be created");
        refresh_discovered_binding(
            &storage,
            legacy_binding_id,
            "gateway-legacy",
            "Legacy Researcher",
            Some(&json!({ "systemPrompt": "Use only the supplied sources." })),
        )
        .await
        .expect("legacy Agent prompt should be backfilled");
        let legacy_profile = get_profile(
            &storage,
            get_binding(&storage, legacy_binding_id)
                .await
                .expect("legacy binding should exist")
                .profile_id,
        )
        .await
        .expect("legacy profile should exist");
        let legacy_version = get_profile_version(
            &storage,
            legacy_profile.id,
            legacy_profile
                .active_version_id
                .expect("legacy profile should have a live version"),
        )
        .await
        .expect("backfilled version should exist");
        assert_eq!(
            legacy_version.persona["systemPrompt"],
            "Use only the supplied sources."
        );

        close_and_remove(storage, &path).await;
    }

    #[tokio::test]
    async fn sqlite_project_gateway_update_moves_the_primary_deployment_assignment() {
        let (storage, path) = sqlite_storage().await;
        let project_id = Uuid::new_v4();
        create_project(
            &storage,
            NewProject {
                id: project_id,
                owner_user_id: None,
                slug: "gateway-move",
                name: "Gateway move",
                description: None,
                gateway_id: "gateway-old",
                binding_ids: &[],
            },
        )
        .await
        .expect("project should be created");
        let deployment_id = primary_deployment_id(&storage, project_id).await;

        update_project(
            &storage,
            project_id,
            ProjectPatch {
                slug: None,
                name: None,
                description_changed: false,
                description: None,
                gateway_id: Some("gateway-new"),
                enabled: None,
                binding_ids: None,
            },
        )
        .await
        .expect("project Gateway should update");

        assert_eq!(
            list_runtime_deployment_gateway_ids(&storage, deployment_id)
                .await
                .expect("deployment Gateways should list"),
            vec!["gateway-new"]
        );
        close_and_remove(storage, &path).await;
    }

    #[tokio::test]
    async fn sqlite_trace_profile_scope_excludes_unprofiled_and_other_profile_traces() {
        let (storage, path) = sqlite_storage().await;
        let project_id = Uuid::new_v4();
        create_project(
            &storage,
            NewProject {
                id: project_id,
                owner_user_id: None,
                slug: "trace-scope",
                name: "Trace scope",
                description: None,
                gateway_id: "gateway-trace-scope",
                binding_ids: &[],
            },
        )
        .await
        .expect("project should be created");
        let allowed_profile_id = Uuid::new_v4();
        let other_profile_id = Uuid::new_v4();
        create_profile(
            &storage,
            allowed_profile_id,
            project_id,
            "allowed-agent",
            "Allowed agent",
            None,
        )
        .await
        .expect("allowed profile should be created");
        create_profile(
            &storage,
            other_profile_id,
            project_id,
            "other-agent",
            "Other agent",
            None,
        )
        .await
        .expect("other profile should be created");

        let allowed_trace_id = create_trace(
            &storage,
            NewTrace {
                request_id: Uuid::new_v4(),
                endpoint_id: None,
                project_id: Some(project_id),
                gateway_session_id: None,
                profile_id: Some(allowed_profile_id),
                profile_version_id: None,
                operation: "runtime.invoke",
                provider_key: None,
                capability_kind: Some("chat"),
                selection_key: None,
                request: &json!({}),
            },
        )
        .await
        .expect("allowed trace should be created");
        for profile_id in [Some(other_profile_id), None] {
            create_trace(
                &storage,
                NewTrace {
                    request_id: Uuid::new_v4(),
                    endpoint_id: None,
                    project_id: Some(project_id),
                    gateway_session_id: None,
                    profile_id,
                    profile_version_id: None,
                    operation: "runtime.invoke",
                    provider_key: None,
                    capability_kind: Some("chat"),
                    selection_key: None,
                    request: &json!({}),
                },
            )
            .await
            .expect("out-of-scope trace should be created");
        }

        let scoped = list_traces(
            &storage,
            TraceListOptions {
                endpoint_id: None,
                project_id: Some(project_id),
                request_id: None,
                trace_id: None,
                allowed_profile_ids: Some(&[allowed_profile_id]),
                created_from: None,
                created_before: None,
                cursor: None,
                limit: 10,
            },
        )
        .await
        .expect("scoped traces should list");
        assert_eq!(scoped.len(), 1);
        assert_eq!(scoped[0].id, allowed_trace_id);
        assert!(list_traces(
            &storage,
            TraceListOptions {
                endpoint_id: None,
                project_id: Some(project_id),
                request_id: None,
                trace_id: None,
                allowed_profile_ids: Some(&[]),
                created_from: None,
                created_before: None,
                cursor: None,
                limit: 10,
            },
        )
        .await
        .expect("empty scope should list")
        .is_empty());
        assert_eq!(
            get_trace_identity(&storage, allowed_trace_id)
                .await
                .expect("trace identity should resolve"),
            TraceIdentity {
                project_id: Some(project_id),
                profile_id: Some(allowed_profile_id),
            }
        );

        close_and_remove(storage, &path).await;
    }

    #[tokio::test]
    async fn sqlite_trace_listing_attributes_a_named_gateway_session() {
        let (storage, path) = sqlite_storage().await;
        let project_id = Uuid::new_v4();
        create_project(
            &storage,
            NewProject {
                id: project_id,
                owner_user_id: None,
                slug: "named-gateway-trace",
                name: "Named Gateway trace",
                description: None,
                gateway_id: "gateway-kitchen-light",
                binding_ids: &[],
            },
        )
        .await
        .expect("project should be created");
        let (gateway_session_id, _) = open_agent_gateway_session(
            &storage,
            "gateway-kitchen-light",
            None,
            &json!([]),
            &json!({
                "name": "Kitchen light",
                "kind": "light",
                "device": { "manufacturer": "Example" },
            }),
        )
        .await
        .expect("Gateway session should open");
        create_uploaded_runtime_trace(
            &storage,
            NewUploadedRuntimeTrace {
                id: Uuid::new_v4(),
                request_id: Uuid::new_v4(),
                project_id,
                gateway_session_id: Some(gateway_session_id),
                profile_id: None,
                profile_version_id: None,
                operation: "runtime.invoke",
                provider_key: Some("light-provider"),
                capability_kind: Some("control"),
                status: "completed",
                latency_ms: 1,
                request: &json!({ "agent": "light-agent" }),
                created_at: Utc::now(),
            },
        )
        .await
        .expect("trace should be created");

        let traces = list_traces(
            &storage,
            TraceListOptions {
                endpoint_id: None,
                project_id: Some(project_id),
                request_id: None,
                trace_id: None,
                allowed_profile_ids: None,
                created_from: None,
                created_before: None,
                cursor: None,
                limit: 10,
            },
        )
        .await
        .expect("trace should list");

        assert_eq!(traces[0].gateway_name.as_deref(), Some("Kitchen light"));
        close_and_remove(storage, &path).await;
    }

    #[tokio::test]
    async fn sqlite_trace_listing_uses_stable_cursor_and_date_window() {
        let (storage, path) = sqlite_storage().await;
        let project_id = Uuid::new_v4();
        create_project(
            &storage,
            NewProject {
                id: project_id,
                owner_user_id: None,
                slug: "trace-pages",
                name: "Trace pages",
                description: None,
                gateway_id: "gateway-trace-pages",
                binding_ids: &[],
            },
        )
        .await
        .expect("project should be created");

        let window_start = Utc::now() - ChronoDuration::hours(2);
        let shared_time = window_start + ChronoDuration::minutes(30);
        let window_end = window_start + ChronoDuration::hours(1);
        let older_id = Uuid::parse_str("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa1").unwrap();
        let shared_low_id = Uuid::parse_str("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa2").unwrap();
        let shared_high_id = Uuid::parse_str("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa3").unwrap();
        let newer_id = Uuid::parse_str("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa4").unwrap();
        for (id, created_at) in [
            (older_id, window_start - ChronoDuration::seconds(1)),
            (shared_low_id, shared_time),
            (shared_high_id, shared_time),
            (newer_id, window_end - ChronoDuration::seconds(1)),
        ] {
            create_uploaded_runtime_trace(
                &storage,
                NewUploadedRuntimeTrace {
                    id,
                    request_id: Uuid::new_v4(),
                    project_id,
                    gateway_session_id: None,
                    profile_id: None,
                    profile_version_id: None,
                    operation: "runtime.invoke",
                    provider_key: Some("local-provider"),
                    capability_kind: Some("chat"),
                    status: "completed",
                    latency_ms: 1,
                    request: &json!({}),
                    created_at,
                },
            )
            .await
            .expect("uploaded trace should be created");
        }

        let first_page = list_traces(
            &storage,
            TraceListOptions {
                endpoint_id: None,
                project_id: Some(project_id),
                request_id: None,
                trace_id: None,
                allowed_profile_ids: None,
                created_from: Some(window_start),
                created_before: Some(window_end),
                cursor: None,
                limit: 2,
            },
        )
        .await
        .expect("first trace page should list");
        assert_eq!(
            first_page.iter().map(|trace| trace.id).collect::<Vec<_>>(),
            vec![newer_id, shared_high_id]
        );

        let second_page = list_traces(
            &storage,
            TraceListOptions {
                endpoint_id: None,
                project_id: Some(project_id),
                request_id: None,
                trace_id: None,
                allowed_profile_ids: None,
                created_from: Some(window_start),
                created_before: Some(window_end),
                cursor: Some(TraceCursor {
                    created_at: first_page[1].created_at,
                    trace_id: first_page[1].id,
                }),
                limit: 2,
            },
        )
        .await
        .expect("second trace page should list");
        assert_eq!(
            second_page.iter().map(|trace| trace.id).collect::<Vec<_>>(),
            vec![shared_low_id]
        );

        close_and_remove(storage, &path).await;
    }

    async fn sqlite_trace_with_root(storage: &Storage, slug: &str) -> (Uuid, Uuid, Uuid) {
        let project_id = Uuid::new_v4();
        create_project(
            storage,
            NewProject {
                id: project_id,
                owner_user_id: None,
                slug,
                name: "Trace observation identity",
                description: None,
                gateway_id: "gateway-trace-observation-identity",
                binding_ids: &[],
            },
        )
        .await
        .expect("project should be created");
        let request_id = Uuid::new_v4();
        let trace_id = create_trace(
            storage,
            NewTrace {
                request_id,
                endpoint_id: None,
                project_id: Some(project_id),
                gateway_session_id: None,
                profile_id: None,
                profile_version_id: None,
                operation: "runtime.invoke",
                provider_key: Some("local-provider"),
                capability_kind: Some("chat"),
                selection_key: None,
                request: &json!({}),
            },
        )
        .await
        .expect("trace should be created");
        let root_id = request_id;
        create_trace_span_with_id(
            storage,
            root_id,
            NewTraceSpan {
                trace_id,
                parent_span_id: None,
                name: "runtime.invoke",
                kind: "agent_gateway",
                observation_type: "generation",
                provider_key: Some("local-provider"),
                capability_kind: Some("chat"),
                model: Some("local-model"),
                model_parameters: None,
                input_summary: None,
                attributes: &json!({}),
            },
        )
        .await
        .expect("root observation should be created");
        (trace_id, request_id, root_id)
    }

    #[tokio::test]
    async fn sqlite_runtime_observation_rejects_root_uuid_collision() {
        let (storage, path) = sqlite_storage().await;
        let (trace_id, _request_id, root_id) =
            sqlite_trace_with_root(&storage, "root-observation-collision").await;

        let result = upsert_runtime_trace_observation(
            &storage,
            RuntimeTraceObservation {
                id: root_id,
                trace_id,
                parent_span_id: Some(root_id),
                name: "Queue",
                kind: "provider_stage",
                observation_type: "span",
                provider_key: Some("local-provider"),
                capability_kind: Some("chat"),
                model: Some("local-model"),
                status: "pending",
                duration_ms: None,
                attributes: &json!({"stage": "queue"}),
                error: None,
            },
        )
        .await;

        assert!(matches!(result, Err(ApiError::Invalid(message))
            if message == "trace observation ID conflicts with an existing observation"));
        let spans = list_trace_spans(&storage, trace_id)
            .await
            .expect("trace observations should list");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].name, "runtime.invoke");
        assert_eq!(spans[0].observation_type, "generation");
        close_and_remove(storage, &path).await;
    }

    #[tokio::test]
    async fn sqlite_runtime_observation_rejects_uuid_reuse_for_another_stage() {
        let (storage, path) = sqlite_storage().await;
        let (trace_id, _request_id, root_id) =
            sqlite_trace_with_root(&storage, "stage-observation-collision").await;
        let observation_id = Uuid::new_v4();
        upsert_runtime_trace_observation(
            &storage,
            RuntimeTraceObservation {
                id: observation_id,
                trace_id,
                parent_span_id: Some(root_id),
                name: "Queue",
                kind: "provider_stage",
                observation_type: "span",
                provider_key: Some("local-provider"),
                capability_kind: Some("chat"),
                model: Some("local-model"),
                status: "pending",
                duration_ms: None,
                attributes: &json!({"stage": "queue"}),
                error: None,
            },
        )
        .await
        .expect("initial observation should be inserted");

        let result = upsert_runtime_trace_observation(
            &storage,
            RuntimeTraceObservation {
                id: observation_id,
                trace_id,
                parent_span_id: Some(root_id),
                name: "Load",
                kind: "provider_stage",
                observation_type: "span",
                provider_key: Some("local-provider"),
                capability_kind: Some("chat"),
                model: Some("local-model"),
                status: "completed",
                duration_ms: Some(5),
                attributes: &json!({"stage": "load"}),
                error: None,
            },
        )
        .await;

        assert!(matches!(result, Err(ApiError::Invalid(message))
            if message == "trace observation ID conflicts with an existing observation"));
        let spans = list_trace_spans(&storage, trace_id)
            .await
            .expect("trace observations should list");
        let observation = spans
            .iter()
            .find(|span| span.id == observation_id)
            .expect("original observation should remain");
        assert_eq!(observation.name, "Queue");
        assert_eq!(observation.status, "pending");
        close_and_remove(storage, &path).await;
    }

    #[tokio::test]
    async fn sqlite_runtime_observation_allows_idempotent_started_to_completed_update() {
        let (storage, path) = sqlite_storage().await;
        let (trace_id, _request_id, root_id) =
            sqlite_trace_with_root(&storage, "stage-observation-update").await;
        let observation_id = Uuid::new_v4();
        for (status, duration_ms) in [("pending", None), ("completed", Some(5))] {
            upsert_runtime_trace_observation(
                &storage,
                RuntimeTraceObservation {
                    id: observation_id,
                    trace_id,
                    parent_span_id: Some(root_id),
                    name: "Queue",
                    kind: "provider_stage",
                    observation_type: "span",
                    provider_key: Some("local-provider"),
                    capability_kind: Some("chat"),
                    model: Some("local-model"),
                    status,
                    duration_ms,
                    attributes: &json!({"stage": "queue"}),
                    error: None,
                },
            )
            .await
            .expect("matching observation update should succeed");
        }
        upsert_runtime_trace_observation(
            &storage,
            RuntimeTraceObservation {
                id: observation_id,
                trace_id,
                parent_span_id: Some(root_id),
                name: "Queue",
                kind: "provider_stage",
                observation_type: "span",
                provider_key: Some("local-provider"),
                capability_kind: Some("chat"),
                model: Some("local-model"),
                status: "completed",
                duration_ms: Some(5),
                attributes: &json!({"stage": "queue"}),
                error: None,
            },
        )
        .await
        .expect("completed observation retry should be idempotent");

        let spans = list_trace_spans(&storage, trace_id)
            .await
            .expect("trace observations should list");
        let observation = spans
            .iter()
            .find(|span| span.id == observation_id)
            .expect("provider observation should remain");
        assert_eq!(observation.status, "completed");
        assert_eq!(observation.duration_ms, Some(5));
        close_and_remove(storage, &path).await;
    }

    #[tokio::test]
    async fn sqlite_supports_the_runtime_resource_lifecycle() {
        let (storage, path) = sqlite_storage().await;
        let project_id = Uuid::new_v4();
        let project = create_project(
            &storage,
            NewProject {
                id: project_id,
                owner_user_id: None,
                slug: "moon-train",
                name: "Moon Train",
                description: Some("Runtime storage contract"),
                gateway_id: "gateway-local",
                binding_ids: &[],
            },
        )
        .await
        .expect("project should be created");
        set_project_runtime_extension(
            &storage,
            project_id,
            "vifu.content-runtime",
            true,
            Some("release-1"),
            &json!({"format": "vifu-content"}),
        )
        .await
        .expect("runtime extension should be stored");

        let secret_keys = vec!["apiKey".to_string()];
        let provider = upsert_provider_connection(
            &storage,
            &project.project.slug,
            NewProviderConnection {
                provider_key: "openai-local",
                source_kind: "custom",
                source_key: "openai-local",
                name: "Local provider",
                provider_type: "openai-compatible",
                base_url: "http://127.0.0.1:8080/v1",
                config: &json!({}),
                encrypted_secret_json: "{}",
                secret_keys: &secret_keys,
                display_secret: Some("configured"),
                status: "configured",
            },
        )
        .await
        .expect("provider should be stored");
        update_provider_connection_status(&storage, provider.id, "ready")
            .await
            .expect("provider status should update");
        let profile_id = Uuid::new_v4();
        create_profile(
            &storage,
            profile_id,
            project_id,
            "mizuki",
            "Mizuki",
            Some("Moon princess"),
        )
        .await
        .expect("profile should be created");
        let capabilities = vec![
            ProfileCapabilityDraft {
                kind: "chat".to_string(),
                provider_type: "openai-compatible".to_string(),
                provider_key: "openai-local".to_string(),
                resource_id: Some("gpt-test".to_string()),
                config: json!({}),
                input_schema: json!({}),
                output_schema: json!({}),
            },
            ProfileCapabilityDraft {
                kind: "embedding".to_string(),
                provider_type: "openai-compatible".to_string(),
                provider_key: "openai-local".to_string(),
                resource_id: Some("text-embedding-test".to_string()),
                config: json!({}),
                input_schema: json!({}),
                output_schema: json!({}),
            },
        ];
        let version = create_profile_version(
            &storage,
            profile_id,
            NewProfileVersion {
                persona: &json!({"files": {"SOUL.md": "Protect the moon train."}}),
                runtime: &json!({}),
                presentation: &json!({"portrait": "mizuki.png"}),
                source: &json!({"type": "custom", "providerKey": "openai-local"}),
                capabilities: &capabilities,
                change_summary: Some("Initial version"),
            },
        )
        .await
        .expect("profile version should be created");
        assert_eq!(
            resolve_profile_route(
                &storage,
                project_id,
                "mizuki",
                "chat",
                Some("player-1"),
                None,
            )
            .await
            .expect("profile route should resolve")
            .profile_version_id,
            version.id
        );
        assert_eq!(
            resolve_profile_route(
                &storage,
                project_id,
                "mizuki",
                "embedding",
                Some("player-1"),
                None,
            )
            .await
            .expect("embedding profile route should resolve")
            .profile_version_id,
            version.id
        );

        let binding_id = Uuid::new_v4();
        create_binding(
            &storage,
            binding_id,
            profile_id,
            "openai-compatible",
            "gateway-local",
            "mizuki",
            &json!({"providerKey": "openai-local"}),
        )
        .await
        .expect("binding should be created");
        attach_project_binding(&storage, project_id, binding_id)
            .await
            .expect("binding should be attached");
        refresh_discovered_binding(
            &storage,
            binding_id,
            "gateway-local",
            "Mizuki Tsukishiro",
            None,
        )
        .await
        .expect("discovered binding should refresh");

        let endpoint_id = Uuid::new_v4();
        create_endpoint(
            &storage,
            NewEndpoint {
                id: endpoint_id,
                slug: "mizuki",
                name: "Mizuki",
                profile_id,
                binding_id,
                enabled: true,
                request_timeout_ms: 30_000,
            },
        )
        .await
        .expect("endpoint should be created");
        update_endpoint(
            &storage,
            endpoint_id,
            EndpointPatch {
                slug: None,
                name: Some("Mizuki Chat"),
                profile_id: None,
                binding_id: None,
                enabled: Some(true),
                request_timeout_ms: Some(20_000),
            },
        )
        .await
        .expect("endpoint should update");

        let api_key_id = Uuid::new_v4();
        create_api_key(
            &storage,
            NewApiKey {
                id: api_key_id,
                project_id,
                name: "Game client",
                agent_scope: &ApiKeyAgentScope::Selected {
                    profile_ids: vec![profile_id],
                },
                permissions: &ApiKeyPermissions::default(),
                key_prefix: "vifu_test",
                key_hash: b"project-key-hash",
            },
        )
        .await
        .expect("API key should be created");
        update_api_key(
            &storage,
            api_key_id,
            ApiKeyPatch {
                project_id: None,
                name: Some("Updated game client"),
                agent_scope: None,
                permissions: None,
            },
        )
        .await
        .expect("API key should update");
        assert_eq!(
            active_api_key_by_hash(&storage, b"project-key-hash")
                .await
                .expect("API key should authenticate")
                .id,
            api_key_id
        );

        register_agent_gateway_credential(
            &storage,
            "gateway-local",
            None,
            "gateway_",
            b"gateway-credential-hash",
        )
        .await
        .expect("gateway credential should be registered");
        assert_eq!(
            authenticate_agent_gateway_credential(&storage, b"gateway-credential-hash")
                .await
                .expect("gateway credential should authenticate"),
            "gateway-local"
        );
        let (gateway_session_id, resumed) = open_agent_gateway_session(
            &storage,
            "gateway-local",
            None,
            &json!([{"id": "mizuki", "name": "Mizuki"}]),
            &json!({"providerId": "openai-local"}),
        )
        .await
        .expect("gateway session should open");
        assert!(!resumed);

        let channel = create_project_runtime_channel(
            &storage,
            NewProjectRuntimeChannel {
                id: Uuid::new_v4(),
                project_id,
                name: "Web player",
                public_id: Uuid::new_v4(),
                launch_key_prefix: "launch_key",
                launch_key_hash: b"launch-key-hash",
                allowed_origins: &["https://example.test".to_string()],
            },
        )
        .await
        .expect("runtime channel should be created");
        create_runtime_launch_session(
            &storage,
            Uuid::new_v4(),
            project_id,
            channel.id,
            b"launch-session-hash",
            Utc::now() + ChronoDuration::minutes(5),
        )
        .await
        .expect("runtime launch session should be created");

        create_realtime_session(
            &storage,
            Uuid::new_v4(),
            project_id,
            profile_id,
            Some(api_key_id),
            b"realtime-session-hash",
            Utc::now() + ChronoDuration::minutes(5),
        )
        .await
        .expect("realtime session should be created");

        let request_id = Uuid::new_v4();
        let trace_id = create_trace(
            &storage,
            NewTrace {
                request_id,
                endpoint_id: Some(endpoint_id),
                project_id: Some(project_id),
                gateway_session_id: Some(gateway_session_id),
                profile_id: Some(profile_id),
                profile_version_id: Some(version.id),
                operation: "chat.completions",
                provider_key: Some("openai-local"),
                capability_kind: Some("chat"),
                selection_key: None,
                request: &json!({"messages": []}),
            },
        )
        .await
        .expect("trace should be created");
        let span_id = create_trace_span(
            &storage,
            NewTraceSpan {
                trace_id,
                parent_span_id: None,
                name: "provider.call",
                kind: "provider",
                observation_type: "generation",
                provider_key: Some("openai-local"),
                capability_kind: Some("chat"),
                model: Some("test-model"),
                model_parameters: Some(&json!({"temperature": 0.2})),
                input_summary: Some(&json!({"messages": 0})),
                attributes: &json!({}),
            },
        )
        .await
        .expect("trace span should be created");
        complete_trace_span(
            &storage,
            span_id,
            "completed",
            5,
            Some(&json!({"tokens": 1})),
            None,
        )
        .await
        .expect("trace span should complete");
        update_trace_generation(
            &storage,
            span_id,
            Some(3),
            Some(&json!({"promptTokens": 4, "completionTokens": 1})),
        )
        .await
        .expect("generation details should update");
        let feedback_target = trace_feedback_target(&storage, project_id, request_id)
            .await
            .expect("feedback target should resolve");
        assert_eq!(feedback_target.parent_span_id, Some(span_id));
        assert_eq!(feedback_target.capability_kind.as_deref(), Some("chat"));
        let score = upsert_trace_score(
            &storage,
            NewTraceScore {
                trace_id,
                span_id: Some(span_id),
                name: "OUTPUT_ACCEPTED",
                data_type: "categorical",
                value: &json!("pass"),
                source: "application",
            },
        )
        .await
        .expect("trace score should be created");
        assert_eq!(score.value, json!("pass"));
        complete_trace(
            &storage,
            request_id,
            "completed",
            10,
            Some(&json!({"ok": true})),
            None,
        )
        .await
        .expect("trace should complete");

        assert_eq!(
            resolve_project_model_route(&storage, project_id, "mizuki")
                .await
                .expect("model should resolve")
                .profile_id,
            profile_id
        );
        let traces = list_traces(
            &storage,
            TraceListOptions {
                endpoint_id: None,
                project_id: Some(project_id),
                request_id: None,
                trace_id: None,
                allowed_profile_ids: None,
                created_from: None,
                created_before: None,
                cursor: None,
                limit: 10,
            },
        )
        .await
        .expect("traces should list");
        assert_eq!(traces.len(), 1);
        assert_eq!(traces[0].provider_name.as_deref(), Some("Local provider"));
        assert_eq!(traces[0].app_outcome.as_deref(), Some("unknown"));
        assert_eq!(
            get_trace_project_id(&storage, trace_id)
                .await
                .expect("trace project should resolve"),
            Some(project_id)
        );
        assert_eq!(
            list_trace_spans(&storage, trace_id)
                .await
                .expect("trace spans should list")
                .first()
                .and_then(|span| span.completion_start_ms),
            Some(3)
        );
        assert_eq!(
            list_trace_scores(&storage, trace_id)
                .await
                .expect("trace scores should list")
                .len(),
            1
        );
        assert_eq!(
            list_public_agents(&storage, project_id, None)
                .await
                .expect("public agents should list")
                .len(),
            1
        );
        assert_eq!(
            list_project_runtime_channels(&storage, project_id)
                .await
                .expect("runtime channels should list")
                .len(),
            1
        );
        assert_eq!(
            list_available_agents(&storage)
                .await
                .expect("agents should list")
                .len(),
            1
        );
        assert_eq!(
            active_runtime_launch_project(&storage, b"launch-session-hash")
                .await
                .expect("launch session should resolve"),
            Some(project_id)
        );
        archive_project_profile(&storage, project_id, profile_id)
            .await
            .expect("profile should archive");
        let archived_sources = list_archived_project_agent_sources(&storage, project_id)
            .await
            .expect("archived profile sources should list");
        assert_eq!(archived_sources.len(), 1);
        assert_eq!(archived_sources[0].profile_id, profile_id);
        assert_eq!(archived_sources[0].provider_key, "openai-local");
        assert_eq!(archived_sources[0].provider_type, "openai-compatible");
        restore_project_profile(&storage, project_id, profile_id)
            .await
            .expect("profile should restore");

        close_and_remove(storage, &path).await;
    }

    #[tokio::test]
    async fn sqlite_runtime_distribution_is_idempotent_and_enforces_its_device_limit() {
        let (storage, path) = sqlite_storage().await;
        let project_id = Uuid::new_v4();
        create_project(
            &storage,
            NewProject {
                id: project_id,
                owner_user_id: Some("user-123"),
                slug: "distributed-runtime",
                name: "Distributed runtime",
                description: None,
                gateway_id: "project-distributed-runtime",
                binding_ids: &[],
            },
        )
        .await
        .expect("project should be created");
        let deployment_id = primary_deployment_id(&storage, project_id).await;
        let distribution_id = Uuid::new_v4();
        let public_id = format!("vifu_di_{}", "a".repeat(64));
        create_runtime_distribution(
            &storage,
            NewRuntimeDistribution {
                id: distribution_id,
                project_id,
                deployment_id,
                name: "Public Android build",
                public_id: &public_id,
                max_gateways: 1,
            },
        )
        .await
        .expect("distribution should be created");

        let first = authorize_runtime_distribution_gateway(
            &storage,
            &public_id,
            "machine-one",
            "gateway-one",
        )
        .await
        .expect("first device should join");
        assert_eq!(first.gateway_id, "gateway-one");
        assert_eq!(first.owner_user_id.as_deref(), Some("user-123"));
        let repeated = authorize_runtime_distribution_gateway(
            &storage,
            &public_id,
            "machine-one",
            "gateway-different",
        )
        .await
        .expect("the same installation should rejoin idempotently");
        assert_eq!(repeated.gateway_id, "gateway-one");
        assert!(matches!(
            authorize_runtime_distribution_gateway(
                &storage,
                &public_id,
                "machine-two",
                "gateway-two",
            )
            .await,
            Err(ApiError::Conflict(_))
        ));
        assert!(list_runtime_deployment_gateway_ids(&storage, deployment_id)
            .await
            .unwrap()
            .contains(&"gateway-one".to_string()));

        revoke_runtime_distribution(&storage, project_id, distribution_id)
            .await
            .expect("distribution should revoke");
        assert!(matches!(
            authorize_runtime_distribution_gateway(
                &storage,
                &public_id,
                "machine-three",
                "gateway-three",
            )
            .await,
            Err(ApiError::Unauthorized)
        ));
        close_and_remove(storage, &path).await;
    }

    #[tokio::test]
    async fn sqlite_app_id_automatically_enrolls_a_gateway_in_the_primary_deployment() {
        let (storage, path) = sqlite_storage().await;
        let project_id = Uuid::new_v4();
        let app = create_project(
            &storage,
            NewProject {
                id: project_id,
                owner_user_id: Some("app-owner"),
                slug: "automatic-app",
                name: "Automatic app",
                description: None,
                gateway_id: "",
                binding_ids: &[],
            },
        )
        .await
        .expect("app should be created");
        assert!(app.project.app_id.starts_with("vifu_app_"));
        assert_eq!(app.project.app_id.len(), "vifu_app_".len() + 64);

        let assignment = authorize_runtime_distribution_gateway(
            &storage,
            &app.project.app_id,
            "app-machine",
            "app-gateway",
        )
        .await
        .expect("App ID should enroll the installation");
        assert_eq!(assignment.gateway_id, "app-gateway");
        assert_eq!(assignment.owner_user_id.as_deref(), Some("app-owner"));

        let deployment_id = primary_deployment_id(&storage, project_id).await;
        assert_eq!(
            list_runtime_deployment_gateway_ids(&storage, deployment_id)
                .await
                .unwrap(),
            vec!["app-gateway".to_string()]
        );
        close_and_remove(storage, &path).await;
    }

    #[tokio::test]
    async fn sqlite_consumes_gateway_enrollment_once_and_assigns_the_project() {
        let (storage, path) = sqlite_storage().await;
        let project_id = Uuid::new_v4();
        create_project(
            &storage,
            NewProject {
                id: project_id,
                owner_user_id: Some("user-123"),
                slug: "remote-project",
                name: "Remote project",
                description: None,
                gateway_id: "project-remote-project",
                binding_ids: &[],
            },
        )
        .await
        .expect("project should be created");
        create_agent_gateway_enrollment(
            &storage,
            NewAgentGatewayEnrollment {
                id: Uuid::new_v4(),
                project_id,
                deployment_id: primary_deployment_id(&storage, project_id).await,
                owner_user_id: "user-123",
                token_hash: b"enrollment-hash",
                expires_at: Utc::now() + ChronoDuration::minutes(5),
            },
        )
        .await
        .expect("enrollment should be created");

        assert_eq!(
            consume_agent_gateway_enrollment(
                &storage,
                b"enrollment-hash",
                "gateway-remote",
                "vifu_gw_remote",
                b"remote-credential-hash",
            )
            .await
            .expect("enrollment should register the gateway"),
            AgentGatewayRegistration::Registered
        );
        assert_eq!(
            get_project(&storage, project_id)
                .await
                .expect("project should load")
                .project
                .gateway_id,
            "gateway-remote"
        );
        assert_eq!(
            authenticate_agent_gateway_credential(&storage, b"remote-credential-hash")
                .await
                .expect("gateway credential should authenticate"),
            "gateway-remote"
        );
        assert_eq!(
            consume_agent_gateway_enrollment(
                &storage,
                b"enrollment-hash",
                "gateway-remote",
                "vifu_gw_remote",
                b"remote-credential-hash",
            )
            .await
            .expect("the exact enrollment retry should be idempotent"),
            AgentGatewayRegistration::Existing
        );
        assert!(matches!(
            consume_agent_gateway_enrollment(
                &storage,
                b"enrollment-hash",
                "gateway-other",
                "vifu_gw_other",
                b"other-credential-hash",
            )
            .await,
            Err(ApiError::Unauthorized)
        ));

        revoke_agent_gateway_credential(&storage, "gateway-remote")
            .await
            .expect("gateway credential should be revoked");
        assert!(matches!(
            authenticate_agent_gateway_credential(&storage, b"remote-credential-hash").await,
            Err(ApiError::Forbidden)
        ));
        create_agent_gateway_enrollment(
            &storage,
            NewAgentGatewayEnrollment {
                id: Uuid::new_v4(),
                project_id,
                deployment_id: primary_deployment_id(&storage, project_id).await,
                owner_user_id: "user-123",
                token_hash: b"replacement-enrollment-hash",
                expires_at: Utc::now() + ChronoDuration::minutes(5),
            },
        )
        .await
        .expect("replacement enrollment should be created");
        assert_eq!(
            consume_agent_gateway_enrollment(
                &storage,
                b"replacement-enrollment-hash",
                "gateway-remote",
                "vifu_gw_replacement",
                b"replacement-credential-hash",
            )
            .await
            .expect("authorized enrollment should rotate the gateway credential"),
            AgentGatewayRegistration::Registered
        );
        assert_eq!(
            authenticate_agent_gateway_credential(&storage, b"replacement-credential-hash")
                .await
                .expect("replacement credential should authenticate"),
            "gateway-remote"
        );
        assert!(matches!(
            authenticate_agent_gateway_credential(&storage, b"remote-credential-hash").await,
            Err(ApiError::Forbidden)
        ));

        close_and_remove(storage, &path).await;
    }

    #[tokio::test]
    async fn sqlite_gateway_enrollment_revokes_prior_tokens_and_rejects_expired_tokens() {
        let (storage, path) = sqlite_storage().await;
        let project_id = Uuid::new_v4();
        create_project(
            &storage,
            NewProject {
                id: project_id,
                owner_user_id: Some("user-123"),
                slug: "rotated-enrollment",
                name: "Rotated enrollment",
                description: None,
                gateway_id: "project-rotated-enrollment",
                binding_ids: &[],
            },
        )
        .await
        .expect("project should be created");
        for (token_hash, expires_at) in [
            (
                b"superseded-enrollment".as_slice(),
                Utc::now() + ChronoDuration::minutes(5),
            ),
            (
                b"current-enrollment".as_slice(),
                Utc::now() + ChronoDuration::minutes(5),
            ),
        ] {
            create_agent_gateway_enrollment(
                &storage,
                NewAgentGatewayEnrollment {
                    id: Uuid::new_v4(),
                    project_id,
                    deployment_id: primary_deployment_id(&storage, project_id).await,
                    owner_user_id: "user-123",
                    token_hash,
                    expires_at,
                },
            )
            .await
            .expect("enrollment should be created");
        }

        assert!(matches!(
            consume_agent_gateway_enrollment(
                &storage,
                b"superseded-enrollment",
                "gateway-superseded",
                "vifu_gw_superseded",
                b"superseded-credential",
            )
            .await,
            Err(ApiError::Unauthorized)
        ));

        let expired_project_id = Uuid::new_v4();
        create_project(
            &storage,
            NewProject {
                id: expired_project_id,
                owner_user_id: Some("user-123"),
                slug: "expired-enrollment",
                name: "Expired enrollment",
                description: None,
                gateway_id: "project-expired-enrollment",
                binding_ids: &[],
            },
        )
        .await
        .expect("expired project should be created");
        create_agent_gateway_enrollment(
            &storage,
            NewAgentGatewayEnrollment {
                id: Uuid::new_v4(),
                project_id: expired_project_id,
                deployment_id: primary_deployment_id(&storage, expired_project_id).await,
                owner_user_id: "user-123",
                token_hash: b"expired-enrollment",
                expires_at: Utc::now() - ChronoDuration::seconds(1),
            },
        )
        .await
        .expect("expired enrollment should be stored");
        assert!(matches!(
            consume_agent_gateway_enrollment(
                &storage,
                b"expired-enrollment",
                "gateway-expired",
                "vifu_gw_expired",
                b"expired-credential",
            )
            .await,
            Err(ApiError::Unauthorized)
        ));

        close_and_remove(storage, &path).await;
    }

    #[tokio::test]
    async fn sqlite_gateway_can_enroll_multiple_projects_for_one_owner_only() {
        let (storage, path) = sqlite_storage().await;
        let first_project_id = Uuid::new_v4();
        let second_project_id = Uuid::new_v4();
        let other_project_id = Uuid::new_v4();
        for (id, owner_user_id, slug) in [
            (first_project_id, "user-123", "gateway-owner-first"),
            (second_project_id, "user-123", "gateway-owner-second"),
            (other_project_id, "user-456", "gateway-other-owner"),
        ] {
            create_project(
                &storage,
                NewProject {
                    id,
                    owner_user_id: Some(owner_user_id),
                    slug,
                    name: slug,
                    description: None,
                    gateway_id: slug,
                    binding_ids: &[],
                },
            )
            .await
            .expect("project should be created");
        }
        for (project_id, owner_user_id, token_hash) in [
            (
                first_project_id,
                "user-123",
                b"owner-first-enrollment".as_slice(),
            ),
            (
                second_project_id,
                "user-123",
                b"owner-second-enrollment".as_slice(),
            ),
            (
                other_project_id,
                "user-456",
                b"other-owner-enrollment".as_slice(),
            ),
        ] {
            create_agent_gateway_enrollment(
                &storage,
                NewAgentGatewayEnrollment {
                    id: Uuid::new_v4(),
                    project_id,
                    deployment_id: primary_deployment_id(&storage, project_id).await,
                    owner_user_id,
                    token_hash,
                    expires_at: Utc::now() + ChronoDuration::minutes(5),
                },
            )
            .await
            .expect("enrollment should be created");
        }

        consume_agent_gateway_enrollment(
            &storage,
            b"owner-first-enrollment",
            "gateway-owner",
            "vifu_gw_owner",
            b"owner-credential",
        )
        .await
        .expect("first owner project should enroll");
        assert_eq!(
            consume_agent_gateway_enrollment(
                &storage,
                b"owner-second-enrollment",
                "gateway-owner",
                "vifu_gw_owner",
                b"owner-credential",
            )
            .await
            .expect("second owner project should enroll"),
            AgentGatewayRegistration::Existing
        );
        assert!(matches!(
            consume_agent_gateway_enrollment(
                &storage,
                b"other-owner-enrollment",
                "gateway-owner",
                "vifu_gw_owner",
                b"owner-credential",
            )
            .await,
            Err(ApiError::Conflict(_))
        ));
        assert_eq!(
            get_project(&storage, second_project_id)
                .await
                .expect("second project should load")
                .project
                .gateway_id,
            "gateway-owner"
        );

        close_and_remove(storage, &path).await;
    }

    #[tokio::test]
    async fn sqlite_device_token_rotation_keeps_a_short_grace_window() {
        let (storage, path) = sqlite_storage().await;
        let machine_id = format!("machine-{}", "a".repeat(64));
        let public_key = format!("public-key-{}", Uuid::new_v4().simple());
        upsert_agent_gateway_machine(&storage, &machine_id, &public_key)
            .await
            .expect("machine should be stored");
        create_agent_gateway_authorization(
            &storage,
            NewAgentGatewayAuthorization {
                gateway_id: "gateway-token-rotation",
                machine_id: &machine_id,
                owner_user_id: Some("token-owner"),
                token_prefix: "vifu_gw_old",
                token_hash: b"old-device-token-hash",
                token_expires_at: Utc::now() + ChronoDuration::days(180),
            },
        )
        .await
        .expect("authorization should be created");

        let rotated = rotate_agent_gateway_authorization(
            &storage,
            RotatedAgentGatewayAuthorization {
                gateway_id: "gateway-token-rotation",
                token_prefix: "vifu_gw_new",
                token_hash: b"new-device-token-hash",
                token_expires_at: Utc::now() + ChronoDuration::days(180),
            },
        )
        .await
        .expect("authorization should rotate");
        assert_eq!(rotated.token_generation, 2);
        for token_hash in [
            b"old-device-token-hash".as_slice(),
            b"new-device-token-hash".as_slice(),
        ] {
            assert_eq!(
                authenticate_agent_gateway_device_token(&storage, token_hash)
                    .await
                    .expect("current and grace tokens should authenticate"),
                "gateway-token-rotation"
            );
        }

        revoke_agent_gateway_authorization(&storage, "gateway-token-rotation")
            .await
            .expect("authorization should be revoked");
        for token_hash in [
            b"old-device-token-hash".as_slice(),
            b"new-device-token-hash".as_slice(),
        ] {
            assert!(matches!(
                authenticate_agent_gateway_device_token(&storage, token_hash).await,
                Err(ApiError::Forbidden)
            ));
        }

        close_and_remove(storage, &path).await;
    }

    #[tokio::test]
    async fn sqlite_consumes_approved_gateway_pairing_by_machine() {
        let (storage, path) = sqlite_storage().await;
        let machine_id = format!("machine-{}", "b".repeat(64));
        let public_key = format!("public-key-{}", Uuid::new_v4().simple());
        upsert_agent_gateway_machine(&storage, &machine_id, &public_key)
            .await
            .expect("machine should be stored");

        let pairing = create_or_get_agent_gateway_pairing(
            &storage,
            &machine_id,
            Utc::now() + ChronoDuration::minutes(5),
        )
        .await
        .expect("pairing should be created");
        resolve_agent_gateway_pairing(&storage, pairing.id, "approved", Some("user-123"))
            .await
            .expect("pairing should be approved");

        let consumed = consume_approved_agent_gateway_pairing_for_machine(&storage, &machine_id)
            .await
            .expect("approved pairing should be consumable by machine")
            .expect("approved pairing should exist");
        assert_eq!(consumed.id, pairing.id);
        assert_eq!(consumed.status, "consumed");
        assert_eq!(consumed.owner_user_id.as_deref(), Some("user-123"));
        assert!(
            consume_approved_agent_gateway_pairing_for_machine(&storage, &machine_id)
                .await
                .expect("second consume should be safe")
                .is_none()
        );

        close_and_remove(storage, &path).await;
    }

    async fn concurrent_gateway_enrollment_should_allow_only_one_binding(
        storage: &Storage,
        project_slug: &str,
    ) {
        let project_id = Uuid::new_v4();
        create_project(
            storage,
            NewProject {
                id: project_id,
                owner_user_id: Some("concurrent-owner"),
                slug: project_slug,
                name: project_slug,
                description: None,
                gateway_id: project_slug,
                binding_ids: &[],
            },
        )
        .await
        .expect("concurrent project should be created");
        create_agent_gateway_enrollment(
            storage,
            NewAgentGatewayEnrollment {
                id: Uuid::new_v4(),
                project_id,
                deployment_id: primary_deployment_id(storage, project_id).await,
                owner_user_id: "concurrent-owner",
                token_hash: project_slug.as_bytes(),
                expires_at: Utc::now() + ChronoDuration::minutes(5),
            },
        )
        .await
        .expect("concurrent enrollment should be created");

        let first = consume_agent_gateway_enrollment(
            storage,
            project_slug.as_bytes(),
            "gateway-concurrent-a",
            "vifu_gw_concurrent_a",
            b"concurrent-credential-a",
        );
        let second = consume_agent_gateway_enrollment(
            storage,
            project_slug.as_bytes(),
            "gateway-concurrent-b",
            "vifu_gw_concurrent_b",
            b"concurrent-credential-b",
        );
        let (first, second) = tokio::join!(first, second);
        assert_eq!(
            usize::from(first.is_ok()) + usize::from(second.is_ok()),
            1,
            "exactly one concurrent enrollment should bind the project"
        );
        delete_project(storage, project_id)
            .await
            .expect("concurrent project should be removed");
    }

    #[tokio::test]
    async fn sqlite_gateway_enrollment_is_atomic_under_concurrency() {
        let (storage, path) = sqlite_storage().await;
        concurrent_gateway_enrollment_should_allow_only_one_binding(
            &storage,
            "sqlite-concurrent-enrollment",
        )
        .await;
        close_and_remove(storage, &path).await;
    }

    #[tokio::test]
    async fn postgres_gateway_enrollment_is_atomic_under_concurrency_when_available() {
        if std::env::var("VIFU_TEST_DATABASE_REQUIRED").as_deref() != Ok("1") {
            eprintln!("skipping PostgreSQL enrollment test outside the CI database job");
            return;
        }
        let database_url = "postgres://vifu@127.0.0.1:5432/vifu";
        let storage = connect(database_url, 5)
            .await
            .expect("PostgreSQL is required for enrollment tests");
        migrate(&storage)
            .await
            .expect("PostgreSQL migrations should run");
        let slug = format!("postgres-concurrent-{}", Uuid::new_v4().simple());
        concurrent_gateway_enrollment_should_allow_only_one_binding(&storage, &slug).await;
        match storage {
            Storage::Postgres(pool) => pool.close().await,
            Storage::Sqlite(_) => unreachable!("PostgreSQL URL should create PostgreSQL storage"),
        }
    }

    #[tokio::test]
    async fn sqlite_lists_only_projects_owned_by_the_canonical_user() {
        let (storage, path) = sqlite_storage().await;
        for (slug, owner_user_id) in [
            ("owner-a-first", Some("user-a")),
            ("owner-b", Some("user-b")),
            ("admin-project", None),
            ("owner-a-second", Some("user-a")),
        ] {
            create_project(
                &storage,
                NewProject {
                    id: Uuid::new_v4(),
                    owner_user_id,
                    slug,
                    name: slug,
                    description: None,
                    gateway_id: "gateway-local",
                    binding_ids: &[],
                },
            )
            .await
            .expect("project should be created");
        }

        let projects = list_projects_for_owner_user_id(&storage, "user-a")
            .await
            .expect("owned projects should list");
        assert_eq!(
            projects
                .iter()
                .map(|project| project.project.slug.as_str())
                .collect::<Vec<_>>(),
            vec!["owner-a-first", "owner-a-second"]
        );
        assert!(projects
            .iter()
            .all(|project| project.project.owner_user_id.as_deref() == Some("user-a")));
        assert_eq!(
            list_projects(&storage)
                .await
                .expect("admin projects should list")
                .len(),
            4
        );
        let admin_project = list_projects(&storage)
            .await
            .expect("admin projects should list")
            .into_iter()
            .find(|project| project.project.slug == "admin-project")
            .expect("unowned project should exist");
        set_project_owner_user_id(&storage, admin_project.project.id, "user-a")
            .await
            .expect("legacy project should be assigned");
        assert_eq!(
            list_projects_for_owner_user_id(&storage, "user-a")
                .await
                .expect("assigned projects should list")
                .len(),
            3
        );

        close_and_remove(storage, &path).await;
    }

    #[tokio::test]
    async fn sqlite_persists_projects_across_pool_restarts() {
        let (storage, path) = sqlite_storage().await;
        let project_id = Uuid::new_v4();
        create_project(
            &storage,
            NewProject {
                id: project_id,
                owner_user_id: None,
                slug: "persistent-project",
                name: "Persistent project",
                description: None,
                gateway_id: "gateway-local",
                binding_ids: &[],
            },
        )
        .await
        .expect("project should be created");
        match storage {
            Storage::Postgres(pool) => pool.close().await,
            Storage::Sqlite(pool) => pool.close().await,
        }

        let reopened = connect(&format!("sqlite://{}", path.display()), 5)
            .await
            .expect("SQLite should reopen");
        migrate(&reopened)
            .await
            .expect("migrations should be repeatable");
        assert_eq!(
            get_project(&reopened, project_id)
                .await
                .expect("project should persist")
                .project
                .slug,
            "persistent-project"
        );

        close_and_remove(reopened, &path).await;
    }
}
