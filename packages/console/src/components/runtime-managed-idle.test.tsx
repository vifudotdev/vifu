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
const provider = {
  id: "provider-1",
  providerKey: "voice-provider",
  providerType: "agent-gateway",
  name: "Voice Provider",
  status: "offline",
  baseUrl: "",
  config: { gatewayId: "gateway-1" },
} as ProjectProvider;
const profile = {
  id: "profile-1",
  name: "Voice Agent",
  activeVersionId: "version-1",
} as AgentProfile;
const detail = {
  profile,
  versions: [{
    version: {
      id: "version-1",
      versionNumber: 1,
      persona: {},
      source: { providerKey: "voice-provider" },
    },
    capabilities: [{ kind: "chat", providerKey: "voice-provider" }],
  }],
} as AgentProfileDetail;
const binding = {
  profileId: "profile-1",
  gatewayId: "gateway-1",
  agentId: "voice",
} as AgentBinding;

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
