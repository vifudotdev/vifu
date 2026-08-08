CREATE TABLE runtime_distributions (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    deployment_id TEXT NOT NULL REFERENCES runtime_deployments(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    public_id TEXT NOT NULL UNIQUE,
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'revoked')),
    max_gateways INTEGER NOT NULL CHECK (max_gateways > 0),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    revoked_at TEXT
);

CREATE INDEX runtime_distributions_project_idx
    ON runtime_distributions(project_id, created_at DESC);

CREATE TABLE runtime_distribution_gateways (
    distribution_id TEXT NOT NULL REFERENCES runtime_distributions(id) ON DELETE CASCADE,
    machine_id TEXT NOT NULL,
    gateway_id TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (distribution_id, machine_id)
);

CREATE INDEX runtime_distribution_gateways_distribution_idx
    ON runtime_distribution_gateways(distribution_id, created_at DESC);
