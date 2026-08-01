CREATE TABLE guest_projects (
    project_id UUID PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    gateway_id TEXT NOT NULL UNIQUE,
    claim_token_hash BYTEA NOT NULL UNIQUE,
    expires_at TIMESTAMPTZ NOT NULL,
    claimed_by TEXT,
    claimed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK ((claimed_by IS NULL) = (claimed_at IS NULL))
);

CREATE INDEX guest_projects_active_idx
    ON guest_projects(expires_at)
    WHERE claimed_at IS NULL;
