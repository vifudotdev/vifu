import { describe, expect, it } from "vitest";

import {
  NODE_INVOKE_REQUEST_EVENT,
  NODE_INVOKE_RESULT_METHOD,
  createErrorResponseFrame,
  createEventFrame,
  createNodeInvokeRequestEvent,
  createNodeInvokeResultRequest,
  createRequestFrame,
  createResponseFrame,
  decodeGatewayFrame,
  encodeGatewayFrame,
  isGatewayFrame,
  isRequestFrame,
} from "../src";

describe("@vifu/protocol gateway frames", () => {
  it("creates req frames with stable field names", () => {
    const frame = createRequestFrame("req-1", "runtime.ping", { ok: true });

    expect(frame).toEqual({
      type: "req",
      id: "req-1",
      method: "runtime.ping",
      params: { ok: true },
    });
    expect(frame).not.toHaveProperty("vifu");
    expect(frame).not.toHaveProperty("name");
    expect(frame).not.toHaveProperty("topic");
  });

  it("creates res frames with ok payload or error", () => {
    expect(createResponseFrame("req-1", { pong: true })).toEqual({
      type: "res",
      id: "req-1",
      ok: true,
      payload: { pong: true },
    });

    expect(
      createErrorResponseFrame("req-1", {
        code: "BAD_REQUEST",
        message: "invalid request",
      }),
    ).toEqual({
      type: "res",
      id: "req-1",
      ok: false,
      error: {
        code: "BAD_REQUEST",
        message: "invalid request",
      },
    });
  });

  it("creates event frames with stable field names", () => {
    expect(createEventFrame("runtime.ready", { ready: true })).toEqual({
      type: "event",
      event: "runtime.ready",
      payload: { ready: true },
    });
  });

  it("keeps node invoke payload field names stable", () => {
    expect(
      createNodeInvokeRequestEvent({
        id: "invoke-1",
        nodeId: "node-1",
        command: "debug.ping",
        paramsJSON: "{\"ping\":\"pong\"}",
        timeoutMs: 5000,
      }),
    ).toEqual({
      type: "event",
      event: NODE_INVOKE_REQUEST_EVENT,
      payload: {
        id: "invoke-1",
        nodeId: "node-1",
        command: "debug.ping",
        paramsJSON: "{\"ping\":\"pong\"}",
        timeoutMs: 5000,
      },
    });

    expect(
      createNodeInvokeResultRequest("ack-1", {
        id: "invoke-1",
        nodeId: "node-1",
        ok: true,
        payloadJSON: "{\"pong\":true}",
      }),
    ).toEqual({
      type: "req",
      id: "ack-1",
      method: NODE_INVOKE_RESULT_METHOD,
      params: {
        id: "invoke-1",
        nodeId: "node-1",
        ok: true,
        payloadJSON: "{\"pong\":true}",
      },
    });
  });

  it("narrows the three gateway frame kinds", () => {
    expect(isGatewayFrame({ type: "req", id: "1", method: "ping" })).toBe(true);
    expect(isGatewayFrame({ type: "res", id: "1", ok: true })).toBe(true);
    expect(isGatewayFrame({ type: "event", event: "tick" })).toBe(true);
    expect(isGatewayFrame({ type: "request", id: "1", method: "ping" })).toBe(false);
    expect(isGatewayFrame({ type: "event", name: "tick" })).toBe(false);
  });

  it("encodes and decodes gateway frame text", () => {
    const frame = createRequestFrame("req-1", "runtime.ping", { ok: true });
    const encoded = encodeGatewayFrame(frame);

    expect(encoded).toBe('{"type":"req","id":"req-1","method":"runtime.ping","params":{"ok":true}}');
    expect(decodeGatewayFrame(encoded)).toEqual(frame);
  });

  it("rejects invalid gateway frame text", () => {
    expect(() => decodeGatewayFrame("")).toThrow("gateway frame is empty");
    expect(() => decodeGatewayFrame("{")).toThrow("invalid gateway frame");
    expect(() =>
      decodeGatewayFrame('{"type":"req","id":"","method":"runtime.ping"}'),
    ).toThrow("invalid gateway frame");
    expect(() =>
      encodeGatewayFrame({ type: "req", id: "", method: "runtime.ping" }),
    ).toThrow("invalid gateway frame");
  });

  it("rejects extra frame fields like the schema", () => {
    expect(isRequestFrame({ type: "req", id: "1", method: "ping", extra: true })).toBe(false);
    expect(isGatewayFrame({ type: "res", id: "1", ok: true, extra: true })).toBe(false);
    expect(isGatewayFrame({ type: "event", event: "tick", extra: true })).toBe(false);
    expect(
      isGatewayFrame({
        type: "res",
        id: "1",
        ok: false,
        error: {
          code: "BAD_REQUEST",
          message: "invalid request",
          extra: true,
        },
      }),
    ).toBe(false);
    expect(
      isGatewayFrame({
        type: "event",
        event: "tick",
        stateVersion: {
          presence: 1,
          health: 1,
          extra: true,
        },
      }),
    ).toBe(false);
  });
});
