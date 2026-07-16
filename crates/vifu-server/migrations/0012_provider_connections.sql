CREATE TABLE provider_connections (
    id UUID PRIMARY KEY,
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    provider_key TEXT NOT NULL,
    name TEXT NOT NULL,
    provider_type TEXT NOT NULL,
    base_url TEXT NOT NULL,
    config JSONB NOT NULL DEFAULT '{}'::jsonb,
    encrypted_secret_json TEXT NOT NULL,
    secret_keys TEXT[] NOT NULL DEFAULT '{}',
    display_secret TEXT,
    status TEXT NOT NULL DEFAULT 'configured',
    last_checked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (project_id, provider_key)
);

CREATE INDEX provider_connections_project_idx
    ON provider_connections (project_id, created_at ASC);
