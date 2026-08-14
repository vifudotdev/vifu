import { RuntimeConsole } from "../../components/runtime-console-host";
import { configuredBrowserApiBaseUrl } from "../../lib/config";
import { loadDashboardData } from "../../lib/dashboard-data";

export const dynamic = "force-dynamic";

export default async function AppsPage() {
  const data = await loadDashboardData("/apps");
  return <RuntimeConsole section="overview" data={data} browserApiBaseUrl={configuredBrowserApiBaseUrl()} />;
}
