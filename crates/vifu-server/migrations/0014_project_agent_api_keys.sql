DROP TABLE endpoint_api_keys;

CREATE TABLE api_keys (
    id UUID PRIMARY KEY,
    key_type TEXT NOT NULL CHECK (key_type IN ('project', 'agent')),
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    agent_id TEXT,
    name TEXT NOT NULL,
    permissions JSONB NOT NULL,
    key_prefix TEXT NOT NULL,
    key_hash BYTEA NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    revoked_at TIMESTAMPTZ,
    CHECK (
        (key_type = 'project' AND agent_id IS NULL)
        OR (key_type = 'agent' AND agent_id IS NOT NULL)
    ),
    CHECK (
        jsonb_typeof(permissions) = 'object'
        AND permissions ?& ARRAY['chatCompletions', 'agents', 'project']
        AND permissions - 'chatCompletions' - 'agents' - 'project' = '{}'::jsonb
        AND permissions->>'chatCompletions' IN ('none', 'access')
        AND permissions->>'agents' IN ('none', 'read', 'write')
        AND permissions->>'project' IN ('none', 'read', 'write')
    )
);

CREATE INDEX api_keys_project_created_idx
    ON api_keys (project_id, created_at DESC);

CREATE INDEX api_keys_project_active_idx
    ON api_keys (project_id, key_type)
    WHERE revoked_at IS NULL;
