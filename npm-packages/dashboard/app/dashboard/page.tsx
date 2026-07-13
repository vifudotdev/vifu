import { RuntimeConsole } from "../../components/runtime-console";
import { configuredBrowserApiBaseUrl, configuredProjectDomain } from "../../lib/config";
import { loadDashboardData } from "../../lib/dashboard-data";

export const dynamic = "force-dynamic";

export default async function DashboardPage() {
  const data = await loadDashboardData("/dashboard");
  return <RuntimeConsole section="overview" data={data} browserApiBaseUrl={configuredBrowserApiBaseUrl()} projectDomain={configuredProjectDomain()} />;
}
