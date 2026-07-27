import { appendApiPath } from "./config";
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
  RuntimeStatus,
  RuntimeProject,
} from "./runtime-types";

export type VifuFetch = (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>;

export type DeploymentClientOptions = {
  apiBaseUrl: string;
  credential?: string | null;
  fetcher?: VifuFetch;
};

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

  async projects(): Promise<RuntimeProject[]> {
    return (await this.request<{ projects: RuntimeProject[] }>("/v1/projects")).projects ?? [];
  }

  async profiles(): Promise<AgentProfile[]> {
    return (await this.request<{ profiles: AgentProfile[] }>("/v1/profiles")).profiles ?? [];
  }

  async projectProfiles(slug: string): Promise<AgentProfile[]> {
    return (await this.request<{ profiles: AgentProfile[] }>(`/v1/project/${encodeURIComponent(slug)}/profiles`)).profiles ?? [];
  }

  async projectProfile(slug: string, profileId: string): Promise<AgentProfileDetail> {
    return this.request<AgentProfileDetail>(
      `/v1/project/${encodeURIComponent(slug)}/profiles/${encodeURIComponent(profileId)}`,
    );
  }

  async projectBindings(slug: string): Promise<AgentBinding[]> {
    return (await this.request<{ bindings: AgentBinding[] }>(`/v1/project/${encodeURIComponent(slug)}/bindings`)).bindings ?? [];
  }

  async projectEndpoints(slug: string): Promise<AgentEndpoint[]> {
    return (await this.request<{ endpoints: AgentEndpoint[] }>(`/v1/project/${encodeURIComponent(slug)}/endpoints`)).endpoints ?? [];
  }

  async projectApiKeys(slug: string): Promise<ApiKeyRecord[]> {
    return (await this.request<{ apiKeys: ApiKeyRecord[] }>(`/v1/project/${encodeURIComponent(slug)}/api-keys`)).apiKeys ?? [];
  }

  async projectAgentGateways(slug: string): Promise<AgentGateway[]> {
    return (await this.request<{ agentGateways: AgentGateway[] }>(`/v1/project/${encodeURIComponent(slug)}/agent-gateways`)).agentGateways ?? [];
  }

  async projectAvailableAgents(slug: string): Promise<AvailableAgent[]> {
    return (await this.request<{ agents: AvailableAgent[] }>(`/v1/project/${encodeURIComponent(slug)}/agents`)).agents ?? [];
  }

  async providerAdapters(): Promise<ProviderAdapter[]> {
    return (await this.request<{ providerAdapters: ProviderAdapter[] }>("/v1/provider-adapters")).providerAdapters ?? [];
  }

  async projectProviderCatalog(slug: string): Promise<ProviderCatalog> {
    const catalog = await this.request<Partial<ProviderCatalog>>(`/v1/project/${encodeURIComponent(slug)}/provider-catalog`);
    return { registry: catalog.registry ?? [], custom: catalog.custom ?? [] };
  }

  async projectProviders(slug: string): Promise<ProjectProvider[]> {
    return (await this.request<{ providers: ProjectProvider[] }>(`/v1/project/${encodeURIComponent(slug)}/providers`)).providers ?? [];
  }

  async projectAgentCandidates(slug: string): Promise<ProjectAgentCandidate[]> {
    return (await this.request<{ candidates: ProjectAgentCandidate[] }>(`/v1/project/${encodeURIComponent(slug)}/agent-candidates`)).candidates ?? [];
  }

  async projectTraces(slug: string): Promise<EndpointTrace[]> {
    return (await this.request<{ traces: EndpointTrace[] }>(`/v1/project/${encodeURIComponent(slug)}/traces?limit=100`)).traces ?? [];
  }

  async request<T>(path: string, init: RequestInit = {}, publicRequest = false): Promise<T> {
    const response = await this.rawRequest(path, init, publicRequest);
    if (response.status === 204) return undefined as T;
    const payload = await response.json().catch(() => null) as T | { error?: unknown } | null;
    if (!response.ok) throw new VifuHttpError(response.status, readErrorMessage(payload), payload);
    return (payload ?? {}) as T;
  }

  async rawRequest(path: string, init: RequestInit = {}, publicRequest = false): Promise<Response> {
    const headers = new Headers(init.headers);
    headers.set("accept", headers.get("accept") ?? "application/json");
    if (init.body && !headers.has("content-type")) headers.set("content-type", "application/json");
    if (!publicRequest && this.credential) headers.set("authorization", `Vifu ${this.credential}`);
    return this.fetcher(appendApiPath(this.apiBaseUrl, path), {
      ...init,
      headers,
      cache: init.cache ?? "no-store",
    });
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
