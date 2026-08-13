import type { DashboardSection } from "@vifu/console/react";

const SECTION_IDS = new Set<DashboardSection>([
  "overview",
  "devices",
  "agents",
  "providers",
  "deployments",
  "api",
  "logs",
  "settings",
]);

export type ConsoleRoute = {
  hash?: string;
  projectSlug?: string;
  search?: string;
  section: DashboardSection;
};

export function readConsoleRoute(pathname: string, search = "", hash = ""): ConsoleRoute {
  const parts = pathname.split("/").filter(Boolean);
  if (parts[0] !== "project") return { section: "overview", search, hash };
  const projectSlug = parts[1] ? decodeURIComponent(parts[1]) : undefined;
  const section = SECTION_IDS.has(parts[2] as DashboardSection)
    ? parts[2] as DashboardSection
    : "overview";
  return { projectSlug, section, search, hash };
}

export function consoleRouteHref(route: ConsoleRoute): string {
  const suffix = `${route.search ?? ""}${route.hash ?? ""}`;
  if (!route.projectSlug) return `/project${suffix}`;
  if (route.section === "overview") return `/project/${encodeURIComponent(route.projectSlug)}${suffix}`;
  return `/project/${encodeURIComponent(route.projectSlug)}/${encodeURIComponent(route.section)}${suffix}`;
}
