ALTER TABLE agent_gateway_enrollments
    ADD COLUMN gateway_id TEXT,
    ADD COLUMN revoked_at TIMESTAMPTZ;

DROP INDEX agent_gateway_enrollments_active_idx;

CREATE INDEX agent_gateway_enrollments_active_idx
    ON agent_gateway_enrollments (project_id, expires_at)
    WHERE consumed_at IS NULL AND revoked_at IS NULL;
