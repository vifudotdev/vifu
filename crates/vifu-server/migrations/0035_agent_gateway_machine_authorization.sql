CREATE TABLE agent_gateway_machines (
    machine_id TEXT PRIMARY KEY,
    public_key TEXT NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE agent_gateway_authorizations (
    gateway_id TEXT PRIMARY KEY,
    machine_id TEXT NOT NULL UNIQUE REFERENCES agent_gateway_machines(machine_id) ON DELETE CASCADE,
    owner_user_id TEXT,
    status TEXT NOT NULL DEFAULT 'active',
    token_prefix TEXT NOT NULL,
    token_hash BYTEA NOT NULL UNIQUE,
    token_generation BIGINT NOT NULL DEFAULT 1,
    token_expires_at TIMESTAMPTZ NOT NULL,
    previous_token_hash BYTEA UNIQUE,
    previous_token_expires_at TIMESTAMPTZ,
    last_used_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    revoked_at TIMESTAMPTZ,
    CHECK (status IN ('active', 'revoked')),
    CHECK ((status = 'revoked') = (revoked_at IS NOT NULL)),
    CHECK ((previous_token_hash IS NULL) = (previous_token_expires_at IS NULL))
);

CREATE INDEX agent_gateway_authorizations_owner_idx
    ON agent_gateway_authorizations(owner_user_id, created_at);

CREATE TABLE agent_gateway_pairing_requests (
    id UUID PRIMARY KEY,
    machine_id TEXT NOT NULL REFERENCES agent_gateway_machines(machine_id) ON DELETE CASCADE,
    status TEXT NOT NULL DEFAULT 'pending',
    owner_user_id TEXT,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    resolved_at TIMESTAMPTZ,
    CHECK (status IN ('pending', 'approved', 'consumed', 'rejected', 'expired')),
    CHECK ((status = 'pending') = (resolved_at IS NULL))
);

CREATE UNIQUE INDEX agent_gateway_pairing_requests_pending_machine_idx
    ON agent_gateway_pairing_requests(machine_id)
    WHERE status = 'pending';

CREATE INDEX agent_gateway_pairing_requests_expiry_idx
    ON agent_gateway_pairing_requests(expires_at)
    WHERE status = 'pending';
