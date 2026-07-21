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
  canvas: boolean;
  agentGateways: boolean;
  providerConnections: boolean;
  traces: boolean;
  gameRuntime: boolean;
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
  bindingIds: string[];
  createdAt: string;
  updatedAt: string;
};

export type ProjectCanvasNode = {
  id: string;
  projectId: string;
  kind: string;
  position: Record<string, unknown>;
  profileId: string | null;
  bindingId: string | null;
  gatewayId: string | null;
  resourceId: string | null;
  config: Record<string, unknown>;
  inputs: Record<string, unknown>;
  outputs: Record<string, unknown>;
  exposed: boolean;
  createdAt: string;
  updatedAt: string;
};

export type ProjectCanvasEdge = {
  id: string;
  projectId: string;
  sourceNodeId: string;
  sourceHandle: string | null;
  targetNodeId: string;
  targetHandle: string | null;
  kind: string;
  config: Record<string, unknown>;
  createdAt: string;
  updatedAt: string;
};

export type ProjectCanvas = {
  project: RuntimeProject;
  nodes: ProjectCanvasNode[];
  edges: ProjectCanvasEdge[];
};

export type GameMetadata = {
  name: string;
  description?: string | null;
  tags: string[];
};

export type GamePortReference = {
  nodeId: string;
  port: string;
};

export type GameSourceNode = {
  id: string;
  type: string;
  version: number;
  config: Record<string, unknown>;
  parentId?: string | null;
  label?: string | null;
  notes?: string | null;
};

export type GameSourceEdge = {
  id: string;
  source: GamePortReference;
  target: GamePortReference;
  condition?: Record<string, unknown> | null;
  managedBy?: string | null;
};

export type GameAgentReference = {
  id: string;
  profileId: string;
  profileVersionId?: string | null;
  capabilities: string[];
  executionDescriptor: Record<string, unknown>;
};

export type GameCharacter = {
  id: string;
  nameMessageId: string;
  roleMessageId?: string | null;
  agentId?: string | null;
  portraitResourceId?: string | null;
  player: boolean;
};

export type GameTranslationPack = {
  sourceHash: string;
  status: "draft" | "reviewed";
  messages: Record<string, string>;
};

export type GameLocalization = {
  sourceLocale: string;
  defaultLocale: string;
  targetLocales: string[];
  sourceMessages: Record<string, string>;
  packs: Record<string, GameTranslationPack>;
};

export type GameResourceReference = {
  id: string;
  versionId: string;
  kind: string;
  contentHash: string;
  approved: boolean;
};

export type GamePresentationResource = {
  id: string;
  kind: string;
  requiredCapabilities: string[];
  required: boolean;
  fallback?: unknown;
};

export type GameSource = {
  schemaVersion: number;
  metadata: GameMetadata;
  entryNodeId: string;
  graph: {
    nodes: GameSourceNode[];
    edges: GameSourceEdge[];
  };
  inputs: Record<string, unknown>;
  outputs: Record<string, unknown>;
  variables: Array<{ id: string; initialValue: unknown; public: boolean }>;
  agents: GameAgentReference[];
  characters: GameCharacter[];
  resources: GameResourceReference[];
  presentationResources: GamePresentationResource[];
  localization: GameLocalization;
  views: Record<string, unknown>;
};

export type GameDraft = {
  projectId: string;
  source: GameSource;
  revision: number;
  contentHash: string;
  createdAt: string;
  updatedAt: string;
};

export type GameReleaseSummary = {
  id: string;
  releaseNumber: number;
  sourceRevision: number;
  contentHash: string;
  createdAt: string;
};

export type GameOverview = {
  projectId: string;
  projectSlug: string;
  draftRevision: number;
  draftHash: string;
  activeRelease: GameReleaseSummary | null;
  unpublishedChanges: boolean;
};

export type GameNodePortDefinition = {
  name: string;
  direction: "input" | "output";
  valueSchema: Record<string, unknown>;
  required: boolean;
};

export type GameNodeDefinition = {
  type: string;
  version: number;
  phase: "production" | "runtime";
  title: string;
  category: string;
  configSchema: Record<string, unknown>;
  ports: GameNodePortDefinition[];
  dynamicInputs: boolean;
  dynamicOutputs: boolean;
  timelineCompatible: boolean;
};

export type GameValidationIssue = {
  severity: "error" | "warning";
  code: string;
  message: string;
  nodeId?: string | null;
  edgeId?: string | null;
  path?: string | null;
};

export type GameBackendResourceSnapshot = {
  id: string;
  versionId: string;
  version: number;
  kind: string;
  contentHash: string;
  content: unknown;
};

export type GameRelease = GameReleaseSummary & {
  projectId: string;
  plan: Record<string, unknown>;
  manifest: Record<string, unknown>;
  backendResources: GameBackendResourceSnapshot[];
  changeSummary: string | null;
};

export type GameResource = {
  id: string;
  projectId: string;
  resourceKey: string;
  name: string;
  kind: string;
  content: unknown;
  version: number;
  contentHash: string;
  approved: boolean;
  createdAt: string;
  updatedAt: string;
};

export type GameAssetVersion = {
  id: string;
  projectId: string;
  assetId: string;
  contentHash: string;
  mimeType: string;
  sizeBytes: number;
  metadata: Record<string, unknown>;
  provenance: Record<string, unknown>;
  rightsStatus: string;
  approvalStatus: "pending" | "approved" | "rejected";
  createdAt: string;
};

export type GameAsset = {
  id: string;
  projectId: string;
  assetKey: string;
  name: string;
  kind: string;
  createdAt: string;
  updatedAt: string;
  versions: GameAssetVersion[];
};

export type GameBuild = {
  id: string;
  projectId: string;
  sourceRevision: number;
  kind: string;
  status: "queued" | "running" | "completed" | "failed" | "cancelled";
  inputHash: string;
  input: Record<string, unknown>;
  output: Record<string, unknown> | null;
  error: Record<string, unknown> | null;
  attempts: number;
  createdAt: string;
  startedAt: string | null;
  completedAt: string | null;
};

export type GameQa = {
  draftRevision: number;
  ready: boolean;
  blockerCount: number;
  warningCount: number;
  issues: GameValidationIssue[];
  requiredHostCapabilities: string[];
  optionalHostCapabilities: string[];
};

export type GameEvent = {
  specversion: string;
  id: string;
  source: string;
  type: string;
  subject?: string | null;
  sequence: number;
  data: unknown;
};

export type GameSnapshot = {
  schemaVersion: number;
  status: "running" | "waiting_input" | "waiting_effect" | "waiting_host" | "completed" | "failed" | "cancelled";
  revision: number;
  locale: string;
  state: Record<string, unknown>;
  pendingHostAction?: {
    actionId: string;
    nodeId: string;
    target: string;
    action: string;
    arguments: unknown;
  } | null;
  publicOutput?: unknown;
  failure?: { code: string; message: string; nodeId?: string | null } | null;
};

export type GameAdvance = {
  snapshot: GameSnapshot;
  events: GameEvent[];
  effects: Array<Record<string, unknown>>;
  nodeExecutions?: Array<{
    sequence: number;
    ordinal: number;
    nodeId: string;
    nodeType: string;
  }>;
};

export type PublicGameAdvance = {
  status: GameSnapshot["status"];
  revision: number;
  publicOutput?: unknown;
  outstandingHostActions: NonNullable<GameSnapshot["pendingHostAction"]>[];
  failure?: GameSnapshot["failure"];
  events: GameEvent[];
};

export type GameSession = {
  id: string;
  projectId: string;
  gameReleaseId: string | null;
  sourceRevision: number | null;
  preview: boolean;
  status: GameSnapshot["status"];
  revision: number;
  snapshot: GameSnapshot;
  host: Record<string, unknown>;
  publicOutput: unknown;
  failure: unknown;
  createdAt: string;
  updatedAt: string;
  completedAt: string | null;
};

export type GameAnalytics = {
  totalSessions: number;
  averageDurationMs: number;
  events: Array<{ eventType: string; count: number }>;
  sessionStatuses: Array<{ status: string; count: number }>;
  recentSessions: GameSession[];
};

export type ProviderAdapterField = {
  key: string;
  label: string;
  kind: string;
  required: boolean;
  secret: boolean;
};

export type ProviderAdapter = {
  id: string;
  category: "local" | "cloud" | "custom" | string;
  name: string;
  description: string;
  capabilities: ProfileCapabilityKind[];
  executionModes: Array<"gateway" | "server" | string>;
  supportsDiscovery: boolean;
  fields: ProviderAdapterField[];
};

export type CustomProvider = {
  id: string;
  providerKey: string;
  name: string;
  providerType: string;
  baseUrl: string;
  config: Record<string, unknown>;
  secretKeys: string[];
  displaySecret: string | null;
  status: string;
  lastCheckedAt: string | null;
  createdAt: string;
  updatedAt: string;
};

export type ProjectProvider = CustomProvider & {
  projectId: string;
  sourceKind: "registry" | "custom";
  sourceKey: string;
};

export type ProviderCatalog = {
  registry: ProviderAdapter[];
  custom: CustomProvider[];
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
  projectId: string | null;
  slug: string;
  name: string;
  description: string | null;
  activeVersionId: string | null;
  archivedAt: string | null;
  createdAt: string;
  updatedAt: string;
};

export type AgentProfileVersion = {
  id: string;
  profileId: string;
  versionNumber: number;
  persona: Record<string, unknown>;
  runtime: Record<string, unknown>;
  presentation: Record<string, unknown>;
  source: Record<string, unknown>;
  contentHash: string;
  changeSummary: string | null;
  archivedAt: string | null;
  createdAt: string;
};

export type ProfileCapabilityKind = "chat" | "speech" | "transcription" | "realtime" | "tool";

export type AgentProfileCapability = {
  id: string;
  profileVersionId: string;
  kind: ProfileCapabilityKind;
  providerType: string;
  providerKey: string;
  resourceId: string | null;
  config: Record<string, unknown>;
  inputSchema: Record<string, unknown>;
  outputSchema: Record<string, unknown>;
  createdAt: string;
};

export type ProfileVersionWithCapabilities = {
  version: AgentProfileVersion;
  capabilities: AgentProfileCapability[];
};

export type AgentProfileRollout = {
  profileId: string;
  profileVersionId: string;
  weightBps: number;
  createdAt: string;
  updatedAt: string;
};

export type AgentProfileDetail = {
  profile: AgentProfile;
  versions: ProfileVersionWithCapabilities[];
  rollout: AgentProfileRollout[];
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

export type ApiKeyAgentScope =
  | { mode: "all" }
  | { mode: "selected"; profileIds: string[] };

export type ApiKeyPermissions = {
  chatCompletions: "none" | "access";
  speech: "none" | "access";
  transcriptions: "none" | "access";
  realtime: "none" | "access";
  agents: "none" | "read" | "write";
  project: "none" | "read" | "write";
  game: "none" | "execute";
};

export type ApiKeyRecord = {
  id: string;
  projectId: string;
  name: string;
  agentScope: ApiKeyAgentScope;
  permissions: ApiKeyPermissions;
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

export type ProjectAgentCandidate = {
  profileId: string | null;
  gatewayId: string;
  id: string;
  name: string;
  status: string;
  providerKey: string;
  providerType: string;
  metadata: Record<string, unknown>;
};

export type EndpointTrace = {
  id: string;
  requestId: string;
  endpointId: string | null;
  projectId: string | null;
  gatewaySessionId: string | null;
  profileId: string | null;
  profileVersionId: string | null;
  profileSlug: string | null;
  profileName: string | null;
  profileVersionNumber: number | null;
  operation: string;
  providerKey: string | null;
  capabilityKind: string | null;
  selectionKey: string | null;
  status: string;
  latencyMs: number | null;
  request: Record<string, unknown>;
  response: unknown;
  error: string | null;
  createdAt: string;
  completedAt: string | null;
};

export type TraceSpan = {
  id: string;
  traceId: string;
  parentSpanId: string | null;
  name: string;
  kind: string;
  status: string;
  providerKey: string | null;
  capabilityKind: string | null;
  durationMs: number | null;
  inputSummary: unknown;
  outputSummary: unknown;
  attributes: Record<string, unknown>;
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
  providerAdapters: ProviderAdapter[];
  traces: EndpointTrace[];
};
