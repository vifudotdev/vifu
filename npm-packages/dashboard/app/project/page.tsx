import { RuntimeConsole } from "../../components/runtime-console-host";
import { configuredBrowserApiBaseUrl } from "../../lib/config";
import { loadDashboardData } from "../../lib/dashboard-data";

export const dynamic = "force-dynamic";

export default async function ProjectPage() {
  const data = await loadDashboardData("/project");
  return <RuntimeConsole section="overview" data={data} browserApiBaseUrl={configuredBrowserApiBaseUrl()} />;
}
