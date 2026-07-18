ALTER TABLE api_keys
    ADD COLUMN scope_mode TEXT;

UPDATE api_keys
SET scope_mode = CASE
    WHEN key_type = 'agent' THEN 'selected'
    ELSE 'all'
END;

ALTER TABLE api_keys
    ALTER COLUMN scope_mode SET NOT NULL,
    ADD CONSTRAINT api_keys_scope_mode_check
        CHECK (scope_mode IN ('all', 'selected')),
    ADD CONSTRAINT api_keys_id_project_key
        UNIQUE (id, project_id);

CREATE TABLE api_key_agent_scopes (
    api_key_id UUID NOT NULL,
    project_id UUID NOT NULL,
    binding_id UUID NOT NULL,
    PRIMARY KEY (api_key_id, binding_id),
    FOREIGN KEY (api_key_id, project_id)
        REFERENCES api_keys(id, project_id)
        ON DELETE CASCADE,
    FOREIGN KEY (project_id, binding_id)
        REFERENCES project_bindings(project_id, binding_id)
        ON DELETE CASCADE
);

INSERT INTO api_key_agent_scopes (api_key_id, project_id, binding_id)
SELECT api_key.id, api_key.project_id, project_binding.binding_id
FROM api_keys AS api_key
JOIN project_bindings AS project_binding
    ON project_binding.project_id = api_key.project_id
JOIN agent_bindings AS binding
    ON binding.id = project_binding.binding_id
    AND binding.agent_id = api_key.agent_id
WHERE api_key.key_type = 'agent';

CREATE INDEX api_key_agent_scopes_project_binding_idx
    ON api_key_agent_scopes (project_id, binding_id);

DROP INDEX api_keys_project_active_idx;

ALTER TABLE api_keys
    DROP CONSTRAINT api_keys_check,
    DROP CONSTRAINT api_keys_key_type_check,
    DROP CONSTRAINT api_keys_permissions_check,
    DROP COLUMN key_type,
    DROP COLUMN agent_id,
    DROP COLUMN permissions;

CREATE INDEX api_keys_project_active_idx
    ON api_keys (project_id, scope_mode)
    WHERE revoked_at IS NULL;

ALTER TABLE endpoint_traces
    DROP CONSTRAINT endpoint_traces_single_resource_check,
    ADD CONSTRAINT endpoint_traces_resource_check
        CHECK (endpoint_id IS NOT NULL OR project_id IS NOT NULL);

CREATE TABLE agent_gateway_credentials (
    gateway_id TEXT PRIMARY KEY,
    credential_prefix TEXT NOT NULL,
    credential_hash BYTEA NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_used_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ
);

CREATE INDEX agent_gateway_credentials_active_idx
    ON agent_gateway_credentials (gateway_id)
    WHERE revoked_at IS NULL;
