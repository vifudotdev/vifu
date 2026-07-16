import { RuntimeConsole } from "../../../components/runtime-console";
import { configuredBrowserApiBaseUrl } from "../../../lib/config";
import { loadDashboardData } from "../../../lib/dashboard-data";

export const dynamic = "force-dynamic";

export default async function ProjectHomePage({ params }: {
  params: Promise<{ projectSlug: string }>;
}) {
  const { projectSlug } = await params;
  const data = await loadDashboardData(`/project/${projectSlug}`, projectSlug);
  return <RuntimeConsole section="health" projectSlug={projectSlug} data={data} browserApiBaseUrl={configuredBrowserApiBaseUrl()} />;
}
