CREATE TABLE provider_stock (
    id UUID PRIMARY KEY,
    provider_key TEXT NOT NULL UNIQUE,
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
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

INSERT INTO provider_stock (
    id,
    provider_key,
    name,
    provider_type,
    base_url,
    config,
    encrypted_secret_json,
    secret_keys,
    display_secret,
    status,
    last_checked_at,
    created_at,
    updated_at
)
SELECT DISTINCT ON (provider_key)
    id,
    provider_key,
    name,
    provider_type,
    base_url,
    config,
    encrypted_secret_json,
    secret_keys,
    display_secret,
    status,
    last_checked_at,
    created_at,
    updated_at
FROM provider_connections
ORDER BY provider_key, updated_at DESC;

CREATE TABLE project_provider_assignments (
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    provider_key TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (project_id, provider_key)
);

INSERT INTO project_provider_assignments (project_id, provider_key, created_at)
SELECT project_id, provider_key, MIN(created_at)
FROM provider_connections
GROUP BY project_id, provider_key
ON CONFLICT DO NOTHING;

CREATE INDEX project_provider_assignments_provider_idx
    ON project_provider_assignments (provider_key, project_id);
