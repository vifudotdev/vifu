import { appendApiPath, configuredApiBaseUrl } from "./config";
import { readErrorMessage, VifuHttpError, type VifuFetch } from "./deployment-client";

export type MagicLinkStartInput = {
  email: string;
  callbackUrl: string;
  returnTo?: string;
};

export type MagicLinkVerifyInput = {
  token?: string | null;
  code?: string | null;
  email?: string | null;
  returnTo?: string;
};

export type MagicLinkVerifyResult = {
  token?: string;
  idToken?: string;
  accessToken?: string;
  expiresAt?: number;
  expiresIn?: number;
  serverSessionId?: string;
  serverSessionExpiresAt?: number;
  serverSessionExpiresIn?: number;
  redirectTo?: string;
  returnTo?: string;
  onboardingRequired?: boolean;
  isNewUser?: boolean;
  displayName?: string;
  [key: string]: unknown;
};

export type CloudDashboardProject = {
  owner?: unknown;
  name?: unknown;
  dashboardPath?: unknown;
  projectCloudSlug?: unknown;
  [key: string]: unknown;
};

export type CloudDashboardData = {
  owner?: Record<string, unknown>;
  projects?: unknown;
  [key: string]: unknown;
};

type CloudClient = {
  startMagicLink(input: MagicLinkStartInput): Promise<Record<string, unknown>>;
  verifyMagicLink(input: MagicLinkVerifyInput): Promise<MagicLinkVerifyResult>;
  magicLinkSession(serverSessionId: string): Promise<MagicLinkVerifyResult>;
  magicLinkSignout(serverSessionId: string): Promise<Record<string, unknown>>;
  billingAccount(): Promise<Record<string, unknown>>;
  dashboard(): Promise<{ dashboard: CloudDashboardData; projects: CloudDashboardProject[] }>;
  completeOnboarding(input: Record<string, unknown>): Promise<Record<string, unknown>>;
  createProject(input: Record<string, unknown>): Promise<Record<string, unknown>>;
};

export function createCloudClient(token?: string | null, fetcher: VifuFetch = fetch): CloudClient {
  const baseUrl = configuredApiBaseUrl();

  async function requestJson<T>(
    path: string,
    init: RequestInit = {},
    options: { auth?: boolean } = {},
  ): Promise<T> {
    const headers = new Headers(init.headers);
    headers.set("accept", headers.get("accept") ?? "application/json");
    if (init.body && !headers.has("content-type")) headers.set("content-type", "application/json");
    if (options.auth !== false && token) headers.set("authorization", `Bearer ${token}`);
    const response = await fetcher(appendApiPath(baseUrl, path), {
      ...init,
      headers,
      cache: init.cache ?? "no-store",
    });
    const payload = await response.json().catch(() => null) as T | { error?: unknown } | null;
    if (!response.ok) throw new VifuHttpError(response.status, readErrorMessage(payload), payload);
    return (payload ?? {}) as T;
  }

  return {
    startMagicLink: (input) => requestJson(
      "/v1/auth/magic-link/start",
      { method: "POST", body: JSON.stringify(input) },
      { auth: false },
    ),
    verifyMagicLink: (input) => requestJson(
      "/v1/auth/magic-link/consume",
      { method: "POST", body: JSON.stringify(input) },
      { auth: false },
    ),
    magicLinkSession: (serverSessionId) => requestJson(
      "/v1/auth/magic-link/session",
      { method: "POST", body: JSON.stringify({ serverSessionId }) },
      { auth: false },
    ),
    magicLinkSignout: (serverSessionId) => requestJson(
      "/v1/auth/magic-link/signout",
      { method: "POST", body: JSON.stringify({ serverSessionId }) },
      { auth: false },
    ),
    billingAccount: () => requestJson("/v1/billing/account"),
    async dashboard() {
      const dashboard = await requestJson<CloudDashboardData>("/v1/agent-console/dashboard");
      return {
        dashboard,
        projects: Array.isArray(dashboard.projects) ? dashboard.projects as CloudDashboardProject[] : [],
      };
    },
    completeOnboarding: (input) => requestJson(
      "/v1/agent-console/onboarding/complete",
      { method: "POST", body: JSON.stringify(input) },
    ),
    createProject: (input) => requestJson(
      "/v1/agent-console/projects",
      { method: "POST", body: JSON.stringify(input) },
    ),
  };
}
