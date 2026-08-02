import "@fontsource/ibm-plex-mono/400.css";
import "@fontsource/ibm-plex-mono/500.css";
import "@fontsource/ibm-plex-mono/600.css";
import "@fontsource/ibm-plex-sans/400.css";
import "@fontsource/ibm-plex-sans/500.css";
import "@fontsource/ibm-plex-sans/600.css";
import "@fontsource/ibm-plex-sans/700.css";
import "@vifu/runtime-console/styles.css";

import { useCallback, useEffect, useMemo, useState } from "react";
import { createRoot } from "react-dom/client";
import {
  RuntimeConsole,
  RuntimeConsoleHostProvider,
  type AgentBinding,
  type AgentEndpoint,
  type AgentGateway,
  type AgentProfile,
  type AgentProfileDetail,
  type ApiKeyRecord,
  type AvailableAgent,
  type DashboardSection,
  type DeploymentStatus,
  type EndpointTrace,
  type ProviderCatalog,
  type ProviderAdapter,
  type ProjectAgentCandidate,
  type ProjectRuntimeRelease,
  type ProjectProvider,
  type RuntimeDeployment,
  type RuntimeConsoleData,
  type RuntimeProject,
  type RuntimeSnapshot,
  type RuntimeStatus,
} from "@vifu/runtime-console/react";
import {
  RuntimeBrowserError,
  runtimeBrowserRequest,
  runtimeBrowserUpload,
} from "@vifu/runtime-console";

const CONSOLE_BASE = "/console";
const SECTION_IDS = new Set<DashboardSection>([
  "overview",
  "agents",
  "providers",
  "deployments",
  "api",
  "logs",
  "settings",
]);

if (typeof window !== "undefined") {
  window.__VIFU_RUNTIME_CONSOLE_API_BASE__ = `${CONSOLE_BASE}/api/runtime`;
}

type ConsoleRoute = {
  projectSlug?: string;
  section: DashboardSection;
};

function EmbeddedRuntimeConsole() {
  const [route, setRoute] = useRoute();
  const [refreshVersion, setRefreshVersion] = useState(0);
  const [data, setData] = useState<RuntimeConsoleData | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const controller = new AbortController();
    setLoading(true);
    setError(null);
    void loadEmbeddedDashboardData(route.projectSlug, controller.signal)
      .then((nextData) => setData(nextData))
      .catch((nextError: unknown) => {
        if (controller.signal.aborted) return;
        setError(errorMessage(nextError));
      })
      .finally(() => {
        if (!controller.signal.aborted) setLoading(false);
      });
    return () => controller.abort();
  }, [refreshVersion, route.projectSlug]);

  const host = useMemo(() => ({
    router: {
      push: (href: string) => setRoute(routeFromHref(href)),
      refresh: () => setRefreshVersion((value) => value + 1),
    },
    request: runtimeBrowserRequest,
    upload: runtimeBrowserUpload,
    projectRootHref: () => `${CONSOLE_BASE}/project`,
    projectHref: (projectSlug: string) => `${CONSOLE_BASE}/project/${encodeURIComponent(projectSlug)}`,
    projectSectionHref: (projectSlug: string, section: string) => `${CONSOLE_BASE}/project/${encodeURIComponent(projectSlug)}/${encodeURIComponent(section)}`,
    logoutAction: undefined,
    brand: {
      label: "Vifu Console",
      lockupSrc: `${CONSOLE_BASE}/brand/vifu-lockup.png`,
      iconSrc: `${CONSOLE_BASE}/brand/vifu-icon-512.png`,
    },
  }), [setRoute]);

  return (
    <RuntimeConsoleHostProvider value={host}>
      {loading && !data ? <EmbeddedState title="Loading console" /> : null}
      {error && !data ? <EmbeddedState title="Console unavailable" message={error} onRetry={() => setRefreshVersion((value) => value + 1)} /> : null}
      {data ? (
        <RuntimeConsole
          section={route.section}
          projectSlug={route.projectSlug}
          data={data}
          browserApiBaseUrl={window.location.origin}
        />
      ) : null}
    </RuntimeConsoleHostProvider>
  );
}

function EmbeddedState({
  title,
  message,
  onRetry,
}: {
  title: string;
  message?: string;
  onRetry?: () => void;
}) {
  return (
    <main className="console-content no-project-content">
      <section className="content-section">
        <h1>{title}</h1>
        {message ? <p>{message}</p> : <p>Connecting to the local Vifu runtime.</p>}
        {onRetry ? <button className="primary-button" type="button" onClick={onRetry}>Retry</button> : null}
      </section>
    </main>
  );
}

function useRoute(): [ConsoleRoute, (route: ConsoleRoute) => void] {
  const [route, setRouteState] = useState(() => readRoute(window.location.pathname));

  useEffect(() => {
    const onPopState = () => setRouteState(readRoute(window.location.pathname));
    window.addEventListener("popstate", onPopState);
    return () => window.removeEventListener("popstate", onPopState);
  }, []);

  const setRoute = useCallback((nextRoute: ConsoleRoute) => {
    const href = routeHref(nextRoute);
    if (href !== window.location.pathname) {
      window.history.pushState(null, "", href);
    }
    setRouteState(nextRoute);
  }, []);

  return [route, setRoute];
}

function routeFromHref(href: string): ConsoleRoute {
  return readRoute(new URL(href, window.location.origin).pathname);
}

function readRoute(pathname: string): ConsoleRoute {
  const relative = pathname.startsWith(CONSOLE_BASE)
    ? pathname.slice(CONSOLE_BASE.length)
    : pathname;
  const parts = relative.split("/").filter(Boolean);
  if (parts[0] !== "project") return { section: "overview" };
  const projectSlug = parts[1] ? decodeURIComponent(parts[1]) : undefined;
  const section = SECTION_IDS.has(parts[2] as DashboardSection)
    ? parts[2] as DashboardSection
    : "overview";
  return { projectSlug, section };
}

function routeHref(route: ConsoleRoute): string {
  if (!route.projectSlug) return `${CONSOLE_BASE}/project`;
  if (route.section === "overview") return `${CONSOLE_BASE}/project/${encodeURIComponent(route.projectSlug)}`;
  return `${CONSOLE_BASE}/project/${encodeURIComponent(route.projectSlug)}/${encodeURIComponent(route.section)}`;
}

async function loadEmbeddedDashboardData(
  projectSlug: string | undefined,
  signal: AbortSignal,
): Promise<RuntimeConsoleData> {
  const status = await runtimeBrowserRequest<RuntimeStatus>("status", "GET", undefined, signal);
  const authority = {
    kind: "local",
    displayName: "Local deployment",
    status: { ...status, auth: { required: false, mode: "none" } } satisfies DeploymentStatus,
  };
  const runtime = await loadRuntimeSnapshot(authority.status, projectSlug, signal);
  const [projectProviders, providerCatalog, agentCandidates] = projectSlug
    ? await Promise.all([
      requestList<ProjectProvider>(`project/${encodeURIComponent(projectSlug)}/providers`, "providers", signal),
      requestCatalog(projectSlug, signal),
      requestList<ProjectAgentCandidate>(`project/${encodeURIComponent(projectSlug)}/agent-candidates`, "candidates", signal),
    ])
    : [[], { registry: [], custom: [] } satisfies ProviderCatalog, []];
  const profileDetails = projectSlug
    ? await Promise.all(runtime.profiles.map((profile) => runtimeBrowserRequest<AgentProfileDetail>(
      `project/${encodeURIComponent(projectSlug)}/profiles/${encodeURIComponent(profile.id)}`,
      "GET",
      undefined,
      signal,
    )))
    : [];

  return {
    authority,
    runtime,
    profileDetails,
    projectProviders,
    providerCatalog,
    agentCandidates,
  };
}

async function loadRuntimeSnapshot(
  status: DeploymentStatus,
  projectSlug: string | undefined,
  signal: AbortSignal,
): Promise<RuntimeSnapshot> {
  const capabilities = status.capabilities;
  const [
    projects,
    profiles,
    bindings,
    endpoints,
    apiKeys,
    agentGateways,
    availableAgents,
    providerAdapters,
    traces,
    deployments,
    releases,
  ] = await Promise.all([
    capabilities.projects ? requestList<RuntimeProject>("projects", "projects", signal) : Promise.resolve([]),
    capabilities.profiles && projectSlug ? requestList<AgentProfile>(`project/${encodeURIComponent(projectSlug)}/profiles`, "profiles", signal) : Promise.resolve([]),
    capabilities.bindings && projectSlug ? requestList<AgentBinding>(`project/${encodeURIComponent(projectSlug)}/bindings`, "bindings", signal) : Promise.resolve([]),
    capabilities.endpoints && projectSlug ? requestList<AgentEndpoint>(`project/${encodeURIComponent(projectSlug)}/endpoints`, "endpoints", signal) : Promise.resolve([]),
    capabilities.apiKeys && projectSlug ? requestList<ApiKeyRecord>(`project/${encodeURIComponent(projectSlug)}/api-keys`, "apiKeys", signal) : Promise.resolve([]),
    capabilities.agentGateways && projectSlug ? requestList<AgentGateway>(`project/${encodeURIComponent(projectSlug)}/agent-gateways`, "agentGateways", signal) : Promise.resolve([]),
    capabilities.agentGateways && projectSlug ? requestList<AvailableAgent>(`project/${encodeURIComponent(projectSlug)}/agents`, "agents", signal) : Promise.resolve([]),
    capabilities.providerConnections ? requestList<ProviderAdapter>("provider-adapters", "providerAdapters", signal) : Promise.resolve([]),
    capabilities.traces && projectSlug ? requestList<EndpointTrace>(`project/${encodeURIComponent(projectSlug)}/traces?limit=100`, "traces", signal) : Promise.resolve([]),
    projectSlug ? requestList<RuntimeDeployment>(`project/${encodeURIComponent(projectSlug)}/deployments`, "deployments", signal) : Promise.resolve([]),
    projectSlug ? requestList<ProjectRuntimeRelease>(`project/${encodeURIComponent(projectSlug)}/runtime-releases`, "releases", signal) : Promise.resolve([]),
  ]);

  return {
    projects,
    profiles,
    bindings,
    endpoints,
    apiKeys,
    agentGateways,
    availableAgents,
    providerAdapters,
    traces,
    deployments,
    releases,
  };
}

async function requestCatalog(projectSlug: string, signal: AbortSignal): Promise<ProviderCatalog> {
  const catalog = await runtimeBrowserRequest<Partial<ProviderCatalog>>(
    `project/${encodeURIComponent(projectSlug)}/provider-catalog`,
    "GET",
    undefined,
    signal,
  );
  return { registry: catalog.registry ?? [], custom: catalog.custom ?? [] };
}

async function requestList<T>(
  path: string,
  key: string,
  signal: AbortSignal,
): Promise<T[]> {
  const payload = await runtimeBrowserRequest<Record<string, T[] | undefined>>(path, "GET", undefined, signal);
  return payload[key] ?? [];
}

function errorMessage(error: unknown): string {
  if (error instanceof RuntimeBrowserError) return error.message;
  if (error instanceof Error) return error.message;
  return "Could not load the Vifu console.";
}

const rootElement = document.getElementById("root");
if (!rootElement) throw new Error("Embedded console root was not found.");
createRoot(rootElement).render(<EmbeddedRuntimeConsole />);
