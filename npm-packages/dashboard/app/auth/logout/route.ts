import { NextResponse } from "next/server";
import { VIFU_ADMIN_SESSION_COOKIE } from "../../../lib/admin-session";
import { dashboardLoginPath } from "../../../lib/config";
import { isSameOriginRequest } from "../../../lib/request-security";

export const dynamic = "force-dynamic";

export async function POST(request: Request): Promise<Response> {
  if (!isSameOriginRequest(request)) {
    return NextResponse.json({ error: { code: "INVALID_ORIGIN", message: "Invalid sign-out request origin." } }, { status: 403 });
  }
  const response = new NextResponse(null, {
    status: 303,
    headers: { location: dashboardLoginPath() },
  });
  response.cookies.set(VIFU_ADMIN_SESSION_COOKIE, "", {
    httpOnly: true,
    path: "/",
    sameSite: "lax",
    maxAge: 0,
  });
  return response;
}
