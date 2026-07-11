import { redirect } from "next/navigation";
import { authLoginUrl, configuredAdminKey, configuredApiBaseUrl } from "./config";
import { DeploymentClient } from "./deployment-client";
import { readDashboardSession, type DashboardSession } from "./session";
import type { DeploymentStatus, RuntimeSnapshot } from "./runtime-types";

export type AuthorityAdapter = {
  kind: "self-hosted" | "cloud";
  status: DeploymentStatus;
  deployment: DeploymentClient;
  displayName: string;
  session: DashboardSession | null;
};

export async function resolveAuthority(options: {
  returnTo?: string;
  redirectToLogin?: boolean;
} = {}): Promise<AuthorityAdapter> {
  const apiBaseUrl = configuredApiBaseUrl();
  const publicClient = new DeploymentClient({ apiBaseUrl });
  const status = await publicClient.status();

  if (status.capabilities.account) {
    const session = await readDashboardSession();
    if (!session?.token) {
      if (options.redirectToLogin !== false) redirect(authLoginUrl(options.returnTo ?? "/dashboard"));
      throw new AuthorityError(401, "Vifu account session required.");
    }
    return createCloudAuthorityAdapter(status, apiBaseUrl, session);
  }

  const adminKey = configuredAdminKey();
  if (!adminKey) {
    throw new AuthorityError(503, "VIFU_ADMIN_KEY is not configured for this deployment.");
  }
  return createSelfHostedAuthorityAdapter(status, apiBaseUrl, adminKey);
}

export function createCloudAuthorityAdapter(
  status: DeploymentStatus,
  apiBaseUrl: string,
  session: DashboardSession,
): AuthorityAdapter {
  return {
    kind: "cloud",
    status,
    deployment: new DeploymentClient({ apiBaseUrl, credential: session.token }),
    displayName: session.displayName ?? "Account",
    session,
  };
}

export function createSelfHostedAuthorityAdapter(
  status: DeploymentStatus,
  apiBaseUrl: string,
  adminKey: string,
): AuthorityAdapter {
  return {
    kind: "self-hosted",
    status,
    deployment: new DeploymentClient({ apiBaseUrl, credential: adminKey }),
    displayName: status.mode === "local" ? "Local deployment" : "Self-hosted deployment",
    session: null,
  };
}

export async function loadRuntimeSnapshot(authority: AuthorityAdapter): Promise<RuntimeSnapshot> {
  const { capabilities } = authority.status;
  const [profiles, bindings, endpoints, apiKeys, connections, traces] = await Promise.all([
    capabilities.profiles ? authority.deployment.profiles() : Promise.resolve([]),
    capabilities.bindings ? authority.deployment.bindings() : Promise.resolve([]),
    capabilities.endpoints ? authority.deployment.endpoints() : Promise.resolve([]),
    capabilities.apiKeys ? authority.deployment.apiKeys() : Promise.resolve([]),
    capabilities.connections ? authority.deployment.connections() : Promise.resolve([]),
    capabilities.traces ? authority.deployment.traces() : Promise.resolve([]),
  ]);
  return { profiles, bindings, endpoints, apiKeys, connections, traces };
}

export class AuthorityError extends Error {
  readonly status: number;

  constructor(status: number, message: string) {
    super(message);
    this.name = "AuthorityError";
    this.status = status;
  }
}
