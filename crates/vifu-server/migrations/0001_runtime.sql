CREATE TABLE agent_profiles (
    id UUID PRIMARY KEY,
    slug TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    description TEXT,
    instructions TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE agent_bindings (
    id UUID PRIMARY KEY,
    profile_id UUID NOT NULL REFERENCES agent_profiles(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    connector_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    config JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (profile_id, connector_id, agent_id)
);

CREATE TABLE agent_endpoints (
    id UUID PRIMARY KEY,
    slug TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    profile_id UUID NOT NULL REFERENCES agent_profiles(id) ON DELETE RESTRICT,
    binding_id UUID NOT NULL REFERENCES agent_bindings(id) ON DELETE RESTRICT,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    request_timeout_ms INTEGER NOT NULL DEFAULT 30000 CHECK (request_timeout_ms BETWEEN 500 AND 120000),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE endpoint_api_keys (
    id UUID PRIMARY KEY,
    endpoint_id UUID NOT NULL REFERENCES agent_endpoints(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    key_prefix TEXT NOT NULL,
    key_hash BYTEA NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    revoked_at TIMESTAMPTZ
);

CREATE TABLE connector_sessions (
    id UUID PRIMARY KEY,
    connector_id TEXT NOT NULL,
    session_id UUID NOT NULL UNIQUE,
    status TEXT NOT NULL,
    agents JSONB NOT NULL DEFAULT '[]'::jsonb,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    connected_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    disconnected_at TIMESTAMPTZ
);

CREATE INDEX connector_sessions_connector_id_idx
    ON connector_sessions (connector_id, connected_at DESC);

CREATE TABLE endpoint_traces (
    id UUID PRIMARY KEY,
    request_id UUID NOT NULL UNIQUE,
    endpoint_id UUID NOT NULL REFERENCES agent_endpoints(id) ON DELETE CASCADE,
    connector_session_id UUID REFERENCES connector_sessions(session_id) ON DELETE SET NULL,
    status TEXT NOT NULL,
    latency_ms BIGINT,
    request JSONB NOT NULL,
    response JSONB,
    error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMPTZ
);

CREATE INDEX endpoint_traces_endpoint_created_idx
    ON endpoint_traces (endpoint_id, created_at DESC);
