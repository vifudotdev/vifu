import "@fontsource/ibm-plex-mono/400.css";
import "@fontsource/ibm-plex-mono/500.css";
import "@fontsource/ibm-plex-mono/600.css";
import "@fontsource/ibm-plex-sans/400.css";
import "@fontsource/ibm-plex-sans/500.css";
import "@fontsource/ibm-plex-sans/600.css";
import "@fontsource/ibm-plex-sans/700.css";
import "@vifu/runtime-console/styles.css";

import { useCallback, useEffect, useMemo, useState, type MouseEvent } from "react";
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
  type RuntimeConsoleLinkProps,
  type RuntimeProject,
  type RuntimeSnapshot,
  type RuntimeStatus,
} from "@vifu/runtime-console/react";
import {
  RuntimeBrowserError,
  runtimeBrowserRequest,
  runtimeBrowserUpload,
} from "@vifu/runtime-console";
import { consoleRouteHref, readConsoleRoute, type ConsoleRoute } from "./route";

const CONSOLE_ROUTE_CHANGE_EVENT = "vifu-console-route-change";
const RUNTIME_API_BASE = "/api/runtime";
if (typeof window !== "undefined") {
  window.__VIFU_RUNTIME_CONSOLE_API_BASE__ = RUNTIME_API_BASE;
}

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
    setData(null);
    void loadEmbeddedDashboardData(route.projectSlug, route.section, controller.signal)
      .then((nextData) => {
        if (!controller.signal.aborted) setData(nextData);
      })
      .catch((nextError: unknown) => {
        if (controller.signal.aborted) return;
        setError(errorMessage(nextError));
      })
      .finally(() => {
        if (!controller.signal.aborted) setLoading(false);
      });
    return () => controller.abort();
  }, [refreshVersion, route.projectSlug, route.section]);

  const host = useMemo(() => ({
    Link: EmbeddedLink,
    router: {
      push: (href: string) => setRoute(routeFromHref(href)),
      refresh: () => setRefreshVersion((value) => value + 1),
    },
    request: runtimeBrowserRequest,
    upload: runtimeBrowserUpload,
    projectRootHref: () => "/project",
    projectHref: (projectSlug: string) => `/project/${encodeURIComponent(projectSlug)}`,
    projectSectionHref: (projectSlug: string, section: string) => `/project/${encodeURIComponent(projectSlug)}/${encodeURIComponent(section)}`,
    logoutAction: undefined,
    brand: {
      label: "Vifu Console",
      lockupSrc: "/brand/vifu-lockup.png",
      iconSrc: "/brand/vifu-icon-512.png",
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
  const [route, setRouteState] = useState(() => readBrowserRoute());

  useEffect(() => {
    const onPopState = () => setRouteState(readBrowserRoute());
    const onConsoleRouteChange = () => setRouteState(readBrowserRoute());
    window.addEventListener("popstate", onPopState);
    window.addEventListener(CONSOLE_ROUTE_CHANGE_EVENT, onConsoleRouteChange);
    return () => {
      window.removeEventListener("popstate", onPopState);
      window.removeEventListener(CONSOLE_ROUTE_CHANGE_EVENT, onConsoleRouteChange);
    };
  }, []);

  const setRoute = useCallback((nextRoute: ConsoleRoute) => {
    pushBrowserRoute(nextRoute);
    setRouteState(nextRoute);
  }, []);

  return [route, setRoute];
}

function EmbeddedLink({
  download,
  href,
  onClick,
  prefetch: _prefetch,
  target,
  ...props
}: RuntimeConsoleLinkProps) {
  return (
    <a
      {...props}
      download={download}
      href={href}
      onClick={(event) => {
        onClick?.(event);
        if (!shouldHandleConsoleLink(event, href, target, download)) return;
        event.preventDefault();
        navigateBrowserHref(href);
      }}
      target={target}
    />
  );
}

function shouldHandleConsoleLink(
  event: MouseEvent<HTMLAnchorElement>,
  href: string,
  target: RuntimeConsoleLinkProps["target"],
  download: RuntimeConsoleLinkProps["download"],
): boolean {
  if (
    event.defaultPrevented ||
    event.button !== 0 ||
    event.altKey ||
    event.ctrlKey ||
    event.metaKey ||
    event.shiftKey ||
    download !== undefined ||
    (target && target !== "_self")
  ) {
    return false;
  }

  const url = new URL(href, window.location.origin);
  return url.origin === window.location.origin && isConsolePath(url.pathname);
}

function isConsolePath(pathname: string): boolean {
  return pathname === "/" || pathname === "/project" || pathname.startsWith("/project/");
}

function navigateBrowserHref(href: string) {
  pushBrowserRoute(routeFromHref(href));
  window.dispatchEvent(new Event(CONSOLE_ROUTE_CHANGE_EVENT));
}

function pushBrowserRoute(route: ConsoleRoute) {
  const href = consoleRouteHref(route);
  const current = `${window.location.pathname}${window.location.search}${window.location.hash}`;
  if (href !== current) {
    window.history.pushState(null, "", href);
  }
}

function routeFromHref(href: string): ConsoleRoute {
  const url = new URL(href, window.location.origin);
  return readConsoleRoute(url.pathname, url.search, url.hash);
}

function readBrowserRoute(): ConsoleRoute {
  return readConsoleRoute(window.location.pathname, window.location.search, window.location.hash);
}

async function loadEmbeddedDashboardData(
  projectSlug: string | undefined,
  section: DashboardSection,
  signal: AbortSignal,
): Promise<RuntimeConsoleData> {
  const status = await runtimeBrowserRequest<RuntimeStatus>("status", "GET", undefined, signal);
  const authority = {
    kind: "local",
    displayName: "Local deployment",
    status: { ...status, auth: { required: false, mode: "none" } } satisfies DeploymentStatus,
  };
  if (section === "logs") {
    const projects = status.capabilities.projects
      ? await requestList<RuntimeProject>("projects", "projects", signal)
      : [];
    return {
      authority,
      runtime: emptyRuntimeSnapshot(projects),
      profileDetails: [],
      projectProviders: [],
      providerCatalog: { registry: [], custom: [] },
      agentCandidates: [],
    };
  }
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

function emptyRuntimeSnapshot(projects: RuntimeProject[]): RuntimeSnapshot {
  return {
    projects,
    profiles: [],
    bindings: [],
    endpoints: [],
    apiKeys: [],
    agentGateways: [],
    availableAgents: [],
    providerAdapters: [],
    traces: [],
    deployments: [],
    releases: [],
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
