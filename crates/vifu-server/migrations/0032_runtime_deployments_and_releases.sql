CREATE TABLE runtime_deployments (
    id UUID PRIMARY KEY,
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    is_primary BOOLEAN NOT NULL DEFAULT FALSE,
    config_sync_enabled BOOLEAN NOT NULL DEFAULT TRUE,
    trace_mode TEXT NOT NULL DEFAULT 'summary'
        CHECK (trace_mode IN ('off', 'summary', 'full')),
    remote_invocation_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    active_release_version BIGINT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (project_id, name),
    CHECK (active_release_version IS NULL OR active_release_version > 0)
);

CREATE UNIQUE INDEX runtime_deployments_one_primary_idx
    ON runtime_deployments(project_id)
    WHERE is_primary;

CREATE TABLE runtime_deployment_gateways (
    deployment_id UUID NOT NULL REFERENCES runtime_deployments(id) ON DELETE CASCADE,
    gateway_id TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (deployment_id, gateway_id)
);

CREATE INDEX runtime_deployment_gateways_gateway_idx
    ON runtime_deployment_gateways(gateway_id);

CREATE TABLE project_runtime_releases (
    id UUID PRIMARY KEY,
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    version BIGINT NOT NULL CHECK (version > 0),
    content_hash TEXT NOT NULL,
    manifest JSONB NOT NULL,
    created_by TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (project_id, version),
    UNIQUE (project_id, content_hash)
);

INSERT INTO runtime_deployments (
    id,
    project_id,
    name,
    is_primary,
    config_sync_enabled,
    trace_mode,
    remote_invocation_enabled
)
SELECT
    gen_random_uuid(),
    project.id,
    'development',
    TRUE,
    TRUE,
    'summary',
    FALSE
FROM projects AS project
ON CONFLICT (project_id, name) DO NOTHING;

INSERT INTO runtime_deployment_gateways (deployment_id, gateway_id)
SELECT deployment.id, project.gateway_id
FROM runtime_deployments AS deployment
JOIN projects AS project ON project.id = deployment.project_id
WHERE deployment.name = 'development'
  AND project.gateway_id <> ''
ON CONFLICT DO NOTHING;

ALTER TABLE agent_gateway_enrollments
    ADD COLUMN deployment_id UUID REFERENCES runtime_deployments(id) ON DELETE CASCADE;

UPDATE agent_gateway_enrollments AS enrollment
SET deployment_id = deployment.id
FROM runtime_deployments AS deployment
WHERE deployment.project_id = enrollment.project_id
  AND deployment.is_primary
  AND enrollment.deployment_id IS NULL;

CREATE INDEX agent_gateway_enrollments_deployment_idx
    ON agent_gateway_enrollments(deployment_id, expires_at)
    WHERE consumed_at IS NULL AND revoked_at IS NULL;
