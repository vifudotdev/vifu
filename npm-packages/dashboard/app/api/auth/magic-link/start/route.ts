import { NextResponse } from "next/server";
import { createCloudClient } from "../../../../../lib/client";
import { authLoginUrl, configuredDashboardOrigin, sanitizeReturnTo } from "../../../../../lib/config";

export async function POST(request: Request): Promise<Response> {
  const contentType = request.headers.get("content-type") ?? "";
  const formMode = contentType.includes("application/x-www-form-urlencoded") || contentType.includes("multipart/form-data");
  const payload = formMode ? await readFormPayload(request) : await readJsonPayload(request);
  const email = payload.email.trim();
  const returnTo = sanitizeReturnTo(payload.returnTo || "/dashboard");
  const dashboardOrigin = configuredDashboardOrigin(request.url);
  const loginUrl = new URL(authLoginUrl(returnTo));

  if (!email) {
    loginUrl.searchParams.set("auth_error", "Email is required.");
    return formMode ? NextResponse.redirect(loginUrl, 303) : jsonError("Email is required.", 400);
  }
  if (!dashboardOrigin) {
    loginUrl.searchParams.set("auth_error", "Vifu dashboard URL is not configured.");
    return formMode ? NextResponse.redirect(loginUrl, 303) : jsonError("Vifu dashboard URL is not configured.", 503);
  }

  const client = createCloudClient();
  if (!client) {
    loginUrl.searchParams.set("auth_error", "Vifu API base URL is not configured.");
    return formMode ? NextResponse.redirect(loginUrl, 303) : jsonError("Vifu API base URL is not configured.", 503);
  }

  try {
    const callbackUrl = new URL("/verify-email", dashboardOrigin);
    await client.startMagicLink({
      email,
      returnTo,
      callbackUrl: callbackUrl.toString(),
    });
  } catch (error) {
    const message = error instanceof Error ? error.message : "Unable to start email sign-in.";
    loginUrl.searchParams.set("auth_error", message);
    return formMode ? NextResponse.redirect(loginUrl, 303) : jsonError(message, 502);
  }

  if (!formMode) return NextResponse.json({ ok: true });
  loginUrl.searchParams.set("sent", "1");
  loginUrl.searchParams.set("email", email);
  loginUrl.searchParams.set("returnTo", returnTo);
  return NextResponse.redirect(loginUrl, 303);
}

async function readFormPayload(request: Request): Promise<{ email: string; returnTo: string }> {
  const form = await request.formData();
  return {
    email: String(form.get("email") ?? ""),
    returnTo: String(form.get("returnTo") ?? ""),
  };
}

async function readJsonPayload(request: Request): Promise<{ email: string; returnTo: string }> {
  const body = await request.json().catch(() => ({})) as Partial<{ email: unknown; returnTo: unknown }>;
  return {
    email: typeof body.email === "string" ? body.email : "",
    returnTo: typeof body.returnTo === "string" ? body.returnTo : "",
  };
}

function jsonError(message: string, status: number): Response {
  return NextResponse.json({ error: { message } }, { status });
}
