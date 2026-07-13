import { createHash, randomBytes, webcrypto } from "node:crypto";
import { sanitizeReturnTo } from "./config";
import { configuredOidcProviderConfig, type OidcProviderConfig } from "./dashboard-auth-config";
import {
  consumeOidcFlowRecord,
  createOidcAccountSession,
  createOidcFlowRecord,
  DashboardAuthError,
} from "./dashboard-auth-store";
import type { AuthenticatedSession } from "./runtime-types";

const FLOW_TTL_SECONDS = 10 * 60;

type OidcProviderMetadata = {
  issuer: string;
  authorization_endpoint: string;
  token_endpoint: string;
  jwks_uri: string;
};

type OidcJsonWebKey = JsonWebKey & {
  alg?: string;
  kid?: string;
  kty?: string;
};

type JsonWebKeySet = {
  keys?: OidcJsonWebKey[];
};

type JwtHeader = {
  alg?: unknown;
  kid?: unknown;
};

type JwtClaims = {
  iss?: unknown;
  aud?: unknown;
  sub?: unknown;
  exp?: unknown;
  nbf?: unknown;
  nonce?: unknown;
  email?: unknown;
  email_verified?: unknown;
  name?: unknown;
  preferred_username?: unknown;
};

export async function startOidcSignIn(input: {
  provider: string;
  returnTo: string;
  requestUrl: string;
}): Promise<{ authorizationUrl: string; browserSecret: string; expiresAt: string }> {
  const config = requireOidcConfig(input.provider, input.requestUrl);
  const metadata = await discoverMetadata(config);
  const state = randomToken();
  const nonce = randomToken();
  const browserSecret = randomToken();
  const pkceVerifier = randomToken();
  const pkceChallenge = createHash("sha256").update(pkceVerifier).digest("base64url");
  const returnTo = sanitizeReturnTo(input.returnTo);
  const authorizationUrl = new URL(metadata.authorization_endpoint);
  authorizationUrl.searchParams.set("response_type", "code");
  authorizationUrl.searchParams.set("client_id", config.clientId);
  authorizationUrl.searchParams.set("redirect_uri", config.redirectUrl);
  authorizationUrl.searchParams.set("scope", config.scopes.join(" "));
  authorizationUrl.searchParams.set("state", state);
  authorizationUrl.searchParams.set("nonce", nonce);
  authorizationUrl.searchParams.set("code_challenge", pkceChallenge);
  authorizationUrl.searchParams.set("code_challenge_method", "S256");
  const expiresAt = await createOidcFlowRecord({
    provider: config.id,
    state,
    browserSecret,
    pkceVerifier,
    nonce,
    returnTo,
    ttlSeconds: FLOW_TTL_SECONDS,
  });
  return {
    authorizationUrl: authorizationUrl.toString(),
    browserSecret,
    expiresAt,
  };
}

export async function completeOidcSignIn(input: {
  provider: string;
  code: string;
  state: string;
  browserSecret: string;
  requestUrl: string;
}): Promise<AuthenticatedSession & { returnTo: string }> {
  const config = requireOidcConfig(input.provider, input.requestUrl);
  const flow = await consumeOidcFlowRecord({
    provider: config.id,
    state: input.state,
    browserSecret: input.browserSecret,
  });
  const metadata = await discoverMetadata(config);
  const token = await exchangeCode({
    config,
    metadata,
    code: input.code,
    pkceVerifier: flow.pkceVerifier,
  });
  const claims = await verifyIdToken({
    idToken: token.id_token,
    config,
    metadata,
    nonce: flow.nonce,
  });
  const session = await createOidcAccountSession({
    provider: config.id,
    subject: claims.subject,
    email: claims.email,
    displayName: claims.displayName,
    bootstrapEmail: config.bootstrapEmail,
  });
  return { ...session, returnTo: flow.returnTo };
}

function requireOidcConfig(provider: string, requestUrl: string): OidcProviderConfig {
  const config = configuredOidcProviderConfig(provider, requestUrl);
  if (!config) throw new DashboardAuthError(404, "Single sign-on provider is not available.");
  return config;
}

async function discoverMetadata(config: OidcProviderConfig): Promise<OidcProviderMetadata> {
  const response = await fetch(new URL(".well-known/openid-configuration", config.issuer), {
    cache: "no-store",
    redirect: "manual",
  });
  if (!response.ok) throw new DashboardAuthError(502, "Single sign-on discovery failed.");
  const metadata = await response.json() as Partial<OidcProviderMetadata>;
  if (
    metadata.issuer !== config.issuer
    || !validHttpsOrLoopback(metadata.authorization_endpoint)
    || !validHttpsOrLoopback(metadata.token_endpoint)
    || !validHttpsOrLoopback(metadata.jwks_uri)
  ) {
    throw new DashboardAuthError(502, "Single sign-on discovery returned invalid metadata.");
  }
  return {
    issuer: metadata.issuer,
    authorization_endpoint: metadata.authorization_endpoint,
    token_endpoint: metadata.token_endpoint,
    jwks_uri: metadata.jwks_uri,
  };
}

async function exchangeCode(input: {
  config: OidcProviderConfig;
  metadata: OidcProviderMetadata;
  code: string;
  pkceVerifier: string;
}): Promise<{ id_token: string }> {
  const body = new URLSearchParams();
  body.set("grant_type", "authorization_code");
  body.set("code", input.code);
  body.set("redirect_uri", input.config.redirectUrl);
  body.set("code_verifier", input.pkceVerifier);
  const response = await fetch(input.metadata.token_endpoint, {
    method: "POST",
    headers: {
      authorization: `Basic ${Buffer.from(`${input.config.clientId}:${input.config.clientSecret}`).toString("base64")}`,
      "content-type": "application/x-www-form-urlencoded",
      accept: "application/json",
    },
    body,
    cache: "no-store",
    redirect: "manual",
  });
  if (!response.ok) throw new DashboardAuthError(401, "Single sign-on token exchange failed.");
  const token = await response.json() as Partial<{ id_token: unknown }>;
  if (typeof token.id_token !== "string" || token.id_token.length > 8192) {
    throw new DashboardAuthError(401, "Single sign-on response did not include a valid identity token.");
  }
  return { id_token: token.id_token };
}

async function verifyIdToken(input: {
  idToken: string;
  config: OidcProviderConfig;
  metadata: OidcProviderMetadata;
  nonce: string;
}): Promise<{ subject: string; email: string; displayName: string | null }> {
  const parts = input.idToken.split(".");
  if (parts.length !== 3) throw new DashboardAuthError(401, "Invalid identity token.");
  const header = decodeJwtPart<JwtHeader>(parts[0]);
  if (header.alg !== "RS256") throw new DashboardAuthError(401, "Unsupported identity token signature.");
  await verifyJwtSignature({
    header,
    signingInput: `${parts[0]}.${parts[1]}`,
    signature: decodeBase64Url(parts[2]),
    jwksUri: input.metadata.jwks_uri,
  });
  const claims = decodeJwtPart<JwtClaims>(parts[1]);
  const now = Math.floor(Date.now() / 1000);
  if (claims.iss !== input.metadata.issuer) throw new DashboardAuthError(401, "Invalid identity token issuer.");
  if (!audienceIncludes(claims.aud, input.config.clientId)) throw new DashboardAuthError(401, "Invalid identity token audience.");
  if (typeof claims.exp !== "number" || claims.exp <= now) throw new DashboardAuthError(401, "Identity token is expired.");
  if (typeof claims.nbf === "number" && claims.nbf > now + 60) throw new DashboardAuthError(401, "Identity token is not valid yet.");
  if (claims.nonce !== input.nonce) throw new DashboardAuthError(401, "Invalid identity token nonce.");
  if (claims.email_verified !== true) throw new DashboardAuthError(403, "Single sign-on email must be verified.");
  const subject = readString(claims.sub, 256);
  const email = readEmail(claims.email);
  if (!subject || !email) throw new DashboardAuthError(403, "Single sign-on did not return a usable identity.");
  return {
    subject,
    email,
    displayName: readString(claims.name, 96) ?? readString(claims.preferred_username, 96) ?? null,
  };
}

async function verifyJwtSignature(input: {
  header: JwtHeader;
  signingInput: string;
  signature: Buffer;
  jwksUri: string;
}): Promise<void> {
  const response = await fetch(input.jwksUri, { cache: "no-store", redirect: "manual" });
  if (!response.ok) throw new DashboardAuthError(502, "Single sign-on key discovery failed.");
  const jwks = await response.json() as JsonWebKeySet;
  const key = jwks.keys?.find((candidate) => {
    if (candidate.kty !== "RSA") return false;
    if (candidate.alg && candidate.alg !== "RS256") return false;
    return !input.header.kid || candidate.kid === input.header.kid;
  });
  if (!key) throw new DashboardAuthError(401, "Identity token signing key is not trusted.");
  const cryptoKey = await webcrypto.subtle.importKey(
    "jwk",
    key,
    { name: "RSASSA-PKCS1-v1_5", hash: "SHA-256" },
    false,
    ["verify"],
  );
  const valid = await webcrypto.subtle.verify(
    "RSASSA-PKCS1-v1_5",
    cryptoKey,
    input.signature,
    Buffer.from(input.signingInput),
  );
  if (!valid) throw new DashboardAuthError(401, "Invalid identity token signature.");
}

function randomToken(): string {
  return randomBytes(32).toString("base64url");
}

function decodeJwtPart<T>(value: string): T {
  try {
    return JSON.parse(decodeBase64Url(value).toString("utf8")) as T;
  } catch {
    throw new DashboardAuthError(401, "Invalid identity token.");
  }
}

function decodeBase64Url(value: string): Buffer {
  if (!/^[A-Za-z0-9_-]+$/.test(value)) throw new DashboardAuthError(401, "Invalid base64url value.");
  return Buffer.from(value, "base64url");
}

function audienceIncludes(value: unknown, clientId: string): boolean {
  if (typeof value === "string") return value === clientId;
  return Array.isArray(value) && value.some((item) => item === clientId);
}

function readString(value: unknown, maxLength: number): string | null {
  if (typeof value !== "string") return null;
  const normalized = value.trim();
  if (!normalized || normalized.length > maxLength || /[\u0000-\u001f\u007f]/.test(normalized)) return null;
  return normalized;
}

function readEmail(value: unknown): string | null {
  const email = readString(value, 320)?.toLowerCase() ?? null;
  return email && /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email) ? email : null;
}

function validHttpsOrLoopback(value: unknown): value is string {
  if (typeof value !== "string") return false;
  try {
    const url = new URL(value);
    if (url.username || url.password || url.hash) return false;
    if (url.protocol === "https:") return true;
    return url.protocol === "http:"
      && (url.hostname === "localhost" || url.hostname === "127.0.0.1" || url.hostname === "::1");
  } catch {
    return false;
  }
}
