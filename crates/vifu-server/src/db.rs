use std::collections::HashSet;

use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use sqlx::types::Json;
use sqlx::{FromRow, PgPool, Postgres, QueryBuilder};
use uuid::Uuid;

use crate::error::{map_database_error, ApiError};
use crate::models::{
    slugify, validate_slug, AgentBinding, AgentEndpoint, AgentGatewayCredential,
    AgentGatewaySession, AgentProfile, ApiKeyAgentScope, ApiKeyPermissions, ApiKeyRecord,
    AvailableAgent, EndpointRoute, EndpointTrace, Project, ProjectCanvas, ProjectCanvasEdge,
    ProjectCanvasNode, ProjectWithBindings, ProviderConnection, ProviderConnectionSecret,
};

#[derive(Debug, FromRow)]
struct ApiKeyRow {
    id: Uuid,
    project_id: Uuid,
    name: String,
    scope_mode: String,
    binding_ids: Vec<Uuid>,
    permissions: Json<ApiKeyPermissions>,
    key_prefix: String,
    created_at: DateTime<Utc>,
    revoked_at: Option<DateTime<Utc>>,
}

impl TryFrom<ApiKeyRow> for ApiKeyRecord {
    type Error = ApiError;

    fn try_from(row: ApiKeyRow) -> Result<Self, Self::Error> {
        let agent_scope = match row.scope_mode.as_str() {
            "all" if row.binding_ids.is_empty() => ApiKeyAgentScope::All,
            "selected" => ApiKeyAgentScope::Selected {
                binding_ids: row.binding_ids,
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
                created_at, updated_at
         FROM projects ORDER BY created_at ASC",
    )
    .fetch_all(pool)
    .await?;
    projects_with_bindings(pool, projects).await
}

pub async fn get_project(pool: &PgPool, id: Uuid) -> Result<ProjectWithBindings, ApiError> {
    let project = sqlx::query_as::<_, Project>(
        "SELECT id, slug, name, description, gateway_id, enabled,
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
        "SELECT id, slug, name, description, gateway_id, enabled,
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

pub async fn create_project(
    pool: &PgPool,
    project: NewProject<'_>,
) -> Result<ProjectWithBindings, ApiError> {
    validate_project_bindings(pool, project.gateway_id, project.binding_ids).await?;
    let mut transaction = pool.begin().await?;
    let created = sqlx::query_as::<_, Project>(
        "INSERT INTO projects
            (id, slug, name, description, gateway_id)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING id, slug, name, description, gateway_id, enabled,
                   created_at, updated_at",
    )
    .bind(project.id)
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
    sync_project_canvas_nodes(pool, project.id).await?;
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
    sync_project_canvas_nodes(pool, id).await?;
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

pub struct NewProviderConnection<'a> {
    pub provider_key: &'a str,
    pub name: &'a str,
    pub provider_type: &'a str,
    pub base_url: &'a str,
    pub config: &'a Value,
    pub encrypted_secret_json: &'a str,
    pub secret_keys: &'a [String],
    pub display_secret: Option<&'a str>,
    pub status: &'a str,
}

pub async fn list_provider_connections(
    pool: &PgPool,
    project_slug: &str,
) -> Result<Vec<ProviderConnection>, ApiError> {
    let project = get_project_by_slug(pool, project_slug).await?;
    let connections = sqlx::query_as::<_, ProviderConnection>(
        "SELECT id, project_id, provider_key, name, provider_type, base_url, config,
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
            (id, project_id, provider_key, name, provider_type, base_url, config,
             encrypted_secret_json, secret_keys, display_secret, status)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
         ON CONFLICT (project_id, provider_key) DO UPDATE SET
            name = EXCLUDED.name,
            provider_type = EXCLUDED.provider_type,
            base_url = EXCLUDED.base_url,
            config = EXCLUDED.config,
            encrypted_secret_json = EXCLUDED.encrypted_secret_json,
            secret_keys = EXCLUDED.secret_keys,
            display_secret = EXCLUDED.display_secret,
            status = EXCLUDED.status,
            updated_at = NOW()
         RETURNING id, project_id, provider_key, name, provider_type, base_url, config,
                   secret_keys, display_secret, status, last_checked_at, created_at, updated_at",
    )
    .bind(Uuid::new_v4())
    .bind(project.project.id)
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

pub async fn get_provider_connection_secret(
    pool: &PgPool,
    id: Uuid,
) -> Result<ProviderConnectionSecret, ApiError> {
    sqlx::query_as::<_, ProviderConnectionSecret>(
        "SELECT id, project_id, provider_key, name, provider_type, base_url, config,
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
        "SELECT id, project_id, provider_key, name, provider_type, base_url, config,
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
         RETURNING id, project_id, provider_key, name, provider_type, base_url, config,
                   secret_keys, display_secret, status, last_checked_at, created_at, updated_at",
    )
    .bind(id)
    .bind(status)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)
}

pub async fn delete_provider_connection(pool: &PgPool, id: Uuid) -> Result<(), ApiError> {
    delete_by_id(pool, "provider_connections", id).await
}

pub async fn delete_provider_connection_by_key(
    pool: &PgPool,
    project_slug: &str,
    provider_key: &str,
) -> Result<(), ApiError> {
    let project = get_project_by_slug(pool, project_slug).await?;
    let result =
        sqlx::query("DELETE FROM provider_connections WHERE project_id = $1 AND provider_key = $2")
            .bind(project.project.id)
            .bind(provider_key)
            .execute(pool)
            .await?;
    if result.rows_affected() == 0 {
        Err(ApiError::NotFound)
    } else {
        Ok(())
    }
}

pub async fn get_project_canvas(pool: &PgPool, slug: &str) -> Result<ProjectCanvas, ApiError> {
    let project = get_project_by_slug(pool, slug).await?;
    sync_project_canvas_nodes(pool, project.project.id).await?;
    let project = get_project(pool, project.project.id).await?;
    Ok(ProjectCanvas {
        nodes: list_canvas_nodes(pool, project.project.id).await?,
        edges: list_canvas_edges(pool, project.project.id).await?,
        project,
    })
}

pub async fn create_canvas_node(
    pool: &PgPool,
    project_slug: &str,
    node: NewCanvasNode<'_>,
) -> Result<ProjectCanvasNode, ApiError> {
    let project = get_project_by_slug(pool, project_slug).await?;
    let mut binding_id = node.binding_id;
    let mut profile_id = node.profile_id;
    if binding_id.is_none() {
        if let (Some(gateway_id), Some(resource_id)) = (node.gateway_id, node.resource_id) {
            let agent_name = node
                .config
                .get("agentName")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .unwrap_or(resource_id);
            let discovered_binding_id =
                ensure_discovered_binding(pool, gateway_id, resource_id, agent_name).await?;
            let binding = get_binding(pool, discovered_binding_id).await?;
            binding_id = Some(binding.id);
            profile_id = Some(binding.profile_id);
        }
    }
    if let Some(binding_id) = binding_id {
        validate_project_bindings(pool, &project.project.gateway_id, &[binding_id]).await?;
        sqlx::query("INSERT INTO project_bindings (project_id, binding_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
            .bind(project.project.id)
            .bind(binding_id)
            .execute(pool)
            .await?;
    }
    sqlx::query_as::<_, ProjectCanvasNode>(
        "INSERT INTO project_canvas_nodes
            (id, project_id, kind, position, profile_id, binding_id, gateway_id,
             resource_id, config, inputs, outputs, exposed)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
         RETURNING id, project_id, kind, position, profile_id, binding_id, gateway_id,
                   resource_id, config, inputs, outputs, exposed, created_at, updated_at",
    )
    .bind(Uuid::new_v4())
    .bind(project.project.id)
    .bind(node.kind)
    .bind(node.position)
    .bind(profile_id)
    .bind(binding_id)
    .bind(node.gateway_id)
    .bind(node.resource_id)
    .bind(node.config)
    .bind(node.inputs)
    .bind(node.outputs)
    .bind(node.exposed)
    .fetch_one(pool)
    .await
    .map_err(map_database_error)
}

pub struct NewCanvasNode<'a> {
    pub kind: &'a str,
    pub position: &'a Value,
    pub profile_id: Option<Uuid>,
    pub binding_id: Option<Uuid>,
    pub gateway_id: Option<&'a str>,
    pub resource_id: Option<&'a str>,
    pub config: &'a Value,
    pub inputs: &'a Value,
    pub outputs: &'a Value,
    pub exposed: bool,
}

pub async fn update_canvas_node(
    pool: &PgPool,
    project_slug: &str,
    node_id: Uuid,
    patch: CanvasNodePatch<'_>,
) -> Result<ProjectCanvasNode, ApiError> {
    let project = get_project_by_slug(pool, project_slug).await?;
    sqlx::query_as::<_, ProjectCanvasNode>(
        "UPDATE project_canvas_nodes SET
            position = COALESCE($3, position),
            config = COALESCE($4, config),
            inputs = COALESCE($5, inputs),
            outputs = COALESCE($6, outputs),
            exposed = COALESCE($7, exposed),
            updated_at = NOW()
         WHERE project_id = $1 AND id = $2
         RETURNING id, project_id, kind, position, profile_id, binding_id, gateway_id,
                   resource_id, config, inputs, outputs, exposed, created_at, updated_at",
    )
    .bind(project.project.id)
    .bind(node_id)
    .bind(patch.position)
    .bind(patch.config)
    .bind(patch.inputs)
    .bind(patch.outputs)
    .bind(patch.exposed)
    .fetch_optional(pool)
    .await
    .map_err(map_database_error)?
    .ok_or(ApiError::NotFound)
}

pub struct CanvasNodePatch<'a> {
    pub position: Option<&'a Value>,
    pub config: Option<&'a Value>,
    pub inputs: Option<&'a Value>,
    pub outputs: Option<&'a Value>,
    pub exposed: Option<bool>,
}

pub async fn delete_canvas_node(
    pool: &PgPool,
    project_slug: &str,
    node_id: Uuid,
) -> Result<(), ApiError> {
    let project = get_project_by_slug(pool, project_slug).await?;
    let mut transaction = pool.begin().await?;
    let binding_id = sqlx::query_scalar::<_, Option<Uuid>>(
        "DELETE FROM project_canvas_nodes
         WHERE project_id = $1 AND id = $2
         RETURNING binding_id",
    )
    .bind(project.project.id)
    .bind(node_id)
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or(ApiError::NotFound)?;
    if let Some(binding_id) = binding_id {
        sqlx::query("DELETE FROM project_bindings WHERE project_id = $1 AND binding_id = $2")
            .bind(project.project.id)
            .bind(binding_id)
            .execute(&mut *transaction)
            .await?;
    }
    transaction.commit().await?;
    Ok(())
}

pub async fn create_canvas_edge(
    pool: &PgPool,
    project_slug: &str,
    edge: NewCanvasEdge<'_>,
) -> Result<ProjectCanvasEdge, ApiError> {
    let project = get_project_by_slug(pool, project_slug).await?;
    sqlx::query_as::<_, ProjectCanvasEdge>(
        "INSERT INTO project_canvas_edges
            (id, project_id, source_node_id, source_handle, target_node_id,
             target_handle, kind, config)
         SELECT $1, $2, $3, $4, $5, $6, $7, $8
         WHERE EXISTS(SELECT 1 FROM project_canvas_nodes WHERE project_id = $2 AND id = $3)
           AND EXISTS(SELECT 1 FROM project_canvas_nodes WHERE project_id = $2 AND id = $5)
         RETURNING id, project_id, source_node_id, source_handle, target_node_id,
                   target_handle, kind, config, created_at, updated_at",
    )
    .bind(Uuid::new_v4())
    .bind(project.project.id)
    .bind(edge.source_node_id)
    .bind(edge.source_handle)
    .bind(edge.target_node_id)
    .bind(edge.target_handle)
    .bind(edge.kind)
    .bind(edge.config)
    .fetch_optional(pool)
    .await
    .map_err(map_database_error)?
    .ok_or_else(|| ApiError::Invalid("edge nodes must belong to the project".to_string()))
}

pub struct NewCanvasEdge<'a> {
    pub source_node_id: Uuid,
    pub source_handle: Option<&'a str>,
    pub target_node_id: Uuid,
    pub target_handle: Option<&'a str>,
    pub kind: &'a str,
    pub config: &'a Value,
}

pub async fn delete_canvas_edge(
    pool: &PgPool,
    project_slug: &str,
    edge_id: Uuid,
) -> Result<(), ApiError> {
    let project = get_project_by_slug(pool, project_slug).await?;
    let result = sqlx::query("DELETE FROM project_canvas_edges WHERE project_id = $1 AND id = $2")
        .bind(project.project.id)
        .bind(edge_id)
        .execute(pool)
        .await
        .map_err(map_database_error)?;
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

async fn list_canvas_nodes(
    pool: &PgPool,
    project_id: Uuid,
) -> Result<Vec<ProjectCanvasNode>, ApiError> {
    sqlx::query_as::<_, ProjectCanvasNode>(
        "SELECT id, project_id, kind, position, profile_id, binding_id, gateway_id,
                resource_id, config, inputs, outputs, exposed, created_at, updated_at
         FROM project_canvas_nodes
         WHERE project_id = $1
         ORDER BY created_at ASC",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await
    .map_err(ApiError::from)
}

async fn list_canvas_edges(
    pool: &PgPool,
    project_id: Uuid,
) -> Result<Vec<ProjectCanvasEdge>, ApiError> {
    sqlx::query_as::<_, ProjectCanvasEdge>(
        "SELECT id, project_id, source_node_id, source_handle, target_node_id,
                target_handle, kind, config, created_at, updated_at
         FROM project_canvas_edges
         WHERE project_id = $1
         ORDER BY created_at ASC",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await
    .map_err(ApiError::from)
}

pub async fn sync_project_canvas_nodes(pool: &PgPool, project_id: Uuid) -> Result<(), ApiError> {
    let project = get_project(pool, project_id).await?;
    let mut binding_ids = project.binding_ids.clone();
    let existing_nodes = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM project_canvas_nodes WHERE project_id = $1",
    )
    .bind(project_id)
    .fetch_one(pool)
    .await?;
    if binding_ids.is_empty() && existing_nodes == 0 {
        let available_agents = list_available_agents(pool).await?;
        for agent in available_agents {
            if agent.status != "connected" {
                continue;
            }
            let binding_id =
                ensure_discovered_binding(pool, &agent.gateway_id, &agent.id, &agent.name).await?;
            sqlx::query(
                "INSERT INTO project_bindings (project_id, binding_id)
                 VALUES ($1, $2)
                 ON CONFLICT DO NOTHING",
            )
            .bind(project_id)
            .bind(binding_id)
            .execute(pool)
            .await?;
            binding_ids.push(binding_id);
        }
        binding_ids.sort_unstable();
        binding_ids.dedup();
    }

    sqlx::query(
        "DELETE FROM project_canvas_nodes node
         WHERE node.project_id = $1
           AND node.binding_id IS NOT NULL
           AND NOT EXISTS (
                SELECT 1 FROM project_bindings binding
                WHERE binding.project_id = node.project_id
                  AND binding.binding_id = node.binding_id
           )",
    )
    .bind(project_id)
    .execute(pool)
    .await?;

    seed_canvas_nodes_for_bindings(pool, project_id, &binding_ids).await
}

async fn seed_canvas_nodes_for_bindings(
    pool: &PgPool,
    project_id: Uuid,
    binding_ids: &[Uuid],
) -> Result<(), ApiError> {
    for (index, binding_id) in binding_ids.iter().enumerate() {
        let binding = get_binding(pool, *binding_id).await?;
        let position = default_canvas_position(index);
        sqlx::query(
            "INSERT INTO project_canvas_nodes
                (id, project_id, kind, position, profile_id, binding_id, gateway_id,
                 resource_id, config, inputs, outputs, exposed)
             VALUES ($1, $2, 'agent', $3, $4, $5, $6, $7, $8, '{}'::jsonb, '{}'::jsonb, TRUE)
             ON CONFLICT DO NOTHING",
        )
        .bind(Uuid::new_v4())
        .bind(project_id)
        .bind(position)
        .bind(binding.profile_id)
        .bind(binding.id)
        .bind(binding.gateway_id)
        .bind(binding.agent_id)
        .bind(binding.config)
        .execute(pool)
        .await
        .map_err(map_database_error)?;
    }
    Ok(())
}

fn default_canvas_position(index: usize) -> Value {
    let column = index % 3;
    let row = index / 3;
    json!({
        "x": 360 + (column as i64 * 280),
        "y": 160 + (row as i64 * 190),
    })
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
    sync_project_canvas_nodes(pool, project_id).await?;
    sqlx::query_as::<_, AgentEndpoint>(
        "SELECT endpoint.id, endpoint.slug, endpoint.name, endpoint.profile_id,
                endpoint.binding_id, endpoint.enabled, endpoint.request_timeout_ms,
                endpoint.created_at, endpoint.updated_at
         FROM agent_endpoints endpoint
         JOIN project_bindings pb ON pb.binding_id = endpoint.binding_id
         JOIN projects project ON project.id = pb.project_id
         JOIN project_canvas_nodes node
           ON node.project_id = project.id
          AND node.binding_id = endpoint.binding_id
          AND node.exposed = TRUE
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
    let rows = sqlx::query_as::<_, ApiKeyRow>(
        "SELECT key.id, key.project_id, key.name, key.scope_mode,
                COALESCE(
                    ARRAY_AGG(scope.binding_id ORDER BY scope.binding_id)
                        FILTER (WHERE scope.binding_id IS NOT NULL),
                    ARRAY[]::UUID[]
                ) AS binding_ids,
                key.permissions, key.key_prefix, key.created_at, key.revoked_at
         FROM api_keys key
         LEFT JOIN api_key_agent_scopes scope ON scope.api_key_id = key.id
         GROUP BY key.id
         ORDER BY key.created_at DESC",
    )
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(ApiKeyRecord::try_from).collect()
}

pub struct NewApiKey<'a> {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: &'a str,
    pub agent_scope: &'a ApiKeyAgentScope,
    pub permissions: &'a ApiKeyPermissions,
    pub key_prefix: &'a str,
    pub key_hash: &'a [u8],
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
    insert_api_key_agent_scopes(
        &mut transaction,
        input.id,
        input.project_id,
        input.agent_scope.binding_ids(),
    )
    .await?;
    transaction.commit().await?;
    get_api_key(pool, input.id).await
}

pub async fn get_api_key(pool: &PgPool, id: Uuid) -> Result<ApiKeyRecord, ApiError> {
    let row = sqlx::query_as::<_, ApiKeyRow>(
        "SELECT key.id, key.project_id, key.name, key.scope_mode,
                COALESCE(
                    ARRAY_AGG(scope.binding_id ORDER BY scope.binding_id)
                        FILTER (WHERE scope.binding_id IS NOT NULL),
                    ARRAY[]::UUID[]
                ) AS binding_ids,
                key.permissions, key.key_prefix, key.created_at, key.revoked_at
         FROM api_keys key
         LEFT JOIN api_key_agent_scopes scope ON scope.api_key_id = key.id
         WHERE key.id = $1
         GROUP BY key.id",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)?;
    ApiKeyRecord::try_from(row)
}

pub struct ApiKeyPatch<'a> {
    pub project_id: Option<Uuid>,
    pub name: Option<&'a str>,
    pub agent_scope: Option<&'a ApiKeyAgentScope>,
    pub permissions: Option<&'a ApiKeyPermissions>,
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
    sqlx::query("DELETE FROM api_key_agent_scopes WHERE api_key_id = $1")
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
    insert_api_key_agent_scopes(
        &mut transaction,
        updated,
        project_id,
        agent_scope.binding_ids(),
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

pub async fn active_api_key_by_hash(
    pool: &PgPool,
    key_hash: &[u8],
) -> Result<ApiKeyRecord, ApiError> {
    let row = sqlx::query_as::<_, ApiKeyRow>(
        "SELECT key.id, key.project_id, key.name, key.scope_mode,
                COALESCE(
                    ARRAY_AGG(scope.binding_id ORDER BY scope.binding_id)
                        FILTER (WHERE scope.binding_id IS NOT NULL),
                    ARRAY[]::UUID[]
                ) AS binding_ids,
                key.permissions, key.key_prefix, key.created_at, key.revoked_at
         FROM api_keys key
         LEFT JOIN api_key_agent_scopes scope ON scope.api_key_id = key.id
         WHERE key.key_hash = $1 AND key.revoked_at IS NULL
         GROUP BY key.id",
    )
    .bind(key_hash)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::Forbidden)?;
    ApiKeyRecord::try_from(row)
}

async fn validate_api_key_agent_scope(
    pool: &PgPool,
    project_id: Uuid,
    agent_scope: &ApiKeyAgentScope,
) -> Result<(), ApiError> {
    let ApiKeyAgentScope::Selected { binding_ids } = agent_scope else {
        return Ok(());
    };
    if binding_ids.is_empty() || binding_ids.len() > 256 {
        return Err(ApiError::Invalid(
            "selected agent access requires between 1 and 256 agents".to_string(),
        ));
    }
    let count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM project_bindings
         WHERE project_id = $1 AND binding_id = ANY($2)",
    )
    .bind(project_id)
    .bind(binding_ids)
    .fetch_one(pool)
    .await?;
    if usize::try_from(count).ok() != Some(binding_ids.len()) {
        return Err(ApiError::Invalid(
            "selected agents must belong to the API key project".to_string(),
        ));
    }
    Ok(())
}

async fn insert_api_key_agent_scopes(
    transaction: &mut sqlx::Transaction<'_, Postgres>,
    api_key_id: Uuid,
    project_id: Uuid,
    binding_ids: &[Uuid],
) -> Result<(), ApiError> {
    for binding_id in binding_ids {
        sqlx::query(
            "INSERT INTO api_key_agent_scopes (api_key_id, project_id, binding_id)
             VALUES ($1, $2, $3)",
        )
        .bind(api_key_id)
        .bind(project_id)
        .bind(binding_id)
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
    let project = get_project_by_slug(pool, project_slug).await?;
    sync_project_canvas_nodes(pool, project.project.id).await?;
    const SELECT: &str =
        "SELECT e.id AS endpoint_id, e.slug AS endpoint_slug, e.name AS endpoint_name,
                e.request_timeout_ms, profile.id AS profile_id, binding.id AS binding_id,
                binding.gateway_id, binding.agent_id, binding.config AS binding_config
         FROM agent_endpoints e
         JOIN agent_profiles profile ON profile.id = e.profile_id
         JOIN agent_bindings binding ON binding.id = e.binding_id
         JOIN project_bindings pb ON pb.binding_id = binding.id
         JOIN projects project ON project.id = pb.project_id
         JOIN project_canvas_nodes node
           ON node.project_id = project.id
          AND node.binding_id = binding.id
          AND node.exposed = TRUE
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
    sync_project_canvas_nodes(pool, project_id).await?;
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
         JOIN project_canvas_nodes node
           ON node.project_id = project.id
          AND node.binding_id = binding.id
          AND node.exposed = TRUE
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentGatewayRegistration {
    Registered,
    Existing,
}

#[derive(Debug, FromRow)]
struct AgentGatewayCredentialSecret {
    credential_hash: Vec<u8>,
    revoked_at: Option<DateTime<Utc>>,
}

pub async fn register_agent_gateway_credential(
    pool: &PgPool,
    gateway_id: &str,
    credential_prefix: &str,
    credential_hash: &[u8],
) -> Result<AgentGatewayRegistration, ApiError> {
    let mut transaction = pool.begin().await?;
    let existing = sqlx::query_as::<_, AgentGatewayCredentialSecret>(
        "SELECT credential_hash, revoked_at
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
                    (gateway_id, credential_prefix, credential_hash)
                 VALUES ($1, $2, $3)",
            )
            .bind(gateway_id)
            .bind(credential_prefix)
            .bind(credential_hash)
            .execute(&mut *transaction)
            .await
            .map_err(map_database_error)?;
            AgentGatewayRegistration::Registered
        }
        Some(existing)
            if existing.revoked_at.is_none() && existing.credential_hash == credential_hash =>
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
    project_id: Option<Uuid>,
    gateway_session_id: Option<Uuid>,
    request: &Value,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO endpoint_traces
            (id, request_id, endpoint_id, project_id, gateway_session_id, status, request)
         VALUES ($1, $2, $3, $4, $5, 'pending', $6)",
    )
    .bind(Uuid::new_v4())
    .bind(request_id)
    .bind(endpoint_id)
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
        "SELECT trace.id, trace.request_id, trace.endpoint_id,
                trace.project_id,
                trace.gateway_session_id, trace.status, trace.latency_ms,
                trace.request, trace.response, trace.error, trace.created_at, trace.completed_at
         FROM endpoint_traces trace",
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
        "provider_connections" => "DELETE FROM provider_connections WHERE id = $1",
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
