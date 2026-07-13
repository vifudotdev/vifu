export type DeploymentMode = "local" | "self-hosted";
export type AuthMode = "none" | "local-password" | "oidc";
export type AuthProviderKind = "password" | "oidc";

export type AuthProvider = {
  id: string;
  kind: AuthProviderKind;
  label: string;
};

export type ServerCapabilities = {
  projects: boolean;
  profiles: boolean;
  endpoints: boolean;
  bindings: boolean;
  apiKeys: boolean;
  agentGateways: boolean;
  traces: boolean;
  jsonRpc: boolean;
};

export type AuthCapability = {
  required?: boolean;
  mode: AuthMode;
  signupEnabled: boolean;
  providers?: AuthProvider[];
};

export type Principal = {
  userId: string;
  email: string;
  displayName?: string | null;
  roles: string[];
  provider: "local" | "oidc";
};

export type AuthenticatedSession = {
  principal: Principal;
  session: {
    token: string;
    expiresAt: string;
  };
};

export type RuntimeProject = {
  id: string;
  slug: string;
  name: string;
  description: string | null;
  gatewayId: string;
  enabled: boolean;
  publishableKeyPrefix: string;
  bindingIds: string[];
  publishableKey?: string;
  createdAt: string;
  updatedAt: string;
};

export type RuntimeStatus = {
  service: string;
  status: string;
  version: string;
  mode: DeploymentMode;
  capabilities: ServerCapabilities;
  agentGateways: number;
};

export type DeploymentStatus = RuntimeStatus & {
  auth: AuthCapability;
};

export type AgentProfile = {
  id: string;
  slug: string;
  name: string;
  description: string | null;
  createdAt: string;
  updatedAt: string;
};

export type AgentBinding = {
  id: string;
  profileId: string;
  provider: string;
  gatewayId: string;
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

export type AgentGateway = {
  id: string;
  gatewayId: string;
  sessionId: string;
  status: string;
  agents: Array<{ id?: string; name?: string }>;
  metadata: Record<string, unknown>;
  connectedAt: string;
  lastSeenAt: string;
  disconnectedAt: string | null;
};

export type AvailableAgent = {
  gatewayId: string;
  id: string;
  name: string;
  status: string;
  metadata: Record<string, unknown>;
};

export type EndpointTrace = {
  id: string;
  requestId: string;
  endpointId: string | null;
  projectId: string | null;
  gatewaySessionId: string | null;
  status: string;
  latencyMs: number | null;
  request: Record<string, unknown>;
  response: unknown;
  error: string | null;
  createdAt: string;
  completedAt: string | null;
};

export type RuntimeSnapshot = {
  projects: RuntimeProject[];
  profiles: AgentProfile[];
  bindings: AgentBinding[];
  endpoints: AgentEndpoint[];
  apiKeys: ApiKeyRecord[];
  agentGateways: AgentGateway[];
  availableAgents: AvailableAgent[];
  traces: EndpointTrace[];
};
