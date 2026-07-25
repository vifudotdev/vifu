ALTER TABLE api_keys DROP CONSTRAINT api_keys_permissions_check;

UPDATE api_keys
SET permissions = permissions - 'game';

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
