import type { RuntimeConsoleData } from "@vifu/console";
import type { DashboardSection } from "../components/runtime-console-host";
import { loadRuntimeSnapshot, resolveAuthority } from "./authority";
import type {
  AgentProfileDetail,
  ProviderCatalog,
  ProjectAgentCandidate,
  ProjectProvider,
  RuntimeSnapshot,
} from "@vifu/console";

export type DashboardData = RuntimeConsoleData & {
  runtime: RuntimeSnapshot;
  profileDetails: AgentProfileDetail[];
  projectProviders: ProjectProvider[];
  providerCatalog: ProviderCatalog;
  agentCandidates: ProjectAgentCandidate[];
};

export async function loadDashboardData(
  returnTo: string,
  projectSlug?: string,
  section: DashboardSection = "overview",
): Promise<DashboardData> {
  const authority = await resolveAuthority({ returnTo });
  if (section === "logs") {
    const projects = authority.status.capabilities.apps
      ? await authority.deployment.apps()
      : [];
    return {
      authority: {
        kind: authority.kind,
        status: authority.status,
        displayName: authority.displayName,
      },
      runtime: emptyRuntimeSnapshot(projects),
      profileDetails: [],
      projectProviders: [],
      providerCatalog: { registry: [], custom: [] },
      agentCandidates: [],
    };
  }
  const [
    runtime,
    projectProviders,
    providerCatalog,
    agentCandidates,
  ] = await Promise.all([
    loadRuntimeSnapshot(authority, projectSlug),
    projectSlug ? authority.deployment.appProviders(projectSlug) : Promise.resolve([]),
    projectSlug ? authority.deployment.appProviderCatalog(projectSlug) : Promise.resolve({ registry: [], custom: [] }),
    projectSlug ? authority.deployment.appAgentCandidates(projectSlug) : Promise.resolve([]),
  ]);
  const profileDetails = projectSlug
    ? await Promise.all(runtime.profiles.map((profile) => authority.deployment.appProfile(projectSlug, profile.id)))
    : [];
  return {
    authority: {
      kind: authority.kind,
      status: authority.status,
      displayName: authority.displayName,
    },
    runtime,
    profileDetails,
    projectProviders,
    providerCatalog,
    agentCandidates,
  };
}

function emptyRuntimeSnapshot(projects: RuntimeSnapshot["projects"]): RuntimeSnapshot {
  return {
    projects,
    profiles: [],
    bindings: [],
    endpoints: [],
    apiKeys: [],
    agentGateways: [],
    availableAgents: [],
    providerAdapters: [],
    traces: [],
    deployments: [],
    releases: [],
  };
}
