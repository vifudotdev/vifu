import type {
  AgentProfile,
  GameAsset,
  GameAssetVersion,
  GameNodeDefinition,
  GameSource,
  GameSourceEdge,
  GameSourceNode,
} from "./runtime-types";

export type CanvasPosition = { x: number; y: number };

export type ShortDramaTrack = {
  id: string;
  label: string;
  kind: "scene" | "media" | "agent" | "interaction" | "cue";
};

export const SHORT_DRAMA_TRACKS: ShortDramaTrack[] = [
  { id: "story", label: "Scenes", kind: "scene" },
  { id: "picture", label: "Picture", kind: "media" },
  { id: "sound", label: "Sound", kind: "media" },
  { id: "cast", label: "Characters", kind: "agent" },
  { id: "interaction", label: "Interaction", kind: "interaction" },
  { id: "cue", label: "Cues", kind: "cue" },
];

const SHORT_DRAMA_TRACK_BY_TYPE: Record<string, string> = {
  episode: "story",
  scene: "story",
  dialogue: "story",
  ending: "story",
  background: "picture",
  character_visual: "picture",
  asset: "picture",
  video: "picture",
  audio: "sound",
  voice: "sound",
  subtitle: "sound",
  agent: "cast",
  character_state: "cast",
  relationship: "cast",
  memory: "cast",
  choice: "interaction",
  input: "interaction",
  condition: "interaction",
  event: "interaction",
  transition: "cue",
  expression: "cue",
  camera_cue: "cue",
  host_action: "cue",
};

const SHORT_DRAMA_PRESENTATION_TOOL_TYPES = new Set([
  "camera_cue",
  "expression",
  "host_action",
  "transition",
]);

export function isShortDramaStoryDefinition(definition: GameNodeDefinition, query = ""): boolean {
  const category = definition.category.toLowerCase();
  const normalized = query.trim().toLowerCase();
  return definition.phase === "runtime"
    && definition.timelineCompatible
    && definition.type !== "end"
    && (
      category === "narrative"
      || category === "flow"
      || SHORT_DRAMA_PRESENTATION_TOOL_TYPES.has(definition.type)
    )
    && (!normalized || `${definition.title} ${definition.category}`.toLowerCase().includes(normalized));
}

export function sourceFingerprint(source: GameSource): string {
  return JSON.stringify(source);
}

export function canvasPosition(source: GameSource, nodeId: string, index = 0): CanvasPosition {
  const canvas = objectValue(source.views.canvas);
  const positions = objectValue(canvas.nodes);
  const position = objectValue(positions[nodeId]);
  return {
    x: numberValue(position.x, 160 + (index % 4) * 280),
    y: numberValue(position.y, 120 + Math.floor(index / 4) * 190),
  };
}

export function nextCanvasNodePosition(index: number): CanvasPosition {
  return {
    x: 180 + (index % 3) * 300,
    y: 140 + Math.floor(index / 3) * 210,
  };
}

export function canvasLayoutPositions(source: GameSource): Record<string, CanvasPosition> {
  const preferred = Object.fromEntries(source.graph.nodes.map((node, index) => [
    node.id,
    canvasPosition(source, node.id, index),
  ]));
  if (!hasCanvasOverlap(Object.values(preferred))) return preferred;

  const depthByNode = new Map<string, number>();
  const entry = source.graph.nodes.some((node) => node.id === source.entryNodeId)
    ? source.entryNodeId
    : source.graph.nodes[0]?.id;
  if (entry) {
    const queue = [entry];
    depthByNode.set(entry, 0);
    while (queue.length > 0) {
      const sourceId = queue.shift();
      if (!sourceId) continue;
      const depth = depthByNode.get(sourceId) ?? 0;
      for (const edge of source.graph.edges.filter((edge) => edge.source.nodeId === sourceId)) {
        if (depthByNode.has(edge.target.nodeId)) continue;
        depthByNode.set(edge.target.nodeId, depth + 1);
        queue.push(edge.target.nodeId);
      }
    }
  }
  const nodesByDepth = new Map<number, string[]>();
  for (const node of source.graph.nodes.filter((node) => depthByNode.has(node.id))) {
    const depth = depthByNode.get(node.id) ?? 0;
    nodesByDepth.set(depth, [...(nodesByDepth.get(depth) ?? []), node.id]);
  }
  const connected = [...nodesByDepth.entries()].flatMap(([depth, nodeIds]) => (
    nodeIds.map((nodeId, row) => [nodeId, { x: 160 + depth * 300, y: 120 + row * 210 }])
  ));
  const connectedRows = Math.max(1, ...[...nodesByDepth.values()].map((nodes) => nodes.length));
  const orphans = source.graph.nodes
    .filter((node) => !depthByNode.has(node.id))
    .map((node, index) => [
      node.id,
      {
        x: 160 + (index % 4) * 300,
        y: 120 + connectedRows * 210 + Math.floor(index / 4) * 210,
      },
    ]);
  return Object.fromEntries([...connected, ...orphans]);
}

function hasCanvasOverlap(positions: CanvasPosition[]): boolean {
  const width = 224;
  const height = 160;
  const gap = 24;
  return positions.some((position, index) => positions.slice(index + 1).some((other) => (
    Math.abs(position.x - other.x) < width + gap
    && Math.abs(position.y - other.y) < height + gap
  )));
}

export function setCanvasPosition(source: GameSource, nodeId: string, position: CanvasPosition): GameSource {
  const canvas = objectValue(source.views.canvas);
  const positions = objectValue(canvas.nodes);
  return {
    ...source,
    views: {
      ...source.views,
      canvas: {
        ...canvas,
        nodes: {
          ...positions,
          [nodeId]: { x: Math.round(position.x), y: Math.round(position.y) },
        },
      },
    },
  };
}

export function removeNodeFromSource(source: GameSource, nodeId: string): GameSource {
  const canvas = objectValue(source.views.canvas);
  const positions = { ...objectValue(canvas.nodes) };
  delete positions[nodeId];
  const shortDrama = objectValue(source.views.shortDrama);
  const trackByNode = { ...objectValue(shortDrama.trackByNode) };
  delete trackByNode[nodeId];
  const sourceNode = source.graph.nodes.find((node) => node.id === nodeId);
  const agentId = sourceNode && ["agent", "tool"].includes(sourceNode.type) && typeof sourceNode.config.agentId === "string"
    ? sourceNode.config.agentId
    : null;
  const referencedElsewhere = agentId
    ? source.graph.nodes.some((node) => node.id !== nodeId && node.config.agentId === agentId)
    : false;
  const logicalResourceId = typeof sourceNode?.config.logicalResourceId === "string"
    ? sourceNode.config.logicalResourceId
    : null;
  const logicalResourceReferencedElsewhere = logicalResourceId
    ? source.graph.nodes.some((node) => node.id !== nodeId && node.config.logicalResourceId === logicalResourceId)
    : false;
  const presentation = objectValue(source.views.presentation);
  const presentationBindings = { ...objectValue(presentation.bindings) };
  if (logicalResourceId && !logicalResourceReferencedElsewhere) delete presentationBindings[logicalResourceId];
  return {
    ...source,
    entryNodeId: source.entryNodeId === nodeId ? "" : source.entryNodeId,
    graph: {
      nodes: source.graph.nodes.filter((node) => node.id !== nodeId),
      edges: source.graph.edges.filter((edge) => edge.source.nodeId !== nodeId && edge.target.nodeId !== nodeId),
    },
    agents: agentId && !referencedElsewhere
      ? source.agents.filter((agent) => agent.id !== agentId)
      : source.agents,
    presentationResources: logicalResourceId && !logicalResourceReferencedElsewhere
      ? source.presentationResources.filter((resource) => resource.id !== logicalResourceId)
      : source.presentationResources,
    views: {
      ...source.views,
      canvas: { ...canvas, nodes: positions },
      shortDrama: { ...shortDrama, trackByNode },
      presentation: { ...presentation, bindings: presentationBindings },
    },
  };
}

export function bindManagedPresentationAsset(
  source: GameSource,
  asset: GameAsset,
  version: GameAssetVersion,
): GameSource {
  const presentation = objectValue(source.views.presentation);
  const bindings = objectValue(presentation.bindings);
  const hasLogicalResource = source.presentationResources.some((resource) => resource.id === asset.assetKey);
  return {
    ...source,
    presentationResources: hasLogicalResource
      ? source.presentationResources
      : [...source.presentationResources, {
        id: asset.assetKey,
        kind: asset.kind,
        requiredCapabilities: [`vifu.presentation.${asset.kind}.v1`],
        required: false,
      }],
    views: {
      ...source.views,
      presentation: {
        ...presentation,
        bindings: {
          ...bindings,
          [asset.assetKey]: {
            kind: "managed-asset-version",
            value: version.id,
          },
        },
      },
    },
  };
}

export function managedPresentationFromSource(source: GameSource): {
  bindings: Record<string, { kind: string; value: unknown }>;
  assetVersionIds: string[];
} | null {
  const rawBindings = objectValue(objectValue(source.views.presentation).bindings);
  const bindings: Record<string, { kind: string; value: unknown }> = {};
  const assetVersionIds = new Set<string>();
  for (const [logicalId, rawBinding] of Object.entries(rawBindings)) {
    const binding = objectValue(rawBinding);
    if (typeof binding.kind !== "string" || !("value" in binding)) continue;
    bindings[logicalId] = { kind: binding.kind, value: binding.value };
    if (binding.kind === "managed-asset-version" && typeof binding.value === "string") {
      assetVersionIds.add(binding.value);
    }
  }
  return Object.keys(bindings).length > 0
    ? { bindings, assetVersionIds: [...assetVersionIds].sort() }
    : null;
}

export function createSourceNode(
  source: GameSource,
  definition: GameNodeDefinition,
  position: CanvasPosition,
  profile?: AgentProfile,
): GameSource {
  const id = uniqueNodeId(source, profile?.slug || definition.type);
  const config = defaultConfig(definition, profile);
  const node: GameSourceNode = {
    id,
    type: definition.type,
    version: definition.version,
    config,
    label: profile?.name ?? definition.title,
  };
  let next = {
    ...source,
    entryNodeId: definition.type === "start" && !source.entryNodeId ? id : source.entryNodeId,
    graph: { ...source.graph, nodes: [...source.graph.nodes, node] },
  };
  if (profile) {
    const agentId = String(config.agentId);
    next = {
      ...next,
      agents: [
        ...source.agents,
        {
          id: agentId,
          profileId: profile.id,
          profileVersionId: profile.activeVersionId,
          capabilities: ["chat"],
          executionDescriptor: {},
        },
      ],
    };
  }
  next = setCanvasPosition(next, id, position);
  if (definition.timelineCompatible) next = setShortDramaTrack(next, id, defaultTrackForNode(node));
  return next;
}

export function replaceSourceNode(source: GameSource, node: GameSourceNode): GameSource {
  const next = {
    ...source,
    graph: {
      ...source.graph,
      nodes: source.graph.nodes.map((current) => current.id === node.id ? node : current),
    },
  };
  if (node.type !== "host_action") return next;
  const target = typeof node.config.target === "string" ? node.config.target.trim() : "";
  if (!target || source.presentationResources.some((resource) => resource.id === target)) return next;
  return {
    ...next,
    presentationResources: [
      ...source.presentationResources,
      {
        id: target,
        kind: "object",
        requiredCapabilities: ["vifu.world.object-action.v1"],
        required: true,
      },
    ],
  };
}

export function addSourceEdge(source: GameSource, edge: GameSourceEdge): GameSource {
  const duplicate = source.graph.edges.some((current) => (
    current.source.nodeId === edge.source.nodeId
    && current.source.port === edge.source.port
    && current.target.nodeId === edge.target.nodeId
    && current.target.port === edge.target.port
  ));
  return duplicate
    ? source
    : { ...source, graph: { ...source.graph, edges: [...source.graph.edges, edge] } };
}

export function removeSourceEdge(source: GameSource, edgeId: string): GameSource {
  return {
    ...source,
    graph: { ...source.graph, edges: source.graph.edges.filter((edge) => edge.id !== edgeId) },
  };
}

export function shortDramaTrack(source: GameSource, node: GameSourceNode): string {
  const shortDrama = objectValue(source.views.shortDrama);
  const trackByNode = objectValue(shortDrama.trackByNode);
  const configured = trackByNode[node.id];
  return typeof configured === "string" ? configured : defaultTrackForNode(node);
}

export function setShortDramaTrack(source: GameSource, nodeId: string, trackId: string): GameSource {
  const shortDrama = objectValue(source.views.shortDrama);
  return {
    ...source,
    views: {
      ...source.views,
      shortDrama: {
        ...shortDrama,
        trackByNode: { ...objectValue(shortDrama.trackByNode), [nodeId]: trackId },
      },
    },
  };
}

export function timelineStart(node: GameSourceNode): number {
  return numberValue(node.config.startMs, 0);
}

export function timelineDuration(node: GameSourceNode): number {
  return Math.max(250, numberValue(node.config.durationMs, defaultDuration(node.type)));
}

export function placeNodeOnTimeline(
  source: GameSource,
  nodeId: string,
  trackId: string,
  startMs: number,
  durationMs?: number,
): GameSource {
  const current = source.graph.nodes.find((node) => node.id === nodeId);
  if (!current) return source;
  const node = {
    ...current,
    config: {
      ...current.config,
      sequenceId: typeof current.config.sequenceId === "string" ? current.config.sequenceId : "main",
      startMs: Math.max(0, Math.round(startMs)),
      durationMs: Math.max(250, Math.round(durationMs ?? timelineDuration(current))),
    },
  };
  return setShortDramaTrack(replaceSourceNode(source, node), nodeId, trackId);
}

export function reorderTimelineNodes(source: GameSource, trackId: string, orderedNodeIds: string[]): GameSource {
  let cursor = 0;
  let next = source;
  for (const nodeId of orderedNodeIds) {
    const node = next.graph.nodes.find((current) => current.id === nodeId);
    if (!node) continue;
    next = placeNodeOnTimeline(next, nodeId, trackId, cursor);
    cursor += timelineDuration(node);
  }
  return next;
}

export function splitTimelineSourceNode(
  source: GameSource,
  nodeId: string,
  outputPort: string,
  inputPort: string,
): { source: GameSource; newNodeId: string } | null {
  const current = source.graph.nodes.find((node) => node.id === nodeId);
  if (!current) return null;
  const durationMs = timelineDuration(current);
  if (durationMs < 500) return null;
  const firstDuration = Math.max(250, Math.round(durationMs / 2));
  const secondDuration = durationMs - firstDuration;
  if (secondDuration < 250) return null;
  const newNodeId = uniqueNodeId(source, `${nodeId}-part`);
  const startMs = timelineStart(current);
  const inMs = numberValue(current.config.inMs, 0);
  const first = {
    ...current,
    config: {
      ...current.config,
      durationMs: firstDuration,
      ...(current.config.inMs !== undefined ? { outMs: inMs + firstDuration } : {}),
    },
  };
  const second: GameSourceNode = {
    ...current,
    id: newNodeId,
    label: current.label ? `${current.label} 2` : undefined,
    config: {
      ...current.config,
      startMs: startMs + firstDuration,
      durationMs: secondDuration,
      ...(current.config.inMs !== undefined ? { inMs: inMs + firstDuration } : {}),
    },
  };
  const outgoing = source.graph.edges.filter((edge) => edge.source.nodeId === nodeId);
  const retainedEdges = source.graph.edges.filter((edge) => edge.source.nodeId !== nodeId);
  const movedEdges = outgoing.map((edge) => ({
    ...edge,
    source: { ...edge.source, nodeId: newNodeId },
  }));
  const connectionId = uniqueEdgeId(source.graph.edges.map((edge) => edge.id), nodeId, newNodeId);
  let next: GameSource = {
    ...source,
    graph: {
      nodes: source.graph.nodes.flatMap((node) => node.id === nodeId ? [first, second] : [node]),
      edges: [
        ...retainedEdges,
        ...movedEdges,
        {
          id: connectionId,
          source: { nodeId, port: outputPort },
          target: { nodeId: newNodeId, port: inputPort },
        },
      ],
    },
  };
  const position = canvasPosition(source, nodeId);
  next = setCanvasPosition(next, newNodeId, { x: position.x + 260, y: position.y });
  next = setShortDramaTrack(next, newNodeId, shortDramaTrack(source, current));
  return { source: next, newNodeId };
}

export function definitionForNode(
  definitions: GameNodeDefinition[],
  node: GameSourceNode,
): GameNodeDefinition | undefined {
  return definitions.find((definition) => definition.type === node.type && definition.version === node.version);
}

export function nodeOutputPorts(definition: GameNodeDefinition | undefined, node: GameSourceNode): string[] {
  if (!definition) return ["next"];
  if (node.type === "choice") {
    const options = Array.isArray(node.config.options) ? node.config.options : [];
    return options.flatMap((option) => {
      const id = objectValue(option).id;
      return typeof id === "string" && id ? [id] : [];
    });
  }
  const ports = definition.ports.filter((port) => port.direction === "output").map((port) => port.name);
  if (definition.dynamicOutputs && ports.length === 0) return ["next"];
  return ports;
}

export function nodeInputPorts(definition: GameNodeDefinition | undefined): string[] {
  return definition?.ports.filter((port) => port.direction === "input").map((port) => port.name) ?? ["in"];
}

export function objectValue(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : {};
}

function defaultConfig(definition: GameNodeDefinition, profile?: AgentProfile): Record<string, unknown> {
  if (profile) return { agentId: `agent.${profile.slug}`, sequenceId: "main", startMs: 0, durationMs: 3000 };
  if (definition.type === "choice") {
    return {
      prompt: "Choose a path",
      options: [
        { id: "option-a", label: "Option A" },
        { id: "option-b", label: "Option B" },
      ],
      sequenceId: "main",
      startMs: 0,
      durationMs: 2000,
    };
  }
  if (definition.type === "host_action") {
    return { target: "", action: "", completion: "required", sequenceId: "main", startMs: 0, durationMs: 1000 };
  }
  if (definition.type === "subtitle") {
    return { locale: "en", sequenceId: "main", startMs: 0, durationMs: defaultDuration(definition.type) };
  }
  if (definition.type === "loop" || definition.type === "for_each") return { maxIterations: 10 };
  if (definition.timelineCompatible) return { sequenceId: "main", startMs: 0, durationMs: defaultDuration(definition.type) };
  return {};
}

function uniqueNodeId(source: GameSource, seed: string): string {
  const base = seed
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "") || "node";
  const existing = new Set(source.graph.nodes.map((node) => node.id));
  if (!existing.has(base)) return base;
  let suffix = 2;
  while (existing.has(`${base}-${suffix}`)) suffix += 1;
  return `${base}-${suffix}`;
}

function uniqueEdgeId(existing: string[], sourceId: string, targetId: string): string {
  const base = `${sourceId}-to-${targetId}`;
  if (!existing.includes(base)) return base;
  let suffix = 2;
  while (existing.includes(`${base}-${suffix}`)) suffix += 1;
  return `${base}-${suffix}`;
}

function defaultTrackForNode(node: GameSourceNode): string {
  return SHORT_DRAMA_TRACK_BY_TYPE[node.type] ?? "cue";
}

function defaultDuration(nodeType: string): number {
  if (["scene", "episode", "video"].includes(nodeType)) return 5000;
  if (["dialogue", "agent", "audio", "voice"].includes(nodeType)) return 3000;
  return 1500;
}

function numberValue(value: unknown, fallback: number): number {
  return typeof value === "number" && Number.isFinite(value) ? value : fallback;
}
