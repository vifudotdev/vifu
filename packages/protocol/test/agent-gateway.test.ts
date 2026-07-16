import { describe, expect, it } from "vitest";

import {
  AGENT_GATEWAY_HELLO_REQUEST_ID,
  VIFU_AGENT_GATEWAY_EVENTS,
  VIFU_AGENT_GATEWAY_METHODS,
  VIFU_AGENT_GATEWAY_PROTOCOL_VERSION,
  createEventFrame,
  createRequestFrame,
  createResponseFrame,
} from "../src";

describe("@vifu/protocol agent gateway frames", () => {
  it("uses a request/response handshake", () => {
    const hello = createRequestFrame(
      AGENT_GATEWAY_HELLO_REQUEST_ID,
      VIFU_AGENT_GATEWAY_METHODS.HELLO,
      {
        protocol: VIFU_AGENT_GATEWAY_PROTOCOL_VERSION,
        gatewayId: "local-gateway",
        agents: [
          {
            id: "default-agent",
            name: "Default",
            metadata: { providerId: "openclaw-local" },
          },
        ],
        metadata: { adapter: "openclaw" },
      },
    );

    expect(hello).toEqual({
      type: "req",
      id: "gateway.hello",
      method: "gateway.hello",
      params: {
        protocol: "vifu.agent-gateway/1",
        gatewayId: "local-gateway",
        agents: [
          {
            id: "default-agent",
            name: "Default",
            metadata: { providerId: "openclaw-local" },
          },
        ],
        metadata: { adapter: "openclaw" },
      },
    });

    expect(
      createResponseFrame(AGENT_GATEWAY_HELLO_REQUEST_ID, {
        connectionId: "connection-id",
        sessionId: "session-id",
        heartbeatIntervalMs: 10000,
        resumed: false,
      }),
    ).toEqual({
      type: "res",
      id: "gateway.hello",
      ok: true,
      payload: {
        connectionId: "connection-id",
        sessionId: "session-id",
        heartbeatIntervalMs: 10000,
        resumed: false,
      },
    });
  });

  it("uses request/response frames for agent invocation", () => {
    const requestId = "4fc4ef6c-f7f9-4d2d-b02c-226402d864aa";
    expect(
      createRequestFrame(requestId, VIFU_AGENT_GATEWAY_METHODS.INVOKE, {
        channelId: 7,
        endpointId: "endpoint-id",
        profileId: "profile-id",
        bindingId: "binding-id",
        agentId: "guide-agent",
        binding: {},
        input: { message: "Hello" },
        timeoutMs: 30000,
      }),
    ).toEqual({
      type: "req",
      id: requestId,
      method: "agent.invoke",
      params: {
        channelId: 7,
        endpointId: "endpoint-id",
        profileId: "profile-id",
        bindingId: "binding-id",
        agentId: "guide-agent",
        binding: {},
        input: { message: "Hello" },
        timeoutMs: 30000,
      },
    });

    expect(
      createResponseFrame(requestId, {
        channelId: 7,
        output: { text: "Hi" },
      }),
    ).toEqual({
      type: "res",
      id: requestId,
      ok: true,
      payload: {
        channelId: 7,
        output: { text: "Hi" },
      },
    });
  });

  it("uses events for cancellation and heartbeats", () => {
    expect(
      createEventFrame(VIFU_AGENT_GATEWAY_EVENTS.CANCEL, {
        requestId: "4fc4ef6c-f7f9-4d2d-b02c-226402d864aa",
        channelId: 7,
      }),
    ).toEqual({
      type: "event",
      event: "agent.cancel",
      payload: {
        requestId: "4fc4ef6c-f7f9-4d2d-b02c-226402d864aa",
        channelId: 7,
      },
    });

    expect(
      createEventFrame(VIFU_AGENT_GATEWAY_EVENTS.HEARTBEAT, {
        sessionId: "session-id",
      }),
    ).toEqual({
      type: "event",
      event: "gateway.heartbeat",
      payload: {
        sessionId: "session-id",
      },
    });
  });
});
