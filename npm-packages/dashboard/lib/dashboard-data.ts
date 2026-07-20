import { loadRuntimeSnapshot, resolveAuthority, type AuthorityAdapter } from "./authority";
import type {
  AgentProfileDetail,
  ProviderCatalog,
  ProjectAgentCandidate,
  ProjectCanvas,
  ProjectProvider,
  RuntimeSnapshot,
} from "./runtime-types";

export type DashboardData = {
  authority: AuthorityAdapter;
  runtime: RuntimeSnapshot;
  canvas?: ProjectCanvas;
  profileDetails: AgentProfileDetail[];
  projectProviders: ProjectProvider[];
  providerCatalog: ProviderCatalog;
  agentCandidates: ProjectAgentCandidate[];
};

export async function loadDashboardData(returnTo: string, projectSlug?: string): Promise<DashboardData> {
  const authority = await resolveAuthority({ returnTo });
  const [runtime, canvas, projectProviders, providerCatalog, agentCandidates] = await Promise.all([
    loadRuntimeSnapshot(authority, projectSlug),
    projectSlug ? authority.deployment.projectCanvas(projectSlug) : Promise.resolve(undefined),
    projectSlug ? authority.deployment.projectProviders(projectSlug) : Promise.resolve([]),
    authority.deployment.providerCatalog(),
    projectSlug ? authority.deployment.projectAgentCandidates(projectSlug) : Promise.resolve([]),
  ]);
  const profileDetails = projectSlug
    ? await Promise.all(runtime.profiles.map((profile) => authority.deployment.projectProfile(projectSlug, profile.id)))
    : [];
  return { authority, runtime, canvas, profileDetails, projectProviders, providerCatalog, agentCandidates };
}
