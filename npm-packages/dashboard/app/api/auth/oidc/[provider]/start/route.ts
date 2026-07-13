import { NextResponse } from "next/server";
import { startOidcSignIn } from "../../../../../../lib/dashboard-oidc";
import { authProviders } from "../../../../../../lib/auth-providers";
import { configuredAuthCapability } from "../../../../../../lib/dashboard-auth-config";
import { dashboardLoginPath, sanitizeReturnTo } from "../../../../../../lib/config";
import { encodeOidcBrowserFlow, oidcFlowCookieOptions, VIFU_OIDC_FLOW_COOKIE } from "../../../../../../lib/oidc-flow";

export const dynamic = "force-dynamic";

export async function GET(request: Request, context: { params: Promise<{ provider: string }> }): Promise<Response> {
  const { provider } = await context.params;
  const requestUrl = new URL(request.url);
  const returnTo = sanitizeReturnTo(requestUrl.searchParams.get("returnTo"));
  try {
    const configured = authProviders(configuredAuthCapability()).find((candidate) => candidate.id === provider && candidate.kind === "oidc");
    if (!configured) {
      const login = new URL(dashboardLoginPath(returnTo), requestUrl.origin);
      login.searchParams.set("auth_error", "Sign-in provider is not available.");
      return NextResponse.redirect(login);
    }
    const flow = await startOidcSignIn({ provider, returnTo, requestUrl: request.url });
    const response = NextResponse.redirect(flow.authorizationUrl);
    response.cookies.set(
      VIFU_OIDC_FLOW_COOKIE,
      encodeOidcBrowserFlow({ provider, browserSecret: flow.browserSecret }),
      oidcFlowCookieOptions(requestUrl, flow.expiresAt),
    );
    return response;
  } catch {
    const login = new URL(dashboardLoginPath(returnTo), requestUrl.origin);
    login.searchParams.set("auth_error", "Single sign-on is temporarily unavailable.");
    return NextResponse.redirect(login);
  }
}
