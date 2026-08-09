import { describe, expect, test } from "vitest";

import {
  MAX_APPLY_POLL_ATTEMPTS,
  runtimeApplyPollDelay,
  runtimeApplyTarget,
} from "./runtime-deployments";
import type { RuntimeDeployment } from "../types";

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
