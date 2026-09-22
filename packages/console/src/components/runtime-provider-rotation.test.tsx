import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, test } from "vitest";
import type {
  AgentBinding,
  AgentProfile,
  AgentProfileDetail,
  AvailableAgent,
  ProjectProvider,
  RuntimeProject,
} from "../types";
import { RuntimeAgentsView } from "./runtime-agents";
import { providerDisplayName } from "./runtime-profile-workbench";

const project = { id: "project-1", slug: "example", name: "Example" } as RuntimeProject;
const timestamp = "2026-09-22T00:00:00Z";
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
const oldProviderKey = "voice-provider--aaaaaaaaaaaaaaaaaaaaaaaa";
const currentProviderKey = "voice-provider--bbbbbbbbbbbbbbbbbbbbbbbb";
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
      source: { providerKey: oldProviderKey },
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
      providerKey: oldProviderKey,
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
  profileId: profile.id,
  provider: "agent-gateway",
  gatewayId: "current-gateway",
  agentId: "voice",
  config: { providerKey: oldProviderKey },
  createdAt: timestamp,
  updatedAt: timestamp,
} satisfies AgentBinding;
const currentProvider = {
  id: "provider-current",
  projectId: project.id,
  providerKey: currentProviderKey,
  providerType: "agent-gateway",
  name: "Current Voice Provider",
  status: "online",
  baseUrl: "",
  config: {
    gatewayId: "current-gateway",
    runtimeProviderKey: "voice-provider",
  },
  secretKeys: [],
  displaySecret: null,
  lastCheckedAt: null,
  sourceKind: "registry",
  sourceKey: "agent-gateway",
  createdAt: timestamp,
  updatedAt: timestamp,
} satisfies ProjectProvider;

function render(availableAgents: AvailableAgent[]) {
  return renderToStaticMarkup(
    <RuntimeAgentsView
      project={project}
      profiles={[profile]}
      profileDetails={[detail]}
      bindings={[binding]}
      availableAgents={availableAgents}
      candidates={[]}
      providerAdapters={[]}
      projectProviders={[currentProvider]}
      gatewayStartsOnRequest
    />,
  );
}

describe("Agent gateway Provider rotation", () => {
  test("uses the current Provider name in Agent details", () => {
    expect(providerDisplayName([currentProvider], oldProviderKey)).toBe("Current Voice Provider");
  });

  test("shows the current Provider name and idle status after scale-to-zero", () => {
    const html = render([]);
    expect(html).toContain("Current Voice Provider");
    expect(html).toContain("Idle");
    expect(html).not.toContain("Unavailable");
  });

  test("uses the current gateway Agent metadata when it is connected", () => {
    const html = render([{
      gatewayId: "current-gateway",
      id: "voice",
      name: "Voice Agent",
      status: "connected",
      metadata: { providerKey: currentProviderKey },
    } satisfies AvailableAgent]);
    expect(html).toContain("Current Voice Provider");
    expect(html).toContain("Online");
    expect(html).not.toContain("Unavailable");
  });
});
