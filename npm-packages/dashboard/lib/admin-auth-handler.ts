import { NextResponse } from "next/server";
import {
  adminKeysMatch,
  adminSessionCookieOptions,
  createAdminSession,
  VIFU_ADMIN_SESSION_COOKIE,
} from "./admin-session";
import {
  configuredAdminKey,
  configuredApiBaseUrl,
  sanitizeReturnTo,
} from "./config";
import { DeploymentClient, VifuHttpError } from "@vifu/console";
import { isSameOriginRequest } from "./request-security";

export async function handleAdminKeyLogin(request: Request): Promise<Response> {
  if (!isSameOriginRequest(request)) {
    return NextResponse.json(
      { error: { code: "INVALID_ORIGIN", message: "Invalid access request origin." } },
      { status: 403 },
    );
  }

  const form = await request.formData();
  const returnTo = sanitizeReturnTo(text(form.get("returnTo")) ?? "/project");
  const submittedAdminKey = text(form.get("adminKey")) ?? "";
  const configuredAdminKeyValue = configuredAdminKey();
  const failureUrl = new URL("/login", "https://dashboard.invalid");
  failureUrl.searchParams.set("returnTo", returnTo);

  if (!configuredAdminKeyValue) {
    failureUrl.searchParams.set("auth_error", "Dashboard access is not configured.");
    return redirectTo(failureUrl);
  }
  if (!adminKeysMatch(submittedAdminKey, configuredAdminKeyValue)) {
    failureUrl.searchParams.set("auth_error", "Invalid admin key.");
    return redirectTo(failureUrl);
  }

  try {
    await new DeploymentClient({
      apiBaseUrl: configuredApiBaseUrl(),
      credential: submittedAdminKey,
    }).verifyAdmin();
  } catch (error) {
    failureUrl.searchParams.set(
      "auth_error",
      error instanceof VifuHttpError && (error.status === 401 || error.status === 403)
        ? "Invalid admin key."
        : "Vifu Runtime is temporarily unavailable.",
    );
    return redirectTo(failureUrl);
  }

  const response = new NextResponse(null, {
    status: 303,
    headers: { location: returnTo },
  });
  response.cookies.set(
    VIFU_ADMIN_SESSION_COOKIE,
    createAdminSession(configuredAdminKeyValue),
    adminSessionCookieOptions(new URL(request.url)),
  );
  return response;
}

function redirectTo(url: URL): NextResponse {
  return new NextResponse(null, {
    status: 303,
    headers: { location: `${url.pathname}${url.search}` },
  });
}

function text(value: FormDataEntryValue | null): string | null {
  if (typeof value !== "string") return null;
  const normalized = value.trim();
  return normalized || null;
}
