import { createCloudClient, type CloudDashboardData, type CloudDashboardProject } from "./client";
import { loadRuntimeSnapshot, resolveAuthority, type AuthorityAdapter } from "./authority";
import type { RuntimeSnapshot } from "./runtime-types";

export type CloudAccountSnapshot = {
  dashboard: CloudDashboardData | null;
  projects: CloudDashboardProject[];
  billing: Record<string, unknown> | null;
  error: string | null;
};

export type DashboardData = {
  authority: AuthorityAdapter;
  runtime: RuntimeSnapshot;
  cloud: CloudAccountSnapshot | null;
};

export async function loadDashboardData(returnTo: string): Promise<DashboardData> {
  const authority = await resolveAuthority({ returnTo });
  const runtimePromise = loadRuntimeSnapshot(authority);
  const cloudPromise = authority.kind === "cloud"
    ? loadCloudAccountSnapshot(authority.session?.token ?? "")
    : Promise.resolve(null);
  const [runtime, cloud] = await Promise.all([runtimePromise, cloudPromise]);
  return { authority, runtime, cloud };
}

async function loadCloudAccountSnapshot(token: string): Promise<CloudAccountSnapshot> {
  const client = createCloudClient(token);
  const [dashboardResult, billingResult] = await Promise.allSettled([
    client.dashboard(),
    client.billingAccount(),
  ]);
  const dashboard = dashboardResult.status === "fulfilled" ? dashboardResult.value : null;
  return {
    dashboard: dashboard?.dashboard ?? null,
    projects: dashboard?.projects ?? [],
    billing: billingResult.status === "fulfilled" ? billingResult.value : null,
    error: dashboardResult.status === "rejected"
      ? errorMessage(dashboardResult.reason)
      : billingResult.status === "rejected"
        ? errorMessage(billingResult.reason)
        : null,
  };
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : "Cloud account data is unavailable.";
}
