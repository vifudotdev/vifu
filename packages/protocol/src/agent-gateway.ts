import type { JsonSchema, JsonValue } from "./json.js";
import {
  NonEmptyStringSchema,
  type EventFrame,
  type RequestFrame,
  type ResponseFrame,
} from "./frame.js";

export const VIFU_AGENT_GATEWAY_PROTOCOL_VERSION = "vifu.agent-gateway/1" as const;

export const AGENT_GATEWAY_HELLO_REQUEST_ID = "gateway.hello" as const;

export const VIFU_AGENT_GATEWAY_METHODS = {
  HELLO: "gateway.hello",
  INVOKE: "agent.invoke",
} as const;

export const VIFU_AGENT_GATEWAY_EVENTS = {
  CHALLENGE: "gateway.challenge",
  PAIRING_REQUIRED: "gateway.pairingRequired",
  CANCEL: "agent.cancel",
  HEARTBEAT: "gateway.heartbeat",
  HEARTBEAT_ACK: "gateway.heartbeatAck",
  ERROR: "gateway.error",
} as const;

export type VifuAgentGatewayMethod =
  (typeof VIFU_AGENT_GATEWAY_METHODS)[keyof typeof VIFU_AGENT_GATEWAY_METHODS];

export type VifuAgentGatewayEvent =
  (typeof VIFU_AGENT_GATEWAY_EVENTS)[keyof typeof VIFU_AGENT_GATEWAY_EVENTS];

export interface AgentGatewayAgentDescriptor {
  id: string;
  name: string;
  metadata?: JsonValue;
}

export interface AgentGatewayHelloParams {
  protocol: typeof VIFU_AGENT_GATEWAY_PROTOCOL_VERSION;
  resumeSessionId?: string;
  agents: AgentGatewayAgentDescriptor[];
  metadata?: JsonValue;
  machine: AgentGatewayMachineProof;
  auth?: AgentGatewayHelloAuth;
  followup?: string;
}

export interface AgentGatewayMachineProof {
  id: string;
  publicKey: string;
  signature: string;
  signedAt: number;
}

export interface AgentGatewayHelloAuth {
  deviceToken?: string;
}

export interface AgentGatewayWelcomeAuth {
  deviceToken: string;
  generation: number;
  expiresAt: string;
}

export interface AgentGatewayWelcomePayload {
  gatewayId: string;
  connectionId: string;
  sessionId: string;
  heartbeatIntervalMs: number;
  resumed: boolean;
  auth?: AgentGatewayWelcomeAuth;
}

export interface AgentGatewayChallengePayload {
  nonce: string;
  timestamp: number;
  audience: string;
}

export interface AgentGatewayPairingRequiredPayload {
  requestId: string;
  authUrl: string;
  retryable: boolean;
  recommendedNextStep: string;
  retryAfterMs: number;
}

export interface AgentGatewayInvokeParams {
  channelId: number;
  endpointId: string;
  profileId: string;
  bindingId: string;
  agentId: string;
  binding: JsonValue;
  input: JsonValue;
  timeoutMs: number;
}

export interface AgentGatewayInvokeResultPayload {
  channelId: number;
  output: JsonValue;
}

export interface AgentGatewayInvokeErrorDetails {
  channelId: number;
}

export interface AgentGatewayCancelPayload {
  requestId: string;
  channelId: number;
}

export interface AgentGatewayHeartbeatPayload {
  sessionId: string;
}

export interface AgentGatewayErrorPayload {
  code: string;
  message: string;
}

export type AgentGatewayHelloRequestFrame = RequestFrame<
  typeof VIFU_AGENT_GATEWAY_METHODS.HELLO,
  AgentGatewayHelloParams
>;

export type AgentGatewayWelcomeResponseFrame = ResponseFrame<
  AgentGatewayWelcomePayload
>;

export type AgentGatewayChallengeEventFrame = EventFrame<
  typeof VIFU_AGENT_GATEWAY_EVENTS.CHALLENGE,
  AgentGatewayChallengePayload
>;

export type AgentGatewayPairingRequiredEventFrame = EventFrame<
  typeof VIFU_AGENT_GATEWAY_EVENTS.PAIRING_REQUIRED,
  AgentGatewayPairingRequiredPayload
>;

export type AgentGatewayInvokeRequestFrame = RequestFrame<
  typeof VIFU_AGENT_GATEWAY_METHODS.INVOKE,
  AgentGatewayInvokeParams
>;

export type AgentGatewayInvokeResultResponseFrame = ResponseFrame<
  AgentGatewayInvokeResultPayload,
  AgentGatewayInvokeErrorDetails
>;

export type AgentGatewayCancelEventFrame = EventFrame<
  typeof VIFU_AGENT_GATEWAY_EVENTS.CANCEL,
  AgentGatewayCancelPayload
>;

export type AgentGatewayHeartbeatEventFrame = EventFrame<
  typeof VIFU_AGENT_GATEWAY_EVENTS.HEARTBEAT,
  AgentGatewayHeartbeatPayload
>;

export type AgentGatewayHeartbeatAckEventFrame = EventFrame<
  typeof VIFU_AGENT_GATEWAY_EVENTS.HEARTBEAT_ACK,
  AgentGatewayHeartbeatPayload
>;

export type AgentGatewayErrorEventFrame = EventFrame<
  typeof VIFU_AGENT_GATEWAY_EVENTS.ERROR,
  AgentGatewayErrorPayload
>;

const UuidStringSchema = {
  type: "string",
  minLength: 1,
} as const satisfies JsonSchema;

export const AgentGatewayAgentDescriptorSchema = {
  type: "object",
  required: ["id", "name"],
  additionalProperties: false,
  properties: {
    id: NonEmptyStringSchema,
    name: NonEmptyStringSchema,
    metadata: {},
  },
} as const satisfies JsonSchema;

export const AgentGatewayHelloParamsSchema = {
  type: "object",
  required: ["protocol", "agents", "machine"],
  additionalProperties: false,
  properties: {
    protocol: { const: VIFU_AGENT_GATEWAY_PROTOCOL_VERSION },
    resumeSessionId: UuidStringSchema,
    agents: {
      type: "array",
      items: AgentGatewayAgentDescriptorSchema,
    },
    metadata: {},
    machine: {
      type: "object",
      required: ["id", "publicKey", "signature", "signedAt"],
      additionalProperties: false,
      properties: {
        id: NonEmptyStringSchema,
        publicKey: NonEmptyStringSchema,
        signature: NonEmptyStringSchema,
        signedAt: { type: "integer", minimum: 1 },
      },
    },
    auth: {
      type: "object",
      additionalProperties: false,
      properties: { deviceToken: NonEmptyStringSchema },
    },
    followup: NonEmptyStringSchema,
  },
} as const satisfies JsonSchema;

export const AgentGatewayWelcomePayloadSchema = {
  type: "object",
  required: ["gatewayId", "connectionId", "sessionId", "heartbeatIntervalMs", "resumed"],
  additionalProperties: false,
  properties: {
    gatewayId: NonEmptyStringSchema,
    connectionId: UuidStringSchema,
    sessionId: UuidStringSchema,
    heartbeatIntervalMs: {
      type: "integer",
      minimum: 1000,
      maximum: 60000,
    },
    resumed: { type: "boolean" },
    auth: {
      type: "object",
      required: ["deviceToken", "generation", "expiresAt"],
      additionalProperties: false,
      properties: {
        deviceToken: NonEmptyStringSchema,
        generation: { type: "integer", minimum: 1 },
        expiresAt: NonEmptyStringSchema,
      },
    },
  },
} as const satisfies JsonSchema;

export const AgentGatewayChallengePayloadSchema = {
  type: "object",
  required: ["nonce", "timestamp", "audience"],
  additionalProperties: false,
  properties: {
    nonce: NonEmptyStringSchema,
    timestamp: { type: "integer", minimum: 1 },
    audience: NonEmptyStringSchema,
  },
} as const satisfies JsonSchema;

export const AgentGatewayPairingRequiredPayloadSchema = {
  type: "object",
  required: ["requestId", "authUrl", "retryable", "recommendedNextStep", "retryAfterMs"],
  additionalProperties: false,
  properties: {
    requestId: UuidStringSchema,
    authUrl: NonEmptyStringSchema,
    retryable: { type: "boolean" },
    recommendedNextStep: NonEmptyStringSchema,
    retryAfterMs: { type: "integer", minimum: 250, maximum: 60000 },
  },
} as const satisfies JsonSchema;

export const AgentGatewayInvokeParamsSchema = {
  type: "object",
  required: [
    "channelId",
    "endpointId",
    "profileId",
    "bindingId",
    "agentId",
    "binding",
    "input",
    "timeoutMs",
  ],
  additionalProperties: false,
  properties: {
    channelId: { type: "integer", minimum: 1 },
    endpointId: UuidStringSchema,
    profileId: UuidStringSchema,
    bindingId: UuidStringSchema,
    agentId: NonEmptyStringSchema,
    binding: {},
    input: {},
    timeoutMs: {
      type: "integer",
      minimum: 500,
      maximum: 120000,
    },
  },
} as const satisfies JsonSchema;

export const AgentGatewayInvokeResultPayloadSchema = {
  type: "object",
  required: ["channelId", "output"],
  additionalProperties: false,
  properties: {
    channelId: { type: "integer", minimum: 1 },
    output: {},
  },
} as const satisfies JsonSchema;

export const AgentGatewayInvokeErrorDetailsSchema = {
  type: "object",
  required: ["channelId"],
  additionalProperties: false,
  properties: {
    channelId: { type: "integer", minimum: 1 },
  },
} as const satisfies JsonSchema;

export const AgentGatewayCancelPayloadSchema = {
  type: "object",
  required: ["requestId", "channelId"],
  additionalProperties: false,
  properties: {
    requestId: UuidStringSchema,
    channelId: { type: "integer", minimum: 1 },
  },
} as const satisfies JsonSchema;

export const AgentGatewayHeartbeatPayloadSchema = {
  type: "object",
  required: ["sessionId"],
  additionalProperties: false,
  properties: {
    sessionId: UuidStringSchema,
  },
} as const satisfies JsonSchema;

export const AgentGatewayErrorPayloadSchema = {
  type: "object",
  required: ["code", "message"],
  additionalProperties: false,
  properties: {
    code: NonEmptyStringSchema,
    message: NonEmptyStringSchema,
  },
} as const satisfies JsonSchema;
