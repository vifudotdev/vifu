use std::collections::HashSet;

use chrono::Utc;
use serde_json::{json, Value};
use sqlx::{PgPool, Postgres, QueryBuilder};
use uuid::Uuid;

use crate::error::{map_database_error, ApiError};
use crate::models::{
    slugify, validate_slug, AgentBinding, AgentEndpoint, AgentGatewaySession, AgentProfile,
    ApiKeyRecord, AvailableAgent, EndpointRoute, EndpointTrace, Project, ProjectAgentRoute,
    ProjectRoute, ProjectWithBindings,
};

pub async fn migrate(pool: &PgPool) -> Result<(), ApiError> {
    sqlx::migrate!("./migrations").run(pool).await?;
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
        "SELECT id, slug, name, description, gateway_id, enabled,
                publishable_key_prefix, created_at, updated_at
         FROM projects ORDER BY created_at ASC",
    )
    .fetch_all(pool)
    .await?;
    projects_with_bindings(pool, projects).await
}

pub async fn get_project(pool: &PgPool, id: Uuid) -> Result<ProjectWithBindings, ApiError> {
    let project = sqlx::query_as::<_, Project>(
        "SELECT id, slug, name, description, gateway_id, enabled,
                publishable_key_prefix, created_at, updated_at
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

pub async fn create_project(
    pool: &PgPool,
    project: NewProject<'_>,
) -> Result<ProjectWithBindings, ApiError> {
    validate_project_bindings(pool, project.gateway_id, project.binding_ids).await?;
    let mut transaction = pool.begin().await?;
    let created = sqlx::query_as::<_, Project>(
        "INSERT INTO projects
            (id, slug, name, description, gateway_id,
             publishable_key_prefix, publishable_key_hash)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         RETURNING id, slug, name, description, gateway_id, enabled,
                   publishable_key_prefix, created_at, updated_at",
    )
    .bind(project.id)
    .bind(project.slug)
    .bind(project.name)
    .bind(project.description)
    .bind(project.gateway_id)
    .bind(project.publishable_key_prefix)
    .bind(project.publishable_key_hash)
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

pub struct NewProject<'a> {
    pub id: Uuid,
    pub slug: &'a str,
    pub name: &'a str,
    pub description: Option<&'a str>,
    pub gateway_id: &'a str,
    pub publishable_key_prefix: &'a str,
    pub publishable_key_hash: &'a [u8],
    pub binding_ids: &'a [Uuid],
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
         RETURNING id, slug, name, description, gateway_id, enabled,
                   publishable_key_prefix, created_at, updated_at",
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

pub struct ProjectPatch<'a> {
    pub slug: Option<&'a str>,
    pub name: Option<&'a str>,
    pub description_changed: bool,
    pub description: Option<&'a str>,
    pub gateway_id: Option<&'a str>,
    pub enabled: Option<bool>,
    pub binding_ids: Option<&'a [Uuid]>,
}

pub async fn delete_project(pool: &PgPool, id: Uuid) -> Result<(), ApiError> {
    delete_by_id(pool, "projects", id).await
}

pub async fn resolve_project_route(pool: &PgPool, slug: &str) -> Result<ProjectRoute, ApiError> {
    sqlx::query_as::<_, ProjectRoute>(
        "SELECT id, slug, gateway_id, publishable_key_hash
         FROM projects WHERE slug = $1 AND enabled = TRUE",
    )
    .bind(slug)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)
}

pub async fn list_project_agent_routes(
    pool: &PgPool,
    project_id: Uuid,
) -> Result<Vec<ProjectAgentRoute>, ApiError> {
    sqlx::query_as::<_, ProjectAgentRoute>(
        "SELECT project.id AS project_id, project.slug AS project_slug,
                profile.id AS profile_id, profile.slug AS profile_slug,
                profile.name AS profile_name,
                binding.id AS binding_id, binding.gateway_id, binding.agent_id,
                binding.config AS binding_config
         FROM projects project
         JOIN project_bindings project_binding ON project_binding.project_id = project.id
         JOIN agent_bindings binding ON binding.id = project_binding.binding_id
         JOIN agent_profiles profile ON profile.id = binding.profile_id
         WHERE project.id = $1 AND project.enabled = TRUE
         ORDER BY project_binding.created_at ASC",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await
    .map_err(ApiError::from)
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

async fn validate_project_bindings(
    pool: &PgPool,
    gateway_id: &str,
    binding_ids: &[Uuid],
) -> Result<(), ApiError> {
    if binding_ids.len() > 256 {
        return Err(ApiError::Invalid(
            "a project supports at most 256 bindings".to_string(),
        ));
    }
    let count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM agent_bindings
         WHERE id = ANY($1) AND gateway_id = $2",
    )
    .bind(binding_ids)
    .bind(gateway_id)
    .fetch_one(pool)
    .await?;
    if usize::try_from(count).ok() == Some(binding_ids.len()) {
        Ok(())
    } else {
        Err(ApiError::Invalid(
            "project bindings must exist on the selected agent gateway".to_string(),
        ))
    }
}

pub async fn list_profiles(pool: &PgPool) -> Result<Vec<AgentProfile>, ApiError> {
    sqlx::query_as::<_, AgentProfile>(
        "SELECT id, slug, name, description, created_at, updated_at
         FROM agent_profiles ORDER BY created_at ASC",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::from)
}

pub async fn get_profile(pool: &PgPool, id: Uuid) -> Result<AgentProfile, ApiError> {
    sqlx::query_as::<_, AgentProfile>(
        "SELECT id, slug, name, description, created_at, updated_at
         FROM agent_profiles WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)
}

pub async fn create_profile(
    pool: &PgPool,
    id: Uuid,
    slug: &str,
    name: &str,
    description: Option<&str>,
) -> Result<AgentProfile, ApiError> {
    sqlx::query_as::<_, AgentProfile>(
        "INSERT INTO agent_profiles (id, slug, name, description)
         VALUES ($1, $2, $3, $4)
         RETURNING id, slug, name, description, created_at, updated_at",
    )
    .bind(id)
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
         RETURNING id, slug, name, description, created_at, updated_at",
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

pub struct ProfilePatch<'a> {
    pub slug: Option<&'a str>,
    pub name: Option<&'a str>,
    pub description_changed: bool,
    pub description: Option<&'a str>,
}

pub async fn delete_profile(pool: &PgPool, id: Uuid) -> Result<(), ApiError> {
    delete_by_id(pool, "agent_profiles", id).await
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
    gateway_id: &str,
    agent_id: &str,
    agent_name: &str,
) -> Result<Uuid, ApiError> {
    if let Some(binding) = find_binding_by_agent_gateway_agent(pool, gateway_id, agent_id).await? {
        return Ok(binding.id);
    }

    let display_name = agent_name.trim();
    let display_name = if display_name.is_empty() {
        agent_id
    } else {
        display_name
    };
    let slug = unique_profile_slug(
        pool,
        &discovered_profile_slug(gateway_id, agent_id, display_name),
    )
    .await?;
    let profile = create_profile(
        pool,
        Uuid::new_v4(),
        &slug,
        display_name,
        Some("Discovered from OpenClaw"),
    )
    .await?;
    let config = json!({
        "source": "openclaw-discovery",
        "agentName": display_name,
    });
    let binding = create_binding(
        pool,
        Uuid::new_v4(),
        profile.id,
        "openclaw",
        gateway_id,
        agent_id,
        &config,
    )
    .await?;
    Ok(binding.id)
}

async fn find_binding_by_agent_gateway_agent(
    pool: &PgPool,
    gateway_id: &str,
    agent_id: &str,
) -> Result<Option<AgentBinding>, ApiError> {
    sqlx::query_as::<_, AgentBinding>(
        "SELECT id, profile_id, provider, gateway_id, agent_id, config, created_at, updated_at
         FROM agent_bindings
         WHERE gateway_id = $1 AND agent_id = $2
         ORDER BY created_at ASC
         LIMIT 1",
    )
    .bind(gateway_id)
    .bind(agent_id)
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

async fn unique_profile_slug(pool: &PgPool, base: &str) -> Result<String, ApiError> {
    let base = if validate_slug(base) {
        base.to_string()
    } else {
        "agent".to_string()
    };
    let mut candidate = base.clone();
    for index in 2..=999 {
        let exists = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM agent_profiles WHERE slug = $1)",
        )
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

pub struct NewEndpoint<'a> {
    pub id: Uuid,
    pub slug: &'a str,
    pub name: &'a str,
    pub profile_id: Uuid,
    pub binding_id: Uuid,
    pub enabled: bool,
    pub request_timeout_ms: i32,
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

pub struct EndpointPatch<'a> {
    pub slug: Option<&'a str>,
    pub name: Option<&'a str>,
    pub profile_id: Option<Uuid>,
    pub binding_id: Option<Uuid>,
    pub enabled: Option<bool>,
    pub request_timeout_ms: Option<i32>,
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
    sqlx::query_as::<_, ApiKeyRecord>(
        "SELECT id, endpoint_id, name, key_prefix, created_at, revoked_at
         FROM endpoint_api_keys ORDER BY created_at DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::from)
}

pub async fn create_api_key(
    pool: &PgPool,
    id: Uuid,
    endpoint_id: Uuid,
    name: &str,
    key_prefix: &str,
    key_hash: &[u8],
) -> Result<ApiKeyRecord, ApiError> {
    sqlx::query_as::<_, ApiKeyRecord>(
        "INSERT INTO endpoint_api_keys (id, endpoint_id, name, key_prefix, key_hash)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING id, endpoint_id, name, key_prefix, created_at, revoked_at",
    )
    .bind(id)
    .bind(endpoint_id)
    .bind(name)
    .bind(key_prefix)
    .bind(key_hash)
    .fetch_one(pool)
    .await
    .map_err(map_database_error)
}

pub async fn revoke_api_key(pool: &PgPool, id: Uuid) -> Result<ApiKeyRecord, ApiError> {
    sqlx::query_as::<_, ApiKeyRecord>(
        "UPDATE endpoint_api_keys SET revoked_at = COALESCE(revoked_at, NOW()) WHERE id = $1
         RETURNING id, endpoint_id, name, key_prefix, created_at, revoked_at",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)
}

pub async fn api_key_matches_endpoint(
    pool: &PgPool,
    endpoint_id: Uuid,
    key_hash: &[u8],
) -> Result<bool, ApiError> {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(
            SELECT 1 FROM endpoint_api_keys
            WHERE endpoint_id = $1 AND key_hash = $2 AND revoked_at IS NULL
         )",
    )
    .bind(endpoint_id)
    .bind(key_hash)
    .fetch_one(pool)
    .await
    .map_err(ApiError::from)
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
            let key = (session.gateway_id.clone(), id.to_string());
            if !seen.insert(key) {
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

pub async fn create_trace(
    pool: &PgPool,
    request_id: Uuid,
    endpoint_id: Uuid,
    gateway_session_id: Option<Uuid>,
    request: &Value,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO endpoint_traces
            (id, request_id, endpoint_id, gateway_session_id, status, request)
         VALUES ($1, $2, $3, $4, 'pending', $5)",
    )
    .bind(Uuid::new_v4())
    .bind(request_id)
    .bind(endpoint_id)
    .bind(gateway_session_id)
    .bind(request)
    .execute(pool)
    .await
    .map_err(map_database_error)?;
    Ok(())
}

pub async fn create_project_trace(
    pool: &PgPool,
    request_id: Uuid,
    project_id: Uuid,
    gateway_session_id: Option<Uuid>,
    request: &Value,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO endpoint_traces
            (id, request_id, project_id, gateway_session_id, status, request)
         VALUES ($1, $2, $3, $4, 'pending', $5)",
    )
    .bind(Uuid::new_v4())
    .bind(request_id)
    .bind(project_id)
    .bind(gateway_session_id)
    .bind(request)
    .execute(pool)
    .await
    .map_err(map_database_error)?;
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
        "SELECT id, request_id, endpoint_id, project_id, gateway_session_id, status, latency_ms,
                request, response, error, created_at, completed_at
         FROM endpoint_traces",
    );
    if let Some(endpoint_id) = endpoint_id {
        query.push(" WHERE endpoint_id = ").push_bind(endpoint_id);
    } else if let Some(project_id) = project_id {
        query.push(" WHERE project_id = ").push_bind(project_id);
    }
    query
        .push(" ORDER BY created_at DESC LIMIT ")
        .push_bind(limit);
    query
        .build_query_as::<EndpointTrace>()
        .fetch_all(pool)
        .await
        .map_err(ApiError::from)
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
        _ => return Err(ApiError::Internal),
    };
    Ok(statement)
}

pub fn elapsed_millis(started_at: std::time::Instant) -> i64 {
    let millis = started_at.elapsed().as_millis();
    i64::try_from(millis).unwrap_or(i64::MAX)
}

pub fn timestamp() -> chrono::DateTime<Utc> {
    Utc::now()
}

#[cfg(test)]
mod tests {
    use super::delete_statement;

    #[test]
    fn project_resources_use_the_scoped_delete_statement() {
        assert_eq!(
            delete_statement("projects").expect("projects must be deletable"),
            "DELETE FROM projects WHERE id = $1"
        );
    }
}
