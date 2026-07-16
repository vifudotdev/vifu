import { loadRuntimeSnapshot, resolveAuthority, type AuthorityAdapter } from "./authority";
import type { ProjectCanvas, ProviderConnection, RuntimeSnapshot } from "./runtime-types";

export type DashboardData = {
  authority: AuthorityAdapter;
  runtime: RuntimeSnapshot;
  canvas?: ProjectCanvas;
  providerConnections: ProviderConnection[];
};

export async function loadDashboardData(returnTo: string, projectSlug?: string): Promise<DashboardData> {
  const authority = await resolveAuthority({ returnTo });
  const [runtime, canvas, providerConnections] = await Promise.all([
    loadRuntimeSnapshot(authority),
    projectSlug ? authority.deployment.projectCanvas(projectSlug) : Promise.resolve(undefined),
    projectSlug ? authority.deployment.providerConnections(projectSlug) : Promise.resolve([]),
  ]);
  return { authority, runtime, canvas, providerConnections };
}
