CREATE TABLE project_code_releases (
    release_id TEXT PRIMARY KEY,
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    deployment_id UUID NOT NULL REFERENCES runtime_deployments(id) ON DELETE CASCADE,
    status TEXT NOT NULL CHECK (status IN ('uploading', 'building', 'ready', 'failed')),
    python_version TEXT NOT NULL,
    dependency_strategy TEXT NOT NULL,
    integration TEXT NOT NULL CHECK (integration IN ('vifu', 'web')),
    http_target TEXT,
    http_interface TEXT,
    source_key TEXT NOT NULL,
    source_sha256 TEXT NOT NULL,
    source_crc32c TEXT NOT NULL,
    source_size_bytes BIGINT NOT NULL,
    source_bytes BIGINT NOT NULL,
    file_count BIGINT NOT NULL,
    build_ref TEXT,
    artifact_digest TEXT,
    service_origin TEXT,
    revision_origin TEXT,
    failure_message TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMPTZ,
    CHECK (source_size_bytes > 0 AND source_bytes > 0 AND file_count > 0),
    CHECK ((integration = 'web' AND http_target IS NOT NULL AND http_interface IN ('asgi', 'wsgi'))
        OR (integration = 'vifu' AND http_target IS NULL AND http_interface IS NULL))
);

CREATE INDEX project_code_releases_by_deployment
    ON project_code_releases(deployment_id, created_at DESC);

CREATE TABLE project_code_deployment_state (
    deployment_id UUID PRIMARY KEY REFERENCES runtime_deployments(id) ON DELETE CASCADE,
    active_release_id TEXT REFERENCES project_code_releases(release_id),
    pending_release_id TEXT REFERENCES project_code_releases(release_id),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
