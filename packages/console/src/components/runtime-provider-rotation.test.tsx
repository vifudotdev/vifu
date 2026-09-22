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
const profile = {
  id: "profile-1",
  name: "Voice Agent",
  activeVersionId: "version-1",
} as AgentProfile;
const oldProviderKey = "voice-provider--aaaaaaaaaaaaaaaaaaaaaaaa";
const currentProviderKey = "voice-provider--bbbbbbbbbbbbbbbbbbbbbbbb";
const detail = {
  profile,
  versions: [{
    version: {
      id: "version-1",
      versionNumber: 1,
      persona: {},
      source: { providerKey: oldProviderKey },
    },
    capabilities: [{ kind: "chat", providerKey: oldProviderKey }],
  }],
} as AgentProfileDetail;
const binding = {
  profileId: profile.id,
  gatewayId: "current-gateway",
  agentId: "voice",
  config: { providerKey: oldProviderKey },
} as AgentBinding;
const currentProvider = {
  id: "provider-current",
  providerKey: currentProviderKey,
  providerType: "agent-gateway",
  name: "Current Voice Provider",
  status: "online",
  baseUrl: "",
  config: {
    gatewayId: "current-gateway",
    runtimeProviderKey: "voice-provider",
  },
} as ProjectProvider;

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
      status: "connected",
      metadata: { providerKey: currentProviderKey },
    } as AvailableAgent]);
    expect(html).toContain("Current Voice Provider");
    expect(html).toContain("Online");
    expect(html).not.toContain("Unavailable");
  });
});
