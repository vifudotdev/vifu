import Image from "next/image";
import Link from "next/link";
import type { LucideIcon } from "lucide-react";
import {
  Activity,
  Boxes,
  Cable,
  CircleUserRound,
  Cloud,
  CreditCard,
  ExternalLink,
  Gauge,
  KeyRound,
  LogOut,
  Network,
  RadioTower,
  Route,
  ScrollText,
  UsersRound,
} from "lucide-react";
import type { DashboardData } from "../lib/dashboard-data";
import type {
  AgentBinding,
  AgentEndpoint,
  AgentProfile,
  ApiKeyRecord,
  ConnectorSession,
  EndpointTrace,
  ServerCapabilities,
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
  RevokeApiKeyButton,
} from "./runtime-actions";

export type DashboardSection =
  | "overview"
  | "profiles"
  | "bindings"
  | "endpoints"
  | "api-keys"
  | "connections"
  | "traces"
  | "account"
  | "projects"
  | "billing";

type NavigationItem = {
  id: DashboardSection;
  label: string;
  icon: LucideIcon;
  capability?: keyof ServerCapabilities;
};

const CORE_NAVIGATION: NavigationItem[] = [
  { id: "overview", label: "Overview", icon: Gauge },
  { id: "profiles", label: "Profiles", icon: Boxes, capability: "profiles" },
  { id: "bindings", label: "Bindings", icon: Cable, capability: "bindings" },
  { id: "endpoints", label: "Endpoints", icon: Route, capability: "endpoints" },
  { id: "api-keys", label: "API keys", icon: KeyRound, capability: "apiKeys" },
  { id: "connections", label: "Connections", icon: RadioTower, capability: "connections" },
  { id: "traces", label: "Traces", icon: ScrollText, capability: "traces" },
];

const CLOUD_NAVIGATION: NavigationItem[] = [
  { id: "account", label: "Account", icon: CircleUserRound, capability: "account" },
  { id: "projects", label: "Projects", icon: UsersRound, capability: "teams" },
  { id: "billing", label: "Billing", icon: CreditCard, capability: "billing" },
];

const SECTION_TITLES: Record<DashboardSection, { eyebrow: string; title: string }> = {
  overview: { eyebrow: "Runtime", title: "Overview" },
  profiles: { eyebrow: "Agents", title: "Agent profiles" },
  bindings: { eyebrow: "Routing", title: "Bindings" },
  endpoints: { eyebrow: "Runtime", title: "Agent endpoints" },
  "api-keys": { eyebrow: "Access", title: "API keys" },
  connections: { eyebrow: "Connectors", title: "Connections" },
  traces: { eyebrow: "Observability", title: "Traces" },
  account: { eyebrow: "Vifu Cloud", title: "Account" },
  projects: { eyebrow: "Vifu Cloud", title: "Projects" },
  billing: { eyebrow: "Vifu Cloud", title: "Billing" },
};

export function RuntimeConsole({ section, data, browserApiBaseUrl }: {
  section: DashboardSection;
  data: DashboardData;
  browserApiBaseUrl: string;
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
        {capabilities.account ? <Navigation title="Cloud" items={CLOUD_NAVIGATION} active={activeSection} capabilities={capabilities} /> : null}
        <div className="sidebar-footer">
          <span>{data.authority.displayName}</span>
          <small>{modeLabel(data.authority.status.mode)}</small>
          {data.authority.kind === "cloud" ? <Link href="/auth/logout"><LogOut aria-hidden="true" />Sign out</Link> : null}
        </div>
      </aside>

      <section className="console-main">
        <header className="console-topbar">
          <div className="runtime-state"><span className="status-dot" />{data.authority.status.status}</div>
          <div className="topbar-meta"><span>v{data.authority.status.version}</span><span>{data.authority.status.connections} connected</span></div>
        </header>
        <header className="page-header">
          <p>{title.eyebrow}</p>
          <h1>{title.title}</h1>
        </header>
        <div className="console-content">
          <DashboardSectionView section={activeSection} data={data} browserApiBaseUrl={browserApiBaseUrl} />
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
        return <Link className={active === item.id ? "active" : ""} href={href} key={item.id}><Icon aria-hidden="true" />{item.label}</Link>;
      })}
    </nav>
  );
}

function DashboardSectionView({ section, data, browserApiBaseUrl }: {
  section: DashboardSection;
  data: DashboardData;
  browserApiBaseUrl: string;
}) {
  if (section === "profiles") return <ProfilesView profiles={data.runtime.profiles} />;
  if (section === "bindings") return <BindingsView profiles={data.runtime.profiles} bindings={data.runtime.bindings} connections={data.runtime.connections} />;
  if (section === "endpoints") return <EndpointsView profiles={data.runtime.profiles} bindings={data.runtime.bindings} endpoints={data.runtime.endpoints} browserApiBaseUrl={browserApiBaseUrl} />;
  if (section === "api-keys") return <ApiKeysView keys={data.runtime.apiKeys} endpoints={data.runtime.endpoints} />;
  if (section === "connections") return <ConnectionsView connections={data.runtime.connections} />;
  if (section === "traces") return <TracesView traces={data.runtime.traces} endpoints={data.runtime.endpoints} />;
  if (section === "account") return <CloudAccountView data={data} />;
  if (section === "projects") return <CloudProjectsView data={data} />;
  if (section === "billing") return <CloudBillingView data={data} />;
  return <OverviewView data={data} />;
}

function OverviewView({ data }: { data: DashboardData }) {
  const online = data.runtime.connections.filter((connection) => connection.status === "connected").length;
  const completed = data.runtime.traces.filter((trace) => trace.status === "completed").length;
  return (
    <>
      <section className="metric-strip" aria-label="Runtime totals">
        <Metric label="Profiles" value={data.runtime.profiles.length} icon={Boxes} />
        <Metric label="Endpoints" value={data.runtime.endpoints.length} icon={Route} />
        <Metric label="Connections" value={online} icon={Network} />
        <Metric label="Completed calls" value={completed} icon={Activity} />
      </section>
      <section className="content-section">
        <SectionHeading title="Deployment" action={<span className="mode-badge">{modeLabel(data.authority.status.mode)}</span>} />
        <dl className="definition-grid">
          <div><dt>Service</dt><dd>{data.authority.status.service}</dd></div>
          <div><dt>Authority</dt><dd>{data.authority.kind === "cloud" ? "Vifu account" : "Deployment admin"}</dd></div>
          <div><dt>WebSocket relay</dt><dd>{data.authority.status.capabilities.websocketRelay ? "Enabled" : "Unavailable"}</dd></div>
          <div><dt>Database</dt><dd>PostgreSQL</dd></div>
        </dl>
      </section>
      <section className="content-section">
        <SectionHeading title="Recent traces" action={<Link href="/dashboard/traces">View all</Link>} />
        <TraceTable traces={data.runtime.traces.slice(0, 8)} endpoints={data.runtime.endpoints} />
      </section>
    </>
  );
}

function ProfilesView({ profiles }: { profiles: AgentProfile[] }) {
  return (
    <>
      <section className="content-section create-section"><SectionHeading title="New profile" /><ProfileCreateForm /></section>
      <section className="content-section"><SectionHeading title="Profiles" count={profiles.length} />
        <ResourceList empty="No profiles yet.">{profiles.map((profile) => (
          <article className="resource-row" key={profile.id}>
            <ResourceIdentity title={profile.name} code={profile.slug} description={profile.description} />
            <div className="resource-meta"><span>{profile.instructions ? "Instructions set" : "No instructions"}</span><time>{formatDate(profile.updatedAt)}</time></div>
            <details className="row-editor"><summary>Edit</summary><ProfileEditForm profile={profile} /></details>
            <DeleteResourceButton path={`profiles/${profile.id}`} label={profile.name} />
          </article>
        ))}</ResourceList>
      </section>
    </>
  );
}

function BindingsView({ profiles, bindings, connections }: { profiles: AgentProfile[]; bindings: AgentBinding[]; connections: ConnectorSession[] }) {
  const profileNames = new Map(profiles.map((profile) => [profile.id, profile.name]));
  return (
    <>
      <section className="content-section create-section"><SectionHeading title="New OpenClaw binding" /><BindingCreateForm profiles={profiles} connections={connections} /></section>
      <section className="content-section"><SectionHeading title="Bindings" count={bindings.length} />
        <ResourceList empty="No bindings yet.">{bindings.map((binding) => (
          <article className="resource-row" key={binding.id}>
            <ResourceIdentity title={profileNames.get(binding.profileId) ?? "Unknown profile"} code={`${binding.connectorId} / ${binding.agentId}`} description={binding.provider} />
            <div className="resource-meta"><span>OpenClaw HTTP</span><time>{formatDate(binding.updatedAt)}</time></div>
            <details className="row-editor"><summary>Edit</summary><BindingEditForm binding={binding} /></details>
            <DeleteResourceButton path={`bindings/${binding.id}`} label={`${binding.connectorId} binding`} />
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

function ConnectionsView({ connections }: { connections: ConnectorSession[] }) {
  return (
    <section className="content-section"><SectionHeading title="Connector sessions" count={connections.length} />
      <ResourceList empty="No connector sessions yet.">{connections.map((connection) => (
        <article className="resource-row" key={connection.id}>
          <ResourceIdentity title={connection.connectorId} code={connection.sessionId} description={`${connection.agents.length} agents`} />
          <div className="resource-meta"><span className={connection.status === "connected" ? "status-label ready" : "status-label off"}>{connection.status}</span><time>{formatDate(connection.lastSeenAt)}</time></div>
        </article>
      ))}</ResourceList>
    </section>
  );
}

function TracesView({ traces, endpoints }: { traces: EndpointTrace[]; endpoints: AgentEndpoint[] }) {
  return <section className="content-section"><SectionHeading title="Endpoint traces" count={traces.length} /><TraceTable traces={traces} endpoints={endpoints} /></section>;
}

function TraceTable({ traces, endpoints }: { traces: EndpointTrace[]; endpoints: AgentEndpoint[] }) {
  const endpointNames = new Map(endpoints.map((endpoint) => [endpoint.id, endpoint.name]));
  if (traces.length === 0) return <EmptyState>No traces yet.</EmptyState>;
  return (
    <div className="table-scroll"><table className="runtime-table"><thead><tr><th>Status</th><th>Endpoint</th><th>Request</th><th>Latency</th><th>Created</th></tr></thead>
      <tbody>{traces.map((trace) => <tr key={trace.id}><td><span className={trace.status === "completed" ? "status-label ready" : trace.status === "pending" ? "status-label pending" : "status-label off"}>{trace.status}</span></td><td>{endpointNames.get(trace.endpointId) ?? shortId(trace.endpointId)}</td><td><code>{shortId(trace.requestId)}</code></td><td>{trace.latencyMs === null ? "-" : `${trace.latencyMs} ms`}</td><td>{formatDate(trace.createdAt)}</td></tr>)}</tbody>
    </table></div>
  );
}

function CloudAccountView({ data }: { data: DashboardData }) {
  const owner = data.cloud?.dashboard?.owner;
  return (
    <section className="content-section"><SectionHeading title="Vifu account" />
      {data.cloud?.error ? <InlineNotice tone="error">{data.cloud.error}</InlineNotice> : null}
      <dl className="definition-grid"><div><dt>Session</dt><dd>Active</dd></div><div><dt>Name</dt><dd>{readText(owner?.displayName) || data.authority.displayName}</dd></div><div><dt>Email</dt><dd>{readText(owner?.email) || "-"}</dd></div><div><dt>Projects</dt><dd>{data.cloud?.projects.length ?? 0}</dd></div></dl>
    </section>
  );
}

function CloudProjectsView({ data }: { data: DashboardData }) {
  const projects = data.cloud?.projects ?? [];
  return (
    <section className="content-section"><SectionHeading title="Cloud projects" count={projects.length} />
      <ResourceList empty="No cloud projects yet.">{projects.map((project, index) => {
        const name = readText(project.name) || `Project ${index + 1}`;
        const href = readText(project.dashboardPath);
        return <article className="resource-row" key={href || name}><ResourceIdentity title={name} code={readText(project.projectCloudSlug) || "managed"} description={readText(project.owner)} />{href ? <Link className="icon-text-button" href={href}>Open<ExternalLink aria-hidden="true" /></Link> : null}</article>;
      })}</ResourceList>
    </section>
  );
}

function CloudBillingView({ data }: { data: DashboardData }) {
  return (
    <section className="content-section"><SectionHeading title="Billing authority" />
      {data.cloud?.error ? <InlineNotice tone="error">{data.cloud.error}</InlineNotice> : null}
      <dl className="definition-grid"><div><dt>Authority</dt><dd>Vifu Cloud</dd></div><div><dt>Status</dt><dd>{data.cloud?.billing ? "Connected" : "Unavailable"}</dd></div></dl>
    </section>
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

function InlineNotice({ tone, children }: { tone: "error" | "info"; children: React.ReactNode }) {
  return <div className={`inline-notice ${tone}`} role={tone === "error" ? "alert" : "status"}>{children}</div>;
}

function isSectionAvailable(section: DashboardSection, capabilities: ServerCapabilities): boolean {
  const item = [...CORE_NAVIGATION, ...CLOUD_NAVIGATION].find((entry) => entry.id === section);
  return Boolean(item && (!item.capability || capabilities[item.capability]));
}

function modeLabel(mode: string): string {
  if (mode === "self-hosted") return "Self-hosted";
  if (mode === "cloud") return "Vifu Cloud";
  return "Local";
}

function formatDate(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.valueOf()) ? "-" : new Intl.DateTimeFormat("en", { dateStyle: "medium", timeStyle: "short" }).format(date);
}

function shortId(value: string): string {
  return value.length > 12 ? `${value.slice(0, 8)}...` : value;
}


function readText(value: unknown): string {
  return typeof value === "string" && value.trim() ? value.trim() : "";
}
