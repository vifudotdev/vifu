import type {
  AgentBinding,
  AgentEndpoint,
  AgentProfile,
  AgentProfileDetail,
  AgentGateway,
  ApiKeyRecord,
  AvailableAgent,
  EndpointTrace,
  ProjectAgentCandidate,
  ProviderCatalog,
  ProviderAdapter,
  ProjectProvider,
  ProjectRuntimeRelease,
  RuntimeStatus,
  RuntimeProject,
  RuntimeDeployment,
} from "./types";

export type VifuFetch = (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>;

export type DeploymentClientOptions = {
  apiBaseUrl: string;
  credential?: string | null;
  fetcher?: VifuFetch;
};

const DEPLOYMENT_REQUEST_TIMEOUT_MS = 8_000;

export class VifuHttpError extends Error {
  readonly status: number;
  readonly payload: unknown;

  constructor(status: number, message: string, payload: unknown) {
    super(message);
    this.name = "VifuHttpError";
    this.status = status;
    this.payload = payload;
  }
}

export class DeploymentClient {
  readonly apiBaseUrl: string;
  readonly credential: string | null;
  private readonly fetcher: VifuFetch;

  constructor(options: DeploymentClientOptions) {
    this.apiBaseUrl = options.apiBaseUrl;
    this.credential = options.credential?.trim() || null;
    this.fetcher = options.fetcher ?? fetch;
  }

  status(): Promise<RuntimeStatus> {
    return this.request<RuntimeStatus>("/v1/status", { method: "GET" }, false);
  }

  verifyAdmin(): Promise<{ valid: boolean }> {
    return this.request("/v1/admin/verify");
  }

  async apps(): Promise<RuntimeProject[]> {
    return (await this.request<{ apps: RuntimeProject[] }>("/v1/apps")).apps ?? [];
  }

  async profiles(): Promise<AgentProfile[]> {
    return (await this.request<{ profiles: AgentProfile[] }>("/v1/profiles")).profiles ?? [];
  }

  async appProfiles(slug: string): Promise<AgentProfile[]> {
    return (await this.request<{ profiles: AgentProfile[] }>(`/v1/apps/${encodeURIComponent(slug)}/profiles`)).profiles ?? [];
  }

  async appProfile(slug: string, profileId: string): Promise<AgentProfileDetail> {
    return this.request<AgentProfileDetail>(
      `/v1/apps/${encodeURIComponent(slug)}/profiles/${encodeURIComponent(profileId)}`,
    );
  }

  async appBindings(slug: string): Promise<AgentBinding[]> {
    return (await this.request<{ bindings: AgentBinding[] }>(`/v1/apps/${encodeURIComponent(slug)}/bindings`)).bindings ?? [];
  }

  async appEndpoints(slug: string): Promise<AgentEndpoint[]> {
    return (await this.request<{ endpoints: AgentEndpoint[] }>(`/v1/apps/${encodeURIComponent(slug)}/endpoints`)).endpoints ?? [];
  }

  async appApiKeys(slug: string): Promise<ApiKeyRecord[]> {
    return (await this.request<{ apiKeys: ApiKeyRecord[] }>(`/v1/apps/${encodeURIComponent(slug)}/api-keys`)).apiKeys ?? [];
  }

  async appAgentGateways(slug: string): Promise<AgentGateway[]> {
    return (await this.request<{ agentGateways: AgentGateway[] }>(`/v1/apps/${encodeURIComponent(slug)}/agent-gateways`)).agentGateways ?? [];
  }

  async appAvailableAgents(slug: string): Promise<AvailableAgent[]> {
    return (await this.request<{ agents: AvailableAgent[] }>(`/v1/apps/${encodeURIComponent(slug)}/agents`)).agents ?? [];
  }

  async providerAdapters(): Promise<ProviderAdapter[]> {
    return (await this.request<{ providerAdapters: ProviderAdapter[] }>("/v1/provider-adapters")).providerAdapters ?? [];
  }

  async appProviderCatalog(slug: string): Promise<ProviderCatalog> {
    const catalog = await this.request<Partial<ProviderCatalog>>(`/v1/apps/${encodeURIComponent(slug)}/provider-catalog`);
    return { registry: catalog.registry ?? [], custom: catalog.custom ?? [] };
  }

  async appProviders(slug: string): Promise<ProjectProvider[]> {
    return (await this.request<{ providers: ProjectProvider[] }>(`/v1/apps/${encodeURIComponent(slug)}/providers`)).providers ?? [];
  }

  async appAgentCandidates(slug: string): Promise<ProjectAgentCandidate[]> {
    return (await this.request<{ candidates: ProjectAgentCandidate[] }>(`/v1/apps/${encodeURIComponent(slug)}/agent-candidates`)).candidates ?? [];
  }

  async appTraces(slug: string): Promise<EndpointTrace[]> {
    return (await this.request<{ traces: EndpointTrace[] }>(`/v1/apps/${encodeURIComponent(slug)}/traces?limit=100`)).traces ?? [];
  }

  async appDeployments(slug: string): Promise<RuntimeDeployment[]> {
    return (await this.request<{ deployments: RuntimeDeployment[] }>(
      `/v1/apps/${encodeURIComponent(slug)}/deployments`,
    )).deployments ?? [];
  }

  async appRuntimeReleases(slug: string): Promise<ProjectRuntimeRelease[]> {
    return (await this.request<{ releases: ProjectRuntimeRelease[] }>(
      `/v1/apps/${encodeURIComponent(slug)}/runtime-releases`,
    )).releases ?? [];
  }

  async request<T>(path: string, init: RequestInit = {}, publicRequest = false): Promise<T> {
    const response = await this.rawRequest(path, init, publicRequest);
    if (response.status === 204) return undefined as T;
    const payload = await response.json().catch(() => null) as T | { error?: unknown } | null;
    if (!response.ok) throw new VifuHttpError(response.status, readErrorMessage(payload), payload);
    return (payload ?? {}) as T;
  }

  async rawRequest(
    path: string,
    init: RequestInit = {},
    publicRequest = false,
    timeoutMs: number | null = DEPLOYMENT_REQUEST_TIMEOUT_MS,
  ): Promise<Response> {
    const headers = new Headers(init.headers);
    headers.set("accept", headers.get("accept") ?? "application/json");
    if (init.body && !headers.has("content-type")) headers.set("content-type", "application/json");
    if (!publicRequest && this.credential) headers.set("authorization", `Vifu ${this.credential}`);
    const controller = new AbortController();
    const parentSignal = init.signal;
    let timedOut = false;
    const abortFromParent = () => controller.abort();
    if (parentSignal?.aborted) controller.abort();
    else parentSignal?.addEventListener("abort", abortFromParent, { once: true });
    const timeout = timeoutMs === null ? null : setTimeout(() => {
      timedOut = true;
      controller.abort();
    }, timeoutMs);
    try {
      return await this.fetcher(appendApiPath(this.apiBaseUrl, path), {
        ...init,
        headers,
        cache: init.cache ?? "no-store",
        signal: controller.signal,
      });
    } catch (error) {
      if (timedOut) throw new VifuHttpError(504, "Vifu API request timed out.", null);
      throw error;
    } finally {
      if (timeout !== null) clearTimeout(timeout);
      parentSignal?.removeEventListener("abort", abortFromParent);
    }
  }
}

export function readErrorMessage(payload: unknown): string {
  if (!payload || typeof payload !== "object") return "Vifu API request failed.";
  const error = (payload as { error?: unknown }).error;
  if (typeof error === "string" && error.trim()) return error.trim();
  if (error && typeof error === "object") {
    const message = (error as { message?: unknown }).message;
    if (typeof message === "string" && message.trim()) return message.trim();
  }
  return "Vifu API request failed.";
}

function appendApiPath(apiBaseUrl: string, requestPath: string): string {
  const value = apiBaseUrl.trim();
  if (!value) throw new Error("Vifu API base URL is not configured.");
  const url = new URL(value);
  if (url.protocol !== "http:" && url.protocol !== "https:") {
    throw new Error("Vifu API base URL must use HTTP or HTTPS.");
  }
  const base = url.toString().replace(/\/$/, "");
  return `${base}${requestPath.startsWith("/") ? requestPath : `/${requestPath}`}`;
}
