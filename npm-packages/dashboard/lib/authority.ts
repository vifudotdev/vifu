import { redirect } from "next/navigation";
import { configuredAdminKey, configuredApiBaseUrl, dashboardLoginPath } from "./config";
import { DeploymentClient } from "./deployment-client";
import { authRequired, hasAuthProvider } from "./auth-providers";
import { configuredAuthCapability } from "./dashboard-auth-config";
import { DashboardAuthError, principalForSessionToken } from "./dashboard-auth-store";
import { readLocalSessionToken } from "./local-session";
import type { AuthCapability, DeploymentStatus, Principal, RuntimeSnapshot } from "./runtime-types";

export type AuthorityAdapter = {
  kind: "self-hosted";
  status: DeploymentStatus;
  deployment: DeploymentClient;
  displayName: string;
  principal: Principal | null;
};

export async function resolveAuthority(options: {
  returnTo?: string;
  redirectToLogin?: boolean;
} = {}): Promise<AuthorityAdapter> {
  const apiBaseUrl = configuredApiBaseUrl();
  const auth = configuredAuthCapability();

  if (hasAuthProvider(auth, "password") || hasAuthProvider(auth, "oidc")) {
    const token = await readLocalSessionToken();
    if (!token) {
      if (options.redirectToLogin !== false) redirect(dashboardLoginPath(options.returnTo ?? "/project"));
      throw new AuthorityError(401, "Local session required.");
    }
    const adminKey = configuredAdminKey();
    if (!adminKey) throw new AuthorityError(503, "VIFU_ADMIN_KEY is not configured for dashboard runtime access.");
    try {
      const principal = await principalForSessionToken(token);
      if (!principal) {
        if (options.redirectToLogin !== false) redirect(dashboardLoginPath(options.returnTo ?? "/project"));
        throw new AuthorityError(401, "Local session is invalid or expired.");
      }
      const deployment = new DeploymentClient({ apiBaseUrl, credential: adminKey });
      const status = await loadRuntimeStatus(apiBaseUrl, auth);
      return createSelfHostedAuthorityAdapter(status, deployment, principal);
    } catch (error) {
      if (error instanceof AuthorityError) throw error;
      if (error instanceof DashboardAuthError && (error.status === 401 || error.status === 403)) {
        if (options.redirectToLogin !== false) redirect(dashboardLoginPath(options.returnTo ?? "/project"));
        throw new AuthorityError(401, "Local session is invalid or expired.");
      }
      if (options.redirectToLogin !== false) redirect(dashboardLoginPath(options.returnTo ?? "/project"));
      throw new AuthorityError(503, "The dashboard authentication store is temporarily unavailable.");
    }
  }

  if (authRequired(auth)) {
    if (options.redirectToLogin !== false) redirect(dashboardLoginPath(options.returnTo ?? "/project"));
    throw new AuthorityError(401, "A dashboard session is required.");
  }

  const adminKey = configuredAdminKey();
  if (!adminKey) {
    throw new AuthorityError(503, "VIFU_ADMIN_KEY is not configured for this deployment.");
  }
  const status = await loadRuntimeStatus(apiBaseUrl, auth);
  return createNoAuthAuthorityAdapter(status, apiBaseUrl, adminKey);
}

async function loadRuntimeStatus(apiBaseUrl: string, auth: AuthCapability): Promise<DeploymentStatus> {
  const status = await new DeploymentClient({ apiBaseUrl }).status();
  return { ...status, auth };
}

export function createNoAuthAuthorityAdapter(
  status: DeploymentStatus,
  apiBaseUrl: string,
  adminKey: string,
): AuthorityAdapter {
  return {
    kind: "self-hosted",
    status,
    deployment: new DeploymentClient({ apiBaseUrl, credential: adminKey }),
    displayName: status.mode === "local" ? "Local deployment" : "Self-hosted deployment",
    principal: null,
  };
}

export function createSelfHostedAuthorityAdapter(
  status: DeploymentStatus,
  deployment: DeploymentClient,
  principal: Principal,
): AuthorityAdapter {
  return {
    kind: "self-hosted",
    status,
    deployment,
    displayName: principal.displayName || principal.email,
    principal,
  };
}

export async function loadRuntimeSnapshot(authority: AuthorityAdapter, projectSlug?: string): Promise<RuntimeSnapshot> {
  const { capabilities } = authority.status;
  const [projects, profiles, bindings, endpoints, apiKeys, agentGateways, availableAgents, providerAdapters, traces] = await Promise.all([
    capabilities.projects ? authority.deployment.projects() : Promise.resolve([]),
    capabilities.profiles
      ? (projectSlug ? authority.deployment.projectProfiles(projectSlug) : authority.deployment.profiles())
      : Promise.resolve([]),
    capabilities.bindings ? authority.deployment.bindings() : Promise.resolve([]),
    capabilities.endpoints ? authority.deployment.endpoints() : Promise.resolve([]),
    capabilities.apiKeys ? authority.deployment.apiKeys() : Promise.resolve([]),
    capabilities.agentGateways ? authority.deployment.agentGateways() : Promise.resolve([]),
    capabilities.agentGateways ? authority.deployment.availableAgents() : Promise.resolve([]),
    capabilities.providerConnections ? authority.deployment.providerAdapters() : Promise.resolve([]),
    capabilities.traces ? authority.deployment.traces() : Promise.resolve([]),
  ]);
  return { projects, profiles, bindings, endpoints, apiKeys, agentGateways, availableAgents, providerAdapters, traces };
}

export class AuthorityError extends Error {
  readonly status: number;

  constructor(status: number, message: string) {
    super(message);
    this.name = "AuthorityError";
    this.status = status;
  }
}
