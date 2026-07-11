import { notFound } from "next/navigation";
import { RuntimeConsole, type DashboardSection } from "../../../components/runtime-console";
import { configuredBrowserApiBaseUrl } from "../../../lib/config";
import { loadDashboardData } from "../../../lib/dashboard-data";

export const dynamic = "force-dynamic";

const SECTIONS = new Set<DashboardSection>([
  "profiles",
  "bindings",
  "endpoints",
  "api-keys",
  "connections",
  "traces",
  "account",
  "projects",
  "billing",
]);

export default async function DashboardSectionPage({ params }: {
  params: Promise<{ section: string }>;
}) {
  const { section } = await params;
  if (!SECTIONS.has(section as DashboardSection)) notFound();
  const data = await loadDashboardData(`/dashboard/${section}`);
  return <RuntimeConsole section={section as DashboardSection} data={data} browserApiBaseUrl={configuredBrowserApiBaseUrl()} />;
}
