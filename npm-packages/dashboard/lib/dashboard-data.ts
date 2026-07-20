import { loadRuntimeSnapshot, resolveAuthority, type AuthorityAdapter } from "./authority";
import type {
  AgentProfileDetail,
  ProjectAgentCandidate,
  ProjectCanvas,
  ProviderStockItem,
  RuntimeSnapshot,
} from "./runtime-types";

export type DashboardData = {
  authority: AuthorityAdapter;
  runtime: RuntimeSnapshot;
  canvas?: ProjectCanvas;
  profileDetails: AgentProfileDetail[];
  projectProviders: ProviderStockItem[];
  providerStock: ProviderStockItem[];
  agentCandidates: ProjectAgentCandidate[];
};

export async function loadDashboardData(returnTo: string, projectSlug?: string): Promise<DashboardData> {
  const authority = await resolveAuthority({ returnTo });
  const [runtime, canvas, projectProviders, providerStock, agentCandidates] = await Promise.all([
    loadRuntimeSnapshot(authority, projectSlug),
    projectSlug ? authority.deployment.projectCanvas(projectSlug) : Promise.resolve(undefined),
    projectSlug ? authority.deployment.projectProviders(projectSlug) : Promise.resolve([]),
    authority.deployment.providerStock(),
    projectSlug ? authority.deployment.projectAgentCandidates(projectSlug) : Promise.resolve([]),
  ]);
  const profileDetails = projectSlug
    ? await Promise.all(runtime.profiles.map((profile) => authority.deployment.projectProfile(projectSlug, profile.id)))
    : [];
  return { authority, runtime, canvas, profileDetails, projectProviders, providerStock, agentCandidates };
}
