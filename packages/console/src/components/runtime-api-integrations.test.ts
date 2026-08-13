import { describe, expect, it } from "vitest";

import type { AgentBinding, AgentProfile, RuntimeDeployment } from "../types";
import { apiAgentOptions, buildCurlExample } from "./runtime-api-integrations";

const timestamp = "2026-08-14T00:00:00Z";

function profile(id: string, name: string): AgentProfile {
  return {
    id,
    projectId: "app-1",
    slug: id,
    name,
    description: null,
    activeVersionId: `${id}-version`,
    archivedAt: null,
    createdAt: timestamp,
    updatedAt: timestamp,
  };
}

function binding(profileId: string, gatewayId: string): AgentBinding {
  return {
    id: `${profileId}-binding`,
    profileId,
    provider: "vifu-runtime",
    gatewayId,
    agentId: profileId,
    config: {},
    createdAt: timestamp,
    updatedAt: timestamp,
  };
}

const deployment: RuntimeDeployment = {
  id: "deployment-1",
  projectId: "app-1",
  name: "development",
  isPrimary: true,
  configSyncEnabled: true,
  traceMode: "summary",
  remoteInvocationEnabled: true,
  activeReleaseVersion: null,
  gatewayIds: ["gateway-current"],
  createdAt: timestamp,
  updatedAt: timestamp,
};

describe("API Integration Agent choices", () => {
  it("does not offer a runtime Agent after its Gateway is detached", () => {
    expect(apiAgentOptions(
      "app-1",
      [profile("old-agent", "Old Agent"), profile("current-agent", "Current Agent")],
      [binding("old-agent", "gateway-old"), binding("current-agent", "gateway-current")],
      [deployment],
    ).map((option) => option.profileId)).toEqual(["current-agent"]);
  });

  it("keeps non-Gateway Agents available", () => {
    expect(apiAgentOptions("app-1", [profile("hosted-agent", "Hosted Agent")], [], [deployment]))
      .toHaveLength(1);
  });
});

describe("API Integration cURL examples", () => {
  it("handles local generated HTTPS certificates", () => {
    expect(buildCurlExample("https://192.168.10.20:6790/v1", "android-chat"))
      .toMatch(/^curl --insecure /);
  });

  it("does not weaken system-trusted HTTPS", () => {
    expect(buildCurlExample("https://api.vifu.dev/v1", "android-chat"))
      .toMatch(/^curl /);
  });
});
