import { beforeEach, describe, expect, test, vi } from "vitest";
import {
  createProjectArchive,
  importProjectArchive,
  readProjectArchive,
  VIFU_PROJECT_FORMAT,
  VIFU_PROJECT_SCHEMA_VERSION,
  type VifuProjectArchiveV1,
} from "./project-archive";
import type { GameSource } from "./runtime-types";

const { runtimeBrowserRequestMock } = vi.hoisted(() => ({
  runtimeBrowserRequestMock: vi.fn(),
}));

vi.mock("./runtime-browser-client", () => ({
  runtimeBrowserRequest: runtimeBrowserRequestMock,
  runtimeBrowserUpload: vi.fn(),
}));

function source(): GameSource {
  return {
    schemaVersion: 1,
    metadata: { name: "Archive test", tags: [] },
    entryNodeId: "start",
    graph: { nodes: [{ id: "start", type: "start", version: 1, config: {} }], edges: [] },
    inputs: { type: "object" },
    outputs: { type: "object" },
    variables: [],
    agents: [],
    characters: [],
    resources: [],
    presentationResources: [],
    localization: {
      sourceLocale: "en",
      defaultLocale: "en",
      targetLocales: [],
      sourceMessages: {},
      packs: {},
    },
    views: {},
  };
}

async function archive(): Promise<VifuProjectArchiveV1> {
  return createProjectArchive({
    format: VIFU_PROJECT_FORMAT,
    schemaVersion: VIFU_PROJECT_SCHEMA_VERSION,
    exportedAt: "2026-07-22T00:00:00.000Z",
    generator: { name: "Vifu", version: "0.1.0" },
    project: { name: "Archive test", description: null, originalSlug: "archive-test" },
    source: source(),
    profiles: [],
    resources: [],
    assets: [],
    providerRequirements: [],
    historyIncluded: false,
  });
}

describe("Vifu project files", () => {
  beforeEach(() => {
    runtimeBrowserRequestMock.mockReset();
  });

  test("round-trips a valid integrity-checked archive", async () => {
    const value = await archive();
    const file = new File([JSON.stringify(value)], "archive-test.vf");

    await expect(readProjectArchive(file)).resolves.toEqual(value);
    expect(value.integrity.digest).toMatch(/^[a-f0-9]{64}$/);
  });

  test("rejects project data changed after export", async () => {
    const value = await archive();
    value.project.name = "Tampered project";
    const file = new File([JSON.stringify(value)], "archive-test.vf");

    await expect(readProjectArchive(file)).rejects.toThrow("failed its integrity check");
  });

  test("rejects a schema-one file with missing project libraries", async () => {
    const value = await archive();
    const { providerRequirements: _providerRequirements, integrity: _integrity, ...incomplete } = value;
    const malformed = {
      ...incomplete,
      integrity: {
        algorithm: "sha256",
        digest: "0".repeat(64),
      },
    };
    const file = new File([JSON.stringify(malformed)], "archive-test.vf");

    await expect(readProjectArchive(file)).rejects.toThrow("project libraries are invalid");
  });

  test("restores provider requirements without archival provenance fields", async () => {
    const value = await archive();
    value.providerRequirements = [{
      providerKey: "story-model",
      name: "Story model",
      providerType: "openai-compatible",
      baseUrl: "https://example.com/v1",
      config: { model: "story-small" },
      sourceKind: "custom",
      sourceKey: "private-source-name",
    }];
    const project = {
      id: "project-id",
      slug: "archive-test",
      name: "Archive test",
      description: null,
      createdAt: "2026-07-22T00:00:00.000Z",
      updatedAt: "2026-07-22T00:00:00.000Z",
    };
    runtimeBrowserRequestMock
      .mockResolvedValueOnce({ projects: [] })
      .mockResolvedValueOnce({ project })
      .mockResolvedValueOnce({ provider: {} })
      .mockResolvedValueOnce({ draft: { revision: 0, contentHash: "hash" } })
      .mockResolvedValueOnce({ draft: {} });

    await expect(importProjectArchive(value)).resolves.toEqual(project);
    expect(runtimeBrowserRequestMock).toHaveBeenNthCalledWith(
      3,
      "project/archive-test/providers/import",
      "POST",
      {
        providerKey: "story-model",
        name: "Story model",
        providerType: "openai-compatible",
        baseUrl: "https://example.com/v1",
        config: { model: "story-small" },
      },
    );
  });
});
