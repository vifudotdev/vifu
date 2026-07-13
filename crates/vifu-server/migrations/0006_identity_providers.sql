ALTER TABLE web_sessions
    ADD COLUMN provider TEXT NOT NULL DEFAULT 'local'
    CHECK (provider IN ('local', 'oidc'));

CREATE TABLE auth_accounts (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    provider_subject TEXT NOT NULL,
    email_verified BOOLEAN NOT NULL DEFAULT FALSE,
    metadata JSONB NOT NULL DEFAULT '{}'::JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (provider, provider_subject)
);

CREATE INDEX auth_accounts_user_idx ON auth_accounts (user_id);

CREATE TABLE oidc_flows (
    id UUID PRIMARY KEY,
    provider TEXT NOT NULL,
    state_hash BYTEA NOT NULL UNIQUE,
    browser_secret_hash BYTEA NOT NULL,
    pkce_verifier TEXT NOT NULL,
    nonce TEXT NOT NULL,
    return_to TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    consumed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX oidc_flows_expiry_idx ON oidc_flows (expires_at);
