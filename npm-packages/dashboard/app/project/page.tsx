import { redirect } from "next/navigation";
import { RuntimeConsole } from "../../components/runtime-console";
import { configuredBrowserApiBaseUrl } from "../../lib/config";
import { loadDashboardData } from "../../lib/dashboard-data";

export const dynamic = "force-dynamic";

export default async function ProjectPage() {
  const data = await loadDashboardData("/project");
  const firstProject = data.runtime.projects[0];
  if (firstProject) redirect(`/project/${firstProject.slug}`);
  return <RuntimeConsole section="overview" data={data} browserApiBaseUrl={configuredBrowserApiBaseUrl()} />;
}
