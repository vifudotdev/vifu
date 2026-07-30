ALTER TABLE agent_gateway_credentials
    ADD COLUMN owner_user_id TEXT;

CREATE INDEX agent_gateway_credentials_owner_idx
    ON agent_gateway_credentials (owner_user_id)
    WHERE owner_user_id IS NOT NULL;

CREATE TABLE agent_gateway_enrollments (
    id UUID PRIMARY KEY,
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    owner_user_id TEXT NOT NULL,
    token_hash BYTEA NOT NULL UNIQUE,
    expires_at TIMESTAMPTZ NOT NULL,
    consumed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX agent_gateway_enrollments_active_idx
    ON agent_gateway_enrollments (expires_at)
    WHERE consumed_at IS NULL;

UPDATE agent_gateway_credentials AS credential
SET owner_user_id = owned.owner_user_id
FROM (
    SELECT gateway_id, MIN(owner_user_id) AS owner_user_id
    FROM projects
    WHERE owner_user_id IS NOT NULL
    GROUP BY gateway_id
    HAVING COUNT(DISTINCT owner_user_id) = 1
) AS owned
WHERE credential.gateway_id = owned.gateway_id
  AND credential.owner_user_id IS NULL;
