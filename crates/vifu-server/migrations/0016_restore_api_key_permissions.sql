ALTER TABLE api_keys
    ADD COLUMN permissions JSONB NOT NULL DEFAULT jsonb_build_object(
        'chatCompletions', 'access',
        'agents', 'none',
        'project', 'none'
    ),
    ADD CONSTRAINT api_keys_permissions_check CHECK (
        jsonb_typeof(permissions) = 'object'
        AND permissions ?& ARRAY['chatCompletions', 'agents', 'project']
        AND permissions - 'chatCompletions' - 'agents' - 'project' = '{}'::jsonb
        AND permissions->>'chatCompletions' IN ('none', 'access')
        AND permissions->>'agents' IN ('none', 'read', 'write')
        AND permissions->>'project' IN ('none', 'read', 'write')
    );
