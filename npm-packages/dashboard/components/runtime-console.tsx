import Image from "next/image";
import Link from "next/link";
import type { LucideIcon } from "lucide-react";
import {
  Activity,
  ChevronDown,
  FolderKanban,
  Gamepad2,
  HeartPulse,
  KeyRound,
  LogOut,
  Plus,
  ScrollText,
  Search,
  Settings,
} from "lucide-react";
import type { DashboardData } from "../lib/dashboard-data";
import { authRequired } from "../lib/auth-providers";
import type {
  AgentBinding,
  AgentEndpoint,
  AgentGateway,
  AgentProfile,
  ApiKeyRecord,
  EndpointTrace,
  ProjectCanvas,
  ProjectCanvasNode,
  ProviderConnection,
  RuntimeProject,
  ServerCapabilities,
} from "../lib/runtime-types";
import { RuntimeTraceWorkbench } from "./runtime-trace-workbench";
import { AppLayout } from "./console-shell";
import { DismissibleDetails } from "./dismissible-details";
import {
  ApiKeyCreateForm,
  DeleteResourceButton,
  ProjectCreateForm,
  ProviderConnectionActions,
  ProviderConnectionForm,
  RevokeApiKeyButton,
} from "./runtime-actions";
import { RuntimeGameplayCanvas } from "./runtime-gameplay-canvas";

export type DashboardSection = "health" | "gameplay" | "api-keys" | "logs" | "settings";

type NavigationItem = {
  id: DashboardSection;
  label: string;
  icon: LucideIcon;
  capability?: keyof ServerCapabilities;
};

const PROJECT_NAVIGATION: NavigationItem[] = [
  { id: "health", label: "Health", icon: HeartPulse },
  { id: "gameplay", label: "Gameplay", icon: Gamepad2, capability: "canvas" },
  { id: "api-keys", label: "API Keys", icon: KeyRound, capability: "apiKeys" },
  { id: "logs", label: "Logs", icon: ScrollText, capability: "traces" },
  { id: "settings", label: "Settings", icon: Settings },
];

const SECTION_TITLES: Record<DashboardSection, string> = {
  health: "Health",
  gameplay: "Gameplay",
  "api-keys": "API Keys",
  logs: "Logs",
  settings: "Settings",
};

export function RuntimeConsole({
  section,
  projectSlug,
  data,
  browserApiBaseUrl,
}: {
  section: DashboardSection;
  projectSlug?: string;
  data: DashboardData;
  browserApiBaseUrl: string;
}) {
  const capabilities = data.authority.status.capabilities;
  const activeSection = isSectionAvailable(section, capabilities) ? section : "health";
  const selectedProject = selectProject(data.runtime.projects, projectSlug, data.canvas);
  const title = SECTION_TITLES[activeSection];
  return (
    <AppLayout
      sidebar={(
        <>
          <Link className="console-brand" href="/project" aria-label="Vifu Dashboard">
            <Image src="/brand/vifu-icon-512.png" width={32} height={32} alt="" priority />
            <span>Vifu</span>
          </Link>
          {selectedProject ? (
            <Navigation project={selectedProject} items={PROJECT_NAVIGATION} active={activeSection} capabilities={capabilities} />
          ) : null}
          {authRequired(data.authority.status.auth) ? (
            <div className="sidebar-footer">
              <form action="/auth/logout" method="post">
                <button className="sidebar-signout" type="submit"><LogOut aria-hidden="true" /><span>Sign out</span></button>
              </form>
            </div>
          ) : null}
        </>
      )}
      header={(
        <>
          <ProjectBreadcrumb
            projects={data.runtime.projects}
            selectedProject={selectedProject}
            activeSection={activeSection}
            availableAgents={data.runtime.availableAgents}
            agentGateways={data.runtime.agentGateways}
          />
          <div className="app-header-meta">
            <div className="runtime-state"><span className="status-dot" />{runtimeStatusLabel(data.authority.status.status)}</div>
            <div className="topbar-meta"><span>v{data.authority.status.version}</span><span>{gatewayCountLabel(data.authority.status.agentGateways)}</span></div>
          </div>
        </>
      )}
    >
      {selectedProject ? (
        <>
          <header className="page-header project-page-header">
            <h1>{title}</h1>
          </header>
          <div className={`console-content ${activeSection}-content`}>
            <ProjectSectionView
              section={activeSection}
              project={selectedProject}
              data={data}
              browserApiBaseUrl={browserApiBaseUrl}
            />
          </div>
        </>
      ) : (
        <NoProjectView data={data} />
      )}
    </AppLayout>
  );
}

function ProjectBreadcrumb({
  projects,
  selectedProject,
  activeSection,
  availableAgents,
  agentGateways,
}: {
  projects: RuntimeProject[];
  selectedProject: RuntimeProject | null;
  activeSection: DashboardSection;
  availableAgents: DashboardData["runtime"]["availableAgents"];
  agentGateways: AgentGateway[];
}) {
  return (
    <nav className="project-breadcrumb" aria-label="Project">
      <DismissibleDetails className="project-switcher">
        <summary>
          <span className="project-avatar"><FolderKanban aria-hidden="true" /></span>
          <strong>{selectedProject?.name ?? "Create project"}</strong>
          <ChevronDown aria-hidden="true" />
        </summary>
        <div className="project-menu">
          <label className="project-search">
            <Search aria-hidden="true" />
            <input type="search" placeholder="Search projects..." />
          </label>
          <span>Projects</span>
          <div className="project-menu-list">
            {projects.map((project) => (
              <Link key={project.id} href={`/project/${project.slug}/${activeSection}`} prefetch={false} title={project.name}>
                <strong>{project.name}</strong>
              </Link>
            ))}
          </div>
          <section className="project-create-panel">
            <div className="project-create-header"><Plus aria-hidden="true" /><span>Create project</span></div>
            <ProjectCreateForm availableAgents={availableAgents} agentGateways={agentGateways} variant="menu" />
          </section>
        </div>
      </DismissibleDetails>
    </nav>
  );
}

function Navigation({ project, items, active, capabilities }: {
  project: RuntimeProject;
  items: NavigationItem[];
  active: DashboardSection;
  capabilities: ServerCapabilities;
}) {
  const visible = items.filter((item) => !item.capability || capabilities[item.capability]);
  return (
    <nav className="console-nav" aria-label="Project navigation">
      {visible.map((item) => {
        const Icon = item.icon;
        const href = `/project/${project.slug}/${item.id}`;
        return (
          <Link className={active === item.id ? "active" : ""} href={href} key={item.id} prefetch={false} title={item.label}>
            <Icon aria-hidden="true" />
            <span>{item.label}</span>
          </Link>
        );
      })}
    </nav>
  );
}

function ProjectSectionView({
  section,
  project,
  data,
  browserApiBaseUrl,
}: {
  section: DashboardSection;
  project: RuntimeProject;
  data: DashboardData;
  browserApiBaseUrl: string;
}) {
  const endpoints = projectEndpoints(project, data.runtime.endpoints);
  if (section === "gameplay") {
    return (
      <RuntimeGameplayCanvas
        project={project}
        canvas={data.canvas}
        profiles={data.runtime.profiles}
        bindings={data.runtime.bindings}
        agentGateways={data.runtime.agentGateways}
        availableAgents={data.runtime.availableAgents}
        endpoints={endpoints}
        traces={projectTraces(data.runtime.traces, project)}
        browserApiBaseUrl={browserApiBaseUrl}
      />
    );
  }
  if (section === "api-keys") return <ApiKeysView project={project} keys={data.runtime.apiKeys} endpoints={endpoints} />;
  if (section === "logs") return <LogsView project={project} traces={projectTraces(data.runtime.traces, project)} />;
  if (section === "settings") {
    return (
      <SettingsView
        project={project}
        endpoints={endpoints}
        browserApiBaseUrl={browserApiBaseUrl}
        providerAdapters={data.runtime.providerAdapters}
        providerConnections={data.providerConnections}
      />
    );
  }
  return <HealthView project={project} canvas={data.canvas} data={data} endpoints={endpoints} browserApiBaseUrl={browserApiBaseUrl} />;
}

function HealthView({
  project,
  canvas,
  data,
  endpoints,
  browserApiBaseUrl,
}: {
  project: RuntimeProject;
  canvas?: ProjectCanvas;
  data: DashboardData;
  endpoints: AgentEndpoint[];
  browserApiBaseUrl: string;
}) {
  const nodes = projectCanvasNodes(project, canvas);
  const primaryEndpoint = endpoints[0];
  const gateways = new Map(data.runtime.agentGateways.map((gateway) => [gateway.gatewayId, gateway]));
  const exposed = nodes.filter((node) => node.exposed).length;
  const connected = nodes.filter((node) => node.gatewayId && gateways.get(node.gatewayId)?.status === "connected").length;
  const failures = projectTraces(data.runtime.traces, project).filter((trace) => trace.status !== "completed" && trace.status !== "pending");
  const connectedGateways = data.runtime.agentGateways.filter((gateway) => gateway.status === "connected").length;
  return (
    <>
      <section className="health-summary">
        <div className="summary-card endpoint-summary">
          <div>
            <span className="status-label ready">Endpoint</span>
            <strong>Endpoint invoke API</strong>
            {primaryEndpoint ? (
              <code>{endpointInvokeUrl(primaryEndpoint, browserApiBaseUrl)}</code>
            ) : (
              <code>No endpoint yet</code>
            )}
          </div>
          <dl>
            <div><dt>Agents</dt><dd>{exposed}</dd></div>
            <div><dt>Connected</dt><dd>{connected}</dd></div>
            <div><dt>Failures</dt><dd>{failures.length}</dd></div>
          </dl>
        </div>
        <Metric label="Canvas nodes" value={nodes.length} icon={Gamepad2} />
        <Metric label="Gateways" value={connectedGateways} icon={Activity} />
        <Metric label="Recent logs" value={projectTraces(data.runtime.traces, project).length} icon={ScrollText} />
      </section>
      {nodes.length === 0 || connectedGateways === 0 ? (
        <SetupRail
          project={project}
          providerCount={data.providerConnections.length}
          agentCount={data.runtime.availableAgents.length}
          exposedCount={exposed}
          connectedGatewayCount={connectedGateways}
        />
      ) : null}
      <section className="content-section">
        <SectionHeading title="Agents exposed by this project" count={nodes.length} action={<Link href={`/project/${project.slug}/gameplay`}>Open Gameplay</Link>} />
        <ResourceList empty="No agents on this project canvas yet.">
          {nodes.map((node) => {
            const title = nodeTitle(node, data.runtime.profiles);
            const resourceId = node.resourceId ?? node.id;
            return (
              <article className="resource-row health-agent-row" key={node.id}>
                <ResourceIdentity title={title} code={resourceId === title ? undefined : resourceId} description={node.gatewayId ? gatewayDisplayLabel(node.gatewayId) : "unbound"} />
                <div className="resource-meta">
                  <span className={node.exposed ? "status-label ready" : "status-label pending"}>{node.exposed ? "Exposed" : "Hidden"}</span>
                  <span className={node.gatewayId && gateways.get(node.gatewayId)?.status === "connected" ? "status-label ready" : "status-label off"}>
                    {node.gatewayId && gateways.get(node.gatewayId)?.status === "connected" ? "Connected" : "Offline"}
                  </span>
                </div>
              </article>
            );
          })}
        </ResourceList>
      </section>
      <section className="content-section">
        <SectionHeading title="Latest logs" action={<Link href={`/project/${project.slug}/logs`}>View logs</Link>} />
        <TraceTable traces={projectTraces(data.runtime.traces, project).slice(0, 8)} project={project} />
      </section>
    </>
  );
}

function ApiKeysView({ project, keys, endpoints }: { project: RuntimeProject; keys: ApiKeyRecord[]; endpoints: AgentEndpoint[] }) {
  const endpointIds = new Set(endpoints.map((endpoint) => endpoint.id));
  const scopedKeys = keys.filter((key) => endpointIds.has(key.endpointId));
  return (
    <>
      <section className="content-section">
        <SectionHeading title="Endpoint access" />
        <div className="definition-grid compact-definition-grid">
          <div><dt>Project</dt><dd>{project.name}</dd></div>
          <div><dt>Endpoints</dt><dd>{endpoints.length}</dd></div>
          <div><dt>Status</dt><dd>{project.enabled ? "Enabled" : "Disabled"}</dd></div>
        </div>
      </section>
      {endpoints.length > 0 ? (
        <section className="content-section create-section"><SectionHeading title="New endpoint API key" /><ApiKeyCreateForm endpoints={endpoints} /></section>
      ) : null}
      <section className="content-section"><SectionHeading title="Endpoint API keys" count={scopedKeys.length} />
        <ResourceList empty="No endpoint API keys for this project.">{scopedKeys.map((key) => (
          <article className="resource-row" key={key.id}>
            <ResourceIdentity title={key.name} code={`${key.keyPrefix}...`} description={endpoints.find((endpoint) => endpoint.id === key.endpointId)?.name ?? "Unknown endpoint"} />
            <div className="resource-meta"><span className={key.revokedAt ? "status-label off" : "status-label ready"}>{key.revokedAt ? "Revoked" : "Active"}</span><time>{formatDate(key.createdAt)}</time></div>
            {!key.revokedAt ? <RevokeApiKeyButton id={key.id} name={key.name} /> : null}
          </article>
        ))}</ResourceList>
      </section>
    </>
  );
}

function LogsView({ project, traces }: { project: RuntimeProject; traces: EndpointTrace[] }) {
  return (
    <section className="content-section trace-section convex-trace-section">
      <RuntimeTraceWorkbench projectId={project.id} traces={traces} />
    </section>
  );
}

function SettingsView({
  project,
  endpoints,
  browserApiBaseUrl,
  providerAdapters,
  providerConnections,
}: {
  project: RuntimeProject;
  endpoints: AgentEndpoint[];
  browserApiBaseUrl: string;
  providerAdapters: DashboardData["runtime"]["providerAdapters"];
  providerConnections: ProviderConnection[];
}) {
  const primaryEndpoint = endpoints[0];
  return (
    <>
      <section className="content-section">
        <SectionHeading title="Project settings" />
        <dl className="definition-grid">
          <div><dt>Name</dt><dd>{project.name}</dd></div>
          <div><dt>Slug</dt><dd>{project.slug}</dd></div>
          <div><dt>Invoke endpoint</dt><dd>{primaryEndpoint ? <code>{endpointInvokeUrl(primaryEndpoint, browserApiBaseUrl)}</code> : "No endpoint yet"}</dd></div>
          <div><dt>Status</dt><dd>{project.enabled ? "Enabled" : "Disabled"}</dd></div>
        </dl>
      </section>
      <section className="content-section create-section">
        <SectionHeading title="Provider settings" />
        <ProviderConnectionForm project={project} adapters={providerAdapters} />
      </section>
      <section className="content-section">
        <SectionHeading title="Configured providers" count={providerConnections.length} />
        <ResourceList empty="No providers configured for this project.">
          {providerConnections.map((connection) => (
            <article className="resource-row" key={connection.id}>
              <ResourceIdentity title={connection.name} code={connection.providerKey} description={connection.baseUrl} />
              <div className="resource-meta">
                <span className={connection.status === "online" ? "status-label ready" : connection.status === "configured" ? "status-label pending" : "status-label off"}>{connection.status}</span>
                {connection.secretKeys.length > 0 ? <span>{connection.secretKeys.join(", ")}</span> : null}
                {connection.displaySecret ? <code>{connection.displaySecret}</code> : null}
              </div>
              <ProviderConnectionActions project={project} connection={connection} />
            </article>
          ))}
        </ResourceList>
      </section>
      <section className="content-section danger-section">
        <SectionHeading title="Danger zone" />
        <div className="settings-danger-row">
          <div>
            <strong>Delete project</strong>
            <p>Remove this project from the dashboard. Detected gateway agents are not deleted.</p>
          </div>
          <DeleteResourceButton path={`projects/${project.id}`} label={project.name} />
        </div>
      </section>
    </>
  );
}

function NoProjectView({ data }: { data: DashboardData }) {
  return (
    <div className="console-content no-project-content">
      <div className="first-run-layout">
        <FirstRunRail />
        <section className="content-section create-section">
          <SectionHeading title="Create your first project" />
          <ProjectCreateForm availableAgents={data.runtime.availableAgents} agentGateways={data.runtime.agentGateways} />
        </section>
      </div>
    </div>
  );
}

function FirstRunRail() {
  return (
    <aside className="setup-rail first-run-rail" aria-label="Setup path">
      <span>Setup path</span>
      <ol>
        <li className="active"><strong>Project</strong><small>Name the game or app you are building.</small></li>
        <li><strong>Provider</strong><small>Connect OpenClaw or another agent provider.</small></li>
        <li><strong>Agents</strong><small>Discover agents and place them on Gameplay.</small></li>
        <li><strong>Endpoint</strong><small>Call this project from your game over HTTP or WebSocket.</small></li>
      </ol>
    </aside>
  );
}

function SetupRail({ project, providerCount, agentCount, exposedCount, connectedGatewayCount }: {
  project: RuntimeProject;
  providerCount: number;
  agentCount: number;
  exposedCount: number;
  connectedGatewayCount: number;
}) {
  const providerReady = providerCount > 0;
  const agentsReady = agentCount > 0;
  const endpointReady = exposedCount > 0;
  const gatewayReady = connectedGatewayCount > 0;
  const nextStep = !providerReady
    ? "Connect an agent provider"
    : !agentsReady
      ? "Discover provider agents"
      : !endpointReady
        ? "Expose an agent endpoint"
        : !gatewayReady
          ? "Reconnect agent gateway"
          : "Project is ready to call";
  return (
    <section className="setup-rail project-setup-rail" aria-label="Project setup">
      <div>
        <span>Next setup step</span>
        <strong>{nextStep}</strong>
      </div>
      <ol>
        <li className="ready"><strong>Project</strong><small>{project.slug}</small></li>
        <li className={providerReady ? "ready" : "active"}><strong>Provider</strong><small>{providerReady ? `${providerCount} configured` : "Add one in Settings"}</small></li>
        <li className={agentsReady ? "ready" : providerReady ? "active" : undefined}><strong>Agents</strong><small>{agentsReady ? `${agentCount} detected` : "Discover agents"}</small></li>
        <li className={endpointReady ? "ready" : agentsReady ? "active" : undefined}><strong>Endpoint</strong><small>{endpointReady ? `${exposedCount} exposed` : "Add agents to Gameplay"}</small></li>
        <li className={gatewayReady ? "ready" : endpointReady ? "active" : undefined}><strong>Gateway</strong><small>{gatewayReady ? `${connectedGatewayCount} online` : "Start gateway"}</small></li>
      </ol>
      <div className="setup-actions">
        {!providerReady ? <Link className="primary-button" href={`/project/${project.slug}/settings`}>Connect provider</Link> : null}
        {providerReady && !endpointReady ? <Link className="secondary-button" href={`/project/${project.slug}/gameplay`}>Open Gameplay</Link> : null}
      </div>
    </section>
  );
}

function TraceTable({ traces, project: _project, detailed = false }: { traces: EndpointTrace[]; project: RuntimeProject; detailed?: boolean }) {
  if (traces.length === 0) return <EmptyState>No logs yet.</EmptyState>;
  if (detailed) {
    return <RuntimeTraceWorkbench projectId={_project.id} traces={traces} />;
  }
  return (
    <div className="trace-compact-list">
      {traces.map((trace) => (
        <article className="trace-compact-row" key={trace.id}>
          <span className={trace.status === "completed" ? "status-label ready" : trace.status === "pending" ? "status-label pending" : "status-label off"}>{trace.status}</span>
          <strong>{shortId(trace.requestId)}</strong>
          <span>{trace.latencyMs === null ? "-" : `${trace.latencyMs} ms`}</span>
          <time>{formatDate(trace.createdAt)}</time>
        </article>
      ))}
    </div>
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

function ResourceIdentity({ title, code, description }: { title: string; code?: string; description?: string | null }) {
  return <div className="resource-identity"><strong>{title}</strong>{code ? <code>{code}</code> : null}{description ? <span>{description}</span> : null}</div>;
}

function EmptyState({ children }: { children: React.ReactNode }) {
  return <div className="empty-state">{children}</div>;
}

function selectProject(projects: RuntimeProject[], projectSlug: string | undefined, canvas: ProjectCanvas | undefined): RuntimeProject | null {
  if (canvas?.project) return canvas.project;
  return projects.find((project) => project.slug === projectSlug) ?? projects[0] ?? null;
}

function isSectionAvailable(section: DashboardSection, capabilities: ServerCapabilities): boolean {
  const item = PROJECT_NAVIGATION.find((entry) => entry.id === section);
  return Boolean(item && (!item.capability || capabilities[item.capability]));
}

function projectCanvasNodes(project: RuntimeProject, canvas?: ProjectCanvas): ProjectCanvasNode[] {
  if (canvas?.project.id === project.id) return canvas.nodes;
  return [];
}

function projectEndpoints(project: RuntimeProject, endpoints: AgentEndpoint[]): AgentEndpoint[] {
  const bindingIds = new Set(project.bindingIds);
  return endpoints.filter((endpoint) => bindingIds.has(endpoint.bindingId));
}

function projectTraces(traces: EndpointTrace[], project: RuntimeProject): EndpointTrace[] {
  return traces.filter((trace) => trace.projectId === project.id);
}

function nodeTitle(node: ProjectCanvasNode, profiles: AgentProfile[]): string {
  const profileName = node.profileId ? profiles.find((profile) => profile.id === node.profileId)?.name : null;
  const resourceName = String(node.config.agentName ?? node.resourceId ?? "Agent").trim();
  if (profileName && profileName !== "Agent") return profileName;
  return resourceName || "Agent";
}

function runtimeStatusLabel(status: string): string {
  return `Runtime ${status.toLowerCase()}`;
}

function gatewayCountLabel(count: number): string {
  return `${count} ${count === 1 ? "gateway" : "gateways"} online`;
}

function endpointInvokeUrl(endpoint: AgentEndpoint, browserApiBaseUrl: string): string {
  const url = new URL(browserApiBaseUrl);
  url.pathname = `/v1/endpoints/${encodeURIComponent(endpoint.slug || endpoint.id)}/invoke`;
  url.search = "";
  return url.toString().replace(/\/$/, "");
}

function formatDate(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.valueOf())
    ? "-"
    : new Intl.DateTimeFormat("en", {
      day: "numeric",
      hour: "numeric",
      minute: "2-digit",
      month: "short",
      timeZone: "UTC",
      timeZoneName: "short",
      year: "numeric",
    }).format(date);
}

function shortId(value: string): string {
  return value.length > 12 ? `${value.slice(0, 8)}...` : value;
}

function gatewayDisplayLabel(value: string): string {
  return value.startsWith("gateway-") ? `Gateway ${shortId(value.replace(/^gateway-/, ""))}` : shortId(value);
}
