CREATE TABLE runtime_deployments (
    id BLOB PRIMARY KEY,
    project_id BLOB NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    is_primary INTEGER NOT NULL DEFAULT 0 CHECK (is_primary IN (0, 1)),
    config_sync_enabled INTEGER NOT NULL DEFAULT 1
        CHECK (config_sync_enabled IN (0, 1)),
    trace_mode TEXT NOT NULL DEFAULT 'summary'
        CHECK (trace_mode IN ('off', 'summary', 'full')),
    remote_invocation_enabled INTEGER NOT NULL DEFAULT 0
        CHECK (remote_invocation_enabled IN (0, 1)),
    active_release_version INTEGER,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (project_id, name),
    CHECK (active_release_version IS NULL OR active_release_version > 0)
);

CREATE UNIQUE INDEX runtime_deployments_one_primary_idx
    ON runtime_deployments(project_id)
    WHERE is_primary = 1;

CREATE TABLE runtime_deployment_gateways (
    deployment_id BLOB NOT NULL REFERENCES runtime_deployments(id) ON DELETE CASCADE,
    gateway_id TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (deployment_id, gateway_id)
);

CREATE INDEX runtime_deployment_gateways_gateway_idx
    ON runtime_deployment_gateways(gateway_id);

CREATE TABLE project_runtime_releases (
    id BLOB PRIMARY KEY,
    project_id BLOB NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    version INTEGER NOT NULL CHECK (version > 0),
    content_hash TEXT NOT NULL,
    manifest TEXT NOT NULL CHECK (json_valid(manifest)),
    created_by TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (project_id, version),
    UNIQUE (project_id, content_hash)
);

INSERT OR IGNORE INTO runtime_deployments (
    id,
    project_id,
    name,
    is_primary,
    config_sync_enabled,
    trace_mode,
    remote_invocation_enabled
)
SELECT
    randomblob(16),
    project.id,
    'development',
    1,
    1,
    'summary',
    0
FROM projects AS project;

INSERT OR IGNORE INTO runtime_deployment_gateways (deployment_id, gateway_id)
SELECT deployment.id, project.gateway_id
FROM runtime_deployments AS deployment
JOIN projects AS project ON project.id = deployment.project_id
WHERE deployment.name = 'development'
  AND project.gateway_id <> '';

ALTER TABLE agent_gateway_enrollments ADD COLUMN deployment_id BLOB
    REFERENCES runtime_deployments(id) ON DELETE CASCADE;

UPDATE agent_gateway_enrollments
SET deployment_id = (
    SELECT deployment.id
    FROM runtime_deployments AS deployment
    WHERE deployment.project_id = agent_gateway_enrollments.project_id
      AND deployment.is_primary = 1
    LIMIT 1
)
WHERE deployment_id IS NULL;

CREATE INDEX agent_gateway_enrollments_deployment_idx
    ON agent_gateway_enrollments(deployment_id, expires_at)
    WHERE consumed_at IS NULL AND revoked_at IS NULL;
