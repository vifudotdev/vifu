"use client";

import type { LucideIcon } from "lucide-react";
import { useState } from "react";
import {
  Bot,
  ChevronDown,
  Check,
  Copy,
  KeyRound,
  LayoutDashboard,
  LogOut,
  Plug,
  ScrollText,
  Settings,
  TabletSmartphone,
} from "lucide-react";
import type { DashboardData } from "../data";
import type {
  AgentBinding,
  AgentEndpoint,
  AgentGateway,
  AgentProfileDetail,
  AvailableAgent,
  EndpointTrace,
  ProjectProvider,
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
import {
  DevicePairingAction,
  RuntimeDeploymentsView,
  RuntimeDevicesView,
} from "./runtime-deployments";
import { useRuntimeLiveRefresh } from "./runtime-live-refresh";
import { RuntimeImage, RuntimeLink, useRuntimeConsoleHost } from "../host";

export type DashboardSection =
  | "overview"
  | "devices"
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

export const PROJECT_NAVIGATION: NavigationItem[] = [
  { id: "overview", label: "Overview", icon: LayoutDashboard },
  { id: "devices", label: "Devices", icon: TabletSmartphone, capability: "agentGateways" },
  { id: "agents", label: "Agents", icon: Bot, capability: "profiles" },
  { id: "providers", label: "Providers", icon: Plug, capability: "providerConnections" },
  { id: "api", label: "API", icon: KeyRound, capability: "apiKeys" },
  { id: "logs", label: "Traces", icon: ScrollText, capability: "traces" },
  { id: "settings", label: "Settings", icon: Settings },
];

const SECTION_TITLES: Record<DashboardSection, string> = {
  overview: "Overview",
  devices: "Devices",
  agents: "Agents",
  providers: "Providers",
  deployments: "Settings",
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
  const requestedSection = isSectionAvailable(section, capabilities) ? section : "overview";
  const activeSection = requestedSection === "deployments" ? "settings" : requestedSection;
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
              section={requestedSection}
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
    <nav className="console-nav" aria-label="App navigation">
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
  const endpoints = appEndpoints(project, data.runtime.endpoints);
  if (section === "devices") {
    return (
      <RuntimeDevicesView
        project={project}
        deployments={data.runtime.deployments}
        agentGateways={data.runtime.agentGateways}
      />
    );
  }
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
  if (section === "api") {
    return (
      <ApiIntegrationsView
        project={project}
        keys={data.runtime.apiKeys}
        profiles={data.runtime.profiles}
        bindings={data.runtime.bindings}
        deployments={data.runtime.deployments}
        browserApiBaseUrl={browserApiBaseUrl}
      />
    );
  }
  if (section === "logs") {
    return (
      <TracesView
        project={project}
        traces={appTraces(data.runtime.traces, project)}
        profileDetails={data.profileDetails}
        providers={data.projectProviders}
      />
    );
  }
  if (section === "settings" || section === "deployments") {
    return (
      <SettingsView
        project={project}
        endpoints={endpoints}
        browserApiBaseUrl={browserApiBaseUrl}
        deployments={data.runtime.deployments}
        releases={data.runtime.releases}
        agentGateways={data.runtime.agentGateways}
        advancedOpen={section === "deployments"}
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
      .map((agent) => `${agent.gatewayId}/${stringValue(agent.metadata.providerKey)}/${agent.id}`),
  );
  const connected = data.runtime.profiles.filter((profile) => {
    const binding = data.runtime.bindings.find((item) => item.profileId === profile.id);
    const detail = data.profileDetails.find((item) => item.profile.id === profile.id);
    const active = detail?.versions.find((item) => item.version.id === profile.activeVersionId);
    const providerKey = typeof active?.version.source.providerKey === "string"
      ? active.version.source.providerKey
      : active?.capabilities[0]?.providerKey ?? stringValue(binding?.config.providerKey);
    if (binding && providerKey) {
      const provider = data.projectProviders.find((item) => item.providerKey === providerKey);
      const gatewayManaged = Boolean(provider && stringValue(provider.config.gatewayId));
      const gatewayOnline = connectedAgentKeys.has(`${binding.gatewayId}/${providerKey}/${binding.agentId}`);
      if (gatewayManaged || gatewayOnline) return gatewayOnline;
    }
    return data.projectProviders.some((provider) => provider.providerKey === providerKey && provider.status === "online");
  }).length;
  const agentTotal = data.runtime.profiles.length;
  const traces = appTraces(data.runtime.traces, project);
  const traceSummary = summarizeTraces(traces);
  useRuntimeLiveRefresh(true);
  return (
    <div className="health-dashboard">
      <section className={`device-connect-rail overview-device-rail${connectedGateways > 0 ? " has-devices" : ""}`}>
        <div className="device-connect-copy">
          <span className="device-connect-signal" aria-hidden="true"><i /></span>
          <div>
            <span>{connectedGateways > 0 ? "Live device status" : "Connect your app"}</span>
            <strong>{connectedGateways > 0 ? `${connectedGateways} ${connectedGateways === 1 ? "device" : "devices"} online` : "Pair your first device"}</strong>
            <p>{connectedGateways > 0 ? "Status and agents refresh here while this page is open." : "Connect a phone, computer, or embedded runtime with one pairing code."}</p>
          </div>
        </div>
        <DevicePairingAction project={project} deployments={data.runtime.deployments} />
      </section>
      <HealthSection title="Summary" defaultOpen>
        <dl className="health-summary-card">
          <div className="wide">
            <dt>App ID</dt>
            <dd><AppIdValue value={project.appId} /></dd>
          </div>
          <div className="wide">
            <dt>HTTP URL</dt>
            <dd><code>{chatCompletionsUrl(browserApiBaseUrl)}</code></dd>
          </div>
          <div className="wide">
            <dt>WS URL</dt>
            <dd><span className="health-unavailable">Not available</span></dd>
          </div>
          <div>
            <dt>Devices</dt>
            <dd>{connectedGateways}/{gatewayCards.length}</dd>
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
          <TraceMetricCard title="Requests" value={String(traces.length)} meta="Recent app traces">
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
      <HealthSection title="Devices" count={`${connectedGateways}/${gatewayCards.length} connected`} defaultOpen>
        {gatewayCards.length > 0 ? (
          <div className="gateway-health-grid">
            {gatewayCards.map(({ gateway, agents }) => (
              <GatewayHealthCard gateway={gateway} agents={agents} key={gateway.gatewayId} />
            ))}
          </div>
        ) : (
          <EmptyState>No devices paired.</EmptyState>
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
          <strong>{gatewayDisplayLabel(gateway)}</strong>
          <code title={gateway.gatewayId}>{shortId(gateway.gatewayId, 18)}</code>
        </div>
        <span className={statusClassName(gateway.status)}>{gateway.status}</span>
      </header>
      <dl>
        <div><dt>Agents</dt><dd>{fallbackAgents.length}</dd></div>
        <div><dt>Type</dt><dd>{gatewayKindLabel(gateway.metadata)}</dd></div>
        <div><dt>Device</dt><dd>{gatewayDeviceLabel(gateway.metadata)}</dd></div>
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

function TracesView({
  profileDetails,
  project,
  providers,
  traces,
}: {
  profileDetails: AgentProfileDetail[];
  project: RuntimeProject;
  providers: ProjectProvider[];
  traces: EndpointTrace[];
}) {
  return (
    <section className="content-section trace-section convex-trace-section">
      <RuntimeTraceWorkbench
        profileDetails={profileDetails}
        projectId={project.id}
        projectSlug={project.slug}
        providers={providers}
        traces={traces}
      />
    </section>
  );
}

function SettingsView({
  project,
  endpoints,
  browserApiBaseUrl,
  deployments,
  releases,
  agentGateways,
  advancedOpen,
}: {
  project: RuntimeProject;
  endpoints: AgentEndpoint[];
  browserApiBaseUrl: string;
  deployments: DashboardData["runtime"]["deployments"];
  releases: DashboardData["runtime"]["releases"];
  agentGateways: AgentGateway[];
  advancedOpen: boolean;
}) {
  const host = useRuntimeConsoleHost();
  const primaryEndpoint = endpoints[0];
  return (
    <div className="settings-workbench">
      <section className="content-section">
        <SectionHeading title="App settings" />
        <dl className="definition-grid">
          <div><dt>Name</dt><dd>{project.name}</dd></div>
          <div><dt>App ID</dt><dd><AppIdValue value={project.appId} /></dd></div>
          <div><dt>Slug</dt><dd>{project.slug}</dd></div>
          <div><dt>Chat completions</dt><dd>{primaryEndpoint ? <code>{chatCompletionsUrl(browserApiBaseUrl)}</code> : "No endpoint yet"}</dd></div>
          <div><dt>Model</dt><dd>{primaryEndpoint ? <code>{primaryEndpoint.slug}</code> : "No endpoint yet"}</dd></div>
          <div><dt>Status</dt><dd>{project.enabled ? "Enabled" : "Disabled"}</dd></div>
        </dl>
      </section>
      <details className="advanced-settings" id="environments" open={advancedOpen}>
        <summary>
          <ChevronDown aria-hidden="true" />
          <div><strong>Advanced runtime configuration</strong><span>Manage multiple environments, device policies, and versioned configuration.</span></div>
          <small>{deployments.length} {deployments.length === 1 ? "environment" : "environments"}</small>
        </summary>
        <div className="advanced-settings-body">
          <RuntimeDeploymentsView
            project={project}
            deployments={deployments}
            releases={releases}
            agentGateways={agentGateways}
          />
        </div>
      </details>
      <section className="content-section danger-section">
        <SectionHeading title="Danger zone" />
        <div className="settings-danger-row">
          <div>
            <strong>Delete app</strong>
            <p>Remove this app from the dashboard. Paired devices and their local agents are not deleted.</p>
          </div>
          <DeleteResourceButton path={`apps/${project.id}`} label={project.name} redirectTo={host.projectRootHref()} />
        </div>
      </section>
    </div>
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
  const runtimeReady = providerReady || gatewayReady;
  const nextStep = !runtimeReady
    ? "Connect a device or provider"
    : !agentsReady
      ? "Add an agent"
      : !endpointReady
        ? "Make the agent callable"
        : "App is ready to call";
  return (
    <section className="setup-rail project-setup-rail" aria-label="App setup">
      <div>
        <span>Next setup step</span>
        <strong>{nextStep}</strong>
      </div>
      <ol>
        <li className="ready"><strong>App</strong><small>{project.slug}</small></li>
        <li className={runtimeReady ? "ready" : "active"}><strong>Runtime</strong><small>{gatewayReady ? `${connectedGatewayCount} devices online` : providerReady ? `${providerCount} providers connected` : "Pair a device or provider"}</small></li>
        <li className={agentsReady ? "ready" : runtimeReady ? "active" : undefined}><strong>Agents</strong><small>{agentsReady ? `${agentCount} available` : "Add or detect agents"}</small></li>
        <li className={endpointReady ? "ready" : agentsReady ? "active" : undefined}><strong>API</strong><small>{endpointReady ? `${callableCount} callable` : "Agents become callable when added"}</small></li>
      </ol>
      <div className="setup-actions">
        {!runtimeReady ? <RuntimeLink className="primary-button" href={host.projectSectionHref(project.slug, "devices")}>Open Devices</RuntimeLink> : null}
        {!runtimeReady ? <RuntimeLink className="secondary-button" href={host.projectSectionHref(project.slug, "providers")}>Connect Provider</RuntimeLink> : null}
        {runtimeReady && !endpointReady ? <RuntimeLink className="secondary-button" href={host.projectSectionHref(project.slug, "agents")}>Add agents</RuntimeLink> : null}
      </div>
    </section>
  );
}

function AppIdValue({ value }: { value: string }) {
  const [copied, setCopied] = useState(false);

  async function copy() {
    await navigator.clipboard.writeText(value);
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1_500);
  }

  return (
    <span className="inline-code-action">
      <code>{value}</code>
      <button className="secondary-button compact" type="button" onClick={copy} aria-label="Copy App ID">
        {copied ? <Check aria-hidden="true" /> : <Copy aria-hidden="true" />}
        {copied ? "Copied" : "Copy"}
      </button>
    </span>
  );
}

function TraceTable({ traces, project: _App, detailed = false }: { traces: EndpointTrace[]; project: RuntimeProject; detailed?: boolean }) {
  if (traces.length === 0) return <EmptyState>No traces yet.</EmptyState>;
  if (detailed) {
    return <RuntimeTraceWorkbench projectId={_App.id} projectSlug={_App.slug} traces={traces} />;
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
  if (section === "deployments") return true;
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
    .sort((a, b) => gatewayStatusRank(a.status) - gatewayStatusRank(b.status)
      || gatewayDisplayLabel(a).localeCompare(gatewayDisplayLabel(b)))
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

function stringValue(value: unknown): string {
  return typeof value === "string" ? value : "";
}

function appEndpoints(project: RuntimeProject, endpoints: AgentEndpoint[]): AgentEndpoint[] {
  const bindingIds = new Set(project.bindingIds);
  return endpoints.filter((endpoint) => bindingIds.has(endpoint.bindingId));
}

function appTraces(traces: EndpointTrace[], project: RuntimeProject): EndpointTrace[] {
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
  return `${count} ${count === 1 ? "device" : "devices"} online`;
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

function gatewayDisplayLabel(gateway: AgentGateway): string {
  const reportedName = stringValue(gateway.metadata.name).trim();
  if (reportedName) return reportedName;
  const value = gateway.gatewayId;
  return value.startsWith("gateway-") ? `Gateway ${shortId(value.replace(/^gateway-/, ""))}` : shortId(value);
}

function gatewayKindLabel(metadata: Record<string, unknown>): string {
  const kind = stringValue(metadata.kind).trim();
  const platform = stringValue(metadata.platform).trim();
  return [kind, platform].filter(Boolean).join(" · ") || "Gateway";
}

function gatewayDeviceLabel(metadata: Record<string, unknown>): string {
  const device = metadata.device && typeof metadata.device === "object"
    ? metadata.device as Record<string, unknown>
    : {};
  const manufacturer = stringValue(device.manufacturer).trim();
  const model = stringValue(device.model).trim();
  return [manufacturer, model].filter(Boolean).join(" ") || "-";
}
