use std::collections::HashSet;
use std::fmt::Write;

use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::types::Json;
use sqlx::{FromRow, PgPool, Postgres, QueryBuilder};
use uuid::Uuid;

use super::types::*;
use crate::error::{map_database_error, ApiError};
use crate::models::{
    slugify, validate_slug, AgentBinding, AgentEndpoint, AgentGatewayCredential,
    AgentGatewaySession, AgentProfile, AgentProfileCapability, AgentProfileRollout,
    AgentProfileVersion, ApiKeyAgentScope, ApiKeyPermissions, ApiKeyRecord, AvailableAgent,
    CustomProvider, CustomProviderSecret, EndpointRoute, EndpointTrace, ProfileCapabilityDraft,
    ProfileRoute, Project, ProjectRuntimeChannel, ProjectRuntimeExtension, ProjectWithBindings,
    ProviderConnection, ProviderConnectionSecret, PublicAgent, RealtimeSession, TraceSpan,
};

#[derive(Debug, FromRow)]
struct ApiKeyRow {
    id: Uuid,
    project_id: Uuid,
    name: String,
    scope_mode: String,
    profile_ids: Vec<Uuid>,
    permissions: Json<ApiKeyPermissions>,
    key_prefix: String,
    created_at: DateTime<Utc>,
    revoked_at: Option<DateTime<Utc>>,
}

impl TryFrom<ApiKeyRow> for ApiKeyRecord {
    type Error = ApiError;

    fn try_from(row: ApiKeyRow) -> Result<Self, Self::Error> {
        let agent_scope = match row.scope_mode.as_str() {
            "all" if row.profile_ids.is_empty() => ApiKeyAgentScope::All,
            "selected" => ApiKeyAgentScope::Selected {
                profile_ids: row.profile_ids,
            },
            _ => return Err(ApiError::Internal),
        };
        Ok(Self {
            id: row.id,
            project_id: row.project_id,
            name: row.name,
            agent_scope,
            permissions: row.permissions.0,
            key_prefix: row.key_prefix,
            created_at: row.created_at,
            revoked_at: row.revoked_at,
        })
    }
}

pub async fn migrate(pool: &PgPool) -> Result<(), ApiError> {
    let mut migrator = sqlx::migrate!("./migrations");
    migrator.set_ignore_missing(true);
    migrator.run(pool).await?;
    Ok(())
}

pub async fn ready(pool: &PgPool) -> Result<(), ApiError> {
    sqlx::query("SELECT 1").execute(pool).await?;
    Ok(())
}

pub async fn mark_agent_gateway_sessions_disconnected(pool: &PgPool) -> Result<(), ApiError> {
    sqlx::query(
        "UPDATE agent_gateway_sessions SET
            status = 'disconnected', last_seen_at = NOW(), disconnected_at = NOW()
         WHERE status = 'connected'",
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_projects(pool: &PgPool) -> Result<Vec<ProjectWithBindings>, ApiError> {
    let projects = sqlx::query_as::<_, Project>(
        "SELECT id, owner_user_id, slug, name, description, gateway_id, enabled,
                created_at, updated_at
         FROM projects ORDER BY created_at ASC",
    )
    .fetch_all(pool)
    .await?;
    projects_with_bindings(pool, projects).await
}

pub async fn list_projects_for_owner_user_id(
    pool: &PgPool,
    owner_user_id: &str,
) -> Result<Vec<ProjectWithBindings>, ApiError> {
    let projects = sqlx::query_as::<_, Project>(
        "SELECT id, owner_user_id, slug, name, description, gateway_id, enabled,
                created_at, updated_at
         FROM projects
         WHERE owner_user_id = $1
         ORDER BY created_at ASC",
    )
    .bind(owner_user_id)
    .fetch_all(pool)
    .await?;
    projects_with_bindings(pool, projects).await
}

pub async fn get_project(pool: &PgPool, id: Uuid) -> Result<ProjectWithBindings, ApiError> {
    let project = sqlx::query_as::<_, Project>(
        "SELECT id, owner_user_id, slug, name, description, gateway_id, enabled,
                created_at, updated_at
         FROM projects WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)?;
    Ok(ProjectWithBindings {
        binding_ids: project_binding_ids(pool, project.id).await?,
        project,
    })
}

pub async fn get_project_by_slug(
    pool: &PgPool,
    slug: &str,
) -> Result<ProjectWithBindings, ApiError> {
    let project = sqlx::query_as::<_, Project>(
        "SELECT id, owner_user_id, slug, name, description, gateway_id, enabled,
                created_at, updated_at
         FROM projects WHERE slug = $1",
    )
    .bind(slug)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)?;
    Ok(ProjectWithBindings {
        binding_ids: project_binding_ids(pool, project.id).await?,
        project,
    })
}

pub async fn set_project_owner_user_id(
    pool: &PgPool,
    id: Uuid,
    owner_user_id: &str,
) -> Result<ProjectWithBindings, ApiError> {
    let project = sqlx::query_as::<_, Project>(
        "UPDATE projects
         SET owner_user_id = $2, updated_at = NOW()
         WHERE id = $1
         RETURNING id, owner_user_id, slug, name, description, gateway_id, enabled,
                   created_at, updated_at",
    )
    .bind(id)
    .bind(owner_user_id)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)?;
    Ok(ProjectWithBindings {
        binding_ids: project_binding_ids(pool, project.id).await?,
        project,
    })
}

pub async fn get_project_runtime_extension(
    pool: &PgPool,
    project_id: Uuid,
) -> Result<Option<ProjectRuntimeExtension>, ApiError> {
    sqlx::query_as::<_, ProjectRuntimeExtension>(
        "SELECT project_id, extension_id, enabled, active_release_ref, metadata,
                created_at, updated_at
         FROM project_runtime_extensions
         WHERE project_id = $1",
    )
    .bind(project_id)
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)
}

pub async fn set_project_runtime_extension(
    pool: &PgPool,
    project_id: Uuid,
    extension_id: &str,
    enabled: bool,
    active_release_ref: Option<&str>,
    metadata: &Value,
) -> Result<ProjectRuntimeExtension, ApiError> {
    sqlx::query_as::<_, ProjectRuntimeExtension>(
        "INSERT INTO project_runtime_extensions
            (project_id, extension_id, enabled, active_release_ref, metadata)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (project_id) DO UPDATE SET
            extension_id = EXCLUDED.extension_id,
            enabled = EXCLUDED.enabled,
            active_release_ref = EXCLUDED.active_release_ref,
            metadata = EXCLUDED.metadata,
            updated_at = NOW()
         RETURNING project_id, extension_id, enabled, active_release_ref, metadata,
                   created_at, updated_at",
    )
    .bind(project_id)
    .bind(extension_id)
    .bind(enabled)
    .bind(active_release_ref)
    .bind(metadata)
    .fetch_one(pool)
    .await
    .map_err(map_database_error)
}

pub async fn delete_project_runtime_extension(
    pool: &PgPool,
    project_id: Uuid,
) -> Result<(), ApiError> {
    let result = sqlx::query("DELETE FROM project_runtime_extensions WHERE project_id = $1")
        .bind(project_id)
        .execute(pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

pub async fn create_project_runtime_channel(
    pool: &PgPool,
    input: NewProjectRuntimeChannel<'_>,
) -> Result<ProjectRuntimeChannel, ApiError> {
    sqlx::query_as::<_, ProjectRuntimeChannel>(
        "INSERT INTO project_runtime_channels
            (id, project_id, name, public_id, launch_key_prefix, launch_key_hash, allowed_origins)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         RETURNING id, project_id, name, public_id, launch_key_prefix, allowed_origins,
                   enabled, created_at, updated_at",
    )
    .bind(input.id)
    .bind(input.project_id)
    .bind(input.name)
    .bind(input.public_id)
    .bind(input.launch_key_prefix)
    .bind(input.launch_key_hash)
    .bind(input.allowed_origins)
    .fetch_one(pool)
    .await
    .map_err(map_database_error)
}

pub async fn list_project_runtime_channels(
    pool: &PgPool,
    project_id: Uuid,
) -> Result<Vec<ProjectRuntimeChannel>, ApiError> {
    sqlx::query_as::<_, ProjectRuntimeChannel>(
        "SELECT id, project_id, name, public_id, launch_key_prefix, allowed_origins,
                enabled, created_at, updated_at
         FROM project_runtime_channels
         WHERE project_id = $1
         ORDER BY created_at DESC",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

pub async fn delete_project_runtime_channel(
    pool: &PgPool,
    project_id: Uuid,
    channel_id: Uuid,
) -> Result<(), ApiError> {
    let result =
        sqlx::query("DELETE FROM project_runtime_channels WHERE id = $1 AND project_id = $2")
            .bind(channel_id)
            .bind(project_id)
            .execute(pool)
            .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

pub async fn runtime_channel_for_launch(
    pool: &PgPool,
    project_id: Uuid,
    public_id: Uuid,
    launch_key_hash: &[u8],
) -> Result<ProjectRuntimeChannel, ApiError> {
    sqlx::query_as::<_, ProjectRuntimeChannel>(
        "SELECT id, project_id, name, public_id, launch_key_prefix, allowed_origins,
                enabled, created_at, updated_at
         FROM project_runtime_channels
         WHERE project_id = $1 AND public_id = $2 AND launch_key_hash = $3 AND enabled = TRUE",
    )
    .bind(project_id)
    .bind(public_id)
    .bind(launch_key_hash)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::Unauthorized)
}

pub async fn create_runtime_launch_session(
    pool: &PgPool,
    id: Uuid,
    project_id: Uuid,
    channel_id: Uuid,
    token_hash: &[u8],
    expires_at: DateTime<Utc>,
) -> Result<(), ApiError> {
    sqlx::query(
        "WITH expired AS (
            DELETE FROM runtime_launch_sessions
            WHERE expires_at <= NOW()
         )
         INSERT INTO runtime_launch_sessions
            (id, project_id, channel_id, token_hash, expires_at)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(id)
    .bind(project_id)
    .bind(channel_id)
    .bind(token_hash)
    .bind(expires_at)
    .execute(pool)
    .await
    .map_err(map_database_error)?;
    Ok(())
}

pub async fn active_runtime_launch_project(
    pool: &PgPool,
    token_hash: &[u8],
) -> Result<Option<Uuid>, ApiError> {
    sqlx::query_scalar(
        "SELECT project_id
         FROM runtime_launch_sessions
         WHERE token_hash = $1 AND expires_at > NOW()",
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)
}

pub async fn create_project(
    pool: &PgPool,
    project: NewProject<'_>,
) -> Result<ProjectWithBindings, ApiError> {
    validate_project_bindings(pool, project.gateway_id, project.binding_ids).await?;
    let mut transaction = pool.begin().await?;
    let created = sqlx::query_as::<_, Project>(
        "INSERT INTO projects
            (id, owner_user_id, slug, name, description, gateway_id)
         VALUES ($1, $2, $3, $4, $5, $6)
         RETURNING id, owner_user_id, slug, name, description, gateway_id, enabled,
                   created_at, updated_at",
    )
    .bind(project.id)
    .bind(project.owner_user_id)
    .bind(project.slug)
    .bind(project.name)
    .bind(project.description)
    .bind(project.gateway_id)
    .fetch_one(&mut *transaction)
    .await
    .map_err(map_database_error)?;
    for binding_id in project.binding_ids {
        sqlx::query("INSERT INTO project_bindings (project_id, binding_id) VALUES ($1, $2)")
            .bind(project.id)
            .bind(binding_id)
            .execute(&mut *transaction)
            .await?;
    }
    transaction.commit().await?;
    Ok(ProjectWithBindings {
        project: created,
        binding_ids: project.binding_ids.to_vec(),
    })
}

pub async fn update_project(
    pool: &PgPool,
    id: Uuid,
    patch: ProjectPatch<'_>,
) -> Result<ProjectWithBindings, ApiError> {
    let current = get_project(pool, id).await?;
    let gateway_id = patch.gateway_id.unwrap_or(&current.project.gateway_id);
    if let Some(binding_ids) = patch.binding_ids {
        validate_project_bindings(pool, gateway_id, binding_ids).await?;
    } else if patch.gateway_id.is_some() {
        validate_project_bindings(pool, gateway_id, &current.binding_ids).await?;
    }
    let mut transaction = pool.begin().await?;
    let project = sqlx::query_as::<_, Project>(
        "UPDATE projects SET
            slug = COALESCE($2, slug),
            name = COALESCE($3, name),
            description = CASE WHEN $4 THEN $5 ELSE description END,
            gateway_id = COALESCE($6, gateway_id),
            enabled = COALESCE($7, enabled),
            updated_at = NOW()
         WHERE id = $1
         RETURNING id, owner_user_id, slug, name, description, gateway_id, enabled,
                   created_at, updated_at",
    )
    .bind(id)
    .bind(patch.slug)
    .bind(patch.name)
    .bind(patch.description_changed)
    .bind(patch.description)
    .bind(patch.gateway_id)
    .bind(patch.enabled)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(map_database_error)?
    .ok_or(ApiError::NotFound)?;
    let binding_ids = if let Some(binding_ids) = patch.binding_ids {
        sqlx::query("DELETE FROM project_bindings WHERE project_id = $1")
            .bind(id)
            .execute(&mut *transaction)
            .await?;
        for binding_id in binding_ids {
            sqlx::query("INSERT INTO project_bindings (project_id, binding_id) VALUES ($1, $2)")
                .bind(id)
                .bind(binding_id)
                .execute(&mut *transaction)
                .await?;
        }
        binding_ids.to_vec()
    } else {
        current.binding_ids
    };
    transaction.commit().await?;
    Ok(ProjectWithBindings {
        project,
        binding_ids,
    })
}

pub async fn delete_project(pool: &PgPool, id: Uuid) -> Result<(), ApiError> {
    delete_by_id(pool, "projects", id).await
}

pub async fn list_provider_connections(
    pool: &PgPool,
    project_slug: &str,
) -> Result<Vec<ProviderConnection>, ApiError> {
    let project = get_project_by_slug(pool, project_slug).await?;
    let connections = sqlx::query_as::<_, ProviderConnection>(
        "SELECT id, project_id, provider_key, source_kind, source_key,
                name, provider_type, base_url, config,
                secret_keys, display_secret, status, last_checked_at, created_at, updated_at
         FROM provider_connections
         WHERE project_id = $1
         ORDER BY created_at ASC",
    )
    .bind(project.project.id)
    .fetch_all(pool)
    .await?;
    Ok(connections)
}

pub async fn upsert_provider_connection(
    pool: &PgPool,
    project_slug: &str,
    connection: NewProviderConnection<'_>,
) -> Result<ProviderConnection, ApiError> {
    let project = get_project_by_slug(pool, project_slug).await?;
    sqlx::query_as::<_, ProviderConnection>(
        "INSERT INTO provider_connections
            (id, project_id, provider_key, source_kind, source_key,
             name, provider_type, base_url, config,
             encrypted_secret_json, secret_keys, display_secret, status)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
         ON CONFLICT (project_id, provider_key) DO UPDATE SET
            source_kind = EXCLUDED.source_kind,
            source_key = EXCLUDED.source_key,
            name = EXCLUDED.name,
            provider_type = EXCLUDED.provider_type,
            base_url = EXCLUDED.base_url,
            config = EXCLUDED.config,
            encrypted_secret_json = EXCLUDED.encrypted_secret_json,
            secret_keys = EXCLUDED.secret_keys,
            display_secret = EXCLUDED.display_secret,
            status = EXCLUDED.status,
            updated_at = NOW()
         RETURNING id, project_id, provider_key, source_kind, source_key,
                   name, provider_type, base_url, config,
                   secret_keys, display_secret, status, last_checked_at, created_at, updated_at",
    )
    .bind(Uuid::new_v4())
    .bind(project.project.id)
    .bind(connection.provider_key)
    .bind(connection.source_kind)
    .bind(connection.source_key)
    .bind(connection.name)
    .bind(connection.provider_type)
    .bind(connection.base_url)
    .bind(connection.config)
    .bind(connection.encrypted_secret_json)
    .bind(connection.secret_keys)
    .bind(connection.display_secret)
    .bind(connection.status)
    .fetch_one(pool)
    .await
    .map_err(map_database_error)
}

pub async fn get_provider_connection_secret(
    pool: &PgPool,
    id: Uuid,
) -> Result<ProviderConnectionSecret, ApiError> {
    sqlx::query_as::<_, ProviderConnectionSecret>(
        "SELECT id, project_id, provider_key, source_kind, source_key,
                name, provider_type, base_url, config,
                encrypted_secret_json, secret_keys, display_secret, status,
                last_checked_at, created_at, updated_at
         FROM provider_connections
         WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)
}

pub async fn get_provider_connection_secret_by_key(
    pool: &PgPool,
    project_slug: &str,
    provider_key: &str,
) -> Result<ProviderConnectionSecret, ApiError> {
    let project = get_project_by_slug(pool, project_slug).await?;
    sqlx::query_as::<_, ProviderConnectionSecret>(
        "SELECT id, project_id, provider_key, source_kind, source_key,
                name, provider_type, base_url, config,
                encrypted_secret_json, secret_keys, display_secret, status,
                last_checked_at, created_at, updated_at
         FROM provider_connections
         WHERE project_id = $1 AND provider_key = $2",
    )
    .bind(project.project.id)
    .bind(provider_key)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)
}

pub async fn update_provider_connection_status(
    pool: &PgPool,
    id: Uuid,
    status: &str,
) -> Result<ProviderConnection, ApiError> {
    sqlx::query_as::<_, ProviderConnection>(
        "UPDATE provider_connections SET status = $2, last_checked_at = NOW(), updated_at = NOW()
         WHERE id = $1
         RETURNING id, project_id, provider_key, source_kind, source_key,
                   name, provider_type, base_url, config,
                   secret_keys, display_secret, status, last_checked_at, created_at, updated_at",
    )
    .bind(id)
    .bind(status)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)
}

pub async fn list_custom_providers(pool: &PgPool) -> Result<Vec<CustomProvider>, ApiError> {
    sqlx::query_as::<_, CustomProvider>(
        "SELECT id, provider_key, name, provider_type, base_url, config,
                secret_keys, display_secret, status, last_checked_at, created_at, updated_at
         FROM custom_providers
         ORDER BY name ASC, provider_key ASC",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::from)
}

pub async fn upsert_custom_provider(
    pool: &PgPool,
    connection: NewProviderConnection<'_>,
) -> Result<CustomProvider, ApiError> {
    sqlx::query_as::<_, CustomProvider>(
        "INSERT INTO custom_providers
            (id, provider_key, name, provider_type, base_url, config,
             encrypted_secret_json, secret_keys, display_secret, status)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
         ON CONFLICT (provider_key) DO UPDATE SET
            name = EXCLUDED.name,
            provider_type = EXCLUDED.provider_type,
            base_url = EXCLUDED.base_url,
            config = EXCLUDED.config,
            encrypted_secret_json = EXCLUDED.encrypted_secret_json,
            secret_keys = EXCLUDED.secret_keys,
            display_secret = EXCLUDED.display_secret,
            status = EXCLUDED.status,
            updated_at = NOW()
         RETURNING id, provider_key, name, provider_type, base_url, config,
                   secret_keys, display_secret, status, last_checked_at, created_at, updated_at",
    )
    .bind(Uuid::new_v4())
    .bind(connection.provider_key)
    .bind(connection.name)
    .bind(connection.provider_type)
    .bind(connection.base_url)
    .bind(connection.config)
    .bind(connection.encrypted_secret_json)
    .bind(connection.secret_keys)
    .bind(connection.display_secret)
    .bind(connection.status)
    .fetch_one(pool)
    .await
    .map_err(map_database_error)
}

pub async fn get_custom_provider_secret_by_key(
    pool: &PgPool,
    provider_key: &str,
) -> Result<CustomProviderSecret, ApiError> {
    sqlx::query_as::<_, CustomProviderSecret>(
        "SELECT id, provider_key, name, provider_type, base_url, config,
                encrypted_secret_json, secret_keys, display_secret, status,
                last_checked_at, created_at, updated_at
         FROM custom_providers
         WHERE provider_key = $1",
    )
    .bind(provider_key)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)
}

pub async fn project_provider_is_assigned(
    pool: &PgPool,
    project_id: Uuid,
    provider_key: &str,
) -> Result<bool, ApiError> {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(
            SELECT 1 FROM provider_connections
            WHERE project_id = $1 AND provider_key = $2
         )",
    )
    .bind(project_id)
    .bind(provider_key)
    .fetch_one(pool)
    .await
    .map_err(ApiError::from)
}

pub async fn list_projects_for_provider_key(
    pool: &PgPool,
    provider_key: &str,
) -> Result<Vec<(Uuid, String)>, ApiError> {
    sqlx::query_as::<_, (Uuid, String)>(
        "SELECT project.id, project.slug
         FROM provider_connections AS provider
         JOIN projects AS project ON project.id = provider.project_id
         WHERE provider.provider_key = $1 AND project.enabled = TRUE
         ORDER BY project.created_at ASC",
    )
    .bind(provider_key)
    .fetch_all(pool)
    .await
    .map_err(ApiError::from)
}

pub async fn list_project_profile_provider_resources(
    pool: &PgPool,
    project_id: Uuid,
) -> Result<Vec<(String, String)>, ApiError> {
    sqlx::query_as::<_, (String, String)>(
        "SELECT DISTINCT capability.provider_key,
                COALESCE(capability.resource_id, version.source->>'resourceId') AS resource_id
         FROM agent_profiles AS profile
         JOIN agent_profile_versions AS version ON version.id = profile.active_version_id
         JOIN agent_profile_capabilities AS capability
           ON capability.profile_version_id = version.id
         WHERE profile.project_id = $1
           AND profile.archived_at IS NULL
           AND COALESCE(capability.resource_id, version.source->>'resourceId') IS NOT NULL",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await
    .map_err(ApiError::from)
}

pub async fn list_archived_project_agent_sources(
    pool: &PgPool,
    project_id: Uuid,
) -> Result<Vec<crate::models::ArchivedProjectAgentSource>, ApiError> {
    sqlx::query_as::<_, crate::models::ArchivedProjectAgentSource>(
        "SELECT profile.id AS profile_id,
                profile.name,
                COALESCE(binding.gateway_id, '') AS gateway_id,
                COALESCE(binding.agent_id, version.source->>'resourceId', profile.slug)
                    AS agent_id,
                COALESCE(
                    NULLIF(binding.config->>'providerKey', ''),
                    capability.provider_key,
                    version.source->>'providerKey',
                    ''
                ) AS provider_key,
                COALESCE(binding.provider, capability.provider_type, version.source->>'type', 'custom')
                    AS provider_type
         FROM agent_profiles AS profile
         LEFT JOIN agent_bindings AS binding ON binding.profile_id = profile.id
         LEFT JOIN agent_profile_versions AS version ON version.id = profile.active_version_id
         LEFT JOIN LATERAL (
             SELECT provider_key, provider_type
             FROM agent_profile_capabilities
             WHERE profile_version_id = version.id
             ORDER BY created_at ASC
             LIMIT 1
         ) AS capability ON TRUE
         WHERE profile.project_id = $1
           AND profile.archived_at IS NOT NULL
         ORDER BY profile.updated_at DESC, profile.name ASC",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await
    .map_err(ApiError::from)
}

pub async fn restore_project_profile(
    pool: &PgPool,
    project_id: Uuid,
    profile_id: Uuid,
) -> Result<AgentProfile, ApiError> {
    let mut transaction = pool.begin().await?;
    let profile = sqlx::query_as::<_, AgentProfile>(
        "UPDATE agent_profiles
         SET archived_at = NULL, updated_at = NOW()
         WHERE id = $1 AND project_id = $2 AND archived_at IS NOT NULL
         RETURNING id, project_id, slug, name, description, active_version_id, archived_at,
                   created_at, updated_at",
    )
    .bind(profile_id)
    .bind(project_id)
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or(ApiError::NotFound)?;
    sqlx::query(
        "INSERT INTO project_bindings (project_id, binding_id)
         SELECT $1, id FROM agent_bindings WHERE profile_id = $2
         ON CONFLICT DO NOTHING",
    )
    .bind(project_id)
    .bind(profile_id)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(profile)
}

pub async fn find_project_profile_by_provider_resource(
    pool: &PgPool,
    project_id: Uuid,
    provider_key: &str,
    agent_id: &str,
) -> Result<Option<(Uuid, bool, Uuid)>, ApiError> {
    sqlx::query_as::<_, (Uuid, bool, Uuid)>(
        "SELECT profile.id,
                profile.archived_at IS NOT NULL AS archived,
                binding.id
         FROM agent_profiles AS profile
         JOIN agent_bindings AS binding ON binding.profile_id = profile.id
         WHERE profile.project_id = $1
           AND binding.agent_id = $2
           AND COALESCE(NULLIF(binding.config->>'providerKey', ''), binding.provider) = $3
         ORDER BY profile.archived_at NULLS FIRST, profile.created_at ASC
         LIMIT 1",
    )
    .bind(project_id)
    .bind(agent_id)
    .bind(provider_key)
    .fetch_optional(pool)
    .await
    .map_err(ApiError::from)
}

pub async fn refresh_discovered_binding(
    pool: &PgPool,
    binding_id: Uuid,
    gateway_id: &str,
    agent_name: &str,
) -> Result<(), ApiError> {
    sqlx::query(
        "UPDATE agent_bindings
         SET gateway_id = $2,
             config = jsonb_set(config, '{agentName}', to_jsonb($3::text), true),
             updated_at = NOW()
         WHERE id = $1",
    )
    .bind(binding_id)
    .bind(gateway_id)
    .bind(agent_name)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn unassign_project_provider(
    pool: &PgPool,
    project_id: Uuid,
    provider_key: &str,
) -> Result<(), ApiError> {
    let referenced = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(
            SELECT 1
            FROM agent_profiles AS profile
            JOIN agent_profile_versions AS version
              ON version.id = profile.active_version_id
            LEFT JOIN agent_profile_capabilities AS capability
              ON capability.profile_version_id = version.id
            WHERE profile.project_id = $1
              AND profile.archived_at IS NULL
              AND (
                   capability.provider_key = $2
                   OR version.source->>'providerKey' = $2
              )
         )",
    )
    .bind(project_id)
    .bind(provider_key)
    .fetch_one(pool)
    .await?;
    if referenced {
        return Err(ApiError::Conflict(
            "Remove or move the agents using this provider before disconnecting it from the project."
                .to_string(),
        ));
    }
    let result = sqlx::query(
        "DELETE FROM provider_connections
         WHERE project_id = $1 AND provider_key = $2",
    )
    .bind(project_id)
    .bind(provider_key)
    .execute(pool)
    .await?;
    if result.rows_affected() == 0 {
        Err(ApiError::NotFound)
    } else {
        Ok(())
    }
}

async fn projects_with_bindings(
    pool: &PgPool,
    projects: Vec<Project>,
) -> Result<Vec<ProjectWithBindings>, ApiError> {
    let mut result = Vec::with_capacity(projects.len());
    for project in projects {
        result.push(ProjectWithBindings {
            binding_ids: project_binding_ids(pool, project.id).await?,
            project,
        });
    }
    Ok(result)
}

async fn project_binding_ids(pool: &PgPool, project_id: Uuid) -> Result<Vec<Uuid>, ApiError> {
    sqlx::query_scalar(
        "SELECT binding_id FROM project_bindings WHERE project_id = $1 ORDER BY created_at ASC",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await
    .map_err(ApiError::from)
}

pub async fn assign_project_binding(
    pool: &PgPool,
    project_id: Uuid,
    binding_id: Uuid,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO project_bindings (project_id, binding_id)
         VALUES ($1, $2)
         ON CONFLICT DO NOTHING",
    )
    .bind(project_id)
    .bind(binding_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn attach_project_binding(
    pool: &PgPool,
    project_id: Uuid,
    binding_id: Uuid,
) -> Result<(), ApiError> {
    validate_project_bindings(pool, "", &[binding_id]).await?;
    sqlx::query(
        "INSERT INTO project_bindings (project_id, binding_id)
         VALUES ($1, $2)
         ON CONFLICT DO NOTHING",
    )
    .bind(project_id)
    .bind(binding_id)
    .execute(pool)
    .await?;
    Ok(())
}

async fn validate_project_bindings(
    pool: &PgPool,
    _gateway_id: &str,
    binding_ids: &[Uuid],
) -> Result<(), ApiError> {
    if binding_ids.len() > 256 {
        return Err(ApiError::Invalid(
            "a project supports at most 256 bindings".to_string(),
        ));
    }
    let count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM agent_bindings
         WHERE id = ANY($1)",
    )
    .bind(binding_ids)
    .fetch_one(pool)
    .await?;
    if usize::try_from(count).ok() == Some(binding_ids.len()) {
        Ok(())
    } else {
        Err(ApiError::Invalid("project bindings must exist".to_string()))
    }
}

pub async fn list_profiles(pool: &PgPool) -> Result<Vec<AgentProfile>, ApiError> {
    sqlx::query_as::<_, AgentProfile>(
        "SELECT id, project_id, slug, name, description, active_version_id, archived_at,
                created_at, updated_at
         FROM agent_profiles
         WHERE archived_at IS NULL
         ORDER BY created_at ASC",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::from)
}

pub async fn list_project_profiles(
    pool: &PgPool,
    project_id: Uuid,
) -> Result<Vec<AgentProfile>, ApiError> {
    sqlx::query_as::<_, AgentProfile>(
        "SELECT id, project_id, slug, name, description, active_version_id, archived_at,
                created_at, updated_at
         FROM agent_profiles
         WHERE project_id = $1 AND archived_at IS NULL
         ORDER BY created_at ASC",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await
    .map_err(ApiError::from)
}

pub async fn get_profile(pool: &PgPool, id: Uuid) -> Result<AgentProfile, ApiError> {
    sqlx::query_as::<_, AgentProfile>(
        "SELECT id, project_id, slug, name, description, active_version_id, archived_at,
                created_at, updated_at
         FROM agent_profiles WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)
}

pub async fn get_project_profile(
    pool: &PgPool,
    project_id: Uuid,
    id: Uuid,
) -> Result<AgentProfile, ApiError> {
    sqlx::query_as::<_, AgentProfile>(
        "SELECT id, project_id, slug, name, description, active_version_id, archived_at,
                created_at, updated_at
         FROM agent_profiles
         WHERE id = $1 AND project_id = $2 AND archived_at IS NULL",
    )
    .bind(id)
    .bind(project_id)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)
}

pub async fn create_profile(
    pool: &PgPool,
    id: Uuid,
    project_id: Uuid,
    slug: &str,
    name: &str,
    description: Option<&str>,
) -> Result<AgentProfile, ApiError> {
    sqlx::query_as::<_, AgentProfile>(
        "INSERT INTO agent_profiles (id, project_id, slug, name, description)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING id, project_id, slug, name, description, active_version_id, archived_at,
                   created_at, updated_at",
    )
    .bind(id)
    .bind(project_id)
    .bind(slug)
    .bind(name)
    .bind(description)
    .fetch_one(pool)
    .await
    .map_err(map_database_error)
}

pub async fn update_profile(
    pool: &PgPool,
    id: Uuid,
    patch: ProfilePatch<'_>,
) -> Result<AgentProfile, ApiError> {
    sqlx::query_as::<_, AgentProfile>(
        "UPDATE agent_profiles SET
            slug = COALESCE($2, slug),
            name = COALESCE($3, name),
            description = CASE WHEN $4 THEN $5 ELSE description END,
            updated_at = NOW()
         WHERE id = $1
         RETURNING id, project_id, slug, name, description, active_version_id, archived_at,
                   created_at, updated_at",
    )
    .bind(id)
    .bind(patch.slug)
    .bind(patch.name)
    .bind(patch.description_changed)
    .bind(patch.description)
    .fetch_optional(pool)
    .await
    .map_err(map_database_error)?
    .ok_or(ApiError::NotFound)
}

pub async fn delete_profile(pool: &PgPool, id: Uuid) -> Result<(), ApiError> {
    let result = sqlx::query(
        "UPDATE agent_profiles
         SET archived_at = COALESCE(archived_at, NOW()), updated_at = NOW()
         WHERE id = $1",
    )
    .bind(id)
    .execute(pool)
    .await?;
    if result.rows_affected() == 0 {
        Err(ApiError::NotFound)
    } else {
        Ok(())
    }
}

pub async fn archive_project_profile(
    pool: &PgPool,
    project_id: Uuid,
    profile_id: Uuid,
) -> Result<(), ApiError> {
    let mut transaction = pool.begin().await?;
    let archived = sqlx::query(
        "UPDATE agent_profiles
         SET archived_at = COALESCE(archived_at, NOW()), updated_at = NOW()
         WHERE id = $1 AND project_id = $2",
    )
    .bind(profile_id)
    .bind(project_id)
    .execute(&mut *transaction)
    .await?;
    if archived.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }

    sqlx::query(
        "UPDATE agent_endpoints
         SET enabled = FALSE, updated_at = NOW()
         WHERE profile_id = $1",
    )
    .bind(profile_id)
    .execute(&mut *transaction)
    .await?;

    sqlx::query(
        "DELETE FROM project_bindings AS project_binding
         USING agent_bindings AS binding
         WHERE project_binding.project_id = $1
           AND project_binding.binding_id = binding.id
           AND binding.profile_id = $2",
    )
    .bind(project_id)
    .bind(profile_id)
    .execute(&mut *transaction)
    .await?;

    transaction.commit().await?;
    Ok(())
}

pub async fn create_profile_version(
    pool: &PgPool,
    profile_id: Uuid,
    input: NewProfileVersion<'_>,
) -> Result<AgentProfileVersion, ApiError> {
    let content_hash = profile_version_content_hash(&input)?;
    let mut transaction = pool.begin().await?;
    let active_version_id = sqlx::query_scalar::<_, Option<Uuid>>(
        "SELECT active_version_id FROM agent_profiles WHERE id = $1 FOR UPDATE",
    )
    .bind(profile_id)
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or(ApiError::NotFound)?;
    let version_number = sqlx::query_scalar::<_, i32>(
        "SELECT COALESCE(MAX(version_number), 0) + 1
         FROM agent_profile_versions
         WHERE profile_id = $1",
    )
    .bind(profile_id)
    .fetch_one(&mut *transaction)
    .await?;
    let version_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO agent_profile_versions
            (id, profile_id, version_number, persona, runtime, presentation, source,
             content_hash, change_summary)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(version_id)
    .bind(profile_id)
    .bind(version_number)
    .bind(input.persona)
    .bind(input.runtime)
    .bind(input.presentation)
    .bind(input.source)
    .bind(content_hash)
    .bind(input.change_summary)
    .execute(&mut *transaction)
    .await
    .map_err(map_database_error)?;
    for capability in input.capabilities {
        sqlx::query(
            "INSERT INTO agent_profile_capabilities
                (id, profile_version_id, kind, provider_type, provider_key, resource_id,
                 config, input_schema, output_schema)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(Uuid::new_v4())
        .bind(version_id)
        .bind(&capability.kind)
        .bind(&capability.provider_type)
        .bind(&capability.provider_key)
        .bind(&capability.resource_id)
        .bind(&capability.config)
        .bind(&capability.input_schema)
        .bind(&capability.output_schema)
        .execute(&mut *transaction)
        .await
        .map_err(map_database_error)?;
    }
    if active_version_id.is_none() {
        sqlx::query(
            "UPDATE agent_profiles
             SET active_version_id = $2, updated_at = NOW()
             WHERE id = $1",
        )
        .bind(profile_id)
        .bind(version_id)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "INSERT INTO agent_profile_rollouts
                (profile_id, profile_version_id, weight_bps)
             VALUES ($1, $2, 10000)",
        )
        .bind(profile_id)
        .bind(version_id)
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;
    get_profile_version(pool, profile_id, version_id).await
}

pub async fn list_profile_versions(
    pool: &PgPool,
    profile_id: Uuid,
) -> Result<Vec<AgentProfileVersion>, ApiError> {
    sqlx::query_as::<_, AgentProfileVersion>(
        "SELECT id, profile_id, version_number, persona, runtime, presentation, source,
                content_hash, change_summary, archived_at, created_at
         FROM agent_profile_versions
         WHERE profile_id = $1
         ORDER BY version_number DESC",
    )
    .bind(profile_id)
    .fetch_all(pool)
    .await
    .map_err(ApiError::from)
}

pub async fn get_profile_version(
    pool: &PgPool,
    profile_id: Uuid,
    version_id: Uuid,
) -> Result<AgentProfileVersion, ApiError> {
    sqlx::query_as::<_, AgentProfileVersion>(
        "SELECT id, profile_id, version_number, persona, runtime, presentation, source,
                content_hash, change_summary, archived_at, created_at
         FROM agent_profile_versions
         WHERE id = $1 AND profile_id = $2",
    )
    .bind(version_id)
    .bind(profile_id)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)
}

pub async fn list_profile_capabilities(
    pool: &PgPool,
    version_id: Uuid,
) -> Result<Vec<AgentProfileCapability>, ApiError> {
    sqlx::query_as::<_, AgentProfileCapability>(
        "SELECT id, profile_version_id, kind, provider_type, provider_key, resource_id,
                config, input_schema, output_schema, created_at
         FROM agent_profile_capabilities
         WHERE profile_version_id = $1
         ORDER BY created_at ASC",
    )
    .bind(version_id)
    .fetch_all(pool)
    .await
    .map_err(ApiError::from)
}

pub async fn list_profile_rollout(
    pool: &PgPool,
    profile_id: Uuid,
) -> Result<Vec<AgentProfileRollout>, ApiError> {
    sqlx::query_as::<_, AgentProfileRollout>(
        "SELECT profile_id, profile_version_id, weight_bps, created_at, updated_at
         FROM agent_profile_rollouts
         WHERE profile_id = $1
         ORDER BY weight_bps DESC, created_at ASC",
    )
    .bind(profile_id)
    .fetch_all(pool)
    .await
    .map_err(ApiError::from)
}

pub async fn set_profile_rollout(
    pool: &PgPool,
    profile_id: Uuid,
    allocations: &[(Uuid, i32)],
) -> Result<Vec<AgentProfileRollout>, ApiError> {
    if allocations.is_empty() || allocations.len() > 10 {
        return Err(ApiError::Invalid(
            "a rollout requires between 1 and 10 versions".to_string(),
        ));
    }
    let mut unique = HashSet::new();
    let total = allocations
        .iter()
        .try_fold(0_i32, |total, (version_id, weight)| {
            if *weight <= 0 || !unique.insert(*version_id) {
                return Err(ApiError::Invalid(
                    "rollout versions must be unique and have a positive weight".to_string(),
                ));
            }
            total
                .checked_add(*weight)
                .ok_or_else(|| ApiError::Invalid("rollout weights are invalid".to_string()))
        })?;
    if total != 10_000 {
        return Err(ApiError::Invalid(
            "rollout weights must total 10000 basis points".to_string(),
        ));
    }

    let version_ids = allocations
        .iter()
        .map(|(version_id, _)| *version_id)
        .collect::<Vec<_>>();
    let versions = sqlx::query_as::<_, AgentProfileVersion>(
        "SELECT id, profile_id, version_number, persona, runtime, presentation, source,
                content_hash, change_summary, archived_at, created_at
         FROM agent_profile_versions
         WHERE profile_id = $1 AND id = ANY($2) AND archived_at IS NULL",
    )
    .bind(profile_id)
    .bind(&version_ids)
    .fetch_all(pool)
    .await?;
    if versions.len() != allocations.len() {
        return Err(ApiError::Invalid(
            "rollout versions must be active versions of this profile".to_string(),
        ));
    }
    let source_managed = versions.iter().any(|version| {
        version.source.get("type").and_then(Value::as_str) == Some("openclaw")
            && version
                .source
                .get("managed")
                .and_then(Value::as_bool)
                .unwrap_or(true)
    });
    if source_managed && allocations.len() != 1 {
        return Err(ApiError::Conflict(
            "source-managed OpenClaw profiles support one active version at a time".to_string(),
        ));
    }

    let active_version_id = allocations
        .iter()
        .max_by_key(|(_, weight)| *weight)
        .map(|(version_id, _)| *version_id)
        .ok_or(ApiError::Internal)?;
    let mut transaction = pool.begin().await?;
    sqlx::query("DELETE FROM agent_profile_rollouts WHERE profile_id = $1")
        .bind(profile_id)
        .execute(&mut *transaction)
        .await?;
    for (version_id, weight_bps) in allocations {
        sqlx::query(
            "INSERT INTO agent_profile_rollouts
                (profile_id, profile_version_id, weight_bps)
             VALUES ($1, $2, $3)",
        )
        .bind(profile_id)
        .bind(version_id)
        .bind(weight_bps)
        .execute(&mut *transaction)
        .await?;
    }
    sqlx::query(
        "UPDATE agent_profiles
         SET active_version_id = $2, updated_at = NOW()
         WHERE id = $1",
    )
    .bind(profile_id)
    .bind(active_version_id)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    list_profile_rollout(pool, profile_id).await
}

pub async fn archive_profile_version(
    pool: &PgPool,
    profile_id: Uuid,
    version_id: Uuid,
) -> Result<AgentProfileVersion, ApiError> {
    let in_rollout = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(
            SELECT 1 FROM agent_profile_rollouts
            WHERE profile_id = $1 AND profile_version_id = $2
         )",
    )
    .bind(profile_id)
    .bind(version_id)
    .fetch_one(pool)
    .await?;
    if in_rollout {
        return Err(ApiError::Conflict(
            "an active rollout version cannot be archived".to_string(),
        ));
    }
    sqlx::query(
        "UPDATE agent_profile_versions
         SET archived_at = COALESCE(archived_at, NOW())
         WHERE id = $1 AND profile_id = $2",
    )
    .bind(version_id)
    .bind(profile_id)
    .execute(pool)
    .await?;
    get_profile_version(pool, profile_id, version_id).await
}

fn profile_version_content_hash(input: &NewProfileVersion<'_>) -> Result<String, ApiError> {
    let payload = json!({
        "persona": input.persona,
        "runtime": input.runtime,
        "presentation": input.presentation,
        "source": input.source,
        "capabilities": input.capabilities,
    });
    let encoded = serde_json::to_vec(&payload).map_err(|_| ApiError::Internal)?;
    let digest = Sha256::digest(encoded);
    let mut hash = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(&mut hash, "{byte:02x}").map_err(|_| ApiError::Internal)?;
    }
    Ok(hash)
}

pub async fn resolve_profile_route(
    pool: &PgPool,
    project_id: Uuid,
    model: &str,
    capability_kind: &str,
    selection_key: Option<&str>,
    version_id: Option<Uuid>,
) -> Result<ProfileRoute, ApiError> {
    let profile_id = if let Ok(profile_id) = Uuid::parse_str(model) {
        sqlx::query_scalar::<_, Uuid>(
            "SELECT profile.id
             FROM agent_profiles AS profile
             WHERE profile.id = $1
               AND profile.project_id = $2
               AND profile.archived_at IS NULL",
        )
        .bind(profile_id)
        .bind(project_id)
        .fetch_optional(pool)
        .await?
        .ok_or(ApiError::NotFound)?
    } else {
        sqlx::query_scalar::<_, Uuid>(
            "SELECT profile.id
             FROM agent_profiles AS profile
             WHERE profile.project_id = $1
               AND profile.slug = $2
               AND profile.archived_at IS NULL",
        )
        .bind(project_id)
        .bind(model)
        .fetch_optional(pool)
        .await?
        .ok_or(ApiError::NotFound)?
    };
    let version_id = match version_id {
        Some(version_id) => {
            get_profile_version(pool, profile_id, version_id).await?;
            version_id
        }
        None => select_profile_version(pool, profile_id, selection_key).await?,
    };
    sqlx::query_as::<_, ProfileRoute>(
        "SELECT profile.id AS profile_id,
                profile.slug AS profile_slug,
                profile.name AS profile_name,
                version.id AS profile_version_id,
                version.version_number,
                capability.id AS capability_id,
                capability.kind AS capability_kind,
                capability.provider_type,
                capability.provider_key,
                capability.resource_id,
                capability.config AS capability_config,
                version.persona,
                version.runtime,
                version.presentation,
                version.source
         FROM agent_profiles AS profile
         JOIN agent_profile_versions AS version
           ON version.id = $2
          AND version.profile_id = profile.id
          AND version.archived_at IS NULL
         JOIN agent_profile_capabilities AS capability
           ON capability.profile_version_id = version.id
          AND capability.kind = $3
         WHERE profile.id = $1
         ORDER BY capability.created_at ASC
         LIMIT 1",
    )
    .bind(profile_id)
    .bind(version_id)
    .bind(capability_kind)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)
}

pub async fn list_public_agents(
    pool: &PgPool,
    project_id: Uuid,
    allowed_profile_ids: Option<&[Uuid]>,
) -> Result<Vec<PublicAgent>, ApiError> {
    let profiles = list_project_profiles(pool, project_id).await?;
    let mut agents = Vec::new();
    for profile in profiles {
        if allowed_profile_ids.is_some_and(|ids| !ids.contains(&profile.id)) {
            continue;
        }
        let Some(version_id) = profile.active_version_id else {
            continue;
        };
        let version = get_profile_version(pool, profile.id, version_id).await?;
        let capabilities = list_profile_capabilities(pool, version.id)
            .await?
            .into_iter()
            .map(|capability| capability.kind)
            .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        agents.push(PublicAgent {
            id: profile.id,
            slug: profile.slug,
            name: profile.name,
            description: profile.description,
            version: version.version_number,
            capabilities,
            presentation: version.presentation,
        });
    }
    agents.sort_by(|left, right| left.slug.cmp(&right.slug));
    Ok(agents)
}

async fn select_profile_version(
    pool: &PgPool,
    profile_id: Uuid,
    selection_key: Option<&str>,
) -> Result<Uuid, ApiError> {
    let rollout = list_profile_rollout(pool, profile_id).await?;
    if rollout.is_empty() {
        return get_profile(pool, profile_id)
            .await?
            .active_version_id
            .ok_or(ApiError::NotFound);
    }
    let mut hasher = Sha256::new();
    hasher.update(profile_id.as_bytes());
    hasher.update(selection_key.unwrap_or("anonymous").as_bytes());
    let digest = hasher.finalize();
    let bucket = u16::from_be_bytes([digest[0], digest[1]]) as i32 % 10_000;
    let mut cursor = 0_i32;
    for allocation in &rollout {
        cursor += allocation.weight_bps;
        if bucket < cursor {
            return Ok(allocation.profile_version_id);
        }
    }
    rollout
        .last()
        .map(|allocation| allocation.profile_version_id)
        .ok_or(ApiError::NotFound)
}

pub async fn list_bindings(pool: &PgPool) -> Result<Vec<AgentBinding>, ApiError> {
    sqlx::query_as::<_, AgentBinding>(
        "SELECT id, profile_id, provider, gateway_id, agent_id, config, created_at, updated_at
         FROM agent_bindings ORDER BY created_at ASC",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::from)
}

pub async fn get_binding(pool: &PgPool, id: Uuid) -> Result<AgentBinding, ApiError> {
    sqlx::query_as::<_, AgentBinding>(
        "SELECT id, profile_id, provider, gateway_id, agent_id, config, created_at, updated_at
         FROM agent_bindings WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)
}

pub async fn create_binding(
    pool: &PgPool,
    id: Uuid,
    profile_id: Uuid,
    provider: &str,
    gateway_id: &str,
    agent_id: &str,
    config: &Value,
) -> Result<AgentBinding, ApiError> {
    sqlx::query_as::<_, AgentBinding>(
        "INSERT INTO agent_bindings
            (id, profile_id, provider, gateway_id, agent_id, config)
         VALUES ($1, $2, $3, $4, $5, $6)
         RETURNING id, profile_id, provider, gateway_id, agent_id, config, created_at, updated_at",
    )
    .bind(id)
    .bind(profile_id)
    .bind(provider)
    .bind(gateway_id)
    .bind(agent_id)
    .bind(config)
    .fetch_one(pool)
    .await
    .map_err(map_database_error)
}

pub async fn ensure_discovered_binding(
    pool: &PgPool,
    project_id: Uuid,
    gateway_id: &str,
    agent_id: &str,
    agent_name: &str,
    provider_key: &str,
) -> Result<Uuid, ApiError> {
    if let Some(binding) =
        find_binding_by_agent_gateway_agent(pool, project_id, gateway_id, agent_id, provider_key)
            .await?
    {
        assign_project_binding(pool, project_id, binding.id).await?;
        return Ok(binding.id);
    }

    let display_name = agent_name.trim();
    let display_name = if display_name.is_empty() {
        agent_id
    } else {
        display_name
    };
    let provider_key =
        if vifu_gateway::protocol::validate_identifier("provider key", provider_key).is_ok() {
            provider_key
        } else {
            "openclaw"
        };
    let slug = unique_profile_slug(
        pool,
        project_id,
        &discovered_profile_slug(gateway_id, agent_id, display_name),
    )
    .await?;
    let profile_id = Uuid::new_v4();
    let binding_id = Uuid::new_v4();
    let version_id = Uuid::new_v4();
    let binding_config = json!({
        "source": "openclaw-discovery",
        "agentName": display_name,
        "providerKey": provider_key,
    });
    let persona = json!({ "files": {} });
    let runtime = json!({});
    let presentation = json!({});
    let source = json!({
        "type": "openclaw",
        "providerKey": provider_key,
        "gatewayId": gateway_id,
        "resourceId": agent_id,
        "managed": true,
    });
    let capability = ProfileCapabilityDraft {
        kind: "chat".to_string(),
        provider_type: "openclaw".to_string(),
        provider_key: provider_key.to_string(),
        resource_id: Some(agent_id.to_string()),
        config: json!({
            "gatewayId": gateway_id,
            "source": "openclaw-discovery",
        }),
        input_schema: json!({}),
        output_schema: json!({}),
    };
    let capabilities = [capability];
    let version_input = NewProfileVersion {
        persona: &persona,
        runtime: &runtime,
        presentation: &presentation,
        source: &source,
        capabilities: &capabilities,
        change_summary: Some("Discovered from OpenClaw"),
    };
    let content_hash = profile_version_content_hash(&version_input)?;
    let mut transaction = pool.begin().await?;
    sqlx::query(
        "INSERT INTO agent_profiles (id, project_id, slug, name, description)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(profile_id)
    .bind(project_id)
    .bind(&slug)
    .bind(display_name)
    .bind("Discovered from OpenClaw")
    .execute(&mut *transaction)
    .await
    .map_err(map_database_error)?;
    sqlx::query(
        "INSERT INTO agent_bindings
            (id, profile_id, provider, gateway_id, agent_id, config)
         VALUES ($1, $2, 'openclaw', $3, $4, $5)",
    )
    .bind(binding_id)
    .bind(profile_id)
    .bind(gateway_id)
    .bind(agent_id)
    .bind(&binding_config)
    .execute(&mut *transaction)
    .await
    .map_err(map_database_error)?;
    sqlx::query(
        "INSERT INTO agent_profile_versions
            (id, profile_id, version_number, persona, runtime, presentation, source,
             content_hash, change_summary)
         VALUES ($1, $2, 1, $3, $4, $5, $6, $7, $8)",
    )
    .bind(version_id)
    .bind(profile_id)
    .bind(&persona)
    .bind(&runtime)
    .bind(&presentation)
    .bind(&source)
    .bind(content_hash)
    .bind("Discovered from OpenClaw")
    .execute(&mut *transaction)
    .await
    .map_err(map_database_error)?;
    sqlx::query(
        "INSERT INTO agent_profile_capabilities
            (id, profile_version_id, kind, provider_type, provider_key, resource_id,
             config, input_schema, output_schema)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(Uuid::new_v4())
    .bind(version_id)
    .bind(&capabilities[0].kind)
    .bind(&capabilities[0].provider_type)
    .bind(&capabilities[0].provider_key)
    .bind(&capabilities[0].resource_id)
    .bind(&capabilities[0].config)
    .bind(&capabilities[0].input_schema)
    .bind(&capabilities[0].output_schema)
    .execute(&mut *transaction)
    .await
    .map_err(map_database_error)?;
    sqlx::query("UPDATE agent_profiles SET active_version_id = $2 WHERE id = $1")
        .bind(profile_id)
        .bind(version_id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query(
        "INSERT INTO agent_profile_rollouts
            (profile_id, profile_version_id, weight_bps)
         VALUES ($1, $2, 10000)",
    )
    .bind(profile_id)
    .bind(version_id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "INSERT INTO project_bindings (project_id, binding_id)
         VALUES ($1, $2)
         ON CONFLICT DO NOTHING",
    )
    .bind(project_id)
    .bind(binding_id)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(binding_id)
}

async fn find_binding_by_agent_gateway_agent(
    pool: &PgPool,
    project_id: Uuid,
    gateway_id: &str,
    agent_id: &str,
    provider_key: &str,
) -> Result<Option<AgentBinding>, ApiError> {
    sqlx::query_as::<_, AgentBinding>(
        "SELECT binding.id, binding.profile_id, binding.provider, binding.gateway_id,
                binding.agent_id, binding.config, binding.created_at, binding.updated_at
         FROM agent_bindings AS binding
         JOIN agent_profiles AS profile ON profile.id = binding.profile_id
         WHERE profile.project_id = $1
           AND profile.archived_at IS NULL
           AND binding.gateway_id = $2
           AND binding.agent_id = $3
           AND COALESCE(NULLIF(binding.config->>'providerKey', ''), binding.provider) = $4
         ORDER BY binding.created_at ASC
         LIMIT 1",
    )
    .bind(project_id)
    .bind(gateway_id)
    .bind(agent_id)
    .bind(provider_key)
    .fetch_optional(pool)
    .await
    .map_err(ApiError::from)
}

fn discovered_profile_slug(gateway_id: &str, agent_id: &str, agent_name: &str) -> String {
    let mut base = slugify(agent_name);
    if validate_slug(&base) {
        return base;
    }
    base = slugify(agent_id);
    if validate_slug(&base) {
        return base;
    }
    base = slugify(&format!("{gateway_id}-{agent_id}"));
    if validate_slug(&base) {
        return base;
    }
    format!("agent-{}", Uuid::new_v4().simple())
        .chars()
        .take(64)
        .collect()
}

async fn unique_profile_slug(
    pool: &PgPool,
    project_id: Uuid,
    base: &str,
) -> Result<String, ApiError> {
    let base = if validate_slug(base) {
        base.to_string()
    } else {
        "agent".to_string()
    };
    let mut candidate = base.clone();
    for index in 2..=999 {
        let exists = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(
                SELECT 1 FROM agent_profiles
                WHERE project_id = $1 AND slug = $2
             )",
        )
        .bind(project_id)
        .bind(&candidate)
        .fetch_one(pool)
        .await?;
        if !exists {
            return Ok(candidate);
        }
        let suffix = format!("-{index}");
        let mut prefix = base
            .chars()
            .take(64usize.saturating_sub(suffix.len()))
            .collect::<String>();
        while prefix.ends_with('-') {
            prefix.pop();
        }
        candidate = format!("{prefix}{suffix}");
    }
    Err(ApiError::Invalid(
        "could not allocate a profile slug for the discovered agent".to_string(),
    ))
}

pub async fn update_binding(
    pool: &PgPool,
    id: Uuid,
    gateway_id: Option<&str>,
    agent_id: Option<&str>,
    config: Option<&Value>,
) -> Result<AgentBinding, ApiError> {
    sqlx::query_as::<_, AgentBinding>(
        "UPDATE agent_bindings SET
            gateway_id = COALESCE($2, gateway_id),
            agent_id = COALESCE($3, agent_id),
            config = COALESCE($4, config),
            updated_at = NOW()
         WHERE id = $1
         RETURNING id, profile_id, provider, gateway_id, agent_id, config, created_at, updated_at",
    )
    .bind(id)
    .bind(gateway_id)
    .bind(agent_id)
    .bind(config)
    .fetch_optional(pool)
    .await
    .map_err(map_database_error)?
    .ok_or(ApiError::NotFound)
}

pub async fn delete_binding(pool: &PgPool, id: Uuid) -> Result<(), ApiError> {
    delete_by_id(pool, "agent_bindings", id).await
}

pub async fn list_endpoints(pool: &PgPool) -> Result<Vec<AgentEndpoint>, ApiError> {
    sqlx::query_as::<_, AgentEndpoint>(
        "SELECT id, slug, name, profile_id, binding_id, enabled, request_timeout_ms,
                created_at, updated_at
         FROM agent_endpoints ORDER BY created_at ASC",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::from)
}

pub async fn list_enabled_endpoints(pool: &PgPool) -> Result<Vec<AgentEndpoint>, ApiError> {
    sqlx::query_as::<_, AgentEndpoint>(
        "SELECT id, slug, name, profile_id, binding_id, enabled, request_timeout_ms,
                created_at, updated_at
         FROM agent_endpoints
         WHERE enabled = TRUE
         ORDER BY created_at ASC",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::from)
}

pub async fn list_enabled_endpoints_for_project(
    pool: &PgPool,
    project_slug: &str,
) -> Result<Vec<AgentEndpoint>, ApiError> {
    let project = get_project_by_slug(pool, project_slug).await?;
    list_enabled_endpoints_for_project_id(pool, project.project.id).await
}

pub async fn list_enabled_endpoints_for_project_id(
    pool: &PgPool,
    project_id: Uuid,
) -> Result<Vec<AgentEndpoint>, ApiError> {
    sqlx::query_as::<_, AgentEndpoint>(
        "SELECT endpoint.id, endpoint.slug, endpoint.name, endpoint.profile_id,
                endpoint.binding_id, endpoint.enabled, endpoint.request_timeout_ms,
                endpoint.created_at, endpoint.updated_at
         FROM agent_endpoints endpoint
         JOIN project_bindings pb ON pb.binding_id = endpoint.binding_id
         JOIN projects project ON project.id = pb.project_id
         WHERE endpoint.enabled = TRUE
           AND project.enabled = TRUE
           AND project.id = $1
         ORDER BY endpoint.created_at ASC",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await
    .map_err(ApiError::from)
}

pub async fn get_endpoint(pool: &PgPool, id: Uuid) -> Result<AgentEndpoint, ApiError> {
    sqlx::query_as::<_, AgentEndpoint>(
        "SELECT id, slug, name, profile_id, binding_id, enabled, request_timeout_ms,
                created_at, updated_at
         FROM agent_endpoints WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)
}

pub async fn create_endpoint(
    pool: &PgPool,
    endpoint: NewEndpoint<'_>,
) -> Result<AgentEndpoint, ApiError> {
    ensure_binding_profile(pool, endpoint.profile_id, endpoint.binding_id).await?;
    sqlx::query_as::<_, AgentEndpoint>(
        "INSERT INTO agent_endpoints
            (id, slug, name, profile_id, binding_id, enabled, request_timeout_ms)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         RETURNING id, slug, name, profile_id, binding_id, enabled, request_timeout_ms,
                   created_at, updated_at",
    )
    .bind(endpoint.id)
    .bind(endpoint.slug)
    .bind(endpoint.name)
    .bind(endpoint.profile_id)
    .bind(endpoint.binding_id)
    .bind(endpoint.enabled)
    .bind(endpoint.request_timeout_ms)
    .fetch_one(pool)
    .await
    .map_err(map_database_error)
}

pub async fn update_endpoint(
    pool: &PgPool,
    id: Uuid,
    patch: EndpointPatch<'_>,
) -> Result<AgentEndpoint, ApiError> {
    let current = get_endpoint(pool, id).await?;
    let profile_id = patch.profile_id.unwrap_or(current.profile_id);
    let binding_id = patch.binding_id.unwrap_or(current.binding_id);
    ensure_binding_profile(pool, profile_id, binding_id).await?;

    sqlx::query_as::<_, AgentEndpoint>(
        "UPDATE agent_endpoints SET
            slug = COALESCE($2, slug),
            name = COALESCE($3, name),
            profile_id = COALESCE($4, profile_id),
            binding_id = COALESCE($5, binding_id),
            enabled = COALESCE($6, enabled),
            request_timeout_ms = COALESCE($7, request_timeout_ms),
            updated_at = NOW()
         WHERE id = $1
         RETURNING id, slug, name, profile_id, binding_id, enabled, request_timeout_ms,
                   created_at, updated_at",
    )
    .bind(id)
    .bind(patch.slug)
    .bind(patch.name)
    .bind(patch.profile_id)
    .bind(patch.binding_id)
    .bind(patch.enabled)
    .bind(patch.request_timeout_ms)
    .fetch_optional(pool)
    .await
    .map_err(map_database_error)?
    .ok_or(ApiError::NotFound)
}

pub async fn delete_endpoint(pool: &PgPool, id: Uuid) -> Result<(), ApiError> {
    delete_by_id(pool, "agent_endpoints", id).await
}

async fn ensure_binding_profile(
    pool: &PgPool,
    profile_id: Uuid,
    binding_id: Uuid,
) -> Result<(), ApiError> {
    let matches = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(
            SELECT 1 FROM agent_bindings WHERE id = $1 AND profile_id = $2
         )",
    )
    .bind(binding_id)
    .bind(profile_id)
    .fetch_one(pool)
    .await?;
    if matches {
        Ok(())
    } else {
        Err(ApiError::Invalid(
            "binding must belong to the endpoint profile".to_string(),
        ))
    }
}

pub async fn list_api_keys(pool: &PgPool) -> Result<Vec<ApiKeyRecord>, ApiError> {
    let rows = sqlx::query_as::<_, ApiKeyRow>(
        "SELECT key.id, key.project_id, key.name, key.scope_mode,
                COALESCE(
                    ARRAY_AGG(scope.profile_id ORDER BY scope.profile_id)
                        FILTER (WHERE scope.profile_id IS NOT NULL),
                    ARRAY[]::UUID[]
                ) AS profile_ids,
                key.permissions, key.key_prefix, key.created_at, key.revoked_at
         FROM api_keys key
         LEFT JOIN api_key_profile_scopes scope ON scope.api_key_id = key.id
         GROUP BY key.id
         ORDER BY key.created_at DESC",
    )
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(ApiKeyRecord::try_from).collect()
}

pub async fn create_api_key(pool: &PgPool, input: NewApiKey<'_>) -> Result<ApiKeyRecord, ApiError> {
    validate_api_key_agent_scope(pool, input.project_id, input.agent_scope).await?;
    let mut transaction = pool.begin().await?;
    sqlx::query(
        "INSERT INTO api_keys
            (id, project_id, name, scope_mode, permissions, key_prefix, key_hash)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(input.id)
    .bind(input.project_id)
    .bind(input.name)
    .bind(input.agent_scope.mode())
    .bind(Json(input.permissions))
    .bind(input.key_prefix)
    .bind(input.key_hash)
    .execute(&mut *transaction)
    .await
    .map_err(map_database_error)?;
    insert_api_key_profile_scopes(
        &mut transaction,
        input.id,
        input.project_id,
        input.agent_scope.profile_ids(),
    )
    .await?;
    transaction.commit().await?;
    get_api_key(pool, input.id).await
}

pub async fn get_api_key(pool: &PgPool, id: Uuid) -> Result<ApiKeyRecord, ApiError> {
    let row = sqlx::query_as::<_, ApiKeyRow>(
        "SELECT key.id, key.project_id, key.name, key.scope_mode,
                COALESCE(
                    ARRAY_AGG(scope.profile_id ORDER BY scope.profile_id)
                        FILTER (WHERE scope.profile_id IS NOT NULL),
                    ARRAY[]::UUID[]
                ) AS profile_ids,
                key.permissions, key.key_prefix, key.created_at, key.revoked_at
         FROM api_keys key
         LEFT JOIN api_key_profile_scopes scope ON scope.api_key_id = key.id
         WHERE key.id = $1
         GROUP BY key.id",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)?;
    ApiKeyRecord::try_from(row)
}

pub async fn update_api_key(
    pool: &PgPool,
    id: Uuid,
    patch: ApiKeyPatch<'_>,
) -> Result<ApiKeyRecord, ApiError> {
    let current = get_api_key(pool, id).await?;
    if current.revoked_at.is_some() {
        return Err(ApiError::Conflict(
            "revoked API keys cannot be edited".to_string(),
        ));
    }
    let project_id = patch.project_id.unwrap_or(current.project_id);
    let agent_scope = patch.agent_scope.unwrap_or(&current.agent_scope);
    validate_api_key_agent_scope(pool, project_id, agent_scope).await?;

    let mut transaction = pool.begin().await?;
    sqlx::query("DELETE FROM api_key_profile_scopes WHERE api_key_id = $1")
        .bind(id)
        .execute(&mut *transaction)
        .await?;
    let updated = sqlx::query_scalar::<_, Uuid>(
        "UPDATE api_keys SET
            project_id = $2,
            name = COALESCE($3, name),
            scope_mode = $4,
            permissions = COALESCE($5, permissions)
         WHERE id = $1 AND revoked_at IS NULL
         RETURNING id",
    )
    .bind(id)
    .bind(project_id)
    .bind(patch.name)
    .bind(agent_scope.mode())
    .bind(patch.permissions.map(Json))
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or_else(|| ApiError::Conflict("only active API keys can be edited".to_string()))?;
    insert_api_key_profile_scopes(
        &mut transaction,
        updated,
        project_id,
        agent_scope.profile_ids(),
    )
    .await?;
    transaction.commit().await?;
    get_api_key(pool, id).await
}

pub async fn revoke_api_key(pool: &PgPool, id: Uuid) -> Result<ApiKeyRecord, ApiError> {
    let updated = sqlx::query_scalar::<_, Uuid>(
        "UPDATE api_keys SET revoked_at = COALESCE(revoked_at, NOW()) WHERE id = $1 RETURNING id",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)?;
    get_api_key(pool, updated).await
}

pub async fn delete_api_key(pool: &PgPool, id: Uuid) -> Result<(), ApiError> {
    let result = sqlx::query("DELETE FROM api_keys WHERE id = $1 AND revoked_at IS NOT NULL")
        .bind(id)
        .execute(pool)
        .await?;
    if result.rows_affected() == 1 {
        return Ok(());
    }

    let exists =
        sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM api_keys WHERE id = $1)")
            .bind(id)
            .fetch_one(pool)
            .await?;
    if exists {
        Err(ApiError::Conflict(
            "revoke the API key before deleting it".to_string(),
        ))
    } else {
        Err(ApiError::NotFound)
    }
}

pub async fn create_realtime_session(
    pool: &PgPool,
    id: Uuid,
    project_id: Uuid,
    profile_id: Uuid,
    api_key_id: Option<Uuid>,
    token_hash: &[u8],
    expires_at: DateTime<Utc>,
) -> Result<RealtimeSession, ApiError> {
    sqlx::query("DELETE FROM realtime_sessions WHERE expires_at <= NOW()")
        .execute(pool)
        .await?;
    sqlx::query_as::<_, RealtimeSession>(
        "INSERT INTO realtime_sessions
            (id, project_id, profile_id, api_key_id, token_hash, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6)
         RETURNING id, project_id, profile_id, api_key_id, expires_at, created_at",
    )
    .bind(id)
    .bind(project_id)
    .bind(profile_id)
    .bind(api_key_id)
    .bind(token_hash)
    .bind(expires_at)
    .fetch_one(pool)
    .await
    .map_err(map_database_error)
}

pub async fn active_realtime_session_by_hash(
    pool: &PgPool,
    project_id: Uuid,
    token_hash: &[u8],
) -> Result<RealtimeSession, ApiError> {
    sqlx::query_as::<_, RealtimeSession>(
        "SELECT id, project_id, profile_id, api_key_id, expires_at, created_at
         FROM realtime_sessions
         WHERE project_id = $1 AND token_hash = $2 AND expires_at > NOW()",
    )
    .bind(project_id)
    .bind(token_hash)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::Unauthorized)
}

pub async fn active_api_key_by_hash(
    pool: &PgPool,
    key_hash: &[u8],
) -> Result<ApiKeyRecord, ApiError> {
    active_api_key_by_hash_optional(pool, key_hash)
        .await?
        .ok_or(ApiError::Forbidden)
}

pub async fn active_api_key_by_hash_optional(
    pool: &PgPool,
    key_hash: &[u8],
) -> Result<Option<ApiKeyRecord>, ApiError> {
    let row = sqlx::query_as::<_, ApiKeyRow>(
        "SELECT key.id, key.project_id, key.name, key.scope_mode,
                COALESCE(
                    ARRAY_AGG(scope.profile_id ORDER BY scope.profile_id)
                        FILTER (WHERE scope.profile_id IS NOT NULL),
                    ARRAY[]::UUID[]
                ) AS profile_ids,
                key.permissions, key.key_prefix, key.created_at, key.revoked_at
         FROM api_keys key
         LEFT JOIN api_key_profile_scopes scope ON scope.api_key_id = key.id
         WHERE key.key_hash = $1 AND key.revoked_at IS NULL
         GROUP BY key.id",
    )
    .bind(key_hash)
    .fetch_optional(pool)
    .await?;
    row.map(ApiKeyRecord::try_from).transpose()
}

async fn validate_api_key_agent_scope(
    pool: &PgPool,
    project_id: Uuid,
    agent_scope: &ApiKeyAgentScope,
) -> Result<(), ApiError> {
    let ApiKeyAgentScope::Selected { profile_ids } = agent_scope else {
        return Ok(());
    };
    if profile_ids.is_empty() || profile_ids.len() > 256 {
        return Err(ApiError::Invalid(
            "selected agent access requires between 1 and 256 agents".to_string(),
        ));
    }
    let count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM agent_profiles
         WHERE project_id = $1 AND id = ANY($2) AND archived_at IS NULL",
    )
    .bind(project_id)
    .bind(profile_ids)
    .fetch_one(pool)
    .await?;
    if usize::try_from(count).ok() != Some(profile_ids.len()) {
        return Err(ApiError::Invalid(
            "selected profiles must belong to the API key project".to_string(),
        ));
    }
    Ok(())
}

async fn insert_api_key_profile_scopes(
    transaction: &mut sqlx::Transaction<'_, Postgres>,
    api_key_id: Uuid,
    project_id: Uuid,
    profile_ids: &[Uuid],
) -> Result<(), ApiError> {
    for profile_id in profile_ids {
        sqlx::query(
            "INSERT INTO api_key_profile_scopes (api_key_id, project_id, profile_id)
             VALUES ($1, $2, $3)",
        )
        .bind(api_key_id)
        .bind(project_id)
        .bind(profile_id)
        .execute(&mut **transaction)
        .await
        .map_err(map_database_error)?;
    }
    Ok(())
}

pub async fn resolve_endpoint_route(
    pool: &PgPool,
    id_or_slug: &str,
) -> Result<EndpointRoute, ApiError> {
    const SELECT: &str =
        "SELECT e.id AS endpoint_id, e.slug AS endpoint_slug, e.name AS endpoint_name,
                e.request_timeout_ms, p.id AS profile_id, b.id AS binding_id,
                b.gateway_id, b.agent_id, b.config AS binding_config
         FROM agent_endpoints e
         JOIN agent_profiles p ON p.id = e.profile_id
         JOIN agent_bindings b ON b.id = e.binding_id
         WHERE e.enabled = TRUE AND ";

    if let Ok(id) = Uuid::parse_str(id_or_slug) {
        sqlx::query_as::<_, EndpointRoute>(&format!("{SELECT} e.id = $1"))
            .bind(id)
            .fetch_optional(pool)
            .await?
            .ok_or(ApiError::NotFound)
    } else {
        sqlx::query_as::<_, EndpointRoute>(&format!("{SELECT} e.slug = $1"))
            .bind(id_or_slug)
            .fetch_optional(pool)
            .await?
            .ok_or(ApiError::NotFound)
    }
}

pub async fn resolve_project_endpoint_route(
    pool: &PgPool,
    project_slug: &str,
    id_or_slug: &str,
) -> Result<EndpointRoute, ApiError> {
    const SELECT: &str =
        "SELECT e.id AS endpoint_id, e.slug AS endpoint_slug, e.name AS endpoint_name,
                e.request_timeout_ms, profile.id AS profile_id, binding.id AS binding_id,
                binding.gateway_id, binding.agent_id, binding.config AS binding_config
         FROM agent_endpoints e
         JOIN agent_profiles profile ON profile.id = e.profile_id
         JOIN agent_bindings binding ON binding.id = e.binding_id
         JOIN project_bindings pb ON pb.binding_id = binding.id
         JOIN projects project ON project.id = pb.project_id
         WHERE e.enabled = TRUE
           AND project.enabled = TRUE
           AND project.slug = $1
           AND ";

    if let Ok(id) = Uuid::parse_str(id_or_slug) {
        sqlx::query_as::<_, EndpointRoute>(&format!("{SELECT} e.id = $2"))
            .bind(project_slug)
            .bind(id)
            .fetch_optional(pool)
            .await?
            .ok_or(ApiError::NotFound)
    } else {
        sqlx::query_as::<_, EndpointRoute>(&format!("{SELECT} e.slug = $2"))
            .bind(project_slug)
            .bind(id_or_slug)
            .fetch_optional(pool)
            .await?
            .ok_or(ApiError::NotFound)
    }
}

pub async fn resolve_project_model_route(
    pool: &PgPool,
    project_id: Uuid,
    model: &str,
) -> Result<EndpointRoute, ApiError> {
    sqlx::query_as::<_, EndpointRoute>(
        "SELECT endpoint.id AS endpoint_id, endpoint.slug AS endpoint_slug,
                endpoint.name AS endpoint_name, endpoint.request_timeout_ms,
                profile.id AS profile_id, binding.id AS binding_id,
                binding.gateway_id, binding.agent_id, binding.config AS binding_config
         FROM agent_endpoints endpoint
         JOIN agent_profiles profile ON profile.id = endpoint.profile_id
         JOIN agent_bindings binding ON binding.id = endpoint.binding_id
         JOIN project_bindings pb ON pb.binding_id = binding.id
         JOIN projects project ON project.id = pb.project_id
         WHERE endpoint.enabled = TRUE
           AND project.enabled = TRUE
           AND project.id = $1
           AND (
                endpoint.slug = $2
                OR endpoint.id::TEXT = $2
                OR profile.slug = $2
                OR binding.agent_id = $2
           )
         ORDER BY
           CASE
             WHEN endpoint.slug = $2 THEN 0
             WHEN profile.slug = $2 THEN 1
             WHEN binding.agent_id = $2 THEN 2
             ELSE 3
           END,
           endpoint.created_at ASC
         LIMIT 1",
    )
    .bind(project_id)
    .bind(model)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)
}

#[derive(Debug, FromRow)]
struct AgentGatewayCredentialSecret {
    credential_hash: Vec<u8>,
    owner_user_id: Option<String>,
    revoked_at: Option<DateTime<Utc>>,
}

pub async fn register_agent_gateway_credential(
    pool: &PgPool,
    gateway_id: &str,
    owner_user_id: Option<&str>,
    credential_prefix: &str,
    credential_hash: &[u8],
) -> Result<AgentGatewayRegistration, ApiError> {
    let mut transaction = pool.begin().await?;
    let existing = sqlx::query_as::<_, AgentGatewayCredentialSecret>(
        "SELECT credential_hash, owner_user_id, revoked_at
         FROM agent_gateway_credentials
         WHERE gateway_id = $1
         FOR UPDATE",
    )
    .bind(gateway_id)
    .fetch_optional(&mut *transaction)
    .await?;

    let registration = match existing {
        None => {
            sqlx::query(
                "INSERT INTO agent_gateway_credentials
                    (gateway_id, owner_user_id, credential_prefix, credential_hash)
                 VALUES ($1, $2, $3, $4)",
            )
            .bind(gateway_id)
            .bind(owner_user_id)
            .bind(credential_prefix)
            .bind(credential_hash)
            .execute(&mut *transaction)
            .await
            .map_err(map_database_error)?;
            AgentGatewayRegistration::Registered
        }
        Some(existing)
            if existing.revoked_at.is_none()
                && existing.credential_hash == credential_hash
                && existing.owner_user_id.as_deref() == owner_user_id =>
        {
            AgentGatewayRegistration::Existing
        }
        Some(existing) if existing.revoked_at.is_none() => {
            return Err(ApiError::Conflict(
                "agent gateway id is already registered".to_string(),
            ));
        }
        Some(_) => return Err(ApiError::AgentGatewayCredentialRevoked),
    };
    transaction.commit().await?;
    Ok(registration)
}

pub async fn create_agent_gateway_enrollment(
    pool: &PgPool,
    input: NewAgentGatewayEnrollment<'_>,
) -> Result<(), ApiError> {
    let mut transaction = pool.begin().await?;
    sqlx::query(
        "UPDATE agent_gateway_enrollments
         SET revoked_at = NOW()
         WHERE project_id = $1
           AND consumed_at IS NULL
           AND revoked_at IS NULL",
    )
    .bind(input.project_id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "INSERT INTO agent_gateway_enrollments
            (id, project_id, owner_user_id, token_hash, expires_at)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(input.id)
    .bind(input.project_id)
    .bind(input.owner_user_id)
    .bind(input.token_hash)
    .bind(input.expires_at)
    .execute(&mut *transaction)
    .await
    .map_err(map_database_error)?;
    transaction.commit().await?;
    Ok(())
}

#[derive(Debug, FromRow)]
struct AgentGatewayEnrollmentSecret {
    project_id: Uuid,
    owner_user_id: String,
    gateway_id: Option<String>,
    consumed_at: Option<DateTime<Utc>>,
    revoked_at: Option<DateTime<Utc>>,
    expires_at: DateTime<Utc>,
}

pub async fn consume_agent_gateway_enrollment(
    pool: &PgPool,
    token_hash: &[u8],
    gateway_id: &str,
    credential_prefix: &str,
    credential_hash: &[u8],
) -> Result<AgentGatewayRegistration, ApiError> {
    let mut transaction = pool.begin().await?;
    let claimed = sqlx::query_as::<_, AgentGatewayEnrollmentSecret>(
        "UPDATE agent_gateway_enrollments
         SET gateway_id = $2, consumed_at = NOW()
         WHERE token_hash = $1
           AND consumed_at IS NULL
           AND revoked_at IS NULL
           AND expires_at > NOW()
         RETURNING project_id, owner_user_id, gateway_id, consumed_at, revoked_at, expires_at",
    )
    .bind(token_hash)
    .bind(gateway_id)
    .fetch_optional(&mut *transaction)
    .await?;
    let (enrollment, is_idempotent_retry) = match claimed {
        Some(enrollment) => (enrollment, false),
        None => (
            sqlx::query_as::<_, AgentGatewayEnrollmentSecret>(
                "SELECT project_id, owner_user_id, gateway_id, consumed_at, revoked_at, expires_at
                 FROM agent_gateway_enrollments
                 WHERE token_hash = $1
                 FOR UPDATE",
            )
            .bind(token_hash)
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or(ApiError::Unauthorized)?,
            true,
        ),
    };
    if enrollment.revoked_at.is_some()
        || (enrollment.consumed_at.is_none() && enrollment.expires_at <= Utc::now())
    {
        return Err(ApiError::Unauthorized);
    }
    if is_idempotent_retry
        && (enrollment.consumed_at.is_none()
            || enrollment.gateway_id.as_deref() != Some(gateway_id))
    {
        return Err(ApiError::Unauthorized);
    }

    let existing = sqlx::query_as::<_, AgentGatewayCredentialSecret>(
        "SELECT credential_hash, owner_user_id, revoked_at
         FROM agent_gateway_credentials
         WHERE gateway_id = $1
         FOR UPDATE",
    )
    .bind(gateway_id)
    .fetch_optional(&mut *transaction)
    .await?;
    let registration = match existing {
        None => {
            sqlx::query(
                "INSERT INTO agent_gateway_credentials
                    (gateway_id, owner_user_id, credential_prefix, credential_hash)
                 VALUES ($1, $2, $3, $4)",
            )
            .bind(gateway_id)
            .bind(&enrollment.owner_user_id)
            .bind(credential_prefix)
            .bind(credential_hash)
            .execute(&mut *transaction)
            .await
            .map_err(map_database_error)?;
            AgentGatewayRegistration::Registered
        }
        Some(existing)
            if existing.revoked_at.is_none()
                && existing.credential_hash == credential_hash
                && existing.owner_user_id.as_deref() == Some(enrollment.owner_user_id.as_str()) =>
        {
            AgentGatewayRegistration::Existing
        }
        Some(existing) if existing.revoked_at.is_none() => {
            return Err(ApiError::Conflict(
                "agent gateway id is already registered".to_string(),
            ));
        }
        Some(_) => return Err(ApiError::AgentGatewayCredentialRevoked),
    };

    let project_update = if is_idempotent_retry {
        sqlx::query(
            "UPDATE projects
             SET updated_at = updated_at
             WHERE id = $1 AND owner_user_id = $2 AND gateway_id = $3",
        )
        .bind(enrollment.project_id)
        .bind(&enrollment.owner_user_id)
        .bind(gateway_id)
        .execute(&mut *transaction)
        .await?
    } else {
        sqlx::query(
            "UPDATE projects
             SET gateway_id = $1, updated_at = NOW()
             WHERE id = $2 AND owner_user_id = $3",
        )
        .bind(gateway_id)
        .bind(enrollment.project_id)
        .bind(&enrollment.owner_user_id)
        .execute(&mut *transaction)
        .await?
    };
    if project_update.rows_affected() != 1 {
        return Err(ApiError::Forbidden);
    }
    transaction.commit().await?;
    Ok(if is_idempotent_retry {
        AgentGatewayRegistration::Existing
    } else {
        registration
    })
}

pub async fn authenticate_agent_gateway_credential(
    pool: &PgPool,
    credential_hash: &[u8],
) -> Result<String, ApiError> {
    sqlx::query_scalar::<_, String>(
        "UPDATE agent_gateway_credentials
         SET last_used_at = NOW()
         WHERE credential_hash = $1 AND revoked_at IS NULL
         RETURNING gateway_id",
    )
    .bind(credential_hash)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::Forbidden)
}

pub async fn revoke_agent_gateway_credential(
    pool: &PgPool,
    gateway_id: &str,
) -> Result<AgentGatewayCredential, ApiError> {
    sqlx::query_as::<_, AgentGatewayCredential>(
        "UPDATE agent_gateway_credentials
         SET revoked_at = COALESCE(revoked_at, NOW())
         WHERE gateway_id = $1
         RETURNING gateway_id, credential_prefix, created_at, last_used_at, revoked_at",
    )
    .bind(gateway_id)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)
}

pub async fn open_agent_gateway_session(
    pool: &PgPool,
    gateway_id: &str,
    resume_session_id: Option<Uuid>,
    agents: &Value,
    metadata: &Value,
) -> Result<(Uuid, bool), ApiError> {
    let mut transaction = pool.begin().await?;
    sqlx::query(
        "UPDATE agent_gateway_sessions SET
            status = 'disconnected', last_seen_at = NOW(), disconnected_at = NOW()
         WHERE gateway_id = $1 AND status = 'connected'",
    )
    .bind(gateway_id)
    .execute(&mut *transaction)
    .await?;

    if let Some(session_id) = resume_session_id {
        let updated = sqlx::query_scalar::<_, Uuid>(
            "UPDATE agent_gateway_sessions SET
                status = 'connected', agents = $3, metadata = $4,
                connected_at = NOW(), last_seen_at = NOW(), disconnected_at = NULL
             WHERE session_id = $1 AND gateway_id = $2
             RETURNING session_id",
        )
        .bind(session_id)
        .bind(gateway_id)
        .bind(agents)
        .bind(metadata)
        .fetch_optional(&mut *transaction)
        .await?;
        if let Some(session_id) = updated {
            transaction.commit().await?;
            return Ok((session_id, true));
        }
    }

    let id = Uuid::new_v4();
    let session_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO agent_gateway_sessions
            (id, gateway_id, session_id, status, agents, metadata)
         VALUES ($1, $2, $3, 'connected', $4, $5)",
    )
    .bind(id)
    .bind(gateway_id)
    .bind(session_id)
    .bind(agents)
    .bind(metadata)
    .execute(&mut *transaction)
    .await
    .map_err(map_database_error)?;
    transaction.commit().await?;
    Ok((session_id, false))
}

pub async fn touch_agent_gateway_session(pool: &PgPool, session_id: Uuid) -> Result<(), ApiError> {
    sqlx::query(
        "UPDATE agent_gateway_sessions SET last_seen_at = NOW(), status = 'connected'
         WHERE session_id = $1",
    )
    .bind(session_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn close_agent_gateway_session(pool: &PgPool, session_id: Uuid) -> Result<(), ApiError> {
    sqlx::query(
        "UPDATE agent_gateway_sessions SET
            status = 'disconnected', last_seen_at = NOW(), disconnected_at = NOW()
         WHERE session_id = $1",
    )
    .bind(session_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_agent_gateway_sessions(
    pool: &PgPool,
) -> Result<Vec<AgentGatewaySession>, ApiError> {
    sqlx::query_as::<_, AgentGatewaySession>(
        "SELECT id, gateway_id, session_id, status, agents, metadata,
                connected_at, last_seen_at, disconnected_at
         FROM agent_gateway_sessions ORDER BY connected_at DESC LIMIT 200",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::from)
}

pub async fn list_available_agents(pool: &PgPool) -> Result<Vec<AvailableAgent>, ApiError> {
    let sessions = list_agent_gateway_sessions(pool).await?;
    let mut seen = HashSet::new();
    let mut agents = Vec::new();

    for session in sessions {
        let Some(items) = session.agents.as_array() else {
            continue;
        };
        for item in items {
            let Some(id) = item.get("id").and_then(Value::as_str).map(str::trim) else {
                continue;
            };
            if id.is_empty() {
                continue;
            }
            let name = item
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or(id)
                .to_string();
            let metadata = item
                .get("metadata")
                .cloned()
                .filter(Value::is_object)
                .unwrap_or_else(|| json!({}));
            let provider_key = metadata
                .get("providerKey")
                .and_then(Value::as_str)
                .unwrap_or("");
            let key = (
                session.gateway_id.clone(),
                provider_key.to_string(),
                id.to_string(),
            );
            if !seen.insert(key) {
                continue;
            }
            agents.push(AvailableAgent {
                gateway_id: session.gateway_id.clone(),
                id: id.to_string(),
                name,
                status: session.status.clone(),
                metadata,
            });
        }
    }

    Ok(agents)
}

pub async fn create_trace(pool: &PgPool, trace: NewTrace<'_>) -> Result<Uuid, ApiError> {
    let trace_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO endpoint_traces
            (id, request_id, endpoint_id, project_id, gateway_session_id, profile_id,
             profile_version_id, operation, provider_key, capability_kind, selection_key,
             status, request)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, 'pending', $12)",
    )
    .bind(trace_id)
    .bind(trace.request_id)
    .bind(trace.endpoint_id)
    .bind(trace.project_id)
    .bind(trace.gateway_session_id)
    .bind(trace.profile_id)
    .bind(trace.profile_version_id)
    .bind(trace.operation)
    .bind(trace.provider_key)
    .bind(trace.capability_kind)
    .bind(trace.selection_key)
    .bind(trace.request)
    .execute(pool)
    .await
    .map_err(map_database_error)?;
    Ok(trace_id)
}

pub async fn create_trace_span(pool: &PgPool, span: NewTraceSpan<'_>) -> Result<Uuid, ApiError> {
    let span_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO trace_spans
            (id, trace_id, parent_span_id, name, kind, status, provider_key,
             capability_kind, input_summary, attributes)
         VALUES ($1, $2, $3, $4, $5, 'pending', $6, $7, $8, $9)",
    )
    .bind(span_id)
    .bind(span.trace_id)
    .bind(span.parent_span_id)
    .bind(span.name)
    .bind(span.kind)
    .bind(span.provider_key)
    .bind(span.capability_kind)
    .bind(span.input_summary)
    .bind(span.attributes)
    .execute(pool)
    .await?;
    Ok(span_id)
}

pub async fn complete_trace_span(
    pool: &PgPool,
    span_id: Uuid,
    status: &str,
    duration_ms: i64,
    output_summary: Option<&Value>,
    error: Option<&str>,
) -> Result<(), ApiError> {
    sqlx::query(
        "UPDATE trace_spans
         SET status = $2, duration_ms = $3, output_summary = $4, error = $5,
             completed_at = NOW()
         WHERE id = $1",
    )
    .bind(span_id)
    .bind(status)
    .bind(duration_ms)
    .bind(output_summary)
    .bind(error)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn complete_trace(
    pool: &PgPool,
    request_id: Uuid,
    status: &str,
    latency_ms: i64,
    response: Option<&Value>,
    error: Option<&str>,
) -> Result<(), ApiError> {
    sqlx::query(
        "UPDATE endpoint_traces SET status = $2, latency_ms = $3, response = $4,
                error = $5, completed_at = NOW()
         WHERE request_id = $1",
    )
    .bind(request_id)
    .bind(status)
    .bind(latency_ms)
    .bind(response)
    .bind(error)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_traces(
    pool: &PgPool,
    endpoint_id: Option<Uuid>,
    project_id: Option<Uuid>,
    limit: i64,
) -> Result<Vec<EndpointTrace>, ApiError> {
    let mut query = QueryBuilder::<Postgres>::new(
        "SELECT trace.id, trace.request_id, trace.endpoint_id,
                trace.project_id, trace.gateway_session_id, trace.profile_id,
                trace.profile_version_id, profile.slug AS profile_slug,
                profile.name AS profile_name,
                profile_version.version_number AS profile_version_number,
                trace.operation, trace.provider_key,
                trace.capability_kind, trace.selection_key, trace.status, trace.latency_ms,
                trace.request, trace.response, trace.error, trace.created_at, trace.completed_at
         FROM endpoint_traces trace
         LEFT JOIN agent_profiles profile ON profile.id = trace.profile_id
         LEFT JOIN agent_profile_versions profile_version
            ON profile_version.id = trace.profile_version_id",
    );
    if let Some(endpoint_id) = endpoint_id {
        query
            .push(" WHERE trace.endpoint_id = ")
            .push_bind(endpoint_id);
    } else if let Some(project_id) = project_id {
        query
            .push(" WHERE trace.project_id = ")
            .push_bind(project_id);
    }
    query
        .push(" ORDER BY trace.created_at DESC LIMIT ")
        .push_bind(limit);
    query
        .build_query_as::<EndpointTrace>()
        .fetch_all(pool)
        .await
        .map_err(ApiError::from)
}

pub async fn list_trace_spans(pool: &PgPool, trace_id: Uuid) -> Result<Vec<TraceSpan>, ApiError> {
    sqlx::query_as::<_, TraceSpan>(
        "SELECT id, trace_id, parent_span_id, name, kind, status, provider_key,
                capability_kind, duration_ms, input_summary, output_summary, attributes,
                error, created_at, completed_at
         FROM trace_spans
         WHERE trace_id = $1
         ORDER BY created_at ASC",
    )
    .bind(trace_id)
    .fetch_all(pool)
    .await
    .map_err(ApiError::from)
}

pub async fn get_trace_project_id(pool: &PgPool, trace_id: Uuid) -> Result<Option<Uuid>, ApiError> {
    sqlx::query_scalar("SELECT project_id FROM endpoint_traces WHERE id = $1")
        .bind(trace_id)
        .fetch_optional(pool)
        .await
        .map_err(ApiError::Database)?
        .ok_or(ApiError::NotFound)
}

async fn delete_by_id(pool: &PgPool, table: &str, id: Uuid) -> Result<(), ApiError> {
    let result = sqlx::query(delete_statement(table)?)
        .bind(id)
        .execute(pool)
        .await
        .map_err(map_database_error)?;
    if result.rows_affected() == 0 {
        Err(ApiError::NotFound)
    } else {
        Ok(())
    }
}

fn delete_statement(table: &str) -> Result<&'static str, ApiError> {
    let statement = match table {
        "agent_profiles" => "DELETE FROM agent_profiles WHERE id = $1",
        "agent_bindings" => "DELETE FROM agent_bindings WHERE id = $1",
        "agent_endpoints" => "DELETE FROM agent_endpoints WHERE id = $1",
        "projects" => "DELETE FROM projects WHERE id = $1",
        "provider_connections" => "DELETE FROM provider_connections WHERE id = $1",
        _ => return Err(ApiError::Internal),
    };
    Ok(statement)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{delete_statement, profile_version_content_hash, NewProfileVersion};

    #[test]
    fn project_resources_use_the_scoped_delete_statement() {
        assert_eq!(
            delete_statement("projects").expect("projects must be deletable"),
            "DELETE FROM projects WHERE id = $1"
        );
    }

    #[test]
    fn profile_version_hash_is_stable_lowercase_hex() {
        let empty = json!({});
        let input = NewProfileVersion {
            persona: &empty,
            runtime: &empty,
            presentation: &empty,
            source: &empty,
            capabilities: &[],
            change_summary: None,
        };

        let hash =
            profile_version_content_hash(&input).expect("profile version hashing must succeed");

        assert_eq!(hash.len(), 64);
        assert!(hash.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(hash, hash.to_ascii_lowercase());
        assert_eq!(
            hash,
            profile_version_content_hash(&input).expect("hashing must be deterministic")
        );
    }
}
