import type { JsonSchema } from "./json.js";
import {
  ErrorShapeSchema,
  NonEmptyStringSchema,
  type ErrorShape,
  type EventFrame,
  type RequestFrame,
} from "./frame.js";

export const NODE_INVOKE_REQUEST_EVENT = "node.invoke.request" as const;
export const NODE_INVOKE_RESULT_METHOD = "node.invoke.result" as const;

export type NodeInvokeRequestPayload = {
  id: string;
  nodeId: string;
  command: string;
  paramsJSON?: string;
  timeoutMs?: number;
  idempotencyKey?: string;
};

export type NodeInvokeResultError = {
  code?: string;
  message?: string;
};

export type NodeInvokeResultParams = {
  id: string;
  nodeId: string;
  ok: boolean;
  payload?: unknown;
  payloadJSON?: string;
  error?: NodeInvokeResultError;
};

export type NodeInvokeRequestEventFrame = EventFrame<
  typeof NODE_INVOKE_REQUEST_EVENT,
  NodeInvokeRequestPayload
>;

export type NodeInvokeResultRequestFrame = RequestFrame<
  typeof NODE_INVOKE_RESULT_METHOD,
  NodeInvokeResultParams
>;

export const NodeInvokeRequestPayloadSchema = {
  type: "object",
  required: ["id", "nodeId", "command"],
  additionalProperties: false,
  properties: {
    id: NonEmptyStringSchema,
    nodeId: NonEmptyStringSchema,
    command: NonEmptyStringSchema,
    paramsJSON: { type: "string" },
    timeoutMs: { type: "integer", minimum: 0 },
    idempotencyKey: NonEmptyStringSchema,
  },
} as const satisfies JsonSchema;

export const NodeInvokeResultErrorSchema = {
  type: "object",
  additionalProperties: false,
  properties: {
    code: NonEmptyStringSchema,
    message: NonEmptyStringSchema,
  },
} as const satisfies JsonSchema;

export const NodeInvokeResultParamsSchema = {
  type: "object",
  required: ["id", "nodeId", "ok"],
  additionalProperties: false,
  properties: {
    id: NonEmptyStringSchema,
    nodeId: NonEmptyStringSchema,
    ok: { type: "boolean" },
    payload: {},
    payloadJSON: { type: "string" },
    error: NodeInvokeResultErrorSchema,
  },
} as const satisfies JsonSchema;

export function createNodeInvokeRequestEvent(
  payload: NodeInvokeRequestPayload,
): NodeInvokeRequestEventFrame {
  return {
    type: "event",
    event: NODE_INVOKE_REQUEST_EVENT,
    payload,
  };
}

export function createNodeInvokeResultRequest(
  id: string,
  params: NodeInvokeResultParams,
): NodeInvokeResultRequestFrame {
  return {
    type: "req",
    id,
    method: NODE_INVOKE_RESULT_METHOD,
    params,
  };
}

export function createNodeInvokeResultError(
  code: string,
  message: string,
): ErrorShape {
  return {
    code,
    message,
  };
}
