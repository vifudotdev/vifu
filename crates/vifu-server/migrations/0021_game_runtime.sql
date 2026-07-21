ALTER TABLE api_keys DROP CONSTRAINT api_keys_permissions_check;

UPDATE api_keys
SET permissions = permissions || jsonb_build_object('game', 'none');

ALTER TABLE api_keys
    ALTER COLUMN permissions SET DEFAULT jsonb_build_object(
        'chatCompletions', 'access',
        'speech', 'none',
        'transcriptions', 'none',
        'realtime', 'none',
        'agents', 'none',
        'project', 'none',
        'game', 'none'
    ),
    ADD CONSTRAINT api_keys_permissions_check CHECK (
        jsonb_typeof(permissions) = 'object'
        AND permissions ?& ARRAY[
            'chatCompletions',
            'speech',
            'transcriptions',
            'realtime',
            'agents',
            'project',
            'game'
        ]
        AND permissions
            - 'chatCompletions'
            - 'speech'
            - 'transcriptions'
            - 'realtime'
            - 'agents'
            - 'project'
            - 'game' = '{}'::jsonb
        AND permissions->>'chatCompletions' IN ('none', 'access')
        AND permissions->>'speech' IN ('none', 'access')
        AND permissions->>'transcriptions' IN ('none', 'access')
        AND permissions->>'realtime' IN ('none', 'access')
        AND permissions->>'agents' IN ('none', 'read', 'write')
        AND permissions->>'project' IN ('none', 'read', 'write')
        AND permissions->>'game' IN ('none', 'execute')
    );

CREATE TABLE project_game_drafts (
    project_id UUID PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    source JSONB NOT NULL,
    revision BIGINT NOT NULL DEFAULT 1 CHECK (revision > 0),
    content_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (jsonb_typeof(source) = 'object')
);

CREATE TABLE project_game_resources (
    id UUID PRIMARY KEY,
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    resource_key TEXT NOT NULL,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    content JSONB NOT NULL,
    version BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
    content_hash TEXT NOT NULL,
    approved BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (project_id, resource_key),
    UNIQUE (id, project_id)
);

CREATE INDEX project_game_resources_project_idx
    ON project_game_resources (project_id, created_at ASC);

CREATE TABLE project_game_assets (
    id UUID PRIMARY KEY,
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    asset_key TEXT NOT NULL,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (project_id, asset_key),
    UNIQUE (id, project_id)
);

CREATE INDEX project_game_assets_project_idx
    ON project_game_assets (project_id, created_at ASC);

CREATE TABLE game_asset_versions (
    id UUID PRIMARY KEY,
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    asset_id UUID NOT NULL,
    content_hash TEXT NOT NULL,
    mime_type TEXT NOT NULL,
    size_bytes BIGINT NOT NULL CHECK (size_bytes >= 0),
    storage_key TEXT NOT NULL,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    provenance JSONB NOT NULL DEFAULT '{}'::jsonb,
    rights_status TEXT NOT NULL DEFAULT 'unreviewed',
    approval_status TEXT NOT NULL DEFAULT 'pending'
        CHECK (approval_status IN ('pending', 'approved', 'rejected')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (project_id, content_hash),
    UNIQUE (id, project_id),
    FOREIGN KEY (asset_id, project_id)
        REFERENCES project_game_assets(id, project_id)
        ON DELETE CASCADE,
    CHECK (jsonb_typeof(metadata) = 'object'),
    CHECK (jsonb_typeof(provenance) = 'object')
);

CREATE INDEX game_asset_versions_asset_idx
    ON game_asset_versions (asset_id, created_at DESC);

CREATE TABLE game_build_jobs (
    id UUID PRIMARY KEY,
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    source_revision BIGINT NOT NULL CHECK (source_revision > 0),
    kind TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'queued'
        CHECK (status IN ('queued', 'running', 'completed', 'failed', 'cancelled')),
    input_hash TEXT NOT NULL,
    input JSONB NOT NULL DEFAULT '{}'::jsonb,
    output JSONB,
    error JSONB,
    lease_owner TEXT,
    lease_expires_at TIMESTAMPTZ,
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    CHECK (jsonb_typeof(input) = 'object')
);

CREATE INDEX game_build_jobs_claim_idx
    ON game_build_jobs (status, created_at ASC);

CREATE TABLE game_releases (
    id UUID PRIMARY KEY,
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    release_number INTEGER NOT NULL CHECK (release_number > 0),
    source_revision BIGINT NOT NULL CHECK (source_revision > 0),
    content_hash TEXT NOT NULL,
    plan JSONB NOT NULL,
    manifest JSONB NOT NULL,
    backend_resources JSONB NOT NULL DEFAULT '[]'::jsonb,
    change_summary TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (project_id, release_number),
    UNIQUE (project_id, content_hash),
    UNIQUE (id, project_id),
    CHECK (jsonb_typeof(plan) = 'object'),
    CHECK (jsonb_typeof(manifest) = 'object'),
    CHECK (jsonb_typeof(backend_resources) = 'array')
);

CREATE INDEX game_releases_project_created_idx
    ON game_releases (project_id, created_at DESC);

CREATE TABLE game_presentation_releases (
    id UUID PRIMARY KEY,
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    game_release_id UUID NOT NULL,
    release_number INTEGER NOT NULL CHECK (release_number > 0),
    content_hash TEXT NOT NULL,
    binding_manifest JSONB NOT NULL,
    asset_version_ids UUID[] NOT NULL DEFAULT ARRAY[]::UUID[],
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (project_id, release_number),
    UNIQUE (project_id, content_hash),
    UNIQUE (id, project_id),
    FOREIGN KEY (game_release_id, project_id)
        REFERENCES game_releases(id, project_id)
        ON DELETE CASCADE,
    CHECK (jsonb_typeof(binding_manifest) = 'object')
);

ALTER TABLE projects
    ADD COLUMN active_game_release_id UUID,
    ADD COLUMN active_game_presentation_release_id UUID,
    ADD CONSTRAINT projects_active_game_release_fk
        FOREIGN KEY (active_game_release_id, id)
        REFERENCES game_releases(id, project_id)
        DEFERRABLE INITIALLY DEFERRED,
    ADD CONSTRAINT projects_active_game_presentation_release_fk
        FOREIGN KEY (active_game_presentation_release_id, id)
        REFERENCES game_presentation_releases(id, project_id)
        DEFERRABLE INITIALLY DEFERRED;

CREATE TABLE game_sessions (
    id UUID PRIMARY KEY,
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    game_release_id UUID NOT NULL,
    api_key_id UUID REFERENCES api_keys(id) ON DELETE SET NULL,
    status TEXT NOT NULL
        CHECK (status IN (
            'running',
            'waiting_input',
            'waiting_effect',
            'waiting_host',
            'completed',
            'failed',
            'cancelled'
        )),
    revision BIGINT NOT NULL DEFAULT 0 CHECK (revision >= 0),
    snapshot JSONB NOT NULL,
    host JSONB NOT NULL,
    public_output JSONB,
    failure JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMPTZ,
    FOREIGN KEY (game_release_id, project_id)
        REFERENCES game_releases(id, project_id)
        ON DELETE RESTRICT,
    CHECK (jsonb_typeof(snapshot) = 'object'),
    CHECK (jsonb_typeof(host) = 'object')
);

CREATE INDEX game_sessions_project_created_idx
    ON game_sessions (project_id, created_at DESC);

CREATE TABLE game_commands (
    id UUID PRIMARY KEY,
    session_id UUID NOT NULL REFERENCES game_sessions(id) ON DELETE CASCADE,
    idempotency_key TEXT NOT NULL,
    command JSONB NOT NULL,
    status TEXT NOT NULL DEFAULT 'processing'
        CHECK (status IN ('processing', 'completed', 'failed')),
    result JSONB,
    error JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMPTZ,
    UNIQUE (session_id, idempotency_key),
    CHECK (jsonb_typeof(command) = 'object')
);

CREATE INDEX game_commands_session_created_idx
    ON game_commands (session_id, created_at ASC);

CREATE TABLE game_events (
    session_id UUID NOT NULL REFERENCES game_sessions(id) ON DELETE CASCADE,
    sequence BIGINT NOT NULL CHECK (sequence > 0),
    event JSONB NOT NULL,
    public BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (session_id, sequence),
    CHECK (jsonb_typeof(event) = 'object')
);

CREATE TABLE game_effects (
    session_id UUID NOT NULL REFERENCES game_sessions(id) ON DELETE CASCADE,
    effect_id TEXT NOT NULL,
    node_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('agent', 'tool')),
    request JSONB NOT NULL,
    result JSONB,
    status TEXT NOT NULL DEFAULT 'queued'
        CHECK (status IN ('queued', 'running', 'completed', 'failed')),
    lease_owner TEXT,
    lease_expires_at TIMESTAMPTZ,
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMPTZ,
    PRIMARY KEY (session_id, effect_id),
    CHECK (jsonb_typeof(request) = 'object')
);

CREATE INDEX game_effects_claim_idx
    ON game_effects (status, lease_expires_at, created_at ASC);

CREATE TABLE game_analytics_events (
    id BIGSERIAL PRIMARY KEY,
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    game_release_id UUID NOT NULL,
    session_id UUID REFERENCES game_sessions(id) ON DELETE SET NULL,
    event_type TEXT NOT NULL,
    dimensions JSONB NOT NULL DEFAULT '{}'::jsonb,
    duration_ms BIGINT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    FOREIGN KEY (game_release_id, project_id)
        REFERENCES game_releases(id, project_id)
        ON DELETE CASCADE,
    CHECK (jsonb_typeof(dimensions) = 'object')
);

CREATE INDEX game_analytics_project_created_idx
    ON game_analytics_events (project_id, created_at DESC);
