ALTER TABLE api_keys DROP CONSTRAINT api_keys_permissions_check;

UPDATE api_keys
SET permissions = jsonb_set(permissions, '{runtime}', '"none"'::jsonb, true);

ALTER TABLE api_keys
    ALTER COLUMN permissions SET DEFAULT jsonb_build_object(
        'chatCompletions', 'access',
        'speech', 'none',
        'transcriptions', 'none',
        'realtime', 'none',
        'runtime', 'none',
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
            'runtime',
            'agents',
            'project'
        ]
        AND permissions
            - 'chatCompletions'
            - 'speech'
            - 'transcriptions'
            - 'realtime'
            - 'runtime'
            - 'agents'
            - 'project' = '{}'::jsonb
        AND permissions->>'chatCompletions' IN ('none', 'access')
        AND permissions->>'speech' IN ('none', 'access')
        AND permissions->>'transcriptions' IN ('none', 'access')
        AND permissions->>'realtime' IN ('none', 'access')
        AND permissions->>'runtime' IN ('none', 'access')
        AND permissions->>'agents' IN ('none', 'read', 'write')
        AND permissions->>'project' IN ('none', 'read', 'write')
    );

CREATE TABLE project_runtime_extensions (
    project_id UUID PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    extension_id TEXT NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    active_release_ref TEXT,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (extension_id ~ '^[A-Za-z0-9._:-]{3,128}$'),
    CHECK (
        active_release_ref IS NULL
        OR (
            length(active_release_ref) BETWEEN 1 AND 512
            AND active_release_ref !~ '[[:cntrl:]]'
        )
    ),
    CHECK (jsonb_typeof(metadata) = 'object')
);

CREATE INDEX project_runtime_extensions_extension_idx
    ON project_runtime_extensions (extension_id);
