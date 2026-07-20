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
  ProjectCanvas,
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

  async projects(): Promise<RuntimeProject[]> {
    return (await this.request<{ projects: RuntimeProject[] }>("/v1/projects")).projects ?? [];
  }

  async projectCanvas(slug: string): Promise<ProjectCanvas> {
    return (await this.request<{ canvas: ProjectCanvas }>(`/v1/project/${encodeURIComponent(slug)}/canvas`)).canvas;
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

  async bindings(): Promise<AgentBinding[]> {
    return (await this.request<{ bindings: AgentBinding[] }>("/v1/bindings")).bindings ?? [];
  }

  async endpoints(): Promise<AgentEndpoint[]> {
    return (await this.request<{ endpoints: AgentEndpoint[] }>("/v1/endpoints")).endpoints ?? [];
  }

  async apiKeys(): Promise<ApiKeyRecord[]> {
    return (await this.request<{ apiKeys: ApiKeyRecord[] }>("/v1/api-keys")).apiKeys ?? [];
  }

  async agentGateways(): Promise<AgentGateway[]> {
    return (await this.request<{ agentGateways: AgentGateway[] }>("/v1/agent-gateways")).agentGateways ?? [];
  }

  async availableAgents(): Promise<AvailableAgent[]> {
    return (await this.request<{ agents: AvailableAgent[] }>("/v1/agents")).agents ?? [];
  }

  async providerAdapters(): Promise<ProviderAdapter[]> {
    return (await this.request<{ providerAdapters: ProviderAdapter[] }>("/v1/provider-adapters")).providerAdapters ?? [];
  }

  async providerCatalog(): Promise<ProviderCatalog> {
    const catalog = await this.request<Partial<ProviderCatalog>>("/v1/provider-catalog");
    return { registry: catalog.registry ?? [], custom: catalog.custom ?? [] };
  }

  async projectProviders(slug: string): Promise<ProjectProvider[]> {
    return (await this.request<{ providers: ProjectProvider[] }>(`/v1/project/${encodeURIComponent(slug)}/providers`)).providers ?? [];
  }

  async projectAgentCandidates(slug: string): Promise<ProjectAgentCandidate[]> {
    return (await this.request<{ candidates: ProjectAgentCandidate[] }>(`/v1/project/${encodeURIComponent(slug)}/agent-candidates`)).candidates ?? [];
  }

  async traces(): Promise<EndpointTrace[]> {
    return (await this.request<{ traces: EndpointTrace[] }>("/v1/traces?limit=100")).traces ?? [];
  }

  async request<T>(path: string, init: RequestInit = {}, publicRequest = false): Promise<T> {
    const headers = new Headers(init.headers);
    headers.set("accept", headers.get("accept") ?? "application/json");
    if (init.body && !headers.has("content-type")) headers.set("content-type", "application/json");
    if (!publicRequest && this.credential) headers.set("authorization", `Bearer ${this.credential}`);
    const response = await this.fetcher(appendApiPath(this.apiBaseUrl, path), {
      ...init,
      headers,
      cache: init.cache ?? "no-store",
    });
    if (response.status === 204) return undefined as T;
    const payload = await response.json().catch(() => null) as T | { error?: unknown } | null;
    if (!response.ok) throw new VifuHttpError(response.status, readErrorMessage(payload), payload);
    return (payload ?? {}) as T;
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
