import Image from "next/image";
import Link from "next/link";
import type { LucideIcon } from "lucide-react";
import {
  Activity,
  Boxes,
  Cable,
  FolderKanban,
  Gauge,
  KeyRound,
  LogOut,
  Network,
  RadioTower,
  Route,
  ScrollText,
} from "lucide-react";
import type { DashboardData } from "../lib/dashboard-data";
import { authRequired } from "../lib/auth-providers";
import type {
  AgentBinding,
  AgentEndpoint,
  AgentProfile,
  ApiKeyRecord,
  AvailableAgent,
  AgentGateway,
  EndpointTrace,
  ServerCapabilities,
  RuntimeProject,
} from "../lib/runtime-types";
import {
  ApiKeyCreateForm,
  BindingCreateForm,
  BindingEditForm,
  DeleteResourceButton,
  EndpointCreateForm,
  EndpointEditForm,
  InvokeEndpointForm,
  ProfileCreateForm,
  ProfileEditForm,
  ProjectCreateForm,
  RevokeApiKeyButton,
} from "./runtime-actions";

export type DashboardSection =
  | "overview"
  | "projects"
  | "profiles"
  | "bindings"
  | "endpoints"
  | "api-keys"
  | "gateways"
  | "traces";

type NavigationItem = {
  id: DashboardSection;
  label: string;
  icon: LucideIcon;
  capability?: keyof ServerCapabilities;
};

const CORE_NAVIGATION: NavigationItem[] = [
  { id: "overview", label: "Overview", icon: Gauge },
  { id: "projects", label: "Projects", icon: FolderKanban, capability: "projects" },
  { id: "profiles", label: "Profiles", icon: Boxes, capability: "profiles" },
  { id: "bindings", label: "Bindings", icon: Cable, capability: "bindings" },
  { id: "endpoints", label: "Endpoints", icon: Route, capability: "endpoints" },
  { id: "api-keys", label: "API keys", icon: KeyRound, capability: "apiKeys" },
  { id: "gateways", label: "Agent Gateways", icon: RadioTower, capability: "agentGateways" },
  { id: "traces", label: "Traces", icon: ScrollText, capability: "traces" },
];

const SECTION_TITLES: Record<DashboardSection, { eyebrow: string; title: string }> = {
  overview: { eyebrow: "Runtime", title: "Overview" },
  projects: { eyebrow: "Runtime", title: "Projects" },
  profiles: { eyebrow: "Agents", title: "Agent profiles" },
  bindings: { eyebrow: "Routing", title: "Bindings" },
  endpoints: { eyebrow: "Runtime", title: "Agent endpoints" },
  "api-keys": { eyebrow: "Access", title: "API keys" },
  gateways: { eyebrow: "Agents", title: "Agent Gateways" },
  traces: { eyebrow: "Observability", title: "Traces" },
};

export function RuntimeConsole({ section, data, browserApiBaseUrl, projectDomain }: {
  section: DashboardSection;
  data: DashboardData;
  browserApiBaseUrl: string;
  projectDomain: string;
}) {
  const capabilities = data.authority.status.capabilities;
  const activeSection = isSectionAvailable(section, capabilities) ? section : "overview";
  const title = SECTION_TITLES[activeSection];
  return (
    <main className="console-app">
      <aside className="console-sidebar">
        <Link className="console-brand" href="/dashboard" aria-label="Vifu Dashboard">
          <Image src="/brand/vifu-icon-512.png" width={32} height={32} alt="" priority />
          <span>Vifu</span>
        </Link>
        <Navigation title="Runtime" items={CORE_NAVIGATION} active={activeSection} capabilities={capabilities} />
        <div className="sidebar-footer">
          <span>{dashboardIdentity(data)}</span>
          <small>Runtime</small>
          {authRequired(data.authority.status.auth) ? (
            <form action="/auth/logout" method="post">
              <button className="sidebar-signout" type="submit"><LogOut aria-hidden="true" />Sign out</button>
            </form>
          ) : null}
        </div>
      </aside>

      <section className="console-main">
        <header className="console-topbar">
          <div className="runtime-state"><span className="status-dot" />{data.authority.status.status}</div>
          <div className="topbar-meta"><span>v{data.authority.status.version}</span><span>{data.authority.status.agentGateways} gateways</span></div>
        </header>
        <header className="page-header">
          <p>{title.eyebrow}</p>
          <h1>{title.title}</h1>
        </header>
        <div className="console-content">
          <DashboardSectionView section={activeSection} data={data} browserApiBaseUrl={browserApiBaseUrl} projectDomain={projectDomain} />
        </div>
      </section>
    </main>
  );
}

function Navigation({ title, items, active, capabilities }: {
  title: string;
  items: NavigationItem[];
  active: DashboardSection;
  capabilities: ServerCapabilities;
}) {
  const visible = items.filter((item) => !item.capability || capabilities[item.capability]);
  if (visible.length === 0) return null;
  return (
    <nav className="console-nav" aria-label={`${title} navigation`}>
      <span>{title}</span>
      {visible.map((item) => {
        const Icon = item.icon;
        const href = item.id === "overview" ? "/dashboard" : `/dashboard/${item.id}`;
        return <Link className={active === item.id ? "active" : ""} href={href} key={item.id} prefetch={false}><Icon aria-hidden="true" />{item.label}</Link>;
      })}
    </nav>
  );
}

function DashboardSectionView({ section, data, browserApiBaseUrl, projectDomain }: {
  section: DashboardSection;
  data: DashboardData;
  browserApiBaseUrl: string;
  projectDomain: string;
}) {
  if (section === "projects") return <ProjectsView projects={data.runtime.projects} availableAgents={data.runtime.availableAgents} agentGateways={data.runtime.agentGateways} browserApiBaseUrl={browserApiBaseUrl} projectDomain={projectDomain} />;
  if (section === "profiles") return <ProfilesView profiles={data.runtime.profiles} />;
  if (section === "bindings") return <BindingsView profiles={data.runtime.profiles} bindings={data.runtime.bindings} agentGateways={data.runtime.agentGateways} />;
  if (section === "endpoints") return <EndpointsView profiles={data.runtime.profiles} bindings={data.runtime.bindings} endpoints={data.runtime.endpoints} browserApiBaseUrl={browserApiBaseUrl} />;
  if (section === "api-keys") return <ApiKeysView keys={data.runtime.apiKeys} endpoints={data.runtime.endpoints} />;
  if (section === "gateways") return <AgentGatewaysView agentGateways={data.runtime.agentGateways} availableAgents={data.runtime.availableAgents} />;
  if (section === "traces") return <TracesView traces={data.runtime.traces} endpoints={data.runtime.endpoints} projects={data.runtime.projects} />;
  return <OverviewView data={data} />;
}

function OverviewView({ data }: { data: DashboardData }) {
  const online = data.runtime.agentGateways.filter((gateway) => gateway.status === "connected").length;
  const completed = data.runtime.traces.filter((trace) => trace.status === "completed").length;
  return (
    <>
      <section className="metric-strip" aria-label="Runtime totals">
        <Metric label="Projects" value={data.runtime.projects.length} icon={FolderKanban} />
        <Metric label="Profiles" value={data.runtime.profiles.length} icon={Boxes} />
        <Metric label="Detected agents" value={data.runtime.availableAgents.filter((agent) => agent.status === "connected").length} icon={Route} />
        <Metric label="Agent gateways" value={online} icon={Network} />
        <Metric label="Completed calls" value={completed} icon={Activity} />
      </section>
      <section className="content-section">
        <SectionHeading title="Runtime status" />
        <dl className="definition-grid">
          <div><dt>Runtime</dt><dd>{data.authority.status.status === "ok" ? "Ready" : data.authority.status.status}</dd></div>
          <div><dt>Agent connection</dt><dd>{online > 0 ? `${online} online` : "Waiting for an agent"}</dd></div>
          <div><dt>Game access</dt><dd>{data.authority.status.capabilities.jsonRpc ? "Ready" : "Unavailable"}</dd></div>
          <div><dt>Run history</dt><dd>{data.authority.status.capabilities.traces ? "Ready" : "Unavailable"}</dd></div>
        </dl>
      </section>
      <section className="content-section">
        <SectionHeading title="Recent traces" action={<Link href="/dashboard/traces">View all</Link>} />
        <TraceTable traces={data.runtime.traces.slice(0, 8)} endpoints={data.runtime.endpoints} projects={data.runtime.projects} />
      </section>
    </>
  );
}

function ProjectsView({ projects, availableAgents, agentGateways, browserApiBaseUrl, projectDomain }: {
  projects: RuntimeProject[];
  availableAgents: AvailableAgent[];
  agentGateways: AgentGateway[];
  browserApiBaseUrl: string;
  projectDomain: string;
}) {
  return (
    <>
      <section className="content-section create-section"><SectionHeading title="New project" /><ProjectCreateForm availableAgents={availableAgents} agentGateways={agentGateways} /></section>
      <section className="content-section"><SectionHeading title="Projects" count={projects.length} />
        <ResourceList empty="No projects yet.">{projects.map((project) => (
          <article className="resource-row" key={project.id}>
            <ResourceIdentity title={project.name} code={projectRpcUrl(project.slug, browserApiBaseUrl, projectDomain)} description={project.description} />
            <div className="resource-meta"><span className={project.enabled ? "status-label ready" : "status-label off"}>{project.enabled ? "Enabled" : "Disabled"}</span><span>{project.bindingIds.length} agents</span><span>JSON-RPC over HTTPS/WSS</span><span>{project.publishableKeyPrefix}...</span></div>
            <DeleteResourceButton path={`projects/${project.id}`} label={project.name} />
          </article>
        ))}</ResourceList>
      </section>
    </>
  );
}

function projectRpcUrl(slug: string, browserApiBaseUrl: string, projectDomain: string): string {
  const url = new URL(browserApiBaseUrl);
  url.hostname = `${slug}.${projectDomain}`;
  url.pathname = "/";
  url.search = "";
  return url.toString().replace(/\/$/, "");
}

function ProfilesView({ profiles }: { profiles: AgentProfile[] }) {
  return (
    <>
      <section className="content-section create-section"><SectionHeading title="New profile" /><ProfileCreateForm /></section>
      <section className="content-section"><SectionHeading title="Profiles" count={profiles.length} />
        <ResourceList empty="No profiles yet.">{profiles.map((profile) => (
          <article className="resource-row" key={profile.id}>
            <ResourceIdentity title={profile.name} code={profile.slug} description={profile.description} />
            <div className="resource-meta"><span>Routing profile</span><time>{formatDate(profile.updatedAt)}</time></div>
            <details className="row-editor"><summary>Edit</summary><ProfileEditForm profile={profile} /></details>
            <DeleteResourceButton path={`profiles/${profile.id}`} label={profile.name} />
          </article>
        ))}</ResourceList>
      </section>
    </>
  );
}

function BindingsView({ profiles, bindings, agentGateways }: { profiles: AgentProfile[]; bindings: AgentBinding[]; agentGateways: AgentGateway[] }) {
  const profileNames = new Map(profiles.map((profile) => [profile.id, profile.name]));
  return (
    <>
      <section className="content-section create-section"><SectionHeading title="New OpenClaw binding" /><BindingCreateForm profiles={profiles} agentGateways={agentGateways} /></section>
      <section className="content-section"><SectionHeading title="Bindings" count={bindings.length} />
        <ResourceList empty="No bindings yet.">{bindings.map((binding) => (
          <article className="resource-row" key={binding.id}>
            <ResourceIdentity title={profileNames.get(binding.profileId) ?? "Unknown profile"} code={`${binding.gatewayId} / ${binding.agentId}`} description={binding.provider} />
            <div className="resource-meta"><span>OpenClaw HTTP</span><time>{formatDate(binding.updatedAt)}</time></div>
            <details className="row-editor"><summary>Edit</summary><BindingEditForm binding={binding} /></details>
            <DeleteResourceButton path={`bindings/${binding.id}`} label={`${binding.gatewayId} binding`} />
          </article>
        ))}</ResourceList>
      </section>
    </>
  );
}

function EndpointsView({ profiles, bindings, endpoints, browserApiBaseUrl }: {
  profiles: AgentProfile[];
  bindings: AgentBinding[];
  endpoints: AgentEndpoint[];
  browserApiBaseUrl: string;
}) {
  const profileNames = new Map(profiles.map((profile) => [profile.id, profile.name]));
  return (
    <>
      <section className="content-section create-section"><SectionHeading title="New endpoint" /><EndpointCreateForm profiles={profiles} bindings={bindings} /></section>
      <section className="content-section"><SectionHeading title="Endpoints" count={endpoints.length} />
        <ResourceList empty="No endpoints yet.">{endpoints.map((endpoint) => (
          <article className="resource-row endpoint-row" key={endpoint.id}>
            <ResourceIdentity title={endpoint.name} code={`${browserApiBaseUrl}/v1/endpoints/${endpoint.slug}/invoke`} description={profileNames.get(endpoint.profileId) ?? "Unknown profile"} />
            <div className="resource-meta"><span className={endpoint.enabled ? "status-label ready" : "status-label off"}>{endpoint.enabled ? "Enabled" : "Disabled"}</span><span>{endpoint.requestTimeoutMs} ms</span></div>
            <InvokeEndpointForm endpoint={endpoint} />
            <details className="row-editor"><summary>Edit</summary><EndpointEditForm endpoint={endpoint} /></details>
            <DeleteResourceButton path={`endpoints/${endpoint.id}`} label={endpoint.name} />
          </article>
        ))}</ResourceList>
      </section>
    </>
  );
}

function ApiKeysView({ keys, endpoints }: { keys: ApiKeyRecord[]; endpoints: AgentEndpoint[] }) {
  const endpointNames = new Map(endpoints.map((endpoint) => [endpoint.id, endpoint.name]));
  return (
    <>
      <section className="content-section create-section"><SectionHeading title="New API key" /><ApiKeyCreateForm endpoints={endpoints} /></section>
      <section className="content-section"><SectionHeading title="API keys" count={keys.length} />
        <ResourceList empty="No API keys yet.">{keys.map((key) => (
          <article className="resource-row" key={key.id}>
            <ResourceIdentity title={key.name} code={`${key.keyPrefix}...`} description={endpointNames.get(key.endpointId) ?? "Unknown endpoint"} />
            <div className="resource-meta"><span className={key.revokedAt ? "status-label off" : "status-label ready"}>{key.revokedAt ? "Revoked" : "Active"}</span><time>{formatDate(key.createdAt)}</time></div>
            {!key.revokedAt ? <RevokeApiKeyButton id={key.id} name={key.name} /> : null}
          </article>
        ))}</ResourceList>
      </section>
    </>
  );
}

function AgentGatewaysView({ agentGateways, availableAgents }: { agentGateways: AgentGateway[]; availableAgents: AvailableAgent[] }) {
  return (
    <>
      <section className="content-section"><SectionHeading title="Detected agents" count={availableAgents.length} />
        <ResourceList empty="No agents reported by an Agent Gateway yet.">{availableAgents.map((agent) => (
          <article className="resource-row" key={`${agent.gatewayId}/${agent.id}`}>
            <ResourceIdentity title={agent.name} code={agent.id} description={agent.gatewayId} />
            <div className="resource-meta"><span className={agent.status === "connected" ? "status-label ready" : "status-label off"}>{agent.status}</span></div>
          </article>
        ))}</ResourceList>
      </section>
      <section className="content-section"><SectionHeading title="Agent Gateway sessions" count={agentGateways.length} />
        <ResourceList empty="No Agent Gateway sessions yet.">{agentGateways.map((gateway) => (
          <article className="resource-row" key={gateway.id}>
            <ResourceIdentity title={gateway.gatewayId} code={gateway.sessionId} description={`${gateway.agents.length} agents`} />
            <div className="resource-meta"><span className={gateway.status === "connected" ? "status-label ready" : "status-label off"}>{gateway.status}</span><time>{formatDate(gateway.lastSeenAt)}</time></div>
          </article>
        ))}</ResourceList>
      </section>
    </>
  );
}

function TracesView({ traces, endpoints, projects }: { traces: EndpointTrace[]; endpoints: AgentEndpoint[]; projects: RuntimeProject[] }) {
  return <section className="content-section"><SectionHeading title="Invocation traces" count={traces.length} /><TraceTable traces={traces} endpoints={endpoints} projects={projects} /></section>;
}

function TraceTable({ traces, endpoints, projects }: { traces: EndpointTrace[]; endpoints: AgentEndpoint[]; projects: RuntimeProject[] }) {
  const resourceNames = new Map([
    ...endpoints.map((endpoint) => [endpoint.id, endpoint.name] as const),
    ...projects.map((project) => [project.id, project.name] as const),
  ]);
  if (traces.length === 0) return <EmptyState>No traces yet.</EmptyState>;
  return (
    <div className="table-scroll"><table className="runtime-table"><thead><tr><th>Status</th><th>Endpoint</th><th>Request</th><th>Latency</th><th>Created</th></tr></thead>
      <tbody>{traces.map((trace) => { const resourceId = trace.endpointId ?? trace.projectId; return <tr key={trace.id}><td><span className={trace.status === "completed" ? "status-label ready" : trace.status === "pending" ? "status-label pending" : "status-label off"}>{trace.status}</span></td><td>{resourceId ? resourceNames.get(resourceId) ?? shortId(resourceId) : "-"}</td><td><code>{shortId(trace.requestId)}</code></td><td>{trace.latencyMs === null ? "-" : `${trace.latencyMs} ms`}</td><td>{formatDate(trace.createdAt)}</td></tr>; })}</tbody>
    </table></div>
  );
}

function Metric({ label, value, icon: Icon }: { label: string; value: number; icon: LucideIcon }) {
  return <article className="metric"><div><Icon aria-hidden="true" /><span>{label}</span></div><strong>{value}</strong></article>;
}

function SectionHeading({ title, count, action }: { title: string; count?: number; action?: React.ReactNode }) {
  return <header className="section-heading"><div><h2>{title}</h2>{count !== undefined ? <span>{count}</span> : null}</div>{action}</header>;
}

function ResourceList({ empty, children }: { empty: string; children: React.ReactNode }) {
  const hasChildren = Array.isArray(children) ? children.length > 0 : Boolean(children);
  return <div className="resource-list">{hasChildren ? children : <EmptyState>{empty}</EmptyState>}</div>;
}

function ResourceIdentity({ title, code, description }: { title: string; code: string; description?: string | null }) {
  return <div className="resource-identity"><strong>{title}</strong><code>{code}</code>{description ? <span>{description}</span> : null}</div>;
}

function EmptyState({ children }: { children: React.ReactNode }) {
  return <div className="empty-state">{children}</div>;
}

function isSectionAvailable(section: DashboardSection, capabilities: ServerCapabilities): boolean {
  const item = CORE_NAVIGATION.find((entry) => entry.id === section);
  if (!item) return false;
  return !item.capability || capabilities[item.capability];
}

function dashboardIdentity(data: DashboardData): string {
  const principal = data.authority.principal;
  return principal?.displayName
    || principal?.email
    || data.authority.displayName
    || "Vifu";
}

function formatDate(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.valueOf()) ? "-" : new Intl.DateTimeFormat("en", { dateStyle: "medium", timeStyle: "short" }).format(date);
}

function shortId(value: string): string {
  return value.length > 12 ? `${value.slice(0, 8)}...` : value;
}
