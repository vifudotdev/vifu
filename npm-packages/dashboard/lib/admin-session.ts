import { createHmac, timingSafeEqual } from "node:crypto";
import { cookies } from "next/headers";

export const VIFU_ADMIN_SESSION_COOKIE = "vifu_admin_session";

const SESSION_LIFETIME_MS = 12 * 60 * 60 * 1000;

type AdminSessionPayload = {
  version: 1;
  expiresAt: number;
};

export async function hasValidAdminSession(adminKey: string): Promise<boolean> {
  const store = await cookies();
  return verifyAdminSession(store.get(VIFU_ADMIN_SESSION_COOKIE)?.value, adminKey);
}

export function createAdminSession(adminKey: string, now = Date.now()): string {
  const payload = Buffer.from(JSON.stringify({
    version: 1,
    expiresAt: now + SESSION_LIFETIME_MS,
  } satisfies AdminSessionPayload), "utf8").toString("base64url");
  return `${payload}.${sign(payload, adminKey)}`;
}

export function verifyAdminSession(
  value: string | null | undefined,
  adminKey: string,
  now = Date.now(),
): boolean {
  if (!value || !adminKey || value.length > 1024) return false;
  const [payload, signature, extra] = value.split(".");
  if (!payload || !signature || extra !== undefined) return false;
  if (!constantTimeEqual(signature, sign(payload, adminKey))) return false;

  try {
    const decoded = JSON.parse(
      Buffer.from(payload, "base64url").toString("utf8"),
    ) as Partial<AdminSessionPayload>;
    return decoded.version === 1
      && typeof decoded.expiresAt === "number"
      && Number.isSafeInteger(decoded.expiresAt)
      && decoded.expiresAt > now;
  } catch {
    return false;
  }
}

export function adminKeysMatch(candidate: string, configured: string): boolean {
  if (!candidate || !configured || candidate.length > 4096) return false;
  return constantTimeEqual(candidate, configured);
}

export function adminSessionCookieOptions(requestUrl: URL) {
  return {
    httpOnly: true,
    secure: requestUrl.protocol === "https:",
    sameSite: "lax" as const,
    path: "/",
  };
}

function sign(payload: string, adminKey: string): string {
  return createHmac("sha256", adminKey).update(payload).digest("base64url");
}

function constantTimeEqual(left: string, right: string): boolean {
  const leftBytes = Buffer.from(left, "utf8");
  const rightBytes = Buffer.from(right, "utf8");
  return leftBytes.length === rightBytes.length
    && timingSafeEqual(leftBytes, rightBytes);
}
