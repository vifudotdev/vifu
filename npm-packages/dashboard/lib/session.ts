import { cookies } from "next/headers";
import { createCloudClient, type MagicLinkVerifyResult } from "./client";

export const VIFU_DASHBOARD_SESSION_COOKIE = "vifu_dashboard_session";
export const VIFU_AUTH_SESSION_COOKIE = "vifu_auth_session";
const MAX_SESSION_AGE_SECONDS = 30 * 24 * 60 * 60;
const SESSION_REFRESH_SKEW_SECONDS = 60;

export type DashboardSession = {
  token: string;
  expiresAt?: number;
  displayName?: string;
  serverSessionId?: string;
  serverSessionExpiresAt?: number;
};

type StoredDashboardSession = {
  token?: string;
  expiresAt?: number;
  displayName?: string;
  serverSessionId?: string;
  serverSessionExpiresAt?: number;
};

export async function readDashboardSession(): Promise<DashboardSession | null> {
  const store = await cookies();
  const stored = decodeStoredSession(store.get(VIFU_DASHBOARD_SESSION_COOKIE)?.value);
  if (!stored) return null;
  if (stored.serverSessionId) return resolveServerSession(stored);
  if (!stored.token) return null;
  const session: DashboardSession = {
    token: stored.token,
    expiresAt: stored.expiresAt,
    displayName: stored.displayName,
  };
  return isExpired(session) ? null : session;
}

export async function resolveDashboardSessionId(serverSessionId: string): Promise<DashboardSession | null> {
  const normalized = readString(serverSessionId);
  if (!normalized) return null;
  return resolveServerSession({ serverSessionId: normalized });
}

export function encodeSession(session: DashboardSession): string {
  const stored: StoredDashboardSession = session.serverSessionId
    ? {
      serverSessionId: session.serverSessionId,
      serverSessionExpiresAt: session.serverSessionExpiresAt,
      displayName: session.displayName,
    }
    : {
      token: session.token,
      expiresAt: session.expiresAt,
      displayName: session.displayName,
    };
  return Buffer.from(JSON.stringify(stored), "utf8").toString("base64url");
}

export function sessionCookieMaxAge(session: DashboardSession): number {
  if (session.serverSessionExpiresAt) return secondsUntil(session.serverSessionExpiresAt, MAX_SESSION_AGE_SECONDS);
  if (!session.expiresAt) return MAX_SESSION_AGE_SECONDS;
  return secondsUntil(session.expiresAt, MAX_SESSION_AGE_SECONDS);
}

export function sharedAuthSessionCookieOptions(url: URL, maxAge: number) {
  const domain = sharedDashboardCookieDomain(url.hostname);
  return {
    httpOnly: true,
    secure: url.protocol === "https:",
    sameSite: "lax" as const,
    path: "/",
    maxAge,
    ...(domain ? { domain } : {}),
  };
}

export function sessionFromMagicLinkResult(result: MagicLinkVerifyResult): DashboardSession | null {
  const token = readString(result.token) ?? readString(result.idToken) ?? readString(result.accessToken);
  if (!token) return null;
  const serverSessionId = readString(result.serverSessionId);
  return {
    token,
    expiresAt: expiresAtFromResult(result, token),
    serverSessionId: serverSessionId ?? undefined,
    serverSessionExpiresAt: timestampMs(result.serverSessionExpiresAt) ?? expiresInMs(result.serverSessionExpiresIn),
    displayName: readString(result.displayName) ?? displayNameFromJwt(readString(result.idToken) ?? token),
  };
}

function decodeStoredSession(value: string | undefined): StoredDashboardSession | null {
  if (!value) return null;
  try {
    const session = JSON.parse(Buffer.from(value, "base64url").toString("utf8")) as StoredDashboardSession;
    if (!session.token && !session.serverSessionId) return null;
    return session;
  } catch {
    return null;
  }
}

async function resolveServerSession(stored: StoredDashboardSession): Promise<DashboardSession | null> {
  const serverSessionId = readString(stored.serverSessionId);
  const client = createCloudClient();
  if (!serverSessionId || !client) return null;
  try {
    const result = await client.magicLinkSession(serverSessionId);
    const session = sessionFromMagicLinkResult({
      ...result,
      serverSessionId,
      serverSessionExpiresAt: stored.serverSessionExpiresAt ?? result.serverSessionExpiresAt,
    });
    if (!session || isExpired(session)) return null;
    return {
      ...session,
      displayName: session.displayName ?? stored.displayName,
    };
  } catch {
    return null;
  }
}

function isExpired(session: DashboardSession): boolean {
  return !!session.expiresAt && session.expiresAt <= Date.now() + SESSION_REFRESH_SKEW_SECONDS * 1000;
}

function expiresAtFromResult(result: MagicLinkVerifyResult, token: string): number | undefined {
  const explicit = timestampMs(result.expiresAt);
  if (explicit) return explicit;
  const fromDuration = expiresInMs(result.expiresIn);
  if (fromDuration) return fromDuration;
  return tokenExpiryMs(token) ?? undefined;
}

function timestampMs(value: unknown): number | undefined {
  if (typeof value !== "number" || !Number.isFinite(value)) return undefined;
  return value < 10_000_000_000 ? value * 1000 : value;
}

function expiresInMs(value: unknown): number | undefined {
  if (typeof value !== "number" || !Number.isFinite(value)) return undefined;
  return Date.now() + Math.max(60, value) * 1000;
}

function secondsUntil(timestamp: number, maxSeconds: number): number {
  const seconds = Math.floor((timestamp - Date.now()) / 1000);
  return Math.max(0, Math.min(maxSeconds, seconds));
}

function sharedDashboardCookieDomain(hostname: string): string | undefined {
  const normalized = hostname.toLowerCase();
  if (normalized === "vifu.dev" || normalized.endsWith(".vifu.dev")) return ".vifu.dev";
  return undefined;
}

function tokenExpiryMs(token: string | undefined): number | null {
  const payload = decodeJwtPayload(token);
  return typeof payload?.exp === "number" ? payload.exp * 1000 : null;
}

function displayNameFromJwt(token: string | undefined): string | undefined {
  const payload = decodeJwtPayload(token);
  return readString(payload?.name)
    ?? readString(payload?.preferred_username)
    ?? readString(payload?.email)
    ?? undefined;
}

function decodeJwtPayload(token: string | undefined): Record<string, unknown> | null {
  const payload = token?.split(".")[1];
  if (!payload) return null;
  try {
    return JSON.parse(Buffer.from(payload, "base64url").toString("utf8")) as Record<string, unknown>;
  } catch {
    return null;
  }
}

function readString(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value.trim() : null;
}
