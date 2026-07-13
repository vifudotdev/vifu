import { cookies } from "next/headers";
import { VIFU_SESSION_COOKIE } from "./session";

export async function readLocalSessionToken(): Promise<string | null> {
  const store = await cookies();
  return decodeLocalDashboardSession(store.get(VIFU_SESSION_COOKIE)?.value);
}

export function encodeLocalDashboardSession(token: string, expiresAt: string): string {
  return Buffer.from(JSON.stringify({
    version: 1,
    adapter: "dashboard",
    token,
    expiresAt,
  }), "utf8").toString("base64url");
}

export function localDashboardSessionCookieOptions(requestUrl: URL, expiresAt: string) {
  const expires = new Date(expiresAt);
  return {
    httpOnly: true,
    secure: requestUrl.protocol === "https:",
    sameSite: "lax" as const,
    path: "/",
    ...(Number.isNaN(expires.valueOf()) ? {} : { expires }),
  };
}

function normalizeSessionToken(value: string | null | undefined): string | null {
  const token = value?.trim() ?? "";
  if (!token || token.length > 512 || /[\u0000-\u001f\u007f]/.test(token)) return null;
  return token;
}

function decodeLocalDashboardSession(value: string | undefined): string | null {
  if (!value) return null;
  try {
    const session = JSON.parse(Buffer.from(value, "base64url").toString("utf8")) as {
      adapter?: unknown;
      token?: unknown;
      expiresAt?: unknown;
    };
    if ((session.adapter !== "dashboard" && session.adapter !== "runtime") || typeof session.token !== "string") return null;
    if (typeof session.expiresAt === "string" && new Date(session.expiresAt).valueOf() <= Date.now()) return null;
    return normalizeSessionToken(session.token);
  } catch {
    return null;
  }
}
