import { NextResponse } from "next/server";
import { sanitizeReturnTo } from "./config";
import { configuredAuthCapability } from "./dashboard-auth-config";
import { createPasswordAccount, DashboardAuthError, loginWithPassword } from "./dashboard-auth-store";
import { encodeLocalDashboardSession, localDashboardSessionCookieOptions } from "./local-session";
import { VIFU_SESSION_COOKIE } from "./session";
import { isSameOriginRequest, readLimitedBody, RequestBodyTooLargeError } from "./request-security";

const MAX_FORM_BYTES = 16 * 1024;

export async function handleLocalAuth(request: Request, intent: "login" | "signup"): Promise<Response> {
  if (!isSameOriginRequest(request)) return jsonError(403, "INVALID_ORIGIN", "Invalid authentication request origin.");
  if (!(request.headers.get("content-type") ?? "").toLowerCase().startsWith("application/x-www-form-urlencoded")) {
    return jsonError(415, "UNSUPPORTED_MEDIA_TYPE", "Authentication requests must use a URL-encoded form.");
  }

  let form: URLSearchParams;
  try {
    form = new URLSearchParams(await readLimitedBody(request, MAX_FORM_BYTES));
  } catch (error) {
    return error instanceof RequestBodyTooLargeError
      ? jsonError(413, "AUTH_REQUEST_TOO_LARGE", "Authentication requests must not exceed 16 KiB.")
      : jsonError(400, "INVALID_AUTH_REQUEST", "Unable to read authentication request.");
  }

  const returnTo = sanitizeReturnTo(form.get("returnTo") ?? "/project");
  const email = (form.get("email") ?? "").trim();
  const password = form.get("password") ?? "";
  const displayName = (form.get("displayName") ?? "").trim();
  const failureUrl = new URL(intent === "signup" ? "/signup" : "/login", "https://dashboard.invalid");
  failureUrl.searchParams.set("returnTo", returnTo);
  if (email) failureUrl.searchParams.set("email", email.slice(0, 320));

  try {
    const auth = configuredAuthCapability();
    const hasPasswordProvider = auth.providers?.some((provider) => provider.kind === "password");
    if (!hasPasswordProvider) throw new DashboardAuthError(404, "Email and password sign-in is not enabled.");
    if (intent === "signup" && !auth.signupEnabled) throw new DashboardAuthError(403, "Account creation is disabled for this deployment.");
    const result = intent === "signup"
      ? await createPasswordAccount({ email, password, displayName })
      : await loginWithPassword({ email, password });
    const response = relativeRedirect(returnTo);
    response.cookies.set(
      VIFU_SESSION_COOKIE,
      encodeLocalDashboardSession(result.session.token, result.session.expiresAt),
      localDashboardSessionCookieOptions(new URL(request.url), result.session.expiresAt),
    );
    return response;
  } catch (error) {
    const message = error instanceof DashboardAuthError && error.status < 500
      ? error.message
      : "Authentication service is temporarily unavailable.";
    failureUrl.searchParams.set("auth_error", message);
    return relativeRedirect(`${failureUrl.pathname}${failureUrl.search}`);
  }
}

function relativeRedirect(location: string): NextResponse {
  return new NextResponse(null, { status: 303, headers: { location } });
}

function jsonError(status: number, code: string, message: string): Response {
  return NextResponse.json({ error: { code, message } }, { status });
}
