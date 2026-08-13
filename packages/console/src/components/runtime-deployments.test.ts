import { describe, expect, test } from "vitest";

import {
  MAX_APPLY_POLL_ATTEMPTS,
  gatewayDeploymentPresentation,
  latestGatewaySession,
  nativeGatewayPairingCode,
  runtimeApplyPollDelay,
  runtimeApplyTarget,
} from "./runtime-deployments";
import type { AgentGateway, RuntimeDeployment } from "../types";

describe("gateway pairing", () => {
  test("copies the complete native code instead of the compact QR bridge", () => {
    expect(nativeGatewayPairingCode({
      serverUrl: "https://macbook.local:6790",
      certificateDer: "AQID",
      certificateSha256: "sha256:synthetic",
      pairingUri: "https://vifu.ai/pair#server=compact",
      pairingDeepLink: "vifu://gateway/enroll?server=complete&certificate=AQID",
      pairingQrSvg: null,
    })).toBe("vifu://gateway/enroll?server=complete&certificate=AQID");
  });
});

describe("runtime deployment apply polling", () => {
  test("uses bounded exponential backoff", () => {
    expect(Array.from({ length: MAX_APPLY_POLL_ATTEMPTS }, (_, attempt) => runtimeApplyPollDelay(attempt)))
      .toEqual([2_000, 4_000, 8_000, 16_000, 30_000, 30_000]);
  });

  test("changes its target when a gateway reports progress", () => {
    const deployment: RuntimeDeployment = {
      id: "deployment-1",
      projectId: "project-1",
      name: "primary",
      isPrimary: true,
      configSyncEnabled: true,
      traceMode: "full",
      remoteInvocationEnabled: true,
      activeReleaseVersion: 2,
      gatewayIds: ["iphone-1"],
      applyStates: [],
      createdAt: "2026-01-01T00:00:00Z",
      updatedAt: "2026-01-01T00:00:00Z",
    };

    expect(runtimeApplyTarget([deployment]))
      .not.toEqual(runtimeApplyTarget([{
        ...deployment,
        applyStates: [{
          deploymentId: deployment.id,
          gatewayId: "iphone-1",
          releaseVersion: 2,
          contentHash: "sha256:release",
          appliedAt: "2026-01-01T00:00:01Z",
        }],
      }]));
  });
});

describe("deployment gateway status", () => {
  test("uses the reported device identity instead of the opaque Gateway ID", () => {
    const gateway: AgentGateway = {
      id: "session-1",
      gatewayId: "gateway-821e06ab006243dd988e0f38b6f28acb",
      sessionId: "session-1",
      status: "connected",
      agents: [{ id: "android-local-chat", name: "Android llama" }],
      metadata: {
        name: "Vifu Starter Baseline · 2407FPN8ER",
        kind: "mobile",
        platform: "android",
        device: { manufacturer: "Xiaomi", model: "2407FPN8ER" },
        application: { name: "Vifu Starter Baseline", version: "0.1.1" },
      },
      connectedAt: "2026-01-01T00:00:00Z",
      lastSeenAt: "2026-01-01T00:00:00Z",
      disconnectedAt: null,
    };

    expect(gatewayDeploymentPresentation(gateway.gatewayId, [gateway])).toMatchObject({
      name: "Vifu Starter Baseline · 2407FPN8ER",
      typeLabel: "Android mobile",
      deviceLabel: "Xiaomi 2407FPN8ER",
      applicationLabel: "Vifu Starter Baseline · v0.1.1",
      agentLabel: "1 agent",
    });
  });

  test("keeps an understandable fallback before a device reports metadata", () => {
    expect(gatewayDeploymentPresentation(
      "gateway-821e06ab006243dd988e0f38b6f28acb",
      [],
    )).toMatchObject({
      name: "Gateway 821e06ab…",
      typeLabel: "Device identity pending",
      deviceLabel: "Waiting for this device to connect",
      applicationLabel: null,
      agentLabel: "No agents reported",
    });
  });

  test("prefers the connected session for a paired gateway", () => {
    const session = (status: string, lastSeenAt: string): AgentGateway => ({
      id: `${status}-${lastSeenAt}`,
      gatewayId: "android-1",
      sessionId: `${status}-session`,
      status,
      agents: [],
      metadata: {},
      connectedAt: "2026-01-01T00:00:00Z",
      lastSeenAt,
      disconnectedAt: status === "connected" ? null : lastSeenAt,
    });

    expect(latestGatewaySession("android-1", [
      session("connected", "2026-01-01T00:00:01Z"),
      session("disconnected", "2026-01-01T00:00:02Z"),
    ])?.status).toBe("connected");
  });

  test("does not borrow another gateway session", () => {
    const gateway: AgentGateway = {
      id: "session-1",
      gatewayId: "android-2",
      sessionId: "session-1",
      status: "connected",
      agents: [],
      metadata: {},
      connectedAt: "2026-01-01T00:00:00Z",
      lastSeenAt: "2026-01-01T00:00:00Z",
      disconnectedAt: null,
    };

    expect(latestGatewaySession("android-1", [gateway])).toBeUndefined();
  });
});
