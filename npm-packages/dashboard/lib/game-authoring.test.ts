import { describe, expect, test } from "vitest";
import {
  canvasLayoutPositions,
  isShortDramaStoryDefinition,
  placeNodeOnTimeline,
  removeNodeFromSource,
  replaceSourceNode,
  setCanvasPosition,
  setShortDramaTrack,
  splitTimelineSourceNode,
} from "./game-authoring";
import type { GameSource, GameSourceNode } from "./runtime-types";

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
    resources: [],
    presentationResources: [],
    locales: ["en"],
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
      shortDrama: { trackByNode: { scene: "story" } },
    });
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
