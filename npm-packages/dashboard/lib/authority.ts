import { redirect } from "next/navigation";
import { configuredAdminKey, configuredApiBaseUrl, dashboardLoginPath } from "./config";
import { DeploymentClient } from "@vifu/console";
import { hasValidAdminSession } from "./admin-session";
import type { DeploymentStatus, RuntimeSnapshot } from "@vifu/console";

export type AuthorityAdapter = {
  kind: "self-hosted";
  status: DeploymentStatus;
  deployment: DeploymentClient;
  displayName: string;
};

export async function resolveAuthority(options: {
  returnTo?: string;
  redirectToLogin?: boolean;
} = {}): Promise<AuthorityAdapter> {
  const apiBaseUrl = configuredApiBaseUrl();
  const adminKey = configuredAdminKey();
  if (!adminKey) {
    throw new AuthorityError(503, "VIFU_ADMIN_KEY is not configured for this deployment.");
  }
  if (!(await hasValidAdminSession(adminKey))) {
    if (options.redirectToLogin !== false) {
      redirect(dashboardLoginPath(options.returnTo ?? "/project"));
    }
    throw new AuthorityError(401, "A valid admin session is required.");
  }
  const deployment = new DeploymentClient({ apiBaseUrl, credential: adminKey });
  const status = await loadRuntimeStatus(apiBaseUrl);
  return {
    kind: "self-hosted",
    status,
    deployment,
    displayName: status.mode === "local" ? "Local deployment" : "Self-hosted deployment",
  };
}

async function loadRuntimeStatus(apiBaseUrl: string): Promise<DeploymentStatus> {
  const status = await new DeploymentClient({ apiBaseUrl }).status();
  return { ...status, auth: { required: true, mode: "admin-key" } };
}

export async function loadRuntimeSnapshot(authority: AuthorityAdapter, projectSlug?: string): Promise<RuntimeSnapshot> {
  const { capabilities } = authority.status;
  const [projects, profiles, bindings, endpoints, apiKeys, agentGateways, availableAgents, providerAdapters, traces, deployments, releases] = await Promise.all([
    capabilities.apps ? authority.deployment.apps() : Promise.resolve([]),
    capabilities.profiles && projectSlug ? authority.deployment.appProfiles(projectSlug) : Promise.resolve([]),
    capabilities.bindings && projectSlug ? authority.deployment.appBindings(projectSlug) : Promise.resolve([]),
    capabilities.endpoints && projectSlug ? authority.deployment.appEndpoints(projectSlug) : Promise.resolve([]),
    capabilities.apiKeys && projectSlug ? authority.deployment.appApiKeys(projectSlug) : Promise.resolve([]),
    capabilities.agentGateways && projectSlug ? authority.deployment.appAgentGateways(projectSlug) : Promise.resolve([]),
    capabilities.agentGateways && projectSlug ? authority.deployment.appAvailableAgents(projectSlug) : Promise.resolve([]),
    capabilities.providerConnections ? authority.deployment.providerAdapters() : Promise.resolve([]),
    capabilities.traces && projectSlug ? authority.deployment.appTraces(projectSlug) : Promise.resolve([]),
    projectSlug ? authority.deployment.appDeployments(projectSlug) : Promise.resolve([]),
    projectSlug ? authority.deployment.appRuntimeReleases(projectSlug) : Promise.resolve([]),
  ]);
  return { projects, profiles, bindings, endpoints, apiKeys, agentGateways, availableAgents, providerAdapters, traces, deployments, releases };
}

export class AuthorityError extends Error {
  readonly status: number;

  constructor(status: number, message: string) {
    super(message);
    this.name = "AuthorityError";
    this.status = status;
  }
}
