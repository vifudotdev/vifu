import { notFound } from "next/navigation";
import { RuntimeConsole, type DashboardSection } from "../../../../components/runtime-console";
import { configuredBrowserApiBaseUrl } from "../../../../lib/config";
import { loadDashboardData } from "../../../../lib/dashboard-data";

export const dynamic = "force-dynamic";

const VIEWS = new Set<DashboardSection>(["health", "agents", "providers", "gameplay", "api", "logs", "settings"]);

export default async function ProjectViewPage({ params }: {
  params: Promise<{ projectSlug: string; view: string }>;
}) {
  const { projectSlug, view } = await params;
  if (!VIEWS.has(view as DashboardSection)) notFound();
  const data = await loadDashboardData(`/project/${projectSlug}/${view}`, projectSlug);
  return (
    <RuntimeConsole
      section={view as DashboardSection}
      projectSlug={projectSlug}
      data={data}
      browserApiBaseUrl={configuredBrowserApiBaseUrl()}
    />
  );
}
