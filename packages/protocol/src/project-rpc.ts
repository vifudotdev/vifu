import type { JsonSchema, JsonValue } from "./json.js";
import type { JsonRpcRequest } from "./jsonrpc.js";

export const VIFU_PROJECT_RPC_METHODS = {
  RPC_DISCOVER: "rpc.discover",
  AGENT_LIST: "agent.list",
  AGENT_INVOKE: "agent.invoke",
} as const;

export const VIFU_PROJECT_RPC_METHOD_NAMES = [
  VIFU_PROJECT_RPC_METHODS.RPC_DISCOVER,
  VIFU_PROJECT_RPC_METHODS.AGENT_LIST,
  VIFU_PROJECT_RPC_METHODS.AGENT_INVOKE,
] as const;

export type VifuProjectRpcMethod =
  (typeof VIFU_PROJECT_RPC_METHOD_NAMES)[number];

export const VIFU_PROJECT_PROTOCOL_NAME = "vifu.project" as const;
export const VIFU_PROJECT_PROTOCOL_VERSION = "0.1" as const;

export const VIFU_PROJECT_CAPABILITIES = [
  "agent.list",
  "agent.invoke",
] as const;

export type VifuProjectCapability =
  (typeof VIFU_PROJECT_CAPABILITIES)[number];

export interface AgentDescriptor {
  id: string;
  name: string;
  agentId: string;
  bindingId: string;
}

export interface AgentListResult {
  agents: AgentDescriptor[];
}

export interface AgentInvokeParams {
  agent?: string;
  message?: string;
  input?: JsonValue;
  context?: JsonValue;
  metadata?: JsonValue;
  timeoutMs?: number;
}

export type AgentInvokeResult = JsonValue;

export type EmptyParams = readonly [] | Record<string, never>;

export type RpcDiscoverRequest = JsonRpcRequest<
  typeof VIFU_PROJECT_RPC_METHODS.RPC_DISCOVER,
  EmptyParams
>;

export type AgentListRequest = JsonRpcRequest<
  typeof VIFU_PROJECT_RPC_METHODS.AGENT_LIST,
  EmptyParams
>;

export type AgentInvokeRequest = JsonRpcRequest<
  typeof VIFU_PROJECT_RPC_METHODS.AGENT_INVOKE,
  AgentInvokeParams
>;

export type VifuProjectRpcRequest =
  | RpcDiscoverRequest
  | AgentListRequest
  | AgentInvokeRequest;

export const EmptyParamsSchema = {
  oneOf: [
    { type: "object", maxProperties: 0 },
    { type: "array", maxItems: 0 },
  ],
} as const satisfies JsonSchema;

export const AgentDescriptorSchema = {
  type: "object",
  required: ["id", "name", "agentId", "bindingId"],
  properties: {
    id: { type: "string" },
    name: { type: "string" },
    agentId: { type: "string" },
    bindingId: { type: "string" },
  },
} as const satisfies JsonSchema;

export const AgentListResultSchema = {
  type: "object",
  required: ["agents"],
  properties: {
    agents: {
      type: "array",
      items: AgentDescriptorSchema,
    },
  },
} as const satisfies JsonSchema;

export const AgentInvokeParamsSchema = {
  type: "object",
  properties: {
    agent: { type: "string" },
    message: { type: "string" },
    input: {},
    context: {},
    metadata: {},
    timeoutMs: {
      type: "integer",
      minimum: 500,
      maximum: 120000,
    },
  },
} as const satisfies JsonSchema;

export const AgentInvokeResultSchema = {} as const satisfies JsonSchema;

export const ProjectDiscoverPayloadSchema = {
  type: "object",
  required: ["project", "protocol", "transports", "capabilities"],
  properties: {
    project: {
      type: "object",
      required: ["id", "slug", "gatewayId"],
      properties: {
        id: { type: "string" },
        slug: { type: "string" },
        gatewayId: { type: "string" },
      },
    },
    protocol: {
      type: "object",
      required: ["name", "version", "methods"],
      properties: {
        name: { const: VIFU_PROJECT_PROTOCOL_NAME },
        version: { const: VIFU_PROJECT_PROTOCOL_VERSION },
        methods: {
          type: "array",
          items: { enum: VIFU_PROJECT_RPC_METHOD_NAMES },
        },
      },
    },
    transports: {
      type: "object",
      required: ["http", "websocket", "jsonrpc", "websocketProtocol"],
      properties: {
        http: { type: "string" },
        websocket: { type: "string" },
        jsonrpc: { const: "2.0" },
        websocketProtocol: { const: "jsonrpc" },
      },
    },
    capabilities: {
      type: "array",
      items: { enum: VIFU_PROJECT_CAPABILITIES },
    },
  },
} as const satisfies JsonSchema;

export const RpcDiscoverResultSchema = ProjectDiscoverPayloadSchema;

export const VifuProjectRpcParamsSchemas = {
  [VIFU_PROJECT_RPC_METHODS.RPC_DISCOVER]: EmptyParamsSchema,
  [VIFU_PROJECT_RPC_METHODS.AGENT_LIST]: EmptyParamsSchema,
  [VIFU_PROJECT_RPC_METHODS.AGENT_INVOKE]: AgentInvokeParamsSchema,
} as const satisfies Record<VifuProjectRpcMethod, JsonSchema>;

export const VifuProjectRpcResultSchemas = {
  [VIFU_PROJECT_RPC_METHODS.RPC_DISCOVER]: RpcDiscoverResultSchema,
  [VIFU_PROJECT_RPC_METHODS.AGENT_LIST]: AgentListResultSchema,
  [VIFU_PROJECT_RPC_METHODS.AGENT_INVOKE]: AgentInvokeResultSchema,
} as const satisfies Record<VifuProjectRpcMethod, JsonSchema>;

export function isVifuProjectRpcMethod(method: string): method is VifuProjectRpcMethod {
  return (VIFU_PROJECT_RPC_METHOD_NAMES as readonly string[]).includes(method);
}

export function createAgentInvokeRequest(
  id: string | number,
  params: AgentInvokeParams,
): AgentInvokeRequest {
  return {
    jsonrpc: "2.0",
    id,
    method: VIFU_PROJECT_RPC_METHODS.AGENT_INVOKE,
    params,
  };
}

export function createAgentListRequest(id: string | number): AgentListRequest {
  return {
    jsonrpc: "2.0",
    id,
    method: VIFU_PROJECT_RPC_METHODS.AGENT_LIST,
  };
}

export function createRpcDiscoverRequest(id: string | number): RpcDiscoverRequest {
  return {
    jsonrpc: "2.0",
    id,
    method: VIFU_PROJECT_RPC_METHODS.RPC_DISCOVER,
  };
}

export type ProjectDiscoverPayload = {
  project: {
    id: string;
    slug: string;
    gatewayId: string;
  };
  protocol: {
    name: typeof VIFU_PROJECT_PROTOCOL_NAME;
    version: typeof VIFU_PROJECT_PROTOCOL_VERSION;
    methods: VifuProjectRpcMethod[];
  };
  transports: {
    http: string;
    websocket: string;
    jsonrpc: "2.0";
    websocketProtocol: "jsonrpc";
  };
  capabilities: VifuProjectCapability[];
};

export interface BuildProjectDiscoverPayloadOptions {
  project: {
    id: string;
    slug: string;
    gatewayId: string;
  };
  httpUrl: string;
  websocketUrl: string;
}

export function buildProjectDiscoverPayload({
  project,
  httpUrl,
  websocketUrl,
}: BuildProjectDiscoverPayloadOptions): ProjectDiscoverPayload {
  return {
    project,
    protocol: {
      name: VIFU_PROJECT_PROTOCOL_NAME,
      version: VIFU_PROJECT_PROTOCOL_VERSION,
      methods: [...VIFU_PROJECT_RPC_METHOD_NAMES],
    },
    transports: {
      http: httpUrl,
      websocket: websocketUrl,
      jsonrpc: "2.0",
      websocketProtocol: "jsonrpc",
    },
    capabilities: [...VIFU_PROJECT_CAPABILITIES],
  };
}
