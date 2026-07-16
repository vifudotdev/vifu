import { describe, expect, it } from "vitest";

import {
  VIFU_PROJECT_RPC_METHODS,
  VIFU_PROJECT_RPC_METHOD_NAMES,
  buildProjectDiscoverPayload,
  createAgentInvokeRequest,
  isVifuProjectRpcMethod,
} from "../src";

describe("@vifu/protocol project rpc", () => {
  it("keeps the V1 method surface narrow", () => {
    expect(VIFU_PROJECT_RPC_METHOD_NAMES).toEqual([
      "rpc.discover",
      "agent.list",
      "agent.invoke",
    ]);
    expect(isVifuProjectRpcMethod("agent.invoke")).toBe(true);
    expect(isVifuProjectRpcMethod("node.invoke")).toBe(false);
  });

  it("creates JSON-RPC requests matching the public README example", () => {
    expect(
      createAgentInvokeRequest(1, {
        agent: "town-guide",
        message: "Open the north gate",
      }),
    ).toEqual({
      jsonrpc: "2.0",
      id: 1,
      method: VIFU_PROJECT_RPC_METHODS.AGENT_INVOKE,
      params: {
        agent: "town-guide",
        message: "Open the north gate",
      },
    });
  });

  it("builds the Vifu discovery payload used by project endpoints", () => {
    const discovery = buildProjectDiscoverPayload({
      project: {
        id: "project-1",
        slug: "demo",
        gatewayId: "openclaw-local",
      },
      httpUrl: "http://demo.localhost:6790",
      websocketUrl: "ws://demo.localhost:6790",
    });

    expect(discovery).toEqual({
      project: {
        id: "project-1",
        slug: "demo",
        gatewayId: "openclaw-local",
      },
      protocol: {
        name: "vifu.project",
        version: "0.1",
        methods: [
          "rpc.discover",
          "agent.list",
          "agent.invoke",
        ],
      },
      transports: {
        http: "http://demo.localhost:6790",
        websocket: "ws://demo.localhost:6790",
        jsonrpc: "2.0",
        websocketProtocol: "jsonrpc",
      },
      capabilities: [
        "agent.list",
        "agent.invoke",
      ],
    });
    expect(discovery.protocol.methods).toEqual([
      "rpc.discover",
      "agent.list",
      "agent.invoke",
    ]);
  });
});
