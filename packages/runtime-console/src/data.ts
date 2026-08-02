import type {
  AgentProfileDetail,
  DeploymentStatus,
  ProviderCatalog,
  ProjectAgentCandidate,
  ProjectProvider,
  RuntimeSnapshot,
} from "./types";

export type RuntimeConsoleAuthority = {
  kind: "local" | "self-hosted" | "cloud" | string;
  status: DeploymentStatus;
  displayName?: string;
};

export type RuntimeConsoleData = {
  authority: RuntimeConsoleAuthority;
  runtime: RuntimeSnapshot;
  profileDetails: AgentProfileDetail[];
  projectProviders: ProjectProvider[];
  providerCatalog: ProviderCatalog;
  agentCandidates: ProjectAgentCandidate[];
};

export type DashboardData = RuntimeConsoleData;
