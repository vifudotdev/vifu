import Image from "next/image";
import Link from "next/link";
import type { LucideIcon } from "lucide-react";
import {
  ChevronDown,
  Gamepad2,
  HeartPulse,
  KeyRound,
  LogOut,
  ScrollText,
  Settings,
} from "lucide-react";
import type { DashboardData } from "../lib/dashboard-data";
import { authRequired } from "../lib/auth-providers";
import type {
  AgentBinding,
  AgentEndpoint,
  AgentGateway,
  AgentProfile,
  AvailableAgent,
  EndpointTrace,
  ProjectCanvas,
  ProjectCanvasNode,
  ProviderConnection,
  RuntimeProject,
  ServerCapabilities,
} from "../lib/runtime-types";
import { RuntimeTraceWorkbench } from "./runtime-trace-workbench";
import { ApiIntegrationsView } from "./runtime-api-integrations";
import { AppLayout } from "./console-shell";
import { ProjectSwitcher } from "./project-switcher";
import {
  DeleteResourceButton,
  ProjectCreateForm,
  ProviderConnectionActions,
  ProviderConnectionForm,
} from "./runtime-actions";
import { RuntimeGameplayCanvas } from "./runtime-gameplay-canvas";

export type DashboardSection = "health" | "gameplay" | "api" | "logs" | "settings";

type NavigationItem = {
  id: DashboardSection;
  label: string;
  icon: LucideIcon;
  capability?: keyof ServerCapabilities;
};

const PROJECT_NAVIGATION: NavigationItem[] = [
  { id: "health", label: "Health", icon: HeartPulse },
  { id: "gameplay", label: "Gameplay", icon: Gamepad2, capability: "canvas" },
  { id: "api", label: "API", icon: KeyRound, capability: "apiKeys" },
  { id: "logs", label: "Logs", icon: ScrollText, capability: "traces" },
  { id: "settings", label: "Settings", icon: Settings },
];

const SECTION_TITLES: Record<DashboardSection, string> = {
  health: "Health",
  gameplay: "Gameplay",
  api: "API Integrations",
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
          <ProjectSwitcher
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
  if (section === "api") {
    return (
      <ApiIntegrationsView
        project={project}
        projects={data.runtime.projects}
        keys={data.runtime.apiKeys}
        endpoints={data.runtime.endpoints}
        bindings={data.runtime.bindings}
        canvas={data.canvas}
        browserApiBaseUrl={browserApiBaseUrl}
      />
    );
  }
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
  const exposed = nodes.filter((node) => node.exposed).length;
  const gatewayCards = gatewayHealthCards(data.runtime.agentGateways, data.runtime.availableAgents);
  const connectedGatewayCards = gatewayCards.filter((item) => item.gateway.status === "connected");
  const connectedGateways = connectedGatewayCards.length;
  const connectedAgentKeys = new Set(
    data.runtime.availableAgents
      .filter((agent) => agent.status === "connected")
      .map((agent) => `${agent.gatewayId}/${agent.id}`),
  );
  const connected = nodes.filter((node) => (
    node.exposed
    && node.gatewayId
    && node.resourceId
    && connectedAgentKeys.has(`${node.gatewayId}/${node.resourceId}`)
  )).length;
  const traces = projectTraces(data.runtime.traces, project);
  const traceSummary = summarizeTraces(traces);
  return (
    <div className="health-dashboard">
      <HealthSection title="Summary" defaultOpen>
        <dl className="health-summary-card">
          <div className="wide">
            <dt>HTTP URL</dt>
            <dd><code>{projectChatCompletionsUrl(project, browserApiBaseUrl)}</code></dd>
          </div>
          <div className="wide">
            <dt>WS URL</dt>
            <dd><span className="health-unavailable">Not available</span></dd>
          </div>
          <div>
            <dt>Gateways</dt>
            <dd>{connectedGateways}</dd>
          </div>
          <div>
            <dt>Agents</dt>
            <dd>{connected}/{exposed}</dd>
          </div>
          <div>
            <dt>Requests</dt>
            <dd>{traces.length}</dd>
          </div>
        </dl>
      </HealthSection>
      {nodes.length === 0 || connectedGateways === 0 ? (
        <SetupRail
          project={project}
          providerCount={data.providerConnections.length}
          agentCount={connectedAgentKeys.size}
          exposedCount={exposed}
          connectedGatewayCount={connectedGateways}
        />
      ) : null}
      <HealthSection title="Traces" count={`${traces.length} recent`} defaultOpen>
        <div className="trace-widget-grid">
          <TraceMetricCard title="Requests" value={String(traces.length)} meta="Recent project traces">
            <MiniBarChart bars={traceSummary.requestBuckets} />
          </TraceMetricCard>
          <TraceMetricCard title="Failure rate" value={`${formatPercent(traceSummary.failureRate)}%`} meta={`${traceSummary.failures} failed / ${traces.length} total`}>
            <StatusStack completed={traceSummary.completed} failed={traceSummary.failures} pending={traceSummary.pending} />
          </TraceMetricCard>
          <TraceMetricCard title="Latency" value={traceSummary.p95LatencyMs === null ? "-" : `${traceSummary.p95LatencyMs} ms`} meta={traceSummary.avgLatencyMs === null ? "No completed latency data" : `avg ${traceSummary.avgLatencyMs} ms`}>
            <MiniBarChart bars={traceSummary.latencyBuckets} />
          </TraceMetricCard>
          <TraceMetricCard title="Recent errors" value={String(traceSummary.failures)} meta="Latest failed traces">
            <ErrorTraceList traces={traceSummary.errorTraces} />
          </TraceMetricCard>
        </div>
      </HealthSection>
      <HealthSection title="Gateways" count={`${connectedGateways} connected`} defaultOpen>
        {connectedGatewayCards.length > 0 ? (
          <div className="gateway-health-grid">
            {connectedGatewayCards.map(({ gateway, agents }) => (
              <GatewayHealthCard gateway={gateway} agents={agents} key={gateway.gatewayId} />
            ))}
          </div>
        ) : (
          <EmptyState>No gateways connected.</EmptyState>
        )}
      </HealthSection>
    </div>
  );
}

function HealthSection({
  children,
  count,
  defaultOpen,
  title,
}: {
  children: React.ReactNode;
  count?: string;
  defaultOpen?: boolean;
  title: string;
}) {
  return (
    <details className="health-section" open={defaultOpen}>
      <summary>
        <ChevronDown aria-hidden="true" />
        <strong>{title}</strong>
        {count ? <span>{count}</span> : null}
      </summary>
      <div className="health-section-body">{children}</div>
    </details>
  );
}

function GatewayHealthCard({ gateway, agents }: { gateway: AgentGateway; agents: AvailableAgent[] }) {
  const fallbackAgents = agents.length > 0
    ? agents
    : gateway.agents.map((agent, index) => ({
      gatewayId: gateway.gatewayId,
      id: agent.id ?? `agent-${index + 1}`,
      name: agent.name ?? agent.id ?? `Agent ${index + 1}`,
      status: gateway.status,
      metadata: {},
    }));
  return (
    <article className="gateway-health-card">
      <header>
        <div>
          <strong>{gatewayDisplayLabel(gateway.gatewayId)}</strong>
          <code>{shortId(gateway.sessionId, 12)}</code>
        </div>
        <span className={statusClassName(gateway.status)}>{gateway.status}</span>
      </header>
      <dl>
        <div><dt>Agents</dt><dd>{fallbackAgents.length}</dd></div>
        <div><dt>Last seen</dt><dd>{formatDate(gateway.lastSeenAt)}</dd></div>
      </dl>
      <div className="gateway-agent-list">
        {fallbackAgents.length > 0 ? fallbackAgents.map((agent) => (
          <div key={`${agent.gatewayId}:${agent.id}`}>
            <span>{agent.name || agent.id}</span>
            <small className={statusClassName(agent.status)}>{agent.status}</small>
          </div>
        )) : <EmptyState>No agents reported.</EmptyState>}
      </div>
    </article>
  );
}

function TraceMetricCard({ children, meta, title, value }: { children: React.ReactNode; meta: string; title: string; value: string }) {
  return (
    <article className="trace-widget-card">
      <header>
        <span>{title}</span>
        <strong>{value}</strong>
        <small>{meta}</small>
      </header>
      {children}
    </article>
  );
}

function MiniBarChart({ bars }: { bars: number[] }) {
  const max = Math.max(...bars, 1);
  const hasData = bars.some((value) => value > 0);
  return (
    <div className={`mini-bar-chart${hasData ? "" : " empty"}`} aria-hidden="true">
      {hasData ? bars.map((value, index) => (
        <i key={index} style={{ height: `${Math.max(4, (value / max) * 100)}%` }} />
      )) : null}
    </div>
  );
}

function StatusStack({ completed, failed, pending }: { completed: number; failed: number; pending: number }) {
  const total = Math.max(completed + failed + pending, 1);
  return (
    <div className="status-stack" aria-hidden="true">
      <i className="ready" style={{ width: `${(completed / total) * 100}%` }} />
      <i className="pending" style={{ width: `${(pending / total) * 100}%` }} />
      <i className="failed" style={{ width: `${(failed / total) * 100}%` }} />
    </div>
  );
}

function ErrorTraceList({ traces }: { traces: EndpointTrace[] }) {
  if (traces.length === 0) return <div className="trace-widget-empty">No recent errors.</div>;
  return (
    <div className="error-trace-list">
      {traces.map((trace) => (
        <div key={trace.id}>
          <code>{shortId(trace.requestId, 8)}</code>
          <span>{trace.error ?? trace.status}</span>
        </div>
      ))}
    </div>
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
          <div><dt>Chat completions</dt><dd>{primaryEndpoint ? <code>{projectChatCompletionsUrl(project, browserApiBaseUrl)}</code> : "No endpoint yet"}</dd></div>
          <div><dt>Model</dt><dd>{primaryEndpoint ? <code>{primaryEndpoint.slug}</code> : "No endpoint yet"}</dd></div>
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

function gatewayStatusMap(gateways: AgentGateway[]): Map<string, AgentGateway> {
  const byId = new Map<string, AgentGateway>();
  for (const gateway of gateways) {
    const current = byId.get(gateway.gatewayId);
    if (!current || (current.status !== "connected" && gateway.status === "connected")) {
      byId.set(gateway.gatewayId, gateway);
    }
  }
  return byId;
}

function gatewayHealthCards(gateways: AgentGateway[], agents: AvailableAgent[]): Array<{ gateway: AgentGateway; agents: AvailableAgent[] }> {
  return Array.from(gatewayStatusMap(gateways).values())
    .sort((a, b) => gatewayStatusRank(a.status) - gatewayStatusRank(b.status) || a.gatewayId.localeCompare(b.gatewayId))
    .map((gateway) => ({
      gateway,
      agents: agents
        .filter((agent) => agent.gatewayId === gateway.gatewayId)
        .sort((a, b) => a.name.localeCompare(b.name)),
    }));
}

function gatewayStatusRank(status: string): number {
  return status === "connected" ? 0 : status === "pending" ? 1 : 2;
}

function projectEndpoints(project: RuntimeProject, endpoints: AgentEndpoint[]): AgentEndpoint[] {
  const bindingIds = new Set(project.bindingIds);
  return endpoints.filter((endpoint) => bindingIds.has(endpoint.bindingId));
}

function projectTraces(traces: EndpointTrace[], project: RuntimeProject): EndpointTrace[] {
  return traces.filter((trace) => trace.projectId === project.id);
}

function summarizeTraces(traces: EndpointTrace[]) {
  const completed = traces.filter((trace) => trace.status === "completed").length;
  const pending = traces.filter((trace) => trace.status === "pending").length;
  const failures = traces.length - completed - pending;
  const latencies = traces
    .map((trace) => trace.latencyMs)
    .filter((value): value is number => typeof value === "number")
    .sort((a, b) => a - b);
  return {
    avgLatencyMs: latencies.length === 0 ? null : Math.round(latencies.reduce((sum, value) => sum + value, 0) / latencies.length),
    completed,
    errorTraces: traces
      .filter((trace) => trace.status !== "completed" && trace.status !== "pending")
      .sort((a, b) => Date.parse(b.createdAt) - Date.parse(a.createdAt))
      .slice(0, 4),
    failureRate: traces.length === 0 ? 0 : (failures / traces.length) * 100,
    failures,
    latencyBuckets: valueBuckets(latencies, 12),
    p95LatencyMs: percentile(latencies, 0.95),
    pending,
    requestBuckets: timeBuckets(traces, 12),
  };
}

function timeBuckets(traces: EndpointTrace[], count: number): number[] {
  const buckets = Array.from({ length: count }, () => 0);
  const times = traces
    .map((trace) => Date.parse(trace.createdAt))
    .filter((value) => Number.isFinite(value));
  if (times.length === 0) return buckets;
  const min = Math.min(...times);
  const max = Math.max(...times);
  const span = Math.max(max - min, 60_000);
  for (const time of times) {
    const index = Math.min(count - 1, Math.floor(((time - min) / span) * count));
    buckets[index] += 1;
  }
  return buckets;
}

function valueBuckets(values: number[], count: number): number[] {
  const buckets = Array.from({ length: count }, () => 0);
  if (values.length === 0) return buckets;
  const max = Math.max(...values, 1);
  for (const value of values) {
    const index = Math.min(count - 1, Math.floor((value / max) * count));
    buckets[index] += 1;
  }
  return buckets;
}

function percentile(values: number[], ratio: number): number | null {
  if (values.length === 0) return null;
  const index = Math.min(values.length - 1, Math.max(0, Math.ceil(values.length * ratio) - 1));
  return Math.round(values[index] ?? 0);
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

function projectChatCompletionsUrl(project: RuntimeProject, browserApiBaseUrl: string): string {
  return `${projectApiBaseUrl(project, browserApiBaseUrl)}/chat/completions`;
}

function projectApiBaseUrl(project: RuntimeProject, browserApiBaseUrl: string): string {
  const url = new URL(browserApiBaseUrl);
  const basePath = url.pathname.replace(/\/+$/, "");
  url.pathname = `${basePath}/${encodeURIComponent(project.slug)}/v1`;
  url.search = "";
  return url.toString().replace(/\/$/, "");
}

function statusClassName(status: string): string {
  const normalized = status.toLowerCase();
  if (normalized === "connected" || normalized === "online" || normalized === "completed" || normalized === "ready") return "status-label ready";
  if (normalized === "pending" || normalized === "connecting") return "status-label pending";
  return "status-label off";
}

function formatPercent(value: number): string {
  return value % 1 === 0 ? String(value) : value.toFixed(1);
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

function shortId(value: string, length = 12): string {
  return value.length > length ? `${value.slice(0, Math.max(4, length - 4))}...` : value;
}

function gatewayDisplayLabel(value: string): string {
  return value.startsWith("gateway-") ? `Gateway ${shortId(value.replace(/^gateway-/, ""))}` : shortId(value);
}
