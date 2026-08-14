import { RuntimeConsole } from "../../../components/runtime-console-host";
import { configuredBrowserApiBaseUrl } from "../../../lib/config";
import { loadDashboardData } from "../../../lib/dashboard-data";

export const dynamic = "force-dynamic";

export default async function AppHomePage({ params }: {
  params: Promise<{ projectSlug: string }>;
}) {
  const { projectSlug } = await params;
  const data = await loadDashboardData(`/apps/${projectSlug}`, projectSlug);
  return <RuntimeConsole section="overview" projectSlug={projectSlug} data={data} browserApiBaseUrl={configuredBrowserApiBaseUrl()} />;
}
