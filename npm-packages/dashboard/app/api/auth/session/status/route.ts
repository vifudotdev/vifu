import { NextResponse } from "next/server";
import { createCloudClient } from "../../../../../lib/client";
import { VIFU_AUTH_SESSION_COOKIE } from "../../../../../lib/session";

const NO_STORE_HEADERS = {
  "cache-control": "no-store",
};

export const dynamic = "force-dynamic";

export async function GET(request: Request): Promise<Response> {
  const serverSessionId = readSessionId(request);
  const client = createCloudClient();
  if (!serverSessionId || !client) return sessionStatus(false);

  try {
    await client.magicLinkSession(serverSessionId);
    return sessionStatus(true);
  } catch {
    return sessionStatus(false);
  }
}

function sessionStatus(authenticated: boolean): Response {
  return NextResponse.json({ authenticated }, { headers: NO_STORE_HEADERS });
}

function readSessionId(request: Request): string | null {
  const fromHeader = readString(request.headers.get("x-vifu-auth-session"));
  if (fromHeader) return fromHeader;
  return readCookie(request.headers.get("cookie"), VIFU_AUTH_SESSION_COOKIE);
}

function readCookie(header: string | null, name: string): string | null {
  if (!header) return null;
  for (const part of header.split(";")) {
    const [rawName, ...rawValue] = part.trim().split("=");
    if (rawName !== name) continue;
    const value = rawValue.join("=");
    try {
      return readString(decodeURIComponent(value));
    } catch {
      return readString(value);
    }
  }
  return null;
}

function readString(value: string | null | undefined): string | null {
  if (!value) return null;
  const trimmed = value.trim();
  if (!trimmed || /[\u0000-\u001f\u007f]/.test(trimmed)) return null;
  return trimmed;
}
