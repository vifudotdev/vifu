ALTER TABLE provider_connections
    ADD COLUMN source_kind TEXT NOT NULL DEFAULT 'registry',
    ADD COLUMN source_key TEXT;

UPDATE provider_connections
SET source_key = provider_type
WHERE source_key IS NULL;

ALTER TABLE provider_connections
    ALTER COLUMN source_key SET NOT NULL,
    ADD CONSTRAINT provider_connections_source_kind_check
        CHECK (source_kind IN ('registry', 'custom'));

INSERT INTO provider_connections (
    id,
    project_id,
    provider_key,
    name,
    provider_type,
    base_url,
    config,
    encrypted_secret_json,
    secret_keys,
    display_secret,
    status,
    last_checked_at,
    created_at,
    updated_at,
    source_kind,
    source_key
)
SELECT
    gen_random_uuid(),
    assignment.project_id,
    stock.provider_key,
    stock.name,
    stock.provider_type,
    stock.base_url,
    stock.config,
    stock.encrypted_secret_json,
    stock.secret_keys,
    stock.display_secret,
    stock.status,
    stock.last_checked_at,
    assignment.created_at,
    stock.updated_at,
    'custom',
    stock.provider_key
FROM project_provider_assignments AS assignment
JOIN provider_stock AS stock ON stock.provider_key = assignment.provider_key
ON CONFLICT (project_id, provider_key) DO NOTHING;

DROP TABLE project_provider_assignments;

ALTER TABLE provider_stock RENAME TO custom_providers;
