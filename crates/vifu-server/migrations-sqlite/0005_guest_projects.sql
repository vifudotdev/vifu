CREATE TABLE guest_projects (
    project_id BLOB PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    gateway_id TEXT NOT NULL UNIQUE,
    claim_token_hash BLOB NOT NULL UNIQUE,
    expires_at TEXT NOT NULL,
    claimed_by TEXT,
    claimed_at TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    CHECK ((claimed_by IS NULL) = (claimed_at IS NULL))
);

CREATE INDEX guest_projects_active_idx
    ON guest_projects(expires_at)
    WHERE claimed_at IS NULL;
