"use client";

import type { LucideIcon } from "lucide-react";
import {
  Bot,
  ChevronDown,
  KeyRound,
  LayoutDashboard,
  LogOut,
  Network,
  Plug,
  ScrollText,
  Settings,
} from "lucide-react";
import type { DashboardData } from "../data";
import type {
  AgentBinding,
  AgentEndpoint,
  AgentGateway,
  AvailableAgent,
  EndpointTrace,
  RuntimeProject,
  ServerCapabilities,
} from "../types";
import { chatCompletionsUrl } from "../inference-url";
import { RuntimeTraceWorkbench } from "./runtime-trace-workbench";
import { ApiIntegrationsView } from "./runtime-api-integrations";
import { AppLayout } from "./console-shell";
import { ProjectHome } from "./project-home";
import { ProjectSwitcher } from "./project-switcher";
import {
  DeleteResourceButton,
} from "./runtime-actions";
import { RuntimeAgentsView } from "./runtime-agents";
import { RuntimeProvidersView } from "./runtime-providers";
import { RuntimeDeploymentsView } from "./runtime-deployments";
import { RuntimeImage, RuntimeLink, useRuntimeConsoleHost } from "../host";

export type DashboardSection =
  | "overview"
  | "agents"
  | "providers"
  | "deployments"
  | "api"
  | "logs"
  | "settings";

export type RuntimeConsoleProps = {
  section: DashboardSection;
  projectSlug?: string;
  data: DashboardData;
  browserApiBaseUrl: string;
};

type NavigationItem = {
  id: DashboardSection;
  label: string;
  icon: LucideIcon;
  capability?: keyof ServerCapabilities;
};

const PROJECT_NAVIGATION: NavigationItem[] = [
  { id: "overview", label: "Overview", icon: LayoutDashboard },
  { id: "agents", label: "Agents", icon: Bot, capability: "profiles" },
  { id: "providers", label: "Providers", icon: Plug, capability: "providerConnections" },
  { id: "deployments", label: "Deployments", icon: Network },
  { id: "api", label: "API", icon: KeyRound, capability: "apiKeys" },
  { id: "logs", label: "Traces", icon: ScrollText, capability: "traces" },
  { id: "settings", label: "Settings", icon: Settings },
];

const SECTION_TITLES: Record<DashboardSection, string> = {
  overview: "Overview",
  agents: "Agents",
  providers: "Providers",
  deployments: "Deployments",
  api: "API Integrations",
  logs: "Traces",
  settings: "Settings",
};

export function RuntimeConsole({
  section,
  projectSlug,
  data,
  browserApiBaseUrl,
}: RuntimeConsoleProps) {
  const host = useRuntimeConsoleHost();
  const brand = host.brand;
  const capabilities = data.authority.status.capabilities;
  const activeSection = isSectionAvailable(section, capabilities) ? section : "overview";
  const selectedProject = projectSlug
    ? selectProject(data.runtime.projects, projectSlug)
    : null;
  const title = SECTION_TITLES[activeSection];
  return (
    <AppLayout
      sidebar={(
        <>
          <RuntimeLink className="console-brand" href={host.projectRootHref()} aria-label={brand?.label ?? "Vifu Console"}>
            {brand?.lockupSrc ? <RuntimeImage className="console-brand-lockup" src={brand.lockupSrc} width={80} height={32} alt="Vifu" priority /> : null}
            {brand?.iconSrc ? <RuntimeImage className="console-brand-mark" src={brand.iconSrc} width={32} height={32} alt="" priority /> : null}
          </RuntimeLink>
          {selectedProject ? (
            <Navigation project={selectedProject} items={PROJECT_NAVIGATION} active={activeSection} capabilities={capabilities} />
          ) : null}
          {data.authority.status.auth.required ? (
            <div className="sidebar-footer">
              <form action={host.logoutAction ?? "/auth/logout"} method="post">
                <button className="sidebar-signout" type="submit"><LogOut aria-hidden="true" /><span>Sign out</span></button>
              </form>
            </div>
          ) : null}
        </>
      )}
      header={(
        <>
          {selectedProject ? (
            <ProjectSwitcher
              projects={data.runtime.projects}
              selectedProject={selectedProject}
              activeSection={activeSection}
            />
          ) : <strong className="project-home-breadcrumb">Home</strong>}
          <div className="app-header-meta">
            <div className="runtime-state"><span className="status-dot" />{runtimeStatusLabel(data.authority.status.status)}</div>
            <div className="topbar-meta"><span>v{data.authority.status.version}</span><span>{gatewayCountLabel(data.authority.status.agentGateways)}</span></div>
          </div>
        </>
      )}
    >
      {selectedProject ? (
        <>
          <header className="page-header project-page-header"><h1>{title}</h1></header>
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
        <ProjectHome
          projects={data.runtime.projects}
          allowGuestClaim={data.authority.status.mode === "cloud"}
        />
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
  const host = useRuntimeConsoleHost();
  const visible = items.filter((item) => !item.capability || capabilities[item.capability]);
  return (
    <nav className="console-nav" aria-label="Project navigation">
      {visible.map((item) => {
        const Icon = item.icon;
        const href = host.projectSectionHref(project.slug, item.id);
        return (
          <RuntimeLink className={active === item.id ? "active" : ""} href={href} key={item.id} prefetch={false} title={item.label}>
            <Icon aria-hidden="true" />
            <span>{item.label}</span>
          </RuntimeLink>
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
  if (section === "agents") {
    return (
      <RuntimeAgentsView
        project={project}
        profiles={data.runtime.profiles}
        profileDetails={data.profileDetails}
        bindings={data.runtime.bindings}
        availableAgents={data.runtime.availableAgents}
        candidates={data.agentCandidates}
        providerAdapters={data.runtime.providerAdapters}
        projectProviders={data.projectProviders}
      />
    );
  }
  if (section === "providers") {
    return (
      <RuntimeProvidersView
        project={project}
        catalog={data.providerCatalog}
        providers={data.projectProviders}
        availableAgents={data.runtime.availableAgents}
      />
    );
  }
  if (section === "deployments") {
    return (
      <RuntimeDeploymentsView
        project={project}
        deployments={data.runtime.deployments}
        releases={data.runtime.releases}
        agentGateways={data.runtime.agentGateways}
      />
    );
  }
  if (section === "api") {
    return (
      <ApiIntegrationsView
        project={project}
        keys={data.runtime.apiKeys}
        profiles={data.runtime.profiles}
        browserApiBaseUrl={browserApiBaseUrl}
      />
    );
  }
  if (section === "logs") return <TracesView project={project} traces={projectTraces(data.runtime.traces, project)} />;
  if (section === "settings") {
    return (
      <SettingsView
        project={project}
        endpoints={endpoints}
        browserApiBaseUrl={browserApiBaseUrl}
      />
    );
  }
  return <HealthView project={project} data={data} endpoints={endpoints} browserApiBaseUrl={browserApiBaseUrl} />;
}

function HealthView({
  project,
  data,
  endpoints,
  browserApiBaseUrl,
}: {
  project: RuntimeProject;
  data: DashboardData;
  endpoints: AgentEndpoint[];
  browserApiBaseUrl: string;
}) {
  const gatewayCards = gatewayHealthCards(data.runtime.agentGateways, data.runtime.availableAgents);
  const connectedGatewayCards = gatewayCards.filter((item) => item.gateway.status === "connected");
  const connectedGateways = connectedGatewayCards.length;
  const connectedAgentKeys = new Set(
    data.runtime.availableAgents
      .filter((agent) => agent.status === "connected")
      .map((agent) => `${agent.gatewayId}/${agent.id}`),
  );
  const connected = data.runtime.profiles.filter((profile) => {
    const binding = data.runtime.bindings.find((item) => item.profileId === profile.id);
    if (binding && connectedAgentKeys.has(`${binding.gatewayId}/${binding.agentId}`)) return true;
    const detail = data.profileDetails.find((item) => item.profile.id === profile.id);
    const active = detail?.versions.find((item) => item.version.id === profile.activeVersionId);
    const providerKey = typeof active?.version.source.providerKey === "string"
      ? active.version.source.providerKey
      : active?.capabilities[0]?.providerKey;
    return data.projectProviders.some((provider) => provider.providerKey === providerKey && provider.status === "online");
  }).length;
  const agentTotal = data.runtime.profiles.length;
  const traces = projectTraces(data.runtime.traces, project);
  const traceSummary = summarizeTraces(traces);
  return (
    <div className="health-dashboard">
      <HealthSection title="Summary" defaultOpen>
        <dl className="health-summary-card">
          <div className="wide">
            <dt>HTTP URL</dt>
            <dd><code>{chatCompletionsUrl(browserApiBaseUrl)}</code></dd>
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
            <dd>{connected}/{agentTotal}</dd>
          </div>
          <div>
            <dt>Requests</dt>
            <dd>{traces.length}</dd>
          </div>
        </dl>
      </HealthSection>
      {data.projectProviders.length === 0 || agentTotal === 0 ? (
        <SetupRail
          project={project}
          providerCount={data.projectProviders.length}
          agentCount={data.runtime.profiles.length}
          callableCount={agentTotal}
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

function TracesView({ project, traces }: { project: RuntimeProject; traces: EndpointTrace[] }) {
  return (
    <section className="content-section trace-section convex-trace-section">
      <RuntimeTraceWorkbench projectId={project.id} projectSlug={project.slug} traces={traces} />
    </section>
  );
}

function SettingsView({
  project,
  endpoints,
  browserApiBaseUrl,
}: {
  project: RuntimeProject;
  endpoints: AgentEndpoint[];
  browserApiBaseUrl: string;
}) {
  const host = useRuntimeConsoleHost();
  const primaryEndpoint = endpoints[0];
  return (
    <>
      <section className="content-section">
        <SectionHeading title="Project settings" />
        <dl className="definition-grid">
          <div><dt>Name</dt><dd>{project.name}</dd></div>
          <div><dt>Slug</dt><dd>{project.slug}</dd></div>
          <div><dt>Chat completions</dt><dd>{primaryEndpoint ? <code>{chatCompletionsUrl(browserApiBaseUrl)}</code> : "No endpoint yet"}</dd></div>
          <div><dt>Model</dt><dd>{primaryEndpoint ? <code>{primaryEndpoint.slug}</code> : "No endpoint yet"}</dd></div>
          <div><dt>Status</dt><dd>{project.enabled ? "Enabled" : "Disabled"}</dd></div>
        </dl>
      </section>
      <section className="content-section danger-section">
        <SectionHeading title="Danger zone" />
        <div className="settings-danger-row">
          <div>
            <strong>Delete project</strong>
            <p>Remove this project from the dashboard. Detected gateway agents are not deleted.</p>
          </div>
          <DeleteResourceButton path={`projects/${project.id}`} label={project.name} redirectTo={host.projectRootHref()} />
        </div>
      </section>
    </>
  );
}

function SetupRail({ project, providerCount, agentCount, callableCount, connectedGatewayCount }: {
  project: RuntimeProject;
  providerCount: number;
  agentCount: number;
  callableCount: number;
  connectedGatewayCount: number;
}) {
  const host = useRuntimeConsoleHost();
  const providerReady = providerCount > 0;
  const agentsReady = agentCount > 0;
  const endpointReady = callableCount > 0;
  const gatewayReady = connectedGatewayCount > 0;
  const nextStep = !providerReady
    ? "Connect an agent provider"
    : !agentsReady
      ? "Discover provider agents"
      : !endpointReady
        ? "Add an agent"
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
        <li className={providerReady ? "ready" : "active"}><strong>Provider</strong><small>{providerReady ? `${providerCount} assigned` : "Assign one in Providers"}</small></li>
        <li className={agentsReady ? "ready" : providerReady ? "active" : undefined}><strong>Agents</strong><small>{agentsReady ? `${agentCount} available` : "Add or detect agents"}</small></li>
        <li className={endpointReady ? "ready" : agentsReady ? "active" : undefined}><strong>Endpoint</strong><small>{endpointReady ? `${callableCount} callable` : "Agents become callable when added"}</small></li>
        <li className={gatewayReady ? "ready" : endpointReady ? "active" : undefined}><strong>Gateway</strong><small>{gatewayReady ? `${connectedGatewayCount} online` : "Start gateway"}</small></li>
      </ol>
      <div className="setup-actions">
        {!providerReady ? <RuntimeLink className="primary-button" href={host.projectSectionHref(project.slug, "providers")}>Open Providers</RuntimeLink> : null}
        {providerReady && !endpointReady ? <RuntimeLink className="secondary-button" href={host.projectSectionHref(project.slug, "agents")}>Add agents</RuntimeLink> : null}
      </div>
    </section>
  );
}

function TraceTable({ traces, project: _project, detailed = false }: { traces: EndpointTrace[]; project: RuntimeProject; detailed?: boolean }) {
  if (traces.length === 0) return <EmptyState>No traces yet.</EmptyState>;
  if (detailed) {
    return <RuntimeTraceWorkbench projectId={_project.id} projectSlug={_project.slug} traces={traces} />;
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

function EmptyState({ children }: { children: React.ReactNode }) {
  return <div className="empty-state">{children}</div>;
}

function selectProject(projects: RuntimeProject[], projectSlug: string | undefined): RuntimeProject | null {
  return projects.find((project) => project.slug === projectSlug) ?? projects[0] ?? null;
}

function isSectionAvailable(section: DashboardSection, capabilities: ServerCapabilities): boolean {
  const item = PROJECT_NAVIGATION.find((entry) => entry.id === section);
  return Boolean(item && (!item.capability || capabilities[item.capability]));
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

function runtimeStatusLabel(status: string): string {
  return `Runtime ${status.toLowerCase()}`;
}

function gatewayCountLabel(count: number): string {
  return `${count} ${count === 1 ? "gateway" : "gateways"} online`;
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
