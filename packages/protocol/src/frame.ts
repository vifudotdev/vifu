import type { JsonSchema } from "./json.js";

export const GATEWAY_FRAME_TYPES = {
  REQUEST: "req",
  RESPONSE: "res",
  EVENT: "event",
} as const;

export const MAX_GATEWAY_FRAME_BYTES = 16 * 1024 * 1024;

export type GatewayFrameType = (typeof GATEWAY_FRAME_TYPES)[keyof typeof GATEWAY_FRAME_TYPES];

export type GatewayFrameId = string;

export type StateVersion = {
  presence: number;
  health: number;
};

export type ErrorShape<Details = unknown> = {
  code: string;
  message: string;
  details?: Details;
  retryable?: boolean;
  retryAfterMs?: number;
};

export type RequestFrame<Method extends string = string, Params = unknown> = {
  type: typeof GATEWAY_FRAME_TYPES.REQUEST;
  id: GatewayFrameId;
  method: Method;
  params?: Params;
};

export type ResponseFrame<Payload = unknown, Details = unknown> = {
  type: typeof GATEWAY_FRAME_TYPES.RESPONSE;
  id: GatewayFrameId;
  ok: boolean;
  payload?: Payload;
  error?: ErrorShape<Details>;
};

export type EventFrame<Event extends string = string, Payload = unknown> = {
  type: typeof GATEWAY_FRAME_TYPES.EVENT;
  event: Event;
  payload?: Payload;
  seq?: number;
  stateVersion?: StateVersion;
};

export type GatewayFrame =
  | RequestFrame
  | ResponseFrame
  | EventFrame;

export const NonEmptyStringSchema = {
  type: "string",
  minLength: 1,
} as const satisfies JsonSchema;

export const StateVersionSchema = {
  type: "object",
  required: ["presence", "health"],
  additionalProperties: false,
  properties: {
    presence: { type: "integer", minimum: 0 },
    health: { type: "integer", minimum: 0 },
  },
} as const satisfies JsonSchema;

export const ErrorShapeSchema = {
  type: "object",
  required: ["code", "message"],
  additionalProperties: false,
  properties: {
    code: NonEmptyStringSchema,
    message: NonEmptyStringSchema,
    details: {},
    retryable: { type: "boolean" },
    retryAfterMs: { type: "integer", minimum: 0 },
  },
} as const satisfies JsonSchema;

export const RequestFrameSchema = {
  type: "object",
  required: ["type", "id", "method"],
  additionalProperties: false,
  properties: {
    type: { const: GATEWAY_FRAME_TYPES.REQUEST },
    id: NonEmptyStringSchema,
    method: NonEmptyStringSchema,
    params: {},
  },
} as const satisfies JsonSchema;

export const ResponseFrameSchema = {
  type: "object",
  required: ["type", "id", "ok"],
  additionalProperties: false,
  properties: {
    type: { const: GATEWAY_FRAME_TYPES.RESPONSE },
    id: NonEmptyStringSchema,
    ok: { type: "boolean" },
    payload: {},
    error: ErrorShapeSchema,
  },
} as const satisfies JsonSchema;

export const EventFrameSchema = {
  type: "object",
  required: ["type", "event"],
  additionalProperties: false,
  properties: {
    type: { const: GATEWAY_FRAME_TYPES.EVENT },
    event: NonEmptyStringSchema,
    payload: {},
    seq: { type: "integer", minimum: 0 },
    stateVersion: StateVersionSchema,
  },
} as const satisfies JsonSchema;

export const GatewayFrameSchema = {
  oneOf: [RequestFrameSchema, ResponseFrameSchema, EventFrameSchema],
} as const satisfies JsonSchema;

export function createRequestFrame<Method extends string, Params = unknown>(
  id: GatewayFrameId,
  method: Method,
  params?: Params,
): RequestFrame<Method, Params> {
  return params === undefined
    ? { type: GATEWAY_FRAME_TYPES.REQUEST, id, method }
    : { type: GATEWAY_FRAME_TYPES.REQUEST, id, method, params };
}

export function createResponseFrame<Payload = unknown>(
  id: GatewayFrameId,
  payload: Payload,
): ResponseFrame<Payload> {
  return {
    type: GATEWAY_FRAME_TYPES.RESPONSE,
    id,
    ok: true,
    payload,
  };
}

export function createErrorResponseFrame<Details = unknown>(
  id: GatewayFrameId,
  error: ErrorShape<Details>,
): ResponseFrame<never, Details> {
  return {
    type: GATEWAY_FRAME_TYPES.RESPONSE,
    id,
    ok: false,
    error,
  };
}

export function createEventFrame<Event extends string, Payload = unknown>(
  event: Event,
  payload?: Payload,
): EventFrame<Event, Payload> {
  return payload === undefined
    ? { type: GATEWAY_FRAME_TYPES.EVENT, event }
    : { type: GATEWAY_FRAME_TYPES.EVENT, event, payload };
}

export function encodeGatewayFrame(frame: GatewayFrame): string {
  if (!isGatewayFrame(frame)) {
    throw new TypeError("invalid gateway frame");
  }
  const encoded = JSON.stringify(frame);
  if (utf8ByteLength(encoded) > MAX_GATEWAY_FRAME_BYTES) {
    throw new RangeError("gateway frame is too large");
  }
  return encoded;
}

export function decodeGatewayFrame(source: string): GatewayFrame {
  if (source.length === 0) {
    throw new TypeError("gateway frame is empty");
  }
  if (utf8ByteLength(source) > MAX_GATEWAY_FRAME_BYTES) {
    throw new RangeError("gateway frame is too large");
  }

  let value: unknown;
  try {
    value = JSON.parse(source);
  } catch {
    throw new TypeError("invalid gateway frame");
  }

  if (!isGatewayFrame(value)) {
    throw new TypeError("invalid gateway frame");
  }
  return value;
}

export function isRequestFrame(value: unknown): value is RequestFrame {
  return (
    isRecord(value) &&
    hasOnlyKeys(value, ["type", "id", "method", "params"]) &&
    value.type === GATEWAY_FRAME_TYPES.REQUEST &&
    isNonEmptyString(value.id) &&
    isNonEmptyString(value.method)
  );
}

export function isResponseFrame(value: unknown): value is ResponseFrame {
  return (
    isRecord(value) &&
    hasOnlyKeys(value, ["type", "id", "ok", "payload", "error"]) &&
    value.type === GATEWAY_FRAME_TYPES.RESPONSE &&
    isNonEmptyString(value.id) &&
    typeof value.ok === "boolean" &&
    (value.error === undefined || isErrorShape(value.error))
  );
}

export function isEventFrame(value: unknown): value is EventFrame {
  return (
    isRecord(value) &&
    hasOnlyKeys(value, ["type", "event", "payload", "seq", "stateVersion"]) &&
    value.type === GATEWAY_FRAME_TYPES.EVENT &&
    isNonEmptyString(value.event) &&
    (value.seq === undefined || isNonNegativeInteger(value.seq)) &&
    (value.stateVersion === undefined || isStateVersion(value.stateVersion))
  );
}

export function isGatewayFrame(value: unknown): value is GatewayFrame {
  return isRequestFrame(value) || isResponseFrame(value) || isEventFrame(value);
}

export function isErrorShape(value: unknown): value is ErrorShape {
  return (
    isRecord(value) &&
    hasOnlyKeys(value, ["code", "message", "details", "retryable", "retryAfterMs"]) &&
    isNonEmptyString(value.code) &&
    isNonEmptyString(value.message) &&
    (value.retryable === undefined || typeof value.retryable === "boolean") &&
    (value.retryAfterMs === undefined || isNonNegativeInteger(value.retryAfterMs))
  );
}

export function isStateVersion(value: unknown): value is StateVersion {
  return (
    isRecord(value) &&
    hasOnlyKeys(value, ["presence", "health"]) &&
    isNonNegativeInteger(value.presence) &&
    isNonNegativeInteger(value.health)
  );
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function hasOnlyKeys(value: Record<string, unknown>, allowedKeys: readonly string[]): boolean {
  return Object.keys(value).every((key) => allowedKeys.includes(key));
}

function isNonEmptyString(value: unknown): value is string {
  return typeof value === "string" && value.length > 0;
}

function isNonNegativeInteger(value: unknown): value is number {
  return typeof value === "number" && Number.isInteger(value) && value >= 0;
}

function utf8ByteLength(value: string): number {
  let bytes = 0;
  for (let index = 0; index < value.length; index += 1) {
    const code = value.charCodeAt(index);
    if (code < 0x80) {
      bytes += 1;
    } else if (code < 0x800) {
      bytes += 2;
    } else if (code >= 0xd800 && code <= 0xdbff && index + 1 < value.length) {
      const next = value.charCodeAt(index + 1);
      if (next >= 0xdc00 && next <= 0xdfff) {
        bytes += 4;
        index += 1;
      } else {
        bytes += 3;
      }
    } else {
      bytes += 3;
    }
  }
  return bytes;
}
