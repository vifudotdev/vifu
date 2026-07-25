CREATE TABLE project_runtime_channels (
    id UUID PRIMARY KEY,
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    public_id UUID NOT NULL UNIQUE,
    launch_key_prefix TEXT NOT NULL,
    launch_key_hash BYTEA NOT NULL UNIQUE,
    allowed_origins TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[],
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (length(name) BETWEEN 1 AND 128),
    CHECK (length(launch_key_prefix) BETWEEN 8 AND 32)
);

CREATE INDEX project_runtime_channels_project_idx
    ON project_runtime_channels (project_id, created_at DESC);

CREATE TABLE runtime_launch_sessions (
    id UUID PRIMARY KEY,
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    channel_id UUID NOT NULL REFERENCES project_runtime_channels(id) ON DELETE CASCADE,
    token_hash BYTEA NOT NULL UNIQUE,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX runtime_launch_sessions_expiry_idx
    ON runtime_launch_sessions (expires_at);
