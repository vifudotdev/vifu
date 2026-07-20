ALTER TABLE agent_profiles
    ADD COLUMN project_id UUID REFERENCES projects(id) ON DELETE CASCADE,
    ADD COLUMN archived_at TIMESTAMPTZ;

UPDATE agent_profiles AS profile
SET project_id = owner.project_id
FROM (
    SELECT DISTINCT ON (binding.profile_id)
        binding.profile_id,
        project_binding.project_id
    FROM agent_bindings AS binding
    JOIN project_bindings AS project_binding
        ON project_binding.binding_id = binding.id
    ORDER BY binding.profile_id, project_binding.created_at ASC
) AS owner
WHERE owner.profile_id = profile.id;

ALTER TABLE agent_profiles DROP CONSTRAINT agent_profiles_slug_key;

CREATE UNIQUE INDEX agent_profiles_project_slug_idx
    ON agent_profiles (project_id, slug)
    WHERE project_id IS NOT NULL;

ALTER TABLE agent_profiles
    ADD CONSTRAINT agent_profiles_id_project_key UNIQUE (id, project_id);

CREATE TABLE agent_profile_versions (
    id UUID PRIMARY KEY,
    profile_id UUID NOT NULL REFERENCES agent_profiles(id) ON DELETE CASCADE,
    version_number INTEGER NOT NULL CHECK (version_number > 0),
    persona JSONB NOT NULL DEFAULT '{"files": {}}'::jsonb,
    runtime JSONB NOT NULL DEFAULT '{}'::jsonb,
    presentation JSONB NOT NULL DEFAULT '{}'::jsonb,
    source JSONB NOT NULL DEFAULT '{}'::jsonb,
    content_hash TEXT NOT NULL,
    change_summary TEXT,
    archived_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (profile_id, version_number),
    UNIQUE (id, profile_id),
    CHECK (jsonb_typeof(persona) = 'object'),
    CHECK (jsonb_typeof(runtime) = 'object'),
    CHECK (jsonb_typeof(presentation) = 'object'),
    CHECK (jsonb_typeof(source) = 'object')
);

CREATE INDEX agent_profile_versions_profile_created_idx
    ON agent_profile_versions (profile_id, version_number DESC);

INSERT INTO agent_profile_versions (
    id,
    profile_id,
    version_number,
    persona,
    runtime,
    presentation,
    source,
    content_hash,
    change_summary,
    created_at
)
SELECT
    gen_random_uuid(),
    profile.id,
    1,
    '{"files": {}}'::jsonb,
    '{}'::jsonb,
    '{}'::jsonb,
    COALESCE(
        (
            SELECT jsonb_build_object(
                'type', binding.provider,
                'gatewayId', binding.gateway_id,
                'resourceId', binding.agent_id,
                'managed', binding.provider = 'openclaw'
            )
            FROM agent_bindings AS binding
            WHERE binding.profile_id = profile.id
            ORDER BY binding.created_at ASC
            LIMIT 1
        ),
        '{}'::jsonb
    ),
    md5(profile.id::text || ':1'),
    'Imported from the existing runtime configuration',
    profile.created_at
FROM agent_profiles AS profile;

CREATE TABLE agent_profile_capabilities (
    id UUID PRIMARY KEY,
    profile_version_id UUID NOT NULL REFERENCES agent_profile_versions(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('chat', 'speech', 'transcription', 'realtime', 'tool')),
    provider_type TEXT NOT NULL,
    provider_key TEXT NOT NULL,
    resource_id TEXT,
    config JSONB NOT NULL DEFAULT '{}'::jsonb,
    input_schema JSONB NOT NULL DEFAULT '{}'::jsonb,
    output_schema JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (jsonb_typeof(config) = 'object'),
    CHECK (jsonb_typeof(input_schema) = 'object'),
    CHECK (jsonb_typeof(output_schema) = 'object')
);

CREATE INDEX agent_profile_capabilities_version_kind_idx
    ON agent_profile_capabilities (profile_version_id, kind, created_at ASC);

INSERT INTO agent_profile_capabilities (
    id,
    profile_version_id,
    kind,
    provider_type,
    provider_key,
    resource_id,
    config,
    input_schema,
    output_schema,
    created_at
)
SELECT
    gen_random_uuid(),
    version.id,
    'chat',
    binding.provider,
    binding.provider,
    binding.agent_id,
    binding.config || jsonb_build_object('gatewayId', binding.gateway_id),
    '{}'::jsonb,
    '{}'::jsonb,
    binding.created_at
FROM agent_profile_versions AS version
JOIN agent_bindings AS binding ON binding.profile_id = version.profile_id
WHERE version.version_number = 1;

ALTER TABLE agent_profiles
    ADD COLUMN active_version_id UUID,
    ADD CONSTRAINT agent_profiles_active_version_fk
        FOREIGN KEY (active_version_id, id)
        REFERENCES agent_profile_versions(id, profile_id)
        DEFERRABLE INITIALLY DEFERRED;

UPDATE agent_profiles AS profile
SET active_version_id = version.id
FROM agent_profile_versions AS version
WHERE version.profile_id = profile.id
  AND version.version_number = 1;

CREATE TABLE agent_profile_rollouts (
    profile_id UUID NOT NULL REFERENCES agent_profiles(id) ON DELETE CASCADE,
    profile_version_id UUID NOT NULL,
    weight_bps INTEGER NOT NULL CHECK (weight_bps BETWEEN 1 AND 10000),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (profile_id, profile_version_id),
    FOREIGN KEY (profile_version_id, profile_id)
        REFERENCES agent_profile_versions(id, profile_id)
        ON DELETE CASCADE
);

INSERT INTO agent_profile_rollouts (profile_id, profile_version_id, weight_bps)
SELECT id, active_version_id, 10000
FROM agent_profiles
WHERE active_version_id IS NOT NULL;

CREATE TABLE api_key_profile_scopes (
    api_key_id UUID NOT NULL,
    project_id UUID NOT NULL,
    profile_id UUID NOT NULL,
    PRIMARY KEY (api_key_id, profile_id),
    FOREIGN KEY (api_key_id, project_id)
        REFERENCES api_keys(id, project_id)
        ON DELETE CASCADE,
    FOREIGN KEY (profile_id, project_id)
        REFERENCES agent_profiles(id, project_id)
        ON DELETE CASCADE
);

INSERT INTO api_key_profile_scopes (api_key_id, project_id, profile_id)
SELECT DISTINCT scope.api_key_id, scope.project_id, binding.profile_id
FROM api_key_agent_scopes AS scope
JOIN agent_bindings AS binding ON binding.id = scope.binding_id
JOIN agent_profiles AS profile
    ON profile.id = binding.profile_id
   AND profile.project_id = scope.project_id;

CREATE INDEX api_key_profile_scopes_project_profile_idx
    ON api_key_profile_scopes (project_id, profile_id);

DROP TABLE api_key_agent_scopes;

ALTER TABLE api_keys DROP CONSTRAINT api_keys_permissions_check;

UPDATE api_keys
SET permissions = permissions || jsonb_build_object(
    'speech', 'none',
    'transcriptions', 'none',
    'realtime', 'none'
);

ALTER TABLE api_keys
    ALTER COLUMN permissions SET DEFAULT jsonb_build_object(
        'chatCompletions', 'access',
        'speech', 'none',
        'transcriptions', 'none',
        'realtime', 'none',
        'agents', 'none',
        'project', 'none'
    ),
    ADD CONSTRAINT api_keys_permissions_check CHECK (
        jsonb_typeof(permissions) = 'object'
        AND permissions ?& ARRAY[
            'chatCompletions',
            'speech',
            'transcriptions',
            'realtime',
            'agents',
            'project'
        ]
        AND permissions
            - 'chatCompletions'
            - 'speech'
            - 'transcriptions'
            - 'realtime'
            - 'agents'
            - 'project' = '{}'::jsonb
        AND permissions->>'chatCompletions' IN ('none', 'access')
        AND permissions->>'speech' IN ('none', 'access')
        AND permissions->>'transcriptions' IN ('none', 'access')
        AND permissions->>'realtime' IN ('none', 'access')
        AND permissions->>'agents' IN ('none', 'read', 'write')
        AND permissions->>'project' IN ('none', 'read', 'write')
    );

ALTER TABLE endpoint_traces
    ADD COLUMN profile_id UUID REFERENCES agent_profiles(id) ON DELETE SET NULL,
    ADD COLUMN profile_version_id UUID REFERENCES agent_profile_versions(id) ON DELETE SET NULL,
    ADD COLUMN operation TEXT NOT NULL DEFAULT 'chat.completions',
    ADD COLUMN provider_key TEXT,
    ADD COLUMN capability_kind TEXT,
    ADD COLUMN selection_key TEXT;

CREATE TABLE trace_spans (
    id UUID PRIMARY KEY,
    trace_id UUID NOT NULL REFERENCES endpoint_traces(id) ON DELETE CASCADE,
    parent_span_id UUID REFERENCES trace_spans(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    status TEXT NOT NULL,
    provider_key TEXT,
    capability_kind TEXT,
    duration_ms BIGINT,
    input_summary JSONB,
    output_summary JSONB,
    attributes JSONB NOT NULL DEFAULT '{}'::jsonb,
    error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMPTZ,
    CHECK (jsonb_typeof(attributes) = 'object')
);

CREATE INDEX trace_spans_trace_created_idx
    ON trace_spans (trace_id, created_at ASC);

CREATE TABLE realtime_sessions (
    id UUID PRIMARY KEY,
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    profile_id UUID NOT NULL,
    api_key_id UUID REFERENCES api_keys(id) ON DELETE CASCADE,
    token_hash BYTEA NOT NULL UNIQUE,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    FOREIGN KEY (profile_id, project_id)
        REFERENCES agent_profiles(id, project_id)
        ON DELETE CASCADE
);

CREATE INDEX realtime_sessions_expires_idx
    ON realtime_sessions (expires_at);
