import { notFound } from "next/navigation";
import { RuntimeConsole, type DashboardSection } from "../../../../components/runtime-console";
import { configuredBrowserApiBaseUrl } from "../../../../lib/config";
import { loadDashboardData } from "../../../../lib/dashboard-data";

export const dynamic = "force-dynamic";

const VIEWS = new Set<DashboardSection>([
  "overview",
  "canvas",
  "short-drama",
  "agents",
  "resources",
  "providers",
  "preview",
  "api",
  "analytics",
  "logs",
  "settings",
]);

export default async function ProjectViewPage({ params }: {
  params: Promise<{ projectSlug: string; view: string }>;
}) {
  const { projectSlug, view } = await params;
  if (!VIEWS.has(view as DashboardSection)) notFound();
  const section = view as DashboardSection;
  const data = await loadDashboardData(`/project/${projectSlug}/${view}`, projectSlug, {
    includeGameSource: section === "canvas" || section === "short-drama" || section === "resources" || section === "preview",
    includeGameLibraries: section === "canvas" || section === "short-drama" || section === "resources" || section === "preview",
    includeGameQa: section === "preview",
    includeGameAnalytics: section === "analytics",
    includeGameSessions: section === "preview",
    includeGameReleases: section === "preview" || section === "api",
  });
  return (
    <RuntimeConsole
      section={section}
      projectSlug={projectSlug}
      data={data}
      browserApiBaseUrl={configuredBrowserApiBaseUrl()}
    />
  );
}
