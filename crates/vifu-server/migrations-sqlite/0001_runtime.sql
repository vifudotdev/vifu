CREATE TABLE projects (
    id BLOB PRIMARY KEY,
    slug TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    description TEXT,
    gateway_id TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE agent_profiles (
    id BLOB PRIMARY KEY,
    project_id BLOB,
    slug TEXT NOT NULL,
    name TEXT NOT NULL,
    description TEXT,
    active_version_id BLOB,
    archived_at TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (id, project_id),
    UNIQUE (project_id, slug),
    FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE CASCADE,
    FOREIGN KEY (active_version_id, id)
        REFERENCES agent_profile_versions(id, profile_id)
        DEFERRABLE INITIALLY DEFERRED
);

CREATE TABLE agent_profile_versions (
    id BLOB PRIMARY KEY,
    profile_id BLOB NOT NULL REFERENCES agent_profiles(id) ON DELETE CASCADE,
    version_number INTEGER NOT NULL CHECK (version_number > 0),
    persona TEXT NOT NULL DEFAULT '{"files": {}}' CHECK (json_valid(persona)),
    runtime TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(runtime)),
    presentation TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(presentation)),
    source TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(source)),
    content_hash TEXT NOT NULL,
    change_summary TEXT,
    archived_at TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (profile_id, version_number),
    UNIQUE (id, profile_id)
);

CREATE INDEX agent_profile_versions_profile_created_idx
    ON agent_profile_versions (profile_id, version_number DESC);

CREATE TABLE agent_profile_capabilities (
    id BLOB PRIMARY KEY,
    profile_version_id BLOB NOT NULL
        REFERENCES agent_profile_versions(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('chat', 'speech', 'transcription', 'realtime', 'tool')),
    provider_type TEXT NOT NULL,
    provider_key TEXT NOT NULL,
    resource_id TEXT,
    config TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(config)),
    input_schema TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(input_schema)),
    output_schema TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(output_schema)),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX agent_profile_capabilities_version_kind_idx
    ON agent_profile_capabilities (profile_version_id, kind, created_at ASC);

CREATE TABLE agent_profile_rollouts (
    profile_id BLOB NOT NULL REFERENCES agent_profiles(id) ON DELETE CASCADE,
    profile_version_id BLOB NOT NULL,
    weight_bps INTEGER NOT NULL CHECK (weight_bps BETWEEN 1 AND 10000),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (profile_id, profile_version_id),
    FOREIGN KEY (profile_version_id, profile_id)
        REFERENCES agent_profile_versions(id, profile_id) ON DELETE CASCADE
);

CREATE TABLE agent_bindings (
    id BLOB PRIMARY KEY,
    profile_id BLOB NOT NULL REFERENCES agent_profiles(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    gateway_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    config TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(config)),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (profile_id, gateway_id, agent_id)
);

CREATE TABLE project_bindings (
    project_id BLOB NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    binding_id BLOB NOT NULL REFERENCES agent_bindings(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (project_id, binding_id)
);

CREATE TABLE agent_endpoints (
    id BLOB PRIMARY KEY,
    slug TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    profile_id BLOB NOT NULL REFERENCES agent_profiles(id) ON DELETE RESTRICT,
    binding_id BLOB NOT NULL REFERENCES agent_bindings(id) ON DELETE RESTRICT,
    enabled INTEGER NOT NULL DEFAULT 1,
    request_timeout_ms INTEGER NOT NULL DEFAULT 30000
        CHECK (request_timeout_ms BETWEEN 500 AND 120000),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE provider_connections (
    id BLOB PRIMARY KEY,
    project_id BLOB NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    provider_key TEXT NOT NULL,
    name TEXT NOT NULL,
    provider_type TEXT NOT NULL,
    base_url TEXT NOT NULL,
    config TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(config)),
    encrypted_secret_json TEXT NOT NULL,
    secret_keys TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(secret_keys)),
    display_secret TEXT,
    status TEXT NOT NULL DEFAULT 'configured',
    last_checked_at TEXT,
    source_kind TEXT NOT NULL DEFAULT 'registry'
        CHECK (source_kind IN ('registry', 'custom')),
    source_key TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (project_id, provider_key)
);

CREATE INDEX provider_connections_project_idx
    ON provider_connections (project_id, created_at ASC);

CREATE TABLE custom_providers (
    id BLOB PRIMARY KEY,
    provider_key TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    provider_type TEXT NOT NULL,
    base_url TEXT NOT NULL,
    config TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(config)),
    encrypted_secret_json TEXT NOT NULL,
    secret_keys TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(secret_keys)),
    display_secret TEXT,
    status TEXT NOT NULL DEFAULT 'configured',
    last_checked_at TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE api_keys (
    id BLOB PRIMARY KEY,
    project_id BLOB NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    scope_mode TEXT NOT NULL CHECK (scope_mode IN ('all', 'selected')),
    permissions TEXT NOT NULL CHECK (json_valid(permissions)),
    key_prefix TEXT NOT NULL,
    key_hash BLOB NOT NULL UNIQUE,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    revoked_at TEXT,
    UNIQUE (id, project_id)
);

CREATE INDEX api_keys_project_created_idx
    ON api_keys (project_id, created_at DESC);

CREATE INDEX api_keys_project_active_idx
    ON api_keys (project_id, scope_mode)
    WHERE revoked_at IS NULL;

CREATE TABLE api_key_profile_scopes (
    api_key_id BLOB NOT NULL,
    project_id BLOB NOT NULL,
    profile_id BLOB NOT NULL,
    PRIMARY KEY (api_key_id, profile_id),
    FOREIGN KEY (api_key_id, project_id)
        REFERENCES api_keys(id, project_id) ON DELETE CASCADE,
    FOREIGN KEY (profile_id, project_id)
        REFERENCES agent_profiles(id, project_id) ON DELETE CASCADE
);

CREATE INDEX api_key_profile_scopes_project_profile_idx
    ON api_key_profile_scopes (project_id, profile_id);

CREATE TABLE agent_gateway_credentials (
    gateway_id TEXT PRIMARY KEY,
    credential_prefix TEXT NOT NULL,
    credential_hash BLOB NOT NULL UNIQUE,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    last_used_at TEXT,
    revoked_at TEXT
);

CREATE TABLE agent_gateway_sessions (
    id BLOB PRIMARY KEY,
    gateway_id TEXT NOT NULL,
    session_id BLOB NOT NULL UNIQUE,
    status TEXT NOT NULL,
    agents TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(agents)),
    metadata TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(metadata)),
    connected_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    last_seen_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    disconnected_at TEXT
);

CREATE INDEX agent_gateway_sessions_gateway_id_idx
    ON agent_gateway_sessions (gateway_id, connected_at DESC);

CREATE UNIQUE INDEX agent_gateway_sessions_one_connected_idx
    ON agent_gateway_sessions (gateway_id)
    WHERE status = 'connected';

CREATE TABLE endpoint_traces (
    id BLOB PRIMARY KEY,
    request_id BLOB NOT NULL UNIQUE,
    endpoint_id BLOB REFERENCES agent_endpoints(id) ON DELETE CASCADE,
    project_id BLOB REFERENCES projects(id) ON DELETE CASCADE,
    gateway_session_id BLOB
        REFERENCES agent_gateway_sessions(session_id) ON DELETE SET NULL,
    profile_id BLOB REFERENCES agent_profiles(id) ON DELETE SET NULL,
    profile_version_id BLOB REFERENCES agent_profile_versions(id) ON DELETE SET NULL,
    operation TEXT NOT NULL DEFAULT 'chat.completions',
    provider_key TEXT,
    capability_kind TEXT,
    selection_key TEXT,
    status TEXT NOT NULL,
    latency_ms INTEGER,
    request TEXT NOT NULL CHECK (json_valid(request)),
    response TEXT CHECK (response IS NULL OR json_valid(response)),
    error TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    completed_at TEXT,
    CHECK (endpoint_id IS NOT NULL OR project_id IS NOT NULL)
);

CREATE INDEX endpoint_traces_endpoint_created_idx
    ON endpoint_traces (endpoint_id, created_at DESC);

CREATE INDEX endpoint_traces_project_created_idx
    ON endpoint_traces (project_id, created_at DESC)
    WHERE project_id IS NOT NULL;

CREATE TABLE trace_spans (
    id BLOB PRIMARY KEY,
    trace_id BLOB NOT NULL REFERENCES endpoint_traces(id) ON DELETE CASCADE,
    parent_span_id BLOB REFERENCES trace_spans(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    status TEXT NOT NULL,
    provider_key TEXT,
    capability_kind TEXT,
    duration_ms INTEGER,
    input_summary TEXT CHECK (input_summary IS NULL OR json_valid(input_summary)),
    output_summary TEXT CHECK (output_summary IS NULL OR json_valid(output_summary)),
    attributes TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(attributes)),
    error TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    completed_at TEXT
);

CREATE INDEX trace_spans_trace_created_idx
    ON trace_spans (trace_id, created_at ASC);

CREATE TABLE realtime_sessions (
    id BLOB PRIMARY KEY,
    project_id BLOB NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    profile_id BLOB NOT NULL,
    api_key_id BLOB REFERENCES api_keys(id) ON DELETE CASCADE,
    token_hash BLOB NOT NULL UNIQUE,
    expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    FOREIGN KEY (profile_id, project_id)
        REFERENCES agent_profiles(id, project_id) ON DELETE CASCADE
);

CREATE INDEX realtime_sessions_expires_idx
    ON realtime_sessions (expires_at);

CREATE TABLE project_runtime_extensions (
    project_id BLOB PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    extension_id TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    active_release_ref TEXT,
    metadata TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(metadata)),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    CHECK (length(extension_id) BETWEEN 3 AND 128),
    CHECK (active_release_ref IS NULL OR length(active_release_ref) BETWEEN 1 AND 512)
);

CREATE INDEX project_runtime_extensions_extension_idx
    ON project_runtime_extensions (extension_id);

CREATE TABLE project_runtime_channels (
    id BLOB PRIMARY KEY,
    project_id BLOB NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    public_id BLOB NOT NULL UNIQUE,
    launch_key_prefix TEXT NOT NULL,
    launch_key_hash BLOB NOT NULL UNIQUE,
    allowed_origins TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(allowed_origins)),
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    CHECK (length(name) BETWEEN 1 AND 128),
    CHECK (length(launch_key_prefix) BETWEEN 8 AND 32)
);

CREATE INDEX project_runtime_channels_project_idx
    ON project_runtime_channels (project_id, created_at DESC);

CREATE TABLE runtime_launch_sessions (
    id BLOB PRIMARY KEY,
    project_id BLOB NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    channel_id BLOB NOT NULL REFERENCES project_runtime_channels(id) ON DELETE CASCADE,
    token_hash BLOB NOT NULL UNIQUE,
    expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX runtime_launch_sessions_expiry_idx
    ON runtime_launch_sessions (expires_at);
