import { NextResponse } from "next/server";
import { dashboardLoginPath } from "../../../lib/config";
import { revokeSessionToken } from "../../../lib/dashboard-auth-store";
import { readLocalSessionToken } from "../../../lib/local-session";
import { isSameOriginRequest } from "../../../lib/request-security";
import { VIFU_SESSION_COOKIE } from "../../../lib/session";

export const dynamic = "force-dynamic";

export async function POST(request: Request): Promise<Response> {
  if (!isSameOriginRequest(request)) {
    return NextResponse.json({ error: { code: "INVALID_ORIGIN", message: "Invalid sign-out request origin." } }, { status: 403 });
  }
  const localToken = await readLocalSessionToken();
  if (localToken) await revokeSessionToken(localToken).catch(() => null);

  const response = new NextResponse(null, {
    status: 303,
    headers: { location: dashboardLoginPath() },
  });
  response.cookies.delete(VIFU_SESSION_COOKIE);
  return response;
}
