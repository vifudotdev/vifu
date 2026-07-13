CREATE TABLE projects (
    id UUID PRIMARY KEY,
    slug TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    description TEXT,
    gateway_id TEXT NOT NULL,
    service_id TEXT NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    access_key_prefix TEXT NOT NULL,
    access_key_hash BYTEA NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE project_bindings (
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    binding_id UUID NOT NULL REFERENCES agent_bindings(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (project_id, binding_id)
);

CREATE INDEX projects_gateway_service_idx
    ON projects (gateway_id, service_id);
