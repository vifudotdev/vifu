import { NextResponse } from "next/server";
import { dashboardLoginPath, sanitizeReturnTo } from "../../../../../../lib/config";
import { completeOidcSignIn } from "../../../../../../lib/dashboard-oidc";
import { encodeLocalDashboardSession, localDashboardSessionCookieOptions } from "../../../../../../lib/local-session";
import { readOidcBrowserFlow, VIFU_OIDC_FLOW_COOKIE } from "../../../../../../lib/oidc-flow";
import { VIFU_SESSION_COOKIE } from "../../../../../../lib/session";

export const dynamic = "force-dynamic";

export async function GET(request: Request, context: { params: Promise<{ provider: string }> }): Promise<Response> {
  const { provider } = await context.params;
  const requestUrl = new URL(request.url);
  const code = bounded(requestUrl.searchParams.get("code"), 4096);
  const state = bounded(requestUrl.searchParams.get("state"), 1024);
  const providerError = bounded(requestUrl.searchParams.get("error"), 256);
  const flow = await readOidcBrowserFlow();
  const login = new URL(dashboardLoginPath(), requestUrl.origin);
  if (providerError || !code || !state || !flow || flow.provider !== provider) {
    login.searchParams.set("auth_error", providerError ? "Sign-in was cancelled." : "Sign-in request is invalid or expired.");
    return clearFlow(NextResponse.redirect(login));
  }

  try {
    const result = await completeOidcSignIn({
      provider,
      code,
      state,
      browserSecret: flow.browserSecret,
      requestUrl: request.url,
    });
    const returnTo = sanitizeReturnTo(result.returnTo);
    const response = NextResponse.redirect(new URL(returnTo, requestUrl.origin));
    response.cookies.set(
      VIFU_SESSION_COOKIE,
      encodeLocalDashboardSession(result.session.token, result.session.expiresAt),
      localDashboardSessionCookieOptions(requestUrl, result.session.expiresAt),
    );
    return clearFlow(response);
  } catch {
    login.searchParams.set("auth_error", "Single sign-on could not be completed.");
    return clearFlow(NextResponse.redirect(login));
  }
}

function clearFlow(response: NextResponse): NextResponse {
  response.cookies.set(VIFU_OIDC_FLOW_COOKIE, "", { path: "/api/auth/oidc", maxAge: 0 });
  return response;
}

function bounded(value: string | null, maxLength: number): string | null {
  const normalized = value?.trim() ?? "";
  return normalized && normalized.length <= maxLength && !/[\u0000-\u001f\u007f]/.test(normalized) ? normalized : null;
}
