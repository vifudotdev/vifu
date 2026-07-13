CREATE TABLE users (
    id UUID PRIMARY KEY,
    email TEXT NOT NULL,
    display_name TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX users_email_lower_idx ON users (LOWER(email));

CREATE TABLE password_credentials (
    user_id UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    password_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE web_sessions (
    id UUID PRIMARY KEY,
    token_hash BYTEA NOT NULL UNIQUE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX web_sessions_user_active_idx
    ON web_sessions (user_id, expires_at DESC)
    WHERE revoked_at IS NULL;

CREATE TABLE memberships (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('admin', 'operator', 'viewer')),
    scope_type TEXT NOT NULL CHECK (scope_type IN ('deployment', 'project')),
    scope_id UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (
        (scope_type = 'deployment' AND scope_id IS NULL)
        OR (scope_type = 'project' AND scope_id IS NOT NULL)
    )
);

CREATE UNIQUE INDEX memberships_deployment_scope_idx
    ON memberships (user_id, scope_type)
    WHERE scope_id IS NULL;

CREATE UNIQUE INDEX memberships_project_scope_idx
    ON memberships (user_id, scope_type, scope_id)
    WHERE scope_id IS NOT NULL;

CREATE TABLE auth_settings (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    signup_enabled BOOLEAN NOT NULL DEFAULT TRUE,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

INSERT INTO auth_settings (singleton, signup_enabled) VALUES (TRUE, TRUE);
