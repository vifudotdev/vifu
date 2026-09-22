import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, test } from "vitest";
import type {
  AgentBinding,
  AgentProfile,
  AgentProfileDetail,
  ProjectProvider,
  RuntimeProject,
} from "../types";
import { RuntimeAgentsView } from "./runtime-agents";
import { RuntimeProvidersView } from "./runtime-providers";

const project = { id: "project-1", slug: "example", name: "Example" } as RuntimeProject;
const timestamp = "2026-09-22T00:00:00Z";
const provider = {
  id: "provider-1",
  projectId: project.id,
  providerKey: "voice-provider",
  providerType: "agent-gateway",
  name: "Voice Provider",
  status: "offline",
  baseUrl: "",
  config: { gatewayId: "gateway-1" },
  secretKeys: [],
  displaySecret: null,
  lastCheckedAt: null,
  sourceKind: "registry",
  sourceKey: "agent-gateway",
  createdAt: timestamp,
  updatedAt: timestamp,
} satisfies ProjectProvider;
const profile = {
  id: "profile-1",
  projectId: project.id,
  slug: "voice-agent",
  name: "Voice Agent",
  description: null,
  activeVersionId: "version-1",
  archivedAt: null,
  createdAt: timestamp,
  updatedAt: timestamp,
} satisfies AgentProfile;
const detail = {
  profile,
  versions: [{
    version: {
      id: "version-1",
      profileId: profile.id,
      versionNumber: 1,
      persona: {},
      runtime: {},
      presentation: {},
      source: { providerKey: "voice-provider" },
      contentHash: "test-content-hash",
      changeSummary: null,
      archivedAt: null,
      createdAt: timestamp,
    },
    capabilities: [{
      id: "capability-1",
      profileVersionId: "version-1",
      kind: "chat",
      providerType: "agent-gateway",
      providerKey: "voice-provider",
      resourceId: null,
      config: {},
      inputSchema: {},
      outputSchema: {},
      createdAt: timestamp,
    }],
  }],
  rollout: [],
} satisfies AgentProfileDetail;
const binding = {
  id: "binding-1",
  profileId: "profile-1",
  provider: "agent-gateway",
  gatewayId: "gateway-1",
  agentId: "voice",
  config: {},
  createdAt: timestamp,
  updatedAt: timestamp,
} satisfies AgentBinding;

describe("on-request Agent gateways", () => {
  test("labels deployed Agent and Provider as idle while their gateway is scaled to zero", () => {
    const agents = renderToStaticMarkup(
      <RuntimeAgentsView
        project={project}
        profiles={[profile]}
        profileDetails={[detail]}
        bindings={[binding]}
        availableAgents={[]}
        candidates={[]}
        providerAdapters={[]}
        projectProviders={[provider]}
        gatewayStartsOnRequest
      />,
    );
    const providers = renderToStaticMarkup(
      <RuntimeProvidersView
        project={project}
        catalog={{ registry: [], custom: [] }}
        providers={[provider]}
        availableAgents={[]}
        gatewayStartsOnRequest
      />,
    );

    expect(agents).toContain("Idle");
    expect(agents).not.toContain("Unavailable");
    expect(providers).toContain("Idle");
  });

  test("keeps the ordinary self-hosted offline label", () => {
    const agents = renderToStaticMarkup(
      <RuntimeAgentsView
        project={project}
        profiles={[profile]}
        profileDetails={[detail]}
        bindings={[binding]}
        availableAgents={[]}
        candidates={[]}
        providerAdapters={[]}
        projectProviders={[provider]}
      />,
    );

    expect(agents).toContain("Unavailable");
  });
});
