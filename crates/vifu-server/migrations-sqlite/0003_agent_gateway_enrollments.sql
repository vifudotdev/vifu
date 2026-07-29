ALTER TABLE agent_gateway_credentials
    ADD COLUMN owner_user_id TEXT;

CREATE INDEX agent_gateway_credentials_owner_idx
    ON agent_gateway_credentials (owner_user_id)
    WHERE owner_user_id IS NOT NULL;

CREATE TABLE agent_gateway_enrollments (
    id BLOB PRIMARY KEY,
    project_id BLOB NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    owner_user_id TEXT NOT NULL,
    token_hash BLOB NOT NULL UNIQUE,
    expires_at TEXT NOT NULL,
    gateway_id TEXT,
    consumed_at TEXT,
    revoked_at TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX agent_gateway_enrollments_active_idx
    ON agent_gateway_enrollments (project_id, expires_at)
    WHERE consumed_at IS NULL AND revoked_at IS NULL;

UPDATE agent_gateway_credentials
SET owner_user_id = (
    SELECT MIN(project.owner_user_id)
    FROM projects AS project
    WHERE project.gateway_id = agent_gateway_credentials.gateway_id
      AND project.owner_user_id IS NOT NULL
)
WHERE owner_user_id IS NULL
  AND (
      SELECT COUNT(DISTINCT project.owner_user_id)
      FROM projects AS project
      WHERE project.gateway_id = agent_gateway_credentials.gateway_id
        AND project.owner_user_id IS NOT NULL
  ) = 1;
