import { loadRuntimeSnapshot, resolveAuthority, type AuthorityAdapter } from "./authority";
import type { RuntimeSnapshot } from "./runtime-types";

export type DashboardData = {
  authority: AuthorityAdapter;
  runtime: RuntimeSnapshot;
};

export async function loadDashboardData(returnTo: string): Promise<DashboardData> {
  const authority = await resolveAuthority({ returnTo });
  const runtime = await loadRuntimeSnapshot(authority);
  return { authority, runtime };
}
