CREATE TABLE agent_gateway_machines (
    machine_id TEXT PRIMARY KEY,
    public_key TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    last_seen_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE agent_gateway_authorizations (
    gateway_id TEXT PRIMARY KEY,
    machine_id TEXT NOT NULL UNIQUE REFERENCES agent_gateway_machines(machine_id) ON DELETE CASCADE,
    owner_user_id TEXT,
    status TEXT NOT NULL DEFAULT 'active',
    token_prefix TEXT NOT NULL,
    token_hash BLOB NOT NULL UNIQUE,
    token_generation INTEGER NOT NULL DEFAULT 1,
    token_expires_at TEXT NOT NULL,
    previous_token_hash BLOB UNIQUE,
    previous_token_expires_at TEXT,
    last_used_at TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    revoked_at TEXT,
    CHECK (status IN ('active', 'revoked')),
    CHECK ((status = 'revoked') = (revoked_at IS NOT NULL)),
    CHECK ((previous_token_hash IS NULL) = (previous_token_expires_at IS NULL))
);

CREATE INDEX agent_gateway_authorizations_owner_idx
    ON agent_gateway_authorizations(owner_user_id, created_at);

CREATE TABLE agent_gateway_pairing_requests (
    id BLOB PRIMARY KEY,
    machine_id TEXT NOT NULL REFERENCES agent_gateway_machines(machine_id) ON DELETE CASCADE,
    status TEXT NOT NULL DEFAULT 'pending',
    owner_user_id TEXT,
    expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    resolved_at TEXT,
    CHECK (status IN ('pending', 'approved', 'consumed', 'rejected', 'expired')),
    CHECK ((status = 'pending') = (resolved_at IS NULL))
);

CREATE UNIQUE INDEX agent_gateway_pairing_requests_pending_machine_idx
    ON agent_gateway_pairing_requests(machine_id)
    WHERE status = 'pending';

CREATE INDEX agent_gateway_pairing_requests_expiry_idx
    ON agent_gateway_pairing_requests(expires_at)
    WHERE status = 'pending';
