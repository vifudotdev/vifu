import { cookies } from "next/headers";

export const VIFU_OIDC_FLOW_COOKIE = "vifu_oidc_flow";

export type OidcBrowserFlow = {
  provider: string;
  browserSecret: string;
};

export function encodeOidcBrowserFlow(flow: OidcBrowserFlow): string {
  return Buffer.from(JSON.stringify({ version: 1, ...flow }), "utf8").toString("base64url");
}

export async function readOidcBrowserFlow(): Promise<OidcBrowserFlow | null> {
  const store = await cookies();
  const value = store.get(VIFU_OIDC_FLOW_COOKIE)?.value;
  if (!value) return null;
  try {
    const flow = JSON.parse(Buffer.from(value, "base64url").toString("utf8")) as Partial<OidcBrowserFlow>;
    if (!validPart(flow.provider, 48) || !validPart(flow.browserSecret, 256)) return null;
    return { provider: flow.provider, browserSecret: flow.browserSecret };
  } catch {
    return null;
  }
}

export function oidcFlowCookieOptions(requestUrl: URL, expiresAt: string) {
  const expires = new Date(expiresAt);
  return {
    httpOnly: true,
    secure: requestUrl.protocol === "https:",
    sameSite: "lax" as const,
    path: "/api/auth/oidc",
    maxAge: 10 * 60,
    ...(Number.isNaN(expires.valueOf()) ? {} : { expires }),
  };
}

function validPart(value: unknown, maxLength: number): value is string {
  return typeof value === "string"
    && value.length > 0
    && value.length <= maxLength
    && !/[\u0000-\u001f\u007f]/.test(value);
}
