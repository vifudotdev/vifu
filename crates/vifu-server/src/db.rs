use chrono::Utc;
use serde_json::Value;
use sqlx::{PgPool, Postgres, QueryBuilder};
use uuid::Uuid;

use crate::error::{map_database_error, ApiError};
use crate::models::{
    AgentBinding, AgentEndpoint, AgentProfile, ApiKeyRecord, ConnectorSession, EndpointRoute,
    EndpointTrace,
};

pub async fn migrate(pool: &PgPool) -> Result<(), ApiError> {
    sqlx::migrate!("./migrations").run(pool).await?;
    Ok(())
}

pub async fn ready(pool: &PgPool) -> Result<(), ApiError> {
    sqlx::query("SELECT 1").execute(pool).await?;
    Ok(())
}

pub async fn mark_connector_sessions_disconnected(pool: &PgPool) -> Result<(), ApiError> {
    sqlx::query(
        "UPDATE connector_sessions SET
            status = 'disconnected', last_seen_at = NOW(), disconnected_at = NOW()
         WHERE status = 'connected'",
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_profiles(pool: &PgPool) -> Result<Vec<AgentProfile>, ApiError> {
    sqlx::query_as::<_, AgentProfile>(
        "SELECT id, slug, name, description, instructions, created_at, updated_at
         FROM agent_profiles ORDER BY created_at ASC",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::from)
}

pub async fn get_profile(pool: &PgPool, id: Uuid) -> Result<AgentProfile, ApiError> {
    sqlx::query_as::<_, AgentProfile>(
        "SELECT id, slug, name, description, instructions, created_at, updated_at
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
    instructions: Option<&str>,
) -> Result<AgentProfile, ApiError> {
    sqlx::query_as::<_, AgentProfile>(
        "INSERT INTO agent_profiles (id, slug, name, description, instructions)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING id, slug, name, description, instructions, created_at, updated_at",
    )
    .bind(id)
    .bind(slug)
    .bind(name)
    .bind(description)
    .bind(instructions)
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
            instructions = CASE WHEN $6 THEN $7 ELSE instructions END,
            updated_at = NOW()
         WHERE id = $1
         RETURNING id, slug, name, description, instructions, created_at, updated_at",
    )
    .bind(id)
    .bind(patch.slug)
    .bind(patch.name)
    .bind(patch.description_changed)
    .bind(patch.description)
    .bind(patch.instructions_changed)
    .bind(patch.instructions)
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
    pub instructions_changed: bool,
    pub instructions: Option<&'a str>,
}

pub async fn delete_profile(pool: &PgPool, id: Uuid) -> Result<(), ApiError> {
    delete_by_id(pool, "agent_profiles", id).await
}

pub async fn list_bindings(pool: &PgPool) -> Result<Vec<AgentBinding>, ApiError> {
    sqlx::query_as::<_, AgentBinding>(
        "SELECT id, profile_id, provider, connector_id, agent_id, config, created_at, updated_at
         FROM agent_bindings ORDER BY created_at ASC",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::from)
}

pub async fn get_binding(pool: &PgPool, id: Uuid) -> Result<AgentBinding, ApiError> {
    sqlx::query_as::<_, AgentBinding>(
        "SELECT id, profile_id, provider, connector_id, agent_id, config, created_at, updated_at
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
    connector_id: &str,
    agent_id: &str,
    config: &Value,
) -> Result<AgentBinding, ApiError> {
    sqlx::query_as::<_, AgentBinding>(
        "INSERT INTO agent_bindings
            (id, profile_id, provider, connector_id, agent_id, config)
         VALUES ($1, $2, $3, $4, $5, $6)
         RETURNING id, profile_id, provider, connector_id, agent_id, config, created_at, updated_at",
    )
    .bind(id)
    .bind(profile_id)
    .bind(provider)
    .bind(connector_id)
    .bind(agent_id)
    .bind(config)
    .fetch_one(pool)
    .await
    .map_err(map_database_error)
}

pub async fn update_binding(
    pool: &PgPool,
    id: Uuid,
    connector_id: Option<&str>,
    agent_id: Option<&str>,
    config: Option<&Value>,
) -> Result<AgentBinding, ApiError> {
    sqlx::query_as::<_, AgentBinding>(
        "UPDATE agent_bindings SET
            connector_id = COALESCE($2, connector_id),
            agent_id = COALESCE($3, agent_id),
            config = COALESCE($4, config),
            updated_at = NOW()
         WHERE id = $1
         RETURNING id, profile_id, provider, connector_id, agent_id, config, created_at, updated_at",
    )
    .bind(id)
    .bind(connector_id)
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
                e.request_timeout_ms, p.id AS profile_id, p.name AS profile_name,
                p.instructions AS profile_instructions, b.id AS binding_id,
                b.connector_id, b.agent_id, b.config AS binding_config
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

pub async fn open_connector_session(
    pool: &PgPool,
    connector_id: &str,
    resume_session_id: Option<Uuid>,
    agents: &Value,
    metadata: &Value,
) -> Result<(Uuid, bool), ApiError> {
    let mut transaction = pool.begin().await?;
    sqlx::query(
        "UPDATE connector_sessions SET
            status = 'disconnected', last_seen_at = NOW(), disconnected_at = NOW()
         WHERE connector_id = $1 AND status = 'connected'",
    )
    .bind(connector_id)
    .execute(&mut *transaction)
    .await?;

    if let Some(session_id) = resume_session_id {
        let updated = sqlx::query_scalar::<_, Uuid>(
            "UPDATE connector_sessions SET
                status = 'connected', agents = $3, metadata = $4,
                connected_at = NOW(), last_seen_at = NOW(), disconnected_at = NULL
             WHERE session_id = $1 AND connector_id = $2
             RETURNING session_id",
        )
        .bind(session_id)
        .bind(connector_id)
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
        "INSERT INTO connector_sessions
            (id, connector_id, session_id, status, agents, metadata)
         VALUES ($1, $2, $3, 'connected', $4, $5)",
    )
    .bind(id)
    .bind(connector_id)
    .bind(session_id)
    .bind(agents)
    .bind(metadata)
    .execute(&mut *transaction)
    .await
    .map_err(map_database_error)?;
    transaction.commit().await?;
    Ok((session_id, false))
}

pub async fn touch_connector_session(pool: &PgPool, session_id: Uuid) -> Result<(), ApiError> {
    sqlx::query(
        "UPDATE connector_sessions SET last_seen_at = NOW(), status = 'connected'
         WHERE session_id = $1",
    )
    .bind(session_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn close_connector_session(pool: &PgPool, session_id: Uuid) -> Result<(), ApiError> {
    sqlx::query(
        "UPDATE connector_sessions SET
            status = 'disconnected', last_seen_at = NOW(), disconnected_at = NOW()
         WHERE session_id = $1",
    )
    .bind(session_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_connector_sessions(pool: &PgPool) -> Result<Vec<ConnectorSession>, ApiError> {
    sqlx::query_as::<_, ConnectorSession>(
        "SELECT id, connector_id, session_id, status, agents, metadata,
                connected_at, last_seen_at, disconnected_at
         FROM connector_sessions ORDER BY connected_at DESC LIMIT 200",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::from)
}

pub async fn create_trace(
    pool: &PgPool,
    request_id: Uuid,
    endpoint_id: Uuid,
    connector_session_id: Option<Uuid>,
    request: &Value,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO endpoint_traces
            (id, request_id, endpoint_id, connector_session_id, status, request)
         VALUES ($1, $2, $3, $4, 'pending', $5)",
    )
    .bind(Uuid::new_v4())
    .bind(request_id)
    .bind(endpoint_id)
    .bind(connector_session_id)
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
    limit: i64,
) -> Result<Vec<EndpointTrace>, ApiError> {
    let mut query = QueryBuilder::<Postgres>::new(
        "SELECT id, request_id, endpoint_id, connector_session_id, status, latency_ms,
                request, response, error, created_at, completed_at
         FROM endpoint_traces",
    );
    if let Some(endpoint_id) = endpoint_id {
        query.push(" WHERE endpoint_id = ").push_bind(endpoint_id);
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
    let sql = match table {
        "agent_profiles" => "DELETE FROM agent_profiles WHERE id = $1",
        "agent_bindings" => "DELETE FROM agent_bindings WHERE id = $1",
        "agent_endpoints" => "DELETE FROM agent_endpoints WHERE id = $1",
        _ => return Err(ApiError::Internal),
    };
    let result = sqlx::query(sql)
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

pub fn elapsed_millis(started_at: std::time::Instant) -> i64 {
    let millis = started_at.elapsed().as_millis();
    i64::try_from(millis).unwrap_or(i64::MAX)
}

pub fn timestamp() -> chrono::DateTime<Utc> {
    Utc::now()
}
