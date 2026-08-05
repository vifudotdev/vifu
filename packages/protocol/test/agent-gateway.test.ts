import { describe, expect, it } from "vitest";

import {
  AGENT_GATEWAY_HELLO_REQUEST_ID,
  VIFU_AGENT_GATEWAY_EVENTS,
  VIFU_AGENT_GATEWAY_FEATURES,
  VIFU_AGENT_GATEWAY_METHODS,
  VIFU_AGENT_GATEWAY_PROTOCOL_VERSION,
  createEventFrame,
  createRequestFrame,
  createResponseFrame,
} from "../src";

describe("@vifu/protocol agent gateway frames", () => {
  it("uses a challenge and machine-signed request/response handshake", () => {
    expect(
      createEventFrame(VIFU_AGENT_GATEWAY_EVENTS.CHALLENGE, {
        nonce: "a".repeat(64),
        timestamp: 42,
        audience: "https://runtime.example.com/v1/agent-gateway/connect",
      }),
    ).toEqual({
      type: "event",
      event: "gateway.challenge",
      payload: {
        nonce: "a".repeat(64),
        timestamp: 42,
        audience: "https://runtime.example.com/v1/agent-gateway/connect",
      },
    });

    const hello = createRequestFrame(
      AGENT_GATEWAY_HELLO_REQUEST_ID,
      VIFU_AGENT_GATEWAY_METHODS.HELLO,
      {
        protocol: VIFU_AGENT_GATEWAY_PROTOCOL_VERSION,
        agents: [
          {
            id: "default-agent",
            name: "Default",
            metadata: { providerId: "openclaw-local" },
          },
        ],
        metadata: { adapter: "openclaw" },
        machine: {
          id: `machine-${"a".repeat(64)}`,
          publicKey: "public-key",
          signature: "signature",
          signedAt: 43,
        },
        auth: { deviceToken: "vifu_gw_device-token" },
      },
    );

    expect(hello).toEqual({
      type: "req",
      id: "gateway.hello",
      method: "gateway.hello",
      params: {
        protocol: "vifu.agent-gateway/1",
        agents: [
          {
            id: "default-agent",
            name: "Default",
            metadata: { providerId: "openclaw-local" },
          },
        ],
        metadata: { adapter: "openclaw" },
        machine: {
          id: `machine-${"a".repeat(64)}`,
          publicKey: "public-key",
          signature: "signature",
          signedAt: 43,
        },
        auth: { deviceToken: "vifu_gw_device-token" },
      },
    });

    expect(
      createResponseFrame(AGENT_GATEWAY_HELLO_REQUEST_ID, {
        gatewayId: "local-gateway",
        connectionId: "connection-id",
        sessionId: "session-id",
        heartbeatIntervalMs: 10000,
        resumed: false,
        auth: {
          deviceToken: "vifu_gw_rotated-device-token",
          generation: 2,
          expiresAt: "2027-01-01T00:00:00Z",
        },
      }),
    ).toEqual({
      type: "res",
      id: "gateway.hello",
      ok: true,
      payload: {
        gatewayId: "local-gateway",
        connectionId: "connection-id",
        sessionId: "session-id",
        heartbeatIntervalMs: 10000,
        resumed: false,
        auth: {
          deviceToken: "vifu_gw_rotated-device-token",
          generation: 2,
          expiresAt: "2027-01-01T00:00:00Z",
        },
      },
    });
  });

  it("uses an event for interactive Gateway authorization", () => {
    expect(
      createEventFrame(VIFU_AGENT_GATEWAY_EVENTS.PAIRING_REQUIRED, {
        requestId: "4fc4ef6c-f7f9-4d2d-b02c-226402d864aa",
        authUrl: "/pair?request=4fc4ef6c-f7f9-4d2d-b02c-226402d864aa",
        retryable: true,
        recommendedNextStep: "approve-in-dashboard",
        retryAfterMs: 2000,
      }),
    ).toEqual({
      type: "event",
      event: "gateway.pairingRequired",
      payload: {
        requestId: "4fc4ef6c-f7f9-4d2d-b02c-226402d864aa",
        authUrl: "/pair?request=4fc4ef6c-f7f9-4d2d-b02c-226402d864aa",
        retryable: true,
        recommendedNextStep: "approve-in-dashboard",
        retryAfterMs: 2000,
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

  it("uses events for cancellation, invocation activity, and heartbeats", () => {
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
      createEventFrame(VIFU_AGENT_GATEWAY_EVENTS.INVOCATION_ACTIVITY_READY, {}),
    ).toEqual({
      type: "event",
      event: "agent.invocationActivity.ready",
      payload: {},
    });

    expect(
      createEventFrame(VIFU_AGENT_GATEWAY_EVENTS.INVOCATION_ACTIVITY, {
        requestId: "4fc4ef6c-f7f9-4d2d-b02c-226402d864aa",
        channelId: 7,
      }),
    ).toEqual({
      type: "event",
      event: "agent.invocationActivity",
      payload: {
        requestId: "4fc4ef6c-f7f9-4d2d-b02c-226402d864aa",
        channelId: 7,
      },
    });

    expect(VIFU_AGENT_GATEWAY_FEATURES.INVOCATION_ACTIVITY).toBe(
      "agent.invocation-activity.v1",
    );

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
