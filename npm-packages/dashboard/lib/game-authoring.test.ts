import { describe, expect, test } from "vitest";
import {
  DEFAULT_SHORT_DRAMA_VIEWPORT,
  addAgentCharacter,
  addSourceEdge,
  canvasLayoutPositions,
  isShortDramaStoryDefinition,
  nextShortDramaStart,
  parseSubtitleCues,
  placeNodeOnTimeline,
  presentationViewport,
  reorderTimelineNodes,
  removeNodeFromSource,
  replaceSourceNode,
  setCanvasPosition,
  setPresentationViewport,
  setShortDramaTrack,
  splitTimelineSourceNode,
} from "./game-authoring";
import type { AgentProfile, GameSource, GameSourceNode } from "./runtime-types";

function source(nodes: GameSourceNode[]): GameSource {
  return {
    schemaVersion: 1,
    metadata: { name: "Authoring test", tags: [] },
    entryNodeId: "start",
    graph: {
      nodes: [
        { id: "start", type: "start", version: 1, config: {}, label: "Start" },
        ...nodes,
      ],
      edges: [],
    },
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

describe("shared GameSource authoring", () => {
  test("Short Drama recognizes Server title-cased Story categories", () => {
    const definition = {
      type: "dialogue",
      version: 1,
      phase: "runtime" as const,
      title: "Dialogue",
      category: "Narrative",
      configSchema: {},
      ports: [],
      dynamicInputs: false,
      dynamicOutputs: false,
      timelineCompatible: true,
    };

    expect(isShortDramaStoryDefinition(definition)).toBe(true);
    expect(isShortDramaStoryDefinition(definition, "dialogue")).toBe(true);
    expect(isShortDramaStoryDefinition(definition, "camera")).toBe(false);
  });

  test("Canvas and Short Drama metadata preserve the same graph", () => {
    const initial = source([
      { id: "scene", type: "scene", version: 1, config: { durationMs: 2000 } },
    ]);
    const canvas = setCanvasPosition(initial, "scene", { x: 320, y: 180 });
    const drama = setShortDramaTrack(canvas, "scene", "story");

    expect(drama.graph).toEqual(initial.graph);
    expect(drama.views).toMatchObject({
      canvas: { nodes: { scene: { x: 320, y: 180 } } },
      presentation: { viewport: DEFAULT_SHORT_DRAMA_VIEWPORT },
      shortDrama: { trackByNode: { scene: "story" } },
    });
    expect(presentationViewport(drama)).toEqual(DEFAULT_SHORT_DRAMA_VIEWPORT);
  });

  test("Short Drama preserves a format selected from either authoring view", () => {
    const initial = source([
      { id: "scene", type: "scene", version: 1, config: { durationMs: 2000 } },
    ]);
    const landscape = setPresentationViewport(initial, {
      width: 1920,
      height: 1080,
      aspectRatio: "16:9",
    });
    const edited = setShortDramaTrack(landscape, "scene", "story");

    expect(presentationViewport(edited)).toEqual({ width: 1920, height: 1080, aspectRatio: "16:9" });
    expect(edited.views.shortDrama).toEqual({ trackByNode: { scene: "story" } });
  });

  test("adding an Agent to the cast does not create a story node", () => {
    const initial = source([]);
    const profile: AgentProfile = {
      id: "profile-mizuki",
      projectId: "project-last-train",
      slug: "mizuki",
      name: "Mizuki",
      description: "Moon Kingdom heir",
      activeVersionId: "version-2",
      archivedAt: null,
      createdAt: "2026-07-21T00:00:00Z",
      updatedAt: "2026-07-21T00:00:00Z",
    };

    const edited = addAgentCharacter(initial, profile);

    expect(edited.graph).toEqual(initial.graph);
    expect(edited.agents).toEqual([expect.objectContaining({
      id: "agent.mizuki",
      profileId: profile.id,
      profileVersionId: profile.activeVersionId,
    })]);
    expect(edited.characters).toEqual([expect.objectContaining({
      agentId: "agent.mizuki",
      player: false,
    })]);
    expect(addAgentCharacter(edited, profile)).toBe(edited);
  });

  test("legacy saved positions cannot overlap nodes that use fallback layout", () => {
    const initial = source([
      { id: "background", type: "background", version: 1, config: {} },
      { id: "finish", type: "end", version: 1, config: {} },
    ]);
    initial.graph.edges = [
      { id: "start-background", source: { nodeId: "start", port: "next" }, target: { nodeId: "background", port: "in" } },
      { id: "background-finish", source: { nodeId: "background", port: "next" }, target: { nodeId: "finish", port: "in" } },
    ];
    initial.views = { canvas: { nodes: { background: { x: 200, y: 200 } } } };

    expect(canvasLayoutPositions(initial)).toEqual({
      start: { x: 160, y: 120 },
      background: { x: 460, y: 120 },
      finish: { x: 760, y: 120 },
    });
  });

  test("unconnected nodes use a visible staging row below the connected graph", () => {
    const initial = source([
      { id: "background", type: "background", version: 1, config: {} },
      { id: "finish", type: "end", version: 1, config: {} },
      { id: "draft-dialogue", type: "dialogue", version: 1, config: {} },
    ]);
    initial.graph.edges = [
      { id: "start-background", source: { nodeId: "start", port: "next" }, target: { nodeId: "background", port: "in" } },
      { id: "background-finish", source: { nodeId: "background", port: "next" }, target: { nodeId: "finish", port: "in" } },
    ];
    initial.views = { canvas: { nodes: { background: { x: 200, y: 200 } } } };

    expect(canvasLayoutPositions(initial)["draft-dialogue"]).toEqual({ x: 160, y: 330 });
  });

  test("timeline edits update the ordinary node seen by Canvas", () => {
    const initial = source([
      { id: "clip", type: "video", version: 1, config: { durationMs: 2000 } },
    ]);
    const edited = placeNodeOnTimeline(initial, "clip", "picture", 1750, 4200);

    expect(edited.graph.nodes.find((node) => node.id === "clip")?.config).toMatchObject({
      sequenceId: "main",
      startMs: 1750,
      durationMs: 4200,
    });
    expect(edited.views.shortDrama).toEqual({ trackByNode: { clip: "picture" } });
    expect(edited.views.presentation).toMatchObject({ viewport: DEFAULT_SHORT_DRAMA_VIEWPORT });
    expect(edited.graph.edges).toEqual(expect.arrayContaining([
      expect.objectContaining({
        source: { nodeId: "start", port: "next" },
        target: { nodeId: "clip", port: "in" },
        managedBy: "shortDrama",
      }),
    ]));
  });

  test("narrative tracks share one insertion cursor while media tracks remain parallel", () => {
    let initial = source([
      { id: "scene", type: "scene", version: 1, config: { sequenceId: "main", startMs: 0, durationMs: 5000 } },
      { id: "music", type: "audio", version: 1, config: { sequenceId: "main", startMs: 0, durationMs: 30_000 } },
      { id: "agent", type: "agent", version: 1, config: { sequenceId: "main", startMs: 5000, durationMs: 3000 } },
    ]);
    initial = setShortDramaTrack(initial, "scene", "story");
    initial = setShortDramaTrack(initial, "music", "sound");
    initial = setShortDramaTrack(initial, "agent", "cast");

    expect(nextShortDramaStart(initial, "interaction")).toBe(8000);
    expect(nextShortDramaStart(initial, "sound")).toBe(30_000);
    expect(nextShortDramaStart(initial, "picture")).toBe(0);
  });

  test("Short Drama parses SRT and WebVTT cues for timeline preview", () => {
    expect(parseSubtitleCues(`1\n00:00:01,250 --> 00:00:03,000\nFirst line\nSecond line\n\n2\n00:03.500 --> 00:05.000 align:center\nNext cue`)).toEqual([
      { startMs: 1250, endMs: 3000, text: "First line\nSecond line" },
      { startMs: 3500, endMs: 5000, text: "Next cue" },
    ]);
  });

  test("Short Drama sequence edits maintain executable routes without replacing Canvas routes", () => {
    let edited = source([
      { id: "scene", type: "scene", version: 1, config: { sequenceId: "main", startMs: 0, durationMs: 1000 } },
      { id: "dialogue", type: "dialogue", version: 1, config: { sequenceId: "main", startMs: 1000, durationMs: 1000 } },
      { id: "advanced", type: "transform", version: 1, config: {} },
    ]);
    edited = placeNodeOnTimeline(edited, "scene", "story", 0);
    edited = placeNodeOnTimeline(edited, "dialogue", "story", 1000);

    expect(edited.graph.edges.filter((edge) => edge.managedBy === "shortDrama").map((edge) => [
      edge.source.nodeId,
      edge.target.nodeId,
    ])).toEqual([
      ["start", "scene"],
      ["scene", "dialogue"],
      ["dialogue", "short-drama-end"],
    ]);

    edited = addSourceEdge(edited, {
      id: "canvas-scene-advanced",
      source: { nodeId: "scene", port: "next" },
      target: { nodeId: "advanced", port: "in" },
    });
    edited = reorderTimelineNodes(edited, "story", ["dialogue", "scene"]);

    expect(edited.graph.edges).toEqual(expect.arrayContaining([
      expect.objectContaining({ id: "canvas-scene-advanced", target: { nodeId: "advanced", port: "in" } }),
    ]));
    expect(edited.graph.edges.some((edge) => (
      edge.managedBy === "shortDrama" && edge.source.nodeId === "scene" && edge.source.port === "next"
    ))).toBe(false);
  });

  test("editing a Host Action declares its engine-neutral target", () => {
    const initial = source([
      {
        id: "gate",
        type: "host_action",
        version: 1,
        config: { target: "", action: "open", completion: "required" },
      },
    ]);
    const gate = {
      ...initial.graph.nodes[1],
      config: { target: "world.main-gate", action: "open", completion: "required" },
    };
    const edited = replaceSourceNode(initial, gate);

    expect(edited.presentationResources).toEqual([
      {
        id: "world.main-gate",
        kind: "object",
        requiredCapabilities: ["vifu.world.object-action.v1"],
        required: true,
      },
    ]);
  });

  test("splitting a timeline clip leaves unrelated advanced graph nodes intact", () => {
    const initial = source([
      {
        id: "clip",
        type: "video",
        version: 1,
        config: { sequenceId: "main", startMs: 0, durationMs: 4000, inMs: 500 },
      },
      { id: "advanced", type: "transform", version: 1, config: { mapping: "private" } },
    ]);
    const split = splitTimelineSourceNode(initial, "clip", "next", "in");

    expect(split).not.toBeNull();
    expect(split?.source.graph.nodes.find((node) => node.id === "advanced")).toEqual(
      initial.graph.nodes.find((node) => node.id === "advanced"),
    );
    expect(split?.source.graph.nodes.filter((node) => node.type === "video")).toHaveLength(2);
  });

  test("Tool nodes retain only Agent references still used by the Game", () => {
    const initial = source([
      { id: "inventory", type: "tool", version: 1, config: { agentId: "agent.steward", tool: "inventory.read" } },
    ]);
    initial.agents = [{
      id: "agent.steward",
      profileId: "profile-steward",
      profileVersionId: "version-steward",
      capabilities: ["chat", "tool"],
      executionDescriptor: {},
    }];

    expect(removeNodeFromSource(initial, "inventory").agents).toEqual([]);
  });
});
