import type { JsonSchema, JsonValue } from "./json.js";
import {
  NonEmptyStringSchema,
  type EventFrame,
  type RequestFrame,
  type ResponseFrame,
} from "./frame.js";

export const VIFU_RUNTIME_BRIDGE_PROTOCOL_VERSION = "vifu.runtime-bridge/1" as const;

export const VIFU_RUNTIME_BRIDGE_METHODS = {
  HELLO: "runtime.hello",
  INVOKE: "runtime.invoke",
  CANCEL: "runtime.cancel",
} as const;

export const VIFU_RUNTIME_BRIDGE_EVENTS = {
  STARTED: "runtime.invocation.started",
  OUTPUT_DELTA: "runtime.invocation.outputDelta",
  COMPLETED: "runtime.invocation.completed",
  FAILED: "runtime.invocation.failed",
  CANCELLED: "runtime.invocation.cancelled",
} as const;

export type RuntimeBridgeMethod =
  (typeof VIFU_RUNTIME_BRIDGE_METHODS)[keyof typeof VIFU_RUNTIME_BRIDGE_METHODS];

export type RuntimeBridgeEvent =
  (typeof VIFU_RUNTIME_BRIDGE_EVENTS)[keyof typeof VIFU_RUNTIME_BRIDGE_EVENTS];

export type RuntimeInvocationData =
  | {
      format: "json";
      value: JsonValue;
    }
  | {
      format: "binary";
      value: number[];
    };

export interface RuntimeBridgeHelloParams {
  protocol: typeof VIFU_RUNTIME_BRIDGE_PROTOCOL_VERSION;
}

export interface RuntimeBridgeHelloPayload {
  protocol: typeof VIFU_RUNTIME_BRIDGE_PROTOCOL_VERSION;
  projectId: string;
}

export interface RuntimeBridgeInvokeParams {
  endpoint: string;
  sessionId?: string;
  data?: RuntimeInvocationData;
  metadata?: JsonValue;
}

export interface RuntimeBridgeInvokePayload {
  handle: string;
}

export interface RuntimeBridgeCancelParams {
  handle: string;
}

export type RuntimeInvocationEventKind =
  | "started"
  | "outputDelta"
  | "completed"
  | "failed"
  | "cancelled";

export interface RuntimeInvocationEvent {
  sequence: number;
  kind: RuntimeInvocationEventKind;
  data?: RuntimeInvocationData;
  error?: string;
}

export interface RuntimeInvocationOutput {
  invocationId: string;
  projectId: string;
  endpoint: string;
  sessionId: string;
  agent: string;
  provider: string;
  capability: string;
  data: RuntimeInvocationData;
  metadata: JsonValue;
  snapshot: {
    revision: number;
    state: JsonValue;
  };
  trace: Array<{
    name: string;
    status: string;
    durationMs: number;
    attributes: JsonValue;
  }>;
}

export interface RuntimeBridgeInvocationEventPayload {
  handle: string;
  event: RuntimeInvocationEvent;
  output?: RuntimeInvocationOutput;
}

export type RuntimeBridgeHelloRequestFrame = RequestFrame<
  typeof VIFU_RUNTIME_BRIDGE_METHODS.HELLO,
  RuntimeBridgeHelloParams
>;

export type RuntimeBridgeHelloResponseFrame = ResponseFrame<RuntimeBridgeHelloPayload>;

export type RuntimeBridgeInvokeRequestFrame = RequestFrame<
  typeof VIFU_RUNTIME_BRIDGE_METHODS.INVOKE,
  RuntimeBridgeInvokeParams
>;

export type RuntimeBridgeInvokeResponseFrame = ResponseFrame<RuntimeBridgeInvokePayload>;

export type RuntimeBridgeCancelRequestFrame = RequestFrame<
  typeof VIFU_RUNTIME_BRIDGE_METHODS.CANCEL,
  RuntimeBridgeCancelParams
>;

export type RuntimeBridgeInvocationEventFrame = EventFrame<
  RuntimeBridgeEvent,
  RuntimeBridgeInvocationEventPayload
>;

export const RuntimeInvocationDataSchema = {
  oneOf: [
    {
      type: "object",
      required: ["format", "value"],
      additionalProperties: false,
      properties: {
        format: { const: "json" },
        value: {},
      },
    },
    {
      type: "object",
      required: ["format", "value"],
      additionalProperties: false,
      properties: {
        format: { const: "binary" },
        value: {
          type: "array",
          items: {
            type: "integer",
            minimum: 0,
            maximum: 255,
          },
        },
      },
    },
  ],
} as const satisfies JsonSchema;

export const RuntimeBridgeHelloParamsSchema = {
  type: "object",
  required: ["protocol"],
  additionalProperties: false,
  properties: {
    protocol: { const: VIFU_RUNTIME_BRIDGE_PROTOCOL_VERSION },
  },
} as const satisfies JsonSchema;

export const RuntimeBridgeInvokeParamsSchema = {
  type: "object",
  required: ["endpoint"],
  additionalProperties: false,
  properties: {
    endpoint: NonEmptyStringSchema,
    sessionId: NonEmptyStringSchema,
    data: RuntimeInvocationDataSchema,
    metadata: {},
  },
} as const satisfies JsonSchema;

export const RuntimeBridgeCancelParamsSchema = {
  type: "object",
  required: ["handle"],
  additionalProperties: false,
  properties: {
    handle: NonEmptyStringSchema,
  },
} as const satisfies JsonSchema;
