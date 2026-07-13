import { randomBytes, randomUUID, createHash } from "node:crypto";
import { compare, hash } from "bcryptjs";
import postgres, { type Sql, type TransactionSql } from "postgres";
import type { AuthenticatedSession, Principal } from "./runtime-types";

const SESSION_TTL_SECONDS = 30 * 24 * 60 * 60;
const MIN_PASSWORD_LENGTH = 12;

let sqlClient: Sql | null = null;
let schemaReady: Promise<void> | null = null;
type QuerySql = Sql | TransactionSql;

export class DashboardAuthError extends Error {
  readonly status: number;

  constructor(status: number, message: string) {
    super(message);
    this.name = "DashboardAuthError";
    this.status = status;
  }
}

export type OidcFlowRecord = {
  pkceVerifier: string;
  nonce: string;
  returnTo: string;
};

export async function createPasswordAccount(input: {
  email: string;
  password: string;
  displayName?: string;
}): Promise<AuthenticatedSession> {
  const email = normalizeEmail(input.email);
  const password = validatePassword(input.password);
  const displayName = normalizeDisplayName(input.displayName);
  const passwordHash = await hash(password, 12);
  const sql = await authDatabase();

  return sql.begin(async (tx) => {
    await ensureSignupSettings(tx);
    const settings = await readSignupSettings(tx, true);
    if (!settings.signupEnabled) throw new DashboardAuthError(403, "Account creation is disabled for this deployment.");

    const [count] = await tx<Array<{ count: string }>>`
      SELECT COUNT(*)::TEXT AS count FROM users
    `;
    const role = Number(count?.count ?? 0) === 0 ? "admin" : "operator";
    const userId = randomUUID();
    try {
      await tx`
        INSERT INTO users (id, email, display_name)
        VALUES (${userId}, ${email}, ${displayName})
      `;
      await tx`
        INSERT INTO password_credentials (user_id, password_hash)
        VALUES (${userId}, ${passwordHash})
      `;
      await tx`
        INSERT INTO memberships (id, user_id, role, scope_type, scope_id)
        VALUES (${randomUUID()}, ${userId}, ${role}, 'deployment', NULL)
      `;
    } catch (error) {
      if (isUniqueViolation(error)) throw new DashboardAuthError(409, "User with email already exists. Please sign in.");
      throw error;
    }

    return createAuthenticatedSession(tx, await principalForUser(tx, userId, "local"));
  });
}

export async function loginWithPassword(input: {
  email: string;
  password: string;
}): Promise<AuthenticatedSession> {
  const email = normalizeEmail(input.email);
  const password = validatePassword(input.password);
  const sql = await authDatabase();
  const [credential] = await sql<Array<{
    user_id: string;
    email: string;
    display_name: string | null;
    password_hash: string;
  }>>`
    SELECT users.id::TEXT AS user_id,
           users.email,
           users.display_name,
           password_credentials.password_hash
    FROM users
    JOIN password_credentials ON password_credentials.user_id = users.id
    WHERE LOWER(users.email) = LOWER(${email})
    LIMIT 1
  `;
  if (!credential) {
    await hash(password, 12);
    throw new DashboardAuthError(401, "Invalid credentials.");
  }
  const valid = credential.password_hash.startsWith("$2")
    ? await compare(password, credential.password_hash)
    : false;
  if (!valid) throw new DashboardAuthError(401, "Invalid credentials.");

  return sql.begin(async (tx) => {
    const principal = await principalForUser(tx, credential.user_id, "local");
    return createAuthenticatedSession(tx, principal);
  });
}

export async function createOidcFlowRecord(input: {
  provider: string;
  state: string;
  browserSecret: string;
  pkceVerifier: string;
  nonce: string;
  returnTo: string;
  ttlSeconds: number;
}): Promise<string> {
  const sql = await authDatabase();
  const provider = normalizeProvider(input.provider);
  const returnTo = normalizeReturnTo(input.returnTo);
  const ttlSeconds = Math.max(60, Math.min(15 * 60, Math.trunc(input.ttlSeconds)));
  const [flow] = await sql<Array<{ expires_at: Date }>>`
    WITH cleanup AS (
      DELETE FROM oidc_flows
      WHERE expires_at < NOW() OR consumed_at < NOW() - INTERVAL '1 hour'
    )
    INSERT INTO oidc_flows (
      id,
      provider,
      state_hash,
      browser_secret_hash,
      pkce_verifier,
      nonce,
      return_to,
      expires_at
    )
    VALUES (
      ${randomUUID()},
      ${provider},
      ${hashSessionToken(input.state)},
      ${hashSessionToken(input.browserSecret)},
      ${input.pkceVerifier},
      ${input.nonce},
      ${returnTo},
      NOW() + (${ttlSeconds}::DOUBLE PRECISION * INTERVAL '1 second')
    )
    RETURNING expires_at
  `;
  if (!flow) throw new DashboardAuthError(500, "Unable to create sign-in flow.");
  return flow.expires_at.toISOString();
}

export async function consumeOidcFlowRecord(input: {
  provider: string;
  state: string;
  browserSecret: string;
}): Promise<OidcFlowRecord> {
  const sql = await authDatabase();
  const provider = normalizeProvider(input.provider);
  const [flow] = await sql<Array<{
    pkce_verifier: string;
    nonce: string;
    return_to: string;
  }>>`
    UPDATE oidc_flows
    SET consumed_at = NOW()
    WHERE provider = ${provider}
      AND state_hash = ${hashSessionToken(input.state)}
      AND browser_secret_hash = ${hashSessionToken(input.browserSecret)}
      AND consumed_at IS NULL
      AND expires_at > NOW()
    RETURNING pkce_verifier, nonce, return_to
  `;
  if (!flow) throw new DashboardAuthError(401, "Sign-in request is invalid or expired.");
  return {
    pkceVerifier: flow.pkce_verifier,
    nonce: flow.nonce,
    returnTo: normalizeReturnTo(flow.return_to),
  };
}

export async function createOidcAccountSession(input: {
  provider: string;
  subject: string;
  email: string;
  displayName?: string | null;
  bootstrapEmail?: string | null;
}): Promise<AuthenticatedSession> {
  const provider = normalizeProvider(input.provider);
  const subject = normalizeProviderSubject(input.subject);
  const email = normalizeEmail(input.email);
  const displayName = normalizeOptionalDisplayName(input.displayName ?? undefined);
  const bootstrapEmail = input.bootstrapEmail?.trim().toLowerCase() || null;
  const sql = await authDatabase();

  return sql.begin(async (tx) => {
    const [existing] = await tx<Array<{ user_id: string }>>`
      SELECT user_id::TEXT AS user_id
      FROM auth_accounts
      WHERE provider = ${provider} AND provider_subject = ${subject}
      LIMIT 1
    `;
    if (existing) {
      return createAuthenticatedSession(tx, await principalForUser(tx, existing.user_id, "oidc"));
    }

    await ensureSignupSettings(tx);
    const settings = await readSignupSettings(tx, true);
    if (!settings.signupEnabled) throw new DashboardAuthError(403, "Account creation is disabled for this deployment.");

    const [emailConflict] = await tx<Array<{ exists: boolean }>>`
      SELECT EXISTS(SELECT 1 FROM users WHERE LOWER(email) = LOWER(${email})) AS exists
    `;
    if (emailConflict?.exists) {
      throw new DashboardAuthError(409, "An account with this email already exists. Sign in with its existing method before linking single sign-on.");
    }

    const [count] = await tx<Array<{ count: string }>>`
      SELECT COUNT(*)::TEXT AS count FROM users
    `;
    const userCount = Number(count?.count ?? 0);
    if (userCount === 0 && (!bootstrapEmail || bootstrapEmail !== email)) {
      throw new DashboardAuthError(403, "The first single sign-on administrator must match VIFU_AUTH_OIDC_BOOTSTRAP_EMAIL.");
    }

    const userId = randomUUID();
    const role = userCount === 0 ? "admin" : "operator";
    await tx`
      INSERT INTO users (id, email, display_name)
      VALUES (${userId}, ${email}, ${displayName})
    `;
    await tx`
      INSERT INTO auth_accounts (id, user_id, provider, provider_subject, email_verified)
      VALUES (${randomUUID()}, ${userId}, ${provider}, ${subject}, TRUE)
    `;
    await tx`
      INSERT INTO memberships (id, user_id, role, scope_type, scope_id)
      VALUES (${randomUUID()}, ${userId}, ${role}, 'deployment', NULL)
    `;
    return createAuthenticatedSession(tx, await principalForUser(tx, userId, "oidc"));
  });
}

export async function principalForSessionToken(token: string | null): Promise<Principal | null> {
  const normalized = normalizeSessionToken(token);
  if (!normalized) return null;
  const sql = await authDatabase();
  const [session] = await sql<Array<{ user_id: string; provider: Principal["provider"] }>>`
    UPDATE web_sessions
    SET last_seen_at = NOW()
    WHERE token_hash = ${hashSessionToken(normalized)}
      AND revoked_at IS NULL
      AND expires_at > NOW()
    RETURNING user_id::TEXT AS user_id, provider
  `;
  if (!session) return null;
  return principalForUser(sql, session.user_id, session.provider);
}

export async function revokeSessionToken(token: string | null): Promise<void> {
  const normalized = normalizeSessionToken(token);
  if (!normalized) return;
  const sql = await authDatabase();
  await sql`
    UPDATE web_sessions
    SET revoked_at = COALESCE(revoked_at, NOW())
    WHERE token_hash = ${hashSessionToken(normalized)}
  `;
}

export async function signupEnabled(): Promise<boolean> {
  const sql = await authDatabase();
  await ensureSignupSettings(sql);
  return (await readSignupSettings(sql, false)).signupEnabled;
}

async function createAuthenticatedSession(tx: QuerySql, principal: Principal): Promise<AuthenticatedSession> {
  const token = generateSessionToken();
  const [session] = await tx<Array<{ expires_at: Date }>>`
    INSERT INTO web_sessions (id, token_hash, user_id, expires_at, provider)
    VALUES (
      ${randomUUID()},
      ${hashSessionToken(token)},
      ${principal.userId},
      NOW() + (${SESSION_TTL_SECONDS}::DOUBLE PRECISION * INTERVAL '1 second'),
      ${principal.provider}
    )
    RETURNING expires_at
  `;
  if (!session) throw new DashboardAuthError(500, "Unable to create a web session.");
  return {
    principal,
    session: {
      token,
      expiresAt: session.expires_at.toISOString(),
    },
  };
}

async function principalForUser(tx: QuerySql, userId: string, provider: Principal["provider"]): Promise<Principal> {
  const [record] = await tx<Array<{
    user_id: string;
    email: string;
    display_name: string | null;
    roles: string[] | null;
  }>>`
    SELECT users.id::TEXT AS user_id,
           users.email,
           users.display_name,
           COALESCE(
             ARRAY_AGG(memberships.role ORDER BY memberships.role)
               FILTER (WHERE memberships.role IS NOT NULL),
             ARRAY[]::TEXT[]
           ) AS roles
    FROM users
    LEFT JOIN memberships ON memberships.user_id = users.id
    WHERE users.id = ${userId}
    GROUP BY users.id
    LIMIT 1
  `;
  if (!record) throw new DashboardAuthError(401, "Session user does not exist.");
  return {
    userId: record.user_id,
    email: record.email,
    displayName: record.display_name,
    roles: record.roles ?? [],
    provider,
  };
}

async function ensureAuthSettings(tx: QuerySql): Promise<void> {
  await tx`
    INSERT INTO auth_settings (singleton, signup_enabled)
    VALUES (TRUE, TRUE)
    ON CONFLICT (singleton) DO NOTHING
  `;
}

async function ensureSignupSettings(tx: QuerySql): Promise<void> {
  await ensureAuthSettings(tx);
  const envValue = process.env.VIFU_SIGNUP_ENABLED?.trim().toLowerCase();
  const signupDisabled = envValue === "false" || process.env.AUTH_DISABLE_SIGNUP === "true";
  const signupEnabled = envValue === "true" && !signupDisabled;
  if (!signupDisabled && !signupEnabled) return;
  await tx`
    UPDATE auth_settings
    SET signup_enabled = ${signupEnabled}, updated_at = NOW()
    WHERE singleton = TRUE AND signup_enabled <> ${signupEnabled}
  `;
}

async function readSignupSettings(tx: QuerySql, lock: boolean): Promise<{ signupEnabled: boolean }> {
  const [settings] = lock
    ? await tx<Array<{ signup_enabled: boolean }>>`
      SELECT signup_enabled
      FROM auth_settings
      WHERE singleton = TRUE
      FOR UPDATE
    `
    : await tx<Array<{ signup_enabled: boolean }>>`
      SELECT signup_enabled
      FROM auth_settings
      WHERE singleton = TRUE
    `;
  return { signupEnabled: settings?.signup_enabled ?? false };
}

function database(): Sql {
  const databaseUrl = process.env.DATABASE_URL?.trim();
  if (!databaseUrl) throw new DashboardAuthError(503, "DATABASE_URL is not configured for dashboard authentication.");
  sqlClient ??= postgres(databaseUrl, {
    max: 5,
    idle_timeout: 30,
    connect_timeout: 5,
    prepare: false,
  });
  return sqlClient;
}

async function authDatabase(): Promise<Sql> {
  const sql = database();
  schemaReady ??= ensureDashboardAuthSchema(sql).catch((error) => {
    schemaReady = null;
    throw error;
  });
  await schemaReady;
  return sql;
}

async function ensureDashboardAuthSchema(sql: Sql): Promise<void> {
  await sql`
    CREATE TABLE IF NOT EXISTS users (
      id UUID PRIMARY KEY,
      email TEXT NOT NULL,
      display_name TEXT,
      created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
      updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
    )
  `;
  await sql`CREATE UNIQUE INDEX IF NOT EXISTS users_email_lower_idx ON users (LOWER(email))`;
  await sql`
    CREATE TABLE IF NOT EXISTS password_credentials (
      user_id UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
      password_hash TEXT NOT NULL,
      created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
      updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
    )
  `;
  await sql`
    CREATE TABLE IF NOT EXISTS web_sessions (
      id UUID PRIMARY KEY,
      token_hash BYTEA NOT NULL UNIQUE,
      user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
      provider TEXT NOT NULL DEFAULT 'local',
      expires_at TIMESTAMPTZ NOT NULL,
      revoked_at TIMESTAMPTZ,
      created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
      last_seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
    )
  `;
  await sql`ALTER TABLE web_sessions ADD COLUMN IF NOT EXISTS provider TEXT NOT NULL DEFAULT 'local'`;
  await sql`
    CREATE INDEX IF NOT EXISTS web_sessions_user_active_idx
    ON web_sessions (user_id, expires_at DESC)
    WHERE revoked_at IS NULL
  `;
  await sql`
    CREATE TABLE IF NOT EXISTS memberships (
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
    )
  `;
  await sql`
    CREATE UNIQUE INDEX IF NOT EXISTS memberships_deployment_scope_idx
    ON memberships (user_id, scope_type)
    WHERE scope_id IS NULL
  `;
  await sql`
    CREATE UNIQUE INDEX IF NOT EXISTS memberships_project_scope_idx
    ON memberships (user_id, scope_type, scope_id)
    WHERE scope_id IS NOT NULL
  `;
  await sql`
    CREATE TABLE IF NOT EXISTS auth_settings (
      singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
      signup_enabled BOOLEAN NOT NULL DEFAULT TRUE,
      updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
    )
  `;
  await sql`
    INSERT INTO auth_settings (singleton, signup_enabled)
    VALUES (TRUE, TRUE)
    ON CONFLICT (singleton) DO NOTHING
  `;
  await sql`
    CREATE TABLE IF NOT EXISTS auth_accounts (
      id UUID PRIMARY KEY,
      user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
      provider TEXT NOT NULL,
      provider_subject TEXT NOT NULL,
      email_verified BOOLEAN NOT NULL DEFAULT FALSE,
      metadata JSONB NOT NULL DEFAULT '{}'::JSONB,
      created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
      updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
      UNIQUE (provider, provider_subject)
    )
  `;
  await sql`CREATE INDEX IF NOT EXISTS auth_accounts_user_idx ON auth_accounts (user_id)`;
  await sql`
    CREATE TABLE IF NOT EXISTS oidc_flows (
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
    )
  `;
  await sql`CREATE INDEX IF NOT EXISTS oidc_flows_expiry_idx ON oidc_flows (expires_at)`;
}

function normalizeEmail(value: string): string {
  const email = value.trim().toLowerCase();
  if (!email || email.length > 320 || !/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email)) {
    throw new DashboardAuthError(400, "A valid email address is required.");
  }
  return email;
}

function validatePassword(value: string): string {
  if (value.length < MIN_PASSWORD_LENGTH || value.length > 128) {
    throw new DashboardAuthError(400, `Password must be between ${MIN_PASSWORD_LENGTH} and 128 characters.`);
  }
  return value;
}

function normalizeDisplayName(value: string | undefined): string {
  const displayName = value?.trim() ?? "";
  if (!displayName || displayName.length > 96) throw new DashboardAuthError(400, "Display name is required.");
  return displayName;
}

function normalizeOptionalDisplayName(value: string | undefined): string | null {
  const displayName = value?.trim() ?? "";
  if (!displayName) return null;
  if (displayName.length > 96) throw new DashboardAuthError(400, "Display name is too long.");
  return displayName;
}

function normalizeProvider(value: string): string {
  const provider = value.trim();
  if (!provider || provider.length > 48 || !/^[a-z0-9-]+$/.test(provider)) {
    throw new DashboardAuthError(400, "Invalid sign-in provider.");
  }
  return provider;
}

function normalizeProviderSubject(value: string): string {
  const subject = value.trim();
  if (!subject || subject.length > 256 || /[\u0000-\u001f\u007f]/.test(subject)) {
    throw new DashboardAuthError(400, "Invalid identity provider subject.");
  }
  return subject;
}

function normalizeReturnTo(value: string | null | undefined): string {
  const returnTo = value?.trim() ?? "";
  if (!returnTo || !returnTo.startsWith("/") || returnTo.startsWith("//") || returnTo.length > 2048 || /[\u0000-\u001f\u007f]/.test(returnTo)) {
    return "/dashboard";
  }
  return returnTo;
}

function normalizeSessionToken(value: string | null | undefined): string | null {
  const token = value?.trim() ?? "";
  if (!token || token.length > 512 || /[\u0000-\u001f\u007f]/.test(token)) return null;
  return token;
}

function generateSessionToken(): string {
  return `vifu_session_${randomBytes(32).toString("base64url")}`;
}

function hashSessionToken(token: string): Buffer {
  return createHash("sha256").update(token).digest();
}

function isUniqueViolation(error: unknown): boolean {
  return typeof error === "object"
    && error !== null
    && "code" in error
    && (error as { code?: unknown }).code === "23505";
}
