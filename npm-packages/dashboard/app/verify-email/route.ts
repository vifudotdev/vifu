import { NextResponse } from "next/server";
import { createCloudClient } from "../../lib/client";
import { authLoginUrl, authOnboardingUrl, configuredDashboardOrigin, sanitizeReturnTo } from "../../lib/config";
import {
  encodeSession,
  sessionCookieMaxAge,
  sessionFromMagicLinkResult,
  sharedAuthSessionCookieOptions,
  VIFU_AUTH_SESSION_COOKIE,
  VIFU_DASHBOARD_SESSION_COOKIE,
} from "../../lib/session";

export const dynamic = "force-dynamic";

export async function GET(request: Request): Promise<Response> {
  const url = new URL(request.url);
  const code = url.searchParams.get("code")?.trim();
  const token = url.searchParams.get("token")?.trim();
  const email = url.searchParams.get("email")?.trim();
  const returnTo = sanitizeReturnTo(url.searchParams.get("returnTo") ?? "/dashboard");
  const dashboardOrigin = configuredDashboardOrigin(request.url) ?? request.url;
  const loginUrl = new URL(authLoginUrl(returnTo));

  if (!code && !token) {
    loginUrl.searchParams.set("auth_error", "Verification link is missing or expired.");
    return NextResponse.redirect(loginUrl);
  }

  const client = createCloudClient();
  if (!client) {
    loginUrl.searchParams.set("auth_error", "Vifu API base URL is not configured.");
    return NextResponse.redirect(loginUrl);
  }

  try {
    const result = await client.verifyMagicLink({ code, token, email, returnTo });
    const session = sessionFromMagicLinkResult(result);
    if (!session) throw new Error("Verification succeeded without a dashboard session.");

    const redirectTo = redirectAfterMagicLink(result, returnTo);
    const response = NextResponse.redirect(new URL(redirectTo, dashboardOrigin));
    const maxAge = sessionCookieMaxAge(session);
    response.cookies.set(VIFU_DASHBOARD_SESSION_COOKIE, encodeSession(session), {
      httpOnly: true,
      secure: url.protocol === "https:",
      sameSite: "lax",
      path: "/",
      maxAge,
    });
    if (session.serverSessionId) {
      response.cookies.set(VIFU_AUTH_SESSION_COOKIE, session.serverSessionId, sharedAuthSessionCookieOptions(url, maxAge));
    }
    return response;
  } catch (error) {
    loginUrl.searchParams.set("auth_error", error instanceof Error ? error.message : "Email verification failed.");
    return NextResponse.redirect(loginUrl);
  }
}

function redirectAfterMagicLink(result: Record<string, unknown>, fallbackReturnTo: string): string {
  const requested = sanitizeReturnTo(
    readString(result.redirectTo)
      ?? readString(result.returnTo)
      ?? fallbackReturnTo,
  );
  if (result.onboardingRequired === true || result.isNewUser === true) return authOnboardingUrl();
  if (requested === "/onboarding" || requested.startsWith("/onboarding?") || requested.startsWith("/onboarding#")) {
    return "/dashboard";
  }
  return requested;
}

function readString(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value.trim() : null;
}
