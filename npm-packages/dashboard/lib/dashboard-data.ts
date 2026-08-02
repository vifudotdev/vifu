import type { RuntimeConsoleData } from "@vifu/runtime-console";
import { loadRuntimeSnapshot, resolveAuthority } from "./authority";
import type {
  AgentProfileDetail,
  ProviderCatalog,
  ProjectAgentCandidate,
  ProjectProvider,
  RuntimeSnapshot,
} from "./runtime-types";

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
): Promise<DashboardData> {
  const authority = await resolveAuthority({ returnTo });
  const [
    runtime,
    projectProviders,
    providerCatalog,
    agentCandidates,
  ] = await Promise.all([
    loadRuntimeSnapshot(authority, projectSlug),
    projectSlug ? authority.deployment.projectProviders(projectSlug) : Promise.resolve([]),
    projectSlug ? authority.deployment.projectProviderCatalog(projectSlug) : Promise.resolve({ registry: [], custom: [] }),
    projectSlug ? authority.deployment.projectAgentCandidates(projectSlug) : Promise.resolve([]),
  ]);
  const profileDetails = projectSlug
    ? await Promise.all(runtime.profiles.map((profile) => authority.deployment.projectProfile(projectSlug, profile.id)))
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
