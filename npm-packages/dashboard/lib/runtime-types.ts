export type DeploymentMode = "local" | "self-hosted" | "cloud";

export type ServerCapabilities = {
  profiles: boolean;
  endpoints: boolean;
  bindings: boolean;
  apiKeys: boolean;
  connections: boolean;
  traces: boolean;
  websocketRelay: boolean;
  account: boolean;
  teams: boolean;
  billing: boolean;
  managedDomains: boolean;
};

export type DeploymentStatus = {
  service: string;
  status: string;
  version: string;
  mode: DeploymentMode;
  capabilities: ServerCapabilities;
  connections: number;
};

export type AgentProfile = {
  id: string;
  slug: string;
  name: string;
  description: string | null;
  instructions: string | null;
  createdAt: string;
  updatedAt: string;
};

export type AgentBinding = {
  id: string;
  profileId: string;
  provider: string;
  connectorId: string;
  agentId: string;
  config: Record<string, unknown>;
  createdAt: string;
  updatedAt: string;
};

export type AgentEndpoint = {
  id: string;
  slug: string;
  name: string;
  profileId: string;
  bindingId: string;
  enabled: boolean;
  requestTimeoutMs: number;
  createdAt: string;
  updatedAt: string;
};

export type ApiKeyRecord = {
  id: string;
  endpointId: string;
  name: string;
  keyPrefix: string;
  key?: string;
  createdAt: string;
  revokedAt: string | null;
};

export type ConnectorSession = {
  id: string;
  connectorId: string;
  sessionId: string;
  status: string;
  agents: Array<{ id?: string; name?: string }>;
  metadata: Record<string, unknown>;
  connectedAt: string;
  lastSeenAt: string;
  disconnectedAt: string | null;
};

export type EndpointTrace = {
  id: string;
  requestId: string;
  endpointId: string;
  connectorSessionId: string | null;
  status: string;
  latencyMs: number | null;
  request: Record<string, unknown>;
  response: unknown;
  error: string | null;
  createdAt: string;
  completedAt: string | null;
};

export type RuntimeSnapshot = {
  profiles: AgentProfile[];
  bindings: AgentBinding[];
  endpoints: AgentEndpoint[];
  apiKeys: ApiKeyRecord[];
  connections: ConnectorSession[];
  traces: EndpointTrace[];
};
