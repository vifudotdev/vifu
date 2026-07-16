import type { JsonSchema, JsonValue } from "./json.js";

export const JSON_RPC_VERSION = "2.0" as const;
export const JSON_RPC_WEBSOCKET_PROTOCOL = "jsonrpc" as const;
export const VIFU_TOKEN_WEBSOCKET_PROTOCOL_PREFIX = "vifu.token." as const;

export type JsonRpcVersion = typeof JSON_RPC_VERSION;
export type JsonRpcId = string | number | null;

export interface JsonRpcRequest<Method extends string = string, Params = unknown> {
  jsonrpc: JsonRpcVersion;
  method: Method;
  id?: JsonRpcId;
  params?: Params;
}

export interface JsonRpcSuccessResponse<Result = unknown> {
  jsonrpc: JsonRpcVersion;
  id: JsonRpcId;
  result: Result;
}

export interface JsonRpcErrorObject<Data = JsonValue> {
  code: number;
  message: string;
  data?: Data;
}

export interface JsonRpcErrorResponse<Data = JsonValue> {
  jsonrpc: JsonRpcVersion;
  id: JsonRpcId;
  error: JsonRpcErrorObject<Data>;
}

export type JsonRpcResponse<Result = unknown, Data = JsonValue> =
  | JsonRpcSuccessResponse<Result>
  | JsonRpcErrorResponse<Data>;

export interface JsonRpcNotification<Method extends string = string, Params = unknown> {
  jsonrpc: JsonRpcVersion;
  method: Method;
  id?: never;
  params?: Params;
}

export type JsonRpcBatchRequest<Request extends JsonRpcRequest = JsonRpcRequest> =
  readonly Request[];

export const JsonRpcIdSchema = {
  oneOf: [{ type: "string" }, { type: "number" }, { type: "null" }],
} as const satisfies JsonSchema;

export const JsonRpcRequestSchema = {
  type: "object",
  required: ["jsonrpc", "method"],
  properties: {
    jsonrpc: { const: JSON_RPC_VERSION },
    id: JsonRpcIdSchema,
    method: { type: "string", minLength: 1 },
    params: {},
  },
} as const satisfies JsonSchema;

export const JsonRpcErrorObjectSchema = {
  type: "object",
  required: ["code", "message"],
  properties: {
    code: { type: "integer" },
    message: { type: "string" },
    data: {},
  },
} as const satisfies JsonSchema;

export const JsonRpcSuccessResponseSchema = {
  type: "object",
  required: ["jsonrpc", "id", "result"],
  properties: {
    jsonrpc: { const: JSON_RPC_VERSION },
    id: JsonRpcIdSchema,
    result: {},
  },
} as const satisfies JsonSchema;

export const JsonRpcErrorResponseSchema = {
  type: "object",
  required: ["jsonrpc", "id", "error"],
  properties: {
    jsonrpc: { const: JSON_RPC_VERSION },
    id: JsonRpcIdSchema,
    error: JsonRpcErrorObjectSchema,
  },
} as const satisfies JsonSchema;
