import { NextResponse } from "next/server";
import { createCloudClient } from "../../../lib/client";
import { authLoginUrl } from "../../../lib/config";
import {
  readDashboardSession,
  sharedAuthSessionCookieOptions,
  VIFU_AUTH_SESSION_COOKIE,
  VIFU_DASHBOARD_SESSION_COOKIE,
} from "../../../lib/session";

export const dynamic = "force-dynamic";

export async function GET(request: Request): Promise<Response> {
  const session = await readDashboardSession();
  const serverSessionId = session?.serverSessionId;
  const client = createCloudClient();
  if (serverSessionId && client) await client.magicLinkSignout(serverSessionId).catch(() => null);

  const response = NextResponse.redirect(new URL(authLoginUrl()));
  response.cookies.delete(VIFU_DASHBOARD_SESSION_COOKIE);
  response.cookies.set(VIFU_AUTH_SESSION_COOKIE, "", sharedAuthSessionCookieOptions(new URL(request.url), 0));
  return response;
}
