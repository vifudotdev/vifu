import { loadRuntimeSnapshot, resolveAuthority, type AuthorityAdapter } from "./authority";
import type {
  AgentProfileDetail,
  GameAnalytics,
  GameAsset,
  GameDraft,
  GameNodeDefinition,
  GameOverview,
  GameQa,
  GameRelease,
  GameResource,
  GameSession,
  ProviderCatalog,
  ProjectAgentCandidate,
  ProjectProvider,
  RuntimeSnapshot,
} from "./runtime-types";

export type DashboardData = {
  authority: AuthorityAdapter;
  runtime: RuntimeSnapshot;
  gameOverview?: GameOverview;
  gameDraft?: GameDraft;
  gameNodeDefinitions: GameNodeDefinition[];
  gameResources: GameResource[];
  gameAssets: GameAsset[];
  gameQa?: GameQa;
  gameAnalytics?: GameAnalytics;
  gameSessions: GameSession[];
  gameReleases: GameRelease[];
  profileDetails: AgentProfileDetail[];
  projectProviders: ProjectProvider[];
  providerCatalog: ProviderCatalog;
  agentCandidates: ProjectAgentCandidate[];
};

export async function loadDashboardData(
  returnTo: string,
  projectSlug?: string,
  options: {
    includeGameSource?: boolean;
    includeGameLibraries?: boolean;
    includeGameQa?: boolean;
    includeGameAnalytics?: boolean;
    includeGameSessions?: boolean;
    includeGameReleases?: boolean;
  } = {},
): Promise<DashboardData> {
  const authority = await resolveAuthority({ returnTo });
  const [
    runtime,
    gameOverview,
    gameDraft,
    gameNodeDefinitions,
    gameResources,
    gameAssets,
    gameQa,
    gameAnalytics,
    gameSessions,
    gameReleases,
    projectProviders,
    providerCatalog,
    agentCandidates,
  ] = await Promise.all([
    loadRuntimeSnapshot(authority, projectSlug),
    projectSlug && authority.status.capabilities.gameRuntime
      ? authority.deployment.gameOverview(projectSlug)
      : Promise.resolve(undefined),
    projectSlug && options.includeGameSource && authority.status.capabilities.gameRuntime
      ? authority.deployment.gameDraft(projectSlug)
      : Promise.resolve(undefined),
    options.includeGameSource && authority.status.capabilities.gameRuntime
      ? authority.deployment.gameNodeDefinitions()
      : Promise.resolve([]),
    projectSlug && options.includeGameLibraries && authority.status.capabilities.gameRuntime
      ? authority.deployment.gameResources(projectSlug)
      : Promise.resolve([]),
    projectSlug && options.includeGameLibraries && authority.status.capabilities.gameRuntime
      ? authority.deployment.gameAssets(projectSlug)
      : Promise.resolve([]),
    projectSlug && options.includeGameQa && authority.status.capabilities.gameRuntime
      ? authority.deployment.gameQa(projectSlug)
      : Promise.resolve(undefined),
    projectSlug && options.includeGameAnalytics && authority.status.capabilities.gameRuntime
      ? authority.deployment.gameAnalytics(projectSlug)
      : Promise.resolve(undefined),
    projectSlug && options.includeGameSessions && authority.status.capabilities.gameRuntime
      ? authority.deployment.gameSessions(projectSlug)
      : Promise.resolve([]),
    projectSlug && options.includeGameReleases && authority.status.capabilities.gameRuntime
      ? authority.deployment.gameReleases(projectSlug)
      : Promise.resolve([]),
    projectSlug ? authority.deployment.projectProviders(projectSlug) : Promise.resolve([]),
    authority.deployment.providerCatalog(),
    projectSlug ? authority.deployment.projectAgentCandidates(projectSlug) : Promise.resolve([]),
  ]);
  const profileDetails = projectSlug
    ? await Promise.all(runtime.profiles.map((profile) => authority.deployment.projectProfile(projectSlug, profile.id)))
    : [];
  return {
    authority,
    runtime,
    gameOverview,
    gameDraft,
    gameNodeDefinitions,
    gameResources,
    gameAssets,
    gameQa,
    gameAnalytics,
    gameSessions,
    gameReleases,
    profileDetails,
    projectProviders,
    providerCatalog,
    agentCandidates,
  };
}
