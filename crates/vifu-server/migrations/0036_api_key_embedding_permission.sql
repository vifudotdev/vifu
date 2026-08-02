ALTER TABLE api_keys DROP CONSTRAINT api_keys_permissions_check;

UPDATE api_keys
SET permissions = jsonb_set(permissions, '{embeddings}', '"none"'::jsonb, true)
WHERE NOT permissions ? 'embeddings';

ALTER TABLE api_keys
    ALTER COLUMN permissions SET DEFAULT jsonb_build_object(
        'chatCompletions', 'access',
        'embeddings', 'access',
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
            'embeddings',
            'speech',
            'transcriptions',
            'realtime',
            'runtime',
            'agents',
            'project'
        ]
        AND permissions
            - 'chatCompletions'
            - 'embeddings'
            - 'speech'
            - 'transcriptions'
            - 'realtime'
            - 'runtime'
            - 'agents'
            - 'project' = '{}'::jsonb
        AND permissions->>'chatCompletions' IN ('none', 'access')
        AND permissions->>'embeddings' IN ('none', 'access')
        AND permissions->>'speech' IN ('none', 'access')
        AND permissions->>'transcriptions' IN ('none', 'access')
        AND permissions->>'realtime' IN ('none', 'access')
        AND permissions->>'runtime' IN ('none', 'access')
        AND permissions->>'agents' IN ('none', 'read', 'write')
        AND permissions->>'project' IN ('none', 'read', 'write')
    );
