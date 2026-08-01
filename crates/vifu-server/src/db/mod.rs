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
    pub async fn assign_runtime_deployment_gateway(storage: &Storage, project_id: Uuid, deployment_id: Uuid, gateway_id: &str) -> Result<(), ApiError>;
    pub async fn unassign_runtime_deployment_gateway(storage: &Storage, project_id: Uuid, deployment_id: Uuid, gateway_id: &str) -> Result<(), ApiError>;
    pub async fn list_runtime_deployments_for_gateway(storage: &Storage, gateway_id: &str) -> Result<Vec<RuntimeDeployment>, ApiError>;
    pub async fn runtime_deployment_allows_remote_invocation(storage: &Storage, project_id: Uuid, gateway_id: &str) -> Result<bool, ApiError>;
    pub async fn create_project_runtime_release(storage: &Storage, input: NewProjectRuntimeRelease<'_>) -> Result<ProjectRuntimeRelease, ApiError>;
    pub async fn list_project_runtime_releases(storage: &Storage, project_id: Uuid) -> Result<Vec<ProjectRuntimeRelease>, ApiError>;
    pub async fn get_project_runtime_release(storage: &Storage, project_id: Uuid, version: i64) -> Result<ProjectRuntimeRelease, ApiError>;
    pub async fn activate_runtime_deployment_release(storage: &Storage, project_id: Uuid, deployment_name: &str, version: i64) -> Result<RuntimeDeployment, ApiError>;
    pub async fn create_guest_project(storage: &Storage, input: NewGuestProject<'_>) -> Result<(), ApiError>;
    pub async fn get_active_guest_project_for_gateway(storage: &Storage, gateway_id: &str) -> Result<Option<(ProjectWithBindings, DateTime<Utc>)>, ApiError>;
    pub async fn count_active_guest_projects(storage: &Storage) -> Result<i64, ApiError>;
    pub async fn prune_expired_guest_projects(storage: &Storage) -> Result<u64, ApiError>;
    pub async fn claim_guest_project(storage: &Storage, claim_token_hash: &[u8], owner_user_id: &str) -> Result<ProjectWithBindings, ApiError>;
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
    pub async fn list_custom_providers(storage: &Storage) -> Result<Vec<CustomProvider>, ApiError>;
    pub async fn upsert_custom_provider(storage: &Storage, connection: NewProviderConnection<'_>) -> Result<CustomProvider, ApiError>;
    pub async fn get_custom_provider_secret_by_key(storage: &Storage, provider_key: &str) -> Result<CustomProviderSecret, ApiError>;
    pub async fn project_provider_is_assigned(storage: &Storage, project_id: Uuid, provider_key: &str) -> Result<bool, ApiError>;
    pub async fn list_projects_for_gateway(storage: &Storage, gateway_id: &str) -> Result<Vec<(Uuid, String)>, ApiError>;
    pub async fn list_projects_for_provider_key(storage: &Storage, provider_key: &str) -> Result<Vec<(Uuid, String)>, ApiError>;
    pub async fn list_project_profile_provider_resources(storage: &Storage, project_id: Uuid) -> Result<Vec<(String, String)>, ApiError>;
    pub async fn list_archived_project_agent_sources(storage: &Storage, project_id: Uuid) -> Result<Vec<ArchivedProjectAgentSource>, ApiError>;
    pub async fn restore_project_profile(storage: &Storage, project_id: Uuid, profile_id: Uuid) -> Result<AgentProfile, ApiError>;
    pub async fn find_project_profile_by_provider_resource(storage: &Storage, project_id: Uuid, provider_key: &str, agent_id: &str) -> Result<Option<(Uuid, bool, Uuid)>, ApiError>;
    pub async fn refresh_discovered_binding(storage: &Storage, binding_id: Uuid, gateway_id: &str, agent_name: &str) -> Result<(), ApiError>;
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
    pub async fn ensure_discovered_binding(storage: &Storage, project_id: Uuid, gateway_id: &str, agent_id: &str, agent_name: &str, provider_key: &str, provider_type: &str) -> Result<Uuid, ApiError>;
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
    pub async fn complete_trace_span(storage: &Storage, span_id: Uuid, status: &str, duration_ms: i64, output_summary: Option<&Value>, error: Option<&str>) -> Result<(), ApiError>;
    pub async fn complete_trace(storage: &Storage, request_id: Uuid, status: &str, latency_ms: i64, response: Option<&Value>, error: Option<&str>) -> Result<(), ApiError>;
    pub async fn list_traces(storage: &Storage, endpoint_id: Option<Uuid>, project_id: Option<Uuid>, limit: i64) -> Result<Vec<EndpointTrace>, ApiError>;
    pub async fn get_trace_project_id(storage: &Storage, trace_id: Uuid) -> Result<Option<Uuid>, ApiError>;
    pub async fn list_trace_spans(storage: &Storage, trace_id: Uuid) -> Result<Vec<TraceSpan>, ApiError>;
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
        upsert_custom_provider(
            &storage,
            NewProviderConnection {
                provider_key: "shared-openai",
                source_kind: "custom",
                source_key: "shared-openai",
                name: "Shared provider",
                provider_type: "openai-compatible",
                base_url: "http://127.0.0.1:8081/v1",
                config: &json!({}),
                encrypted_secret_json: "{}",
                secret_keys: &secret_keys,
                display_secret: Some("configured"),
                status: "configured",
            },
        )
        .await
        .expect("custom provider should be stored");
        assert_eq!(
            list_custom_providers(&storage)
                .await
                .expect("custom providers should list")
                .len(),
            1
        );

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
        let capabilities = vec![ProfileCapabilityDraft {
            kind: "chat".to_string(),
            provider_type: "openai-compatible".to_string(),
            provider_key: "openai-local".to_string(),
            resource_id: Some("gpt-test".to_string()),
            config: json!({}),
            input_schema: json!({}),
            output_schema: json!({}),
        }];
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
        refresh_discovered_binding(&storage, binding_id, "gateway-local", "Mizuki Tsukishiro")
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
                provider_key: Some("openai-local"),
                capability_kind: Some("chat"),
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
        assert_eq!(
            list_traces(&storage, None, Some(project_id), 10)
                .await
                .expect("traces should list")
                .len(),
            1
        );
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
