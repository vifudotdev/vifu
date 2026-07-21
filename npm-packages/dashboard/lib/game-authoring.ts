import type {
  AgentProfile,
  GameAsset,
  GameAssetVersion,
  GameCharacter,
  GameNodeDefinition,
  GameSource,
  GameSourceEdge,
  GameSourceNode,
} from "./runtime-types";

const SHORT_DRAMA_EDGE_OWNER = "shortDrama";
const SHORT_DRAMA_END_NODE_ID = "short-drama-end";
const SHORT_DRAMA_MEDIA_TRACKS = new Set(["picture", "sound"]);

export type GameViewport = {
  width: number;
  height: number;
  aspectRatio: string;
};

export const DEFAULT_GAME_VIEWPORT: GameViewport = {
  width: 1920,
  height: 1080,
  aspectRatio: "16:9",
};

export const DEFAULT_SHORT_DRAMA_VIEWPORT: GameViewport = {
  width: 1080,
  height: 1920,
  aspectRatio: "9:16",
};

export type CanvasPosition = { x: number; y: number };

export type ShortDramaTrack = {
  id: string;
  label: string;
  kind: "scene" | "media" | "agent" | "interaction" | "cue";
};

export type SubtitleCue = {
  startMs: number;
  endMs: number;
  text: string;
};

export const SHORT_DRAMA_TRACKS: ShortDramaTrack[] = [
  { id: "story", label: "Scenes", kind: "scene" },
  { id: "picture", label: "Picture", kind: "media" },
  { id: "sound", label: "Sound", kind: "media" },
  { id: "cast", label: "Characters", kind: "agent" },
  { id: "interaction", label: "Interaction", kind: "interaction" },
  { id: "cue", label: "Cues", kind: "cue" },
];

export function parseSubtitleCues(content: string): SubtitleCue[] {
  return content
    .replace(/\r\n?/g, "\n")
    .split(/\n{2,}/)
    .flatMap((block) => {
      const lines = block.split("\n").map((line) => line.trimEnd());
      const timingIndex = lines.findIndex((line) => line.includes("-->"));
      if (timingIndex < 0) return [];
      const [rawStart, rawEnd] = lines[timingIndex].split("-->", 2);
      const startMs = subtitleTimestampMs(rawStart);
      const endMs = subtitleTimestampMs(rawEnd);
      const text = lines.slice(timingIndex + 1).join("\n").trim();
      if (startMs === null || endMs === null || endMs <= startMs || !text) return [];
      return [{ startMs, endMs, text }];
    });
}

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

export function messageReference(messageId: string): { $message: string } {
  return { $message: messageId };
}

export function messageReferenceId(value: unknown): string | null {
  const object = objectValue(value);
  return Object.keys(object).length === 1 && typeof object.$message === "string"
    ? object.$message
    : null;
}

export function localizedMessage(source: GameSource, messageId: string, locale: string): string {
  if (locale === source.localization.sourceLocale) {
    return source.localization.sourceMessages[messageId] ?? "";
  }
  return source.localization.packs[locale]?.messages[messageId]
    ?? source.localization.sourceMessages[messageId]
    ?? "";
}

export function setLocalizedMessage(
  source: GameSource,
  messageId: string,
  locale: string,
  value: string,
): GameSource {
  if (locale === source.localization.sourceLocale) {
    return {
      ...source,
      localization: {
        ...source.localization,
        sourceMessages: { ...source.localization.sourceMessages, [messageId]: value },
      },
    };
  }
  const pack = source.localization.packs[locale] ?? {
    sourceHash: "",
    status: "draft" as const,
    messages: {},
  };
  return {
    ...source,
    localization: {
      ...source.localization,
      packs: {
        ...source.localization.packs,
        [locale]: {
          ...pack,
          status: "draft",
          messages: { ...pack.messages, [messageId]: value },
        },
      },
    },
  };
}

export async function localizationSourceHash(messages: Record<string, string>): Promise<string> {
  const canonical = JSON.stringify(Object.fromEntries(
    Object.entries(messages).sort(([left], [right]) => left.localeCompare(right)),
  ));
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(canonical));
  return `sha256:${[...new Uint8Array(digest)].map((value) => value.toString(16).padStart(2, "0")).join("")}`;
}

export function supportedLocales(source: GameSource): string[] {
  return [...new Set([source.localization.sourceLocale, ...source.localization.targetLocales])];
}

export function addPlayerCharacter(source: GameSource, name: string, role: string): GameSource {
  const existing = source.characters.find((character) => character.player);
  const id = existing?.id ?? uniqueCharacterId(source.characters, name || "player");
  const nameMessageId = existing?.nameMessageId ?? `character.${id}.name`;
  const roleMessageId = existing?.roleMessageId ?? `character.${id}.role`;
  const character: GameCharacter = {
    id,
    nameMessageId,
    roleMessageId,
    player: true,
  };
  return {
    ...source,
    characters: existing
      ? source.characters.map((current) => current.id === existing.id ? character : current)
      : [character, ...source.characters],
    localization: {
      ...source.localization,
      sourceMessages: {
        ...source.localization.sourceMessages,
        [nameMessageId]: name.trim() || "Player",
        [roleMessageId]: role.trim() || "Player character",
      },
    },
  };
}

export function addAgentCharacter(source: GameSource, profile: AgentProfile): GameSource {
  const agentId = `agent.${profile.slug}`;
  if (source.characters.some((character) => character.agentId === agentId)) return source;

  const characterId = uniqueCharacterId(source.characters, profile.slug);
  const nameMessageId = `character.${characterId}.name`;
  const roleMessageId = `character.${characterId}.role`;
  return {
    ...source,
    agents: source.agents.some((agent) => agent.id === agentId)
      ? source.agents
      : [...source.agents, {
          id: agentId,
          profileId: profile.id,
          profileVersionId: profile.activeVersionId,
          capabilities: ["chat"],
          executionDescriptor: {},
        }],
    characters: [...source.characters, {
      id: characterId,
      nameMessageId,
      roleMessageId,
      agentId,
      player: false,
    }],
    localization: {
      ...source.localization,
      sourceMessages: {
        ...source.localization.sourceMessages,
        [nameMessageId]: profile.name,
        [roleMessageId]: profile.description?.trim() || "Game character",
      },
    },
  };
}

export function removeCharacter(source: GameSource, characterId: string): GameSource {
  const character = source.characters.find((item) => item.id === characterId);
  if (!character) return source;
  const sourceMessages = { ...source.localization.sourceMessages };
  delete sourceMessages[character.nameMessageId];
  if (character.roleMessageId) delete sourceMessages[character.roleMessageId];
  const packs = Object.fromEntries(Object.entries(source.localization.packs).map(([locale, pack]) => {
    const messages = { ...pack.messages };
    delete messages[character.nameMessageId];
    if (character.roleMessageId) delete messages[character.roleMessageId];
    return [locale, { ...pack, status: "draft" as const, messages }];
  }));
  const agentStillUsed = character.agentId && source.graph.nodes.some((node) => node.config.agentId === character.agentId);
  return {
    ...source,
    characters: source.characters.filter((item) => item.id !== characterId),
    agents: character.agentId && !agentStillUsed
      ? source.agents.filter((agent) => agent.id !== character.agentId)
      : source.agents,
    localization: { ...source.localization, sourceMessages, packs },
  };
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

export function presentationViewport(
  source: GameSource,
  fallback: GameViewport = DEFAULT_GAME_VIEWPORT,
): GameViewport {
  const presentation = objectValue(source.views.presentation);
  const viewport = objectValue(presentation.viewport);
  const width = positiveInteger(viewport.width);
  const height = positiveInteger(viewport.height);
  if (width && height) {
    return {
      width,
      height,
      aspectRatio: aspectRatioLabel(width, height),
    };
  }

  const legacyShortDrama = objectValue(source.views.shortDrama);
  if (legacyShortDrama.aspectRatio === DEFAULT_SHORT_DRAMA_VIEWPORT.aspectRatio) {
    return DEFAULT_SHORT_DRAMA_VIEWPORT;
  }
  return fallback;
}

export function setPresentationViewport(source: GameSource, viewport: GameViewport): GameSource {
  const width = positiveInteger(viewport.width) ?? DEFAULT_GAME_VIEWPORT.width;
  const height = positiveInteger(viewport.height) ?? DEFAULT_GAME_VIEWPORT.height;
  const presentation = objectValue(source.views.presentation);
  const shortDrama = { ...objectValue(source.views.shortDrama) };
  delete shortDrama.aspectRatio;
  return {
    ...source,
    views: {
      ...source.views,
      presentation: {
        ...presentation,
        viewport: {
          width,
          height,
          aspectRatio: aspectRatioLabel(width, height),
        },
      },
      shortDrama,
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
  const representedByCharacter = agentId
    ? source.characters.some((character) => character.agentId === agentId)
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
  const next = {
    ...source,
    entryNodeId: source.entryNodeId === nodeId ? "" : source.entryNodeId,
    graph: {
      nodes: source.graph.nodes.filter((node) => node.id !== nodeId),
      edges: source.graph.edges.filter((edge) => edge.source.nodeId !== nodeId && edge.target.nodeId !== nodeId),
    },
    agents: agentId && !referencedElsewhere && !representedByCharacter
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
  return rebuildShortDramaRoutes(next);
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
  const { config, messages } = defaultConfig(definition, id, profile);
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
    localization: {
      ...source.localization,
      sourceMessages: { ...source.localization.sourceMessages, ...messages },
    },
  };
  if (profile) {
    const agentId = String(config.agentId);
    const characterId = uniqueCharacterId(source.characters, profile.slug);
    const nameMessageId = `character.${characterId}.name`;
    const roleMessageId = `character.${characterId}.role`;
    const hasAgent = source.agents.some((agent) => agent.id === agentId);
    const existingCharacter = source.characters.find((character) => character.agentId === agentId);
    next = {
      ...next,
      agents: hasAgent ? source.agents : [...source.agents, {
          id: agentId,
          profileId: profile.id,
          profileVersionId: profile.activeVersionId,
          capabilities: ["chat"],
          executionDescriptor: {},
        }],
      characters: existingCharacter ? source.characters : [...source.characters, {
        id: characterId,
        nameMessageId,
        roleMessageId,
        agentId,
        player: false,
      }],
      localization: existingCharacter ? next.localization : {
        ...next.localization,
        sourceMessages: {
          ...next.localization.sourceMessages,
          [nameMessageId]: profile.name,
          [roleMessageId]: profile.description?.trim() || "Game character",
        },
      },
    };
  }
  next = setCanvasPosition(next, id, position);
  if (definition.timelineCompatible) next = setShortDramaTrack(next, id, defaultTrackForNode(node));
  return next;
}

export function replaceSourceNode(source: GameSource, node: GameSourceNode): GameSource {
  let next = {
    ...source,
    graph: {
      ...source.graph,
      nodes: source.graph.nodes.map((current) => current.id === node.id ? node : current),
    },
  };
  if (node.type === "host_action") {
    const target = typeof node.config.target === "string" ? node.config.target.trim() : "";
    if (target && !source.presentationResources.some((resource) => resource.id === target)) {
      next = {
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
  }
  return typeof node.config.sequenceId === "string" || source.graph.edges.some((edge) => edge.managedBy === SHORT_DRAMA_EDGE_OWNER)
    ? rebuildShortDramaRoutes(next)
    : next;
}

export function addSourceEdge(source: GameSource, edge: GameSourceEdge): GameSource {
  const duplicate = source.graph.edges.some((current) => (
    current.source.nodeId === edge.source.nodeId
    && current.source.port === edge.source.port
    && current.target.nodeId === edge.target.nodeId
    && current.target.port === edge.target.port
  ));
  if (duplicate) return source;
  const edges = edge.managedBy === SHORT_DRAMA_EDGE_OWNER
    ? [...source.graph.edges, edge]
    : [
        ...source.graph.edges.filter((current) => !(
          current.managedBy === SHORT_DRAMA_EDGE_OWNER
          && current.source.nodeId === edge.source.nodeId
          && current.source.port === edge.source.port
        )),
        edge,
      ];
  return { ...source, graph: { ...source.graph, edges } };
}

export function removeSourceEdge(source: GameSource, edgeId: string): GameSource {
  const next = {
    ...source,
    graph: { ...source.graph, edges: source.graph.edges.filter((edge) => edge.id !== edgeId) },
  };
  return rebuildShortDramaRoutes(next);
}

export function shortDramaTrack(source: GameSource, node: GameSourceNode): string {
  const shortDrama = objectValue(source.views.shortDrama);
  const trackByNode = objectValue(shortDrama.trackByNode);
  const configured = trackByNode[node.id];
  return typeof configured === "string" ? configured : defaultTrackForNode(node);
}

export function setShortDramaTrack(source: GameSource, nodeId: string, trackId: string): GameSource {
  const shortDrama = objectValue(source.views.shortDrama);
  const normalizedShortDrama = { ...shortDrama };
  delete normalizedShortDrama.aspectRatio;
  const next = {
    ...source,
    views: {
      ...source.views,
      shortDrama: {
        ...normalizedShortDrama,
        trackByNode: { ...objectValue(shortDrama.trackByNode), [nodeId]: trackId },
      },
    },
  };
  const presentation = objectValue(source.views.presentation);
  return Object.keys(objectValue(presentation.viewport)).length > 0
    ? next
    : setPresentationViewport(next, DEFAULT_SHORT_DRAMA_VIEWPORT);
}

export function timelineStart(node: GameSourceNode): number {
  return numberValue(node.config.startMs, 0);
}

export function timelineDuration(node: GameSourceNode): number {
  return Math.max(250, numberValue(node.config.durationMs, defaultDuration(node.type)));
}

export function nextShortDramaStart(
  source: GameSource,
  trackId: string,
  excludeNodeId?: string,
): number {
  const mediaTrack = SHORT_DRAMA_MEDIA_TRACKS.has(trackId);
  return source.graph.nodes.reduce((maximum, node) => {
    if (node.id === excludeNodeId || node.config.sequenceId !== "main") return maximum;
    const nodeTrack = shortDramaTrack(source, node);
    const sharesTimeline = mediaTrack
      ? nodeTrack === trackId
      : !SHORT_DRAMA_MEDIA_TRACKS.has(nodeTrack);
    return sharesTimeline
      ? Math.max(maximum, timelineStart(node) + timelineDuration(node))
      : maximum;
  }, 0);
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
  return rebuildShortDramaRoutes(setShortDramaTrack(replaceSourceNode(source, node), nodeId, trackId));
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
  return rebuildShortDramaRoutes(next);
}

export function rebuildShortDramaRoutes(source: GameSource): GameSource {
  const preservedEdges = source.graph.edges.filter((edge) => edge.managedBy !== SHORT_DRAMA_EDGE_OWNER);
  const timelineNodes = source.graph.nodes.filter((node) => (
    node.type !== "start"
    && node.type !== "end"
    && typeof node.config.sequenceId === "string"
  ));
  if (timelineNodes.length === 0) {
    return source.graph.edges.length === preservedEdges.length
      ? source
      : { ...source, graph: { ...source.graph, edges: preservedEdges } };
  }

  let nodes = source.graph.nodes;
  if (!nodes.some((node) => node.id === SHORT_DRAMA_END_NODE_ID)) {
    nodes = [...nodes, {
      id: SHORT_DRAMA_END_NODE_ID,
      type: "end",
      version: 1,
      config: {},
      label: "End",
    }];
  }
  const nodeIds = new Set(nodes.map((node) => node.id));
  const sourceOrder = new Map(nodes.map((node, index) => [node.id, index]));
  const groups = new Map<string, GameSourceNode[]>();
  for (const node of timelineNodes) {
    const sequence = String(node.config.sequenceId || "main");
    groups.set(sequence, [...(groups.get(sequence) ?? []), node]);
  }
  for (const sequenceNodes of groups.values()) {
    sequenceNodes.sort((left, right) => (
      timelineStart(left) - timelineStart(right)
      || trackPriority(shortDramaTrack(source, left)) - trackPriority(shortDramaTrack(source, right))
      || (sourceOrder.get(left.id) ?? 0) - (sourceOrder.get(right.id) ?? 0)
    ));
  }

  const customPorts = new Set(preservedEdges.map((edge) => `${edge.source.nodeId}:${edge.source.port}`));
  const generated: GameSourceEdge[] = [];
  const existingIds = preservedEdges.map((edge) => edge.id);
  const connect = (sourceNodeId: string, sourcePort: string, targetNodeId: string) => {
    if (!nodeIds.has(targetNodeId) || customPorts.has(`${sourceNodeId}:${sourcePort}`)) return;
    const id = uniqueEdgeId([...existingIds, ...generated.map((edge) => edge.id)], `${sourceNodeId}-${sourcePort}`, targetNodeId);
    generated.push({
      id,
      source: { nodeId: sourceNodeId, port: sourcePort },
      target: { nodeId: targetNodeId, port: "in" },
      managedBy: SHORT_DRAMA_EDGE_OWNER,
    });
  };

  const main = groups.get("main") ?? [];
  if (main[0]) connect(source.entryNodeId, "next", main[0].id);
  for (const sequenceNodes of groups.values()) {
    sequenceNodes.forEach((node, index) => {
      const nextNodeId = sequenceNodes[index + 1]?.id ?? SHORT_DRAMA_END_NODE_ID;
      if (node.type === "choice") {
        const options = Array.isArray(node.config.options) ? node.config.options : [];
        for (const rawOption of options) {
          const option = objectValue(rawOption);
          const optionId = typeof option.id === "string" ? option.id : "";
          const target = typeof option.targetNodeId === "string" && nodeIds.has(option.targetNodeId)
            ? option.targetNodeId
            : nextNodeId;
          if (optionId) connect(node.id, optionId, target);
        }
      } else {
        connect(node.id, "next", node.type === "ending" ? SHORT_DRAMA_END_NODE_ID : nextNodeId);
      }
    });
  }
  return {
    ...source,
    graph: { nodes, edges: [...preservedEdges, ...generated] },
  };
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
          managedBy: SHORT_DRAMA_EDGE_OWNER,
        },
      ],
    },
  };
  const position = canvasPosition(source, nodeId);
  next = setCanvasPosition(next, newNodeId, { x: position.x + 260, y: position.y });
  next = setShortDramaTrack(next, newNodeId, shortDramaTrack(source, current));
  return { source: rebuildShortDramaRoutes(next), newNodeId };
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

function positiveInteger(value: unknown): number | null {
  return typeof value === "number" && Number.isInteger(value) && value > 0 ? value : null;
}

function aspectRatioLabel(width: number, height: number): string {
  const divisor = greatestCommonDivisor(width, height);
  return `${width / divisor}:${height / divisor}`;
}

function greatestCommonDivisor(left: number, right: number): number {
  let a = Math.abs(left);
  let b = Math.abs(right);
  while (b > 0) [a, b] = [b, a % b];
  return a || 1;
}

function defaultConfig(
  definition: GameNodeDefinition,
  nodeId: string,
  profile?: AgentProfile,
): { config: Record<string, unknown>; messages: Record<string, string> } {
  const messageId = (field: string) => `node.${nodeId}.${field}`;
  const messages: Record<string, string> = {};
  const localized = (field: string, value: string) => {
    const id = messageId(field);
    messages[id] = value;
    return messageReference(id);
  };
  const timeline = { sequenceId: "main", startMs: 0, durationMs: defaultDuration(definition.type) };
  if (profile) {
    return {
      config: {
        agentId: `agent.${profile.slug}`,
        prompt: localized("prompt", `Respond as ${profile.name} while preserving the established story facts and emotional stakes.`),
        allowedStateChanges: [],
        fallback: { dialogue: localized("fallback", `${profile.name} pauses, choosing their next words carefully.`), stateChanges: [] },
        blocking: true,
        outputSchema: agentOutputSchema(),
        ...timeline,
      },
      messages,
    };
  }
  if (definition.type === "choice") {
    return {
      config: {
        prompt: localized("prompt", "What will you do?"),
        options: [
          { id: "option-a", label: localized("option-a", "First choice"), mutations: [] },
          { id: "option-b", label: localized("option-b", "Second choice"), mutations: [] },
        ],
        ...timeline,
      },
      messages,
    };
  }
  if (definition.type === "host_action") {
    return { config: { target: "", action: "", completion: "required", ...timeline }, messages };
  }
  if (definition.type === "subtitle") {
    return { config: { text: localized("text", "Subtitle"), ...timeline }, messages };
  }
  if (definition.type === "scene") {
    return {
      config: {
        title: localized("title", "New scene"),
        description: localized("description", "Describe the setting and dramatic beat."),
        ...timeline,
      },
      messages,
    };
  }
  if (definition.type === "dialogue") {
    return {
      config: { speakerId: "", text: localized("text", "Write the next line."), blocking: true, ...timeline },
      messages,
    };
  }
  if (definition.type === "ending") {
    return {
      config: {
        endingId: nodeId,
        title: localized("title", "New ending"),
        text: localized("text", "Describe how this ending resolves the story."),
        sequenceId: `ending-${nodeId}`,
        startMs: 0,
        durationMs: defaultDuration(definition.type),
      },
      messages,
    };
  }
  if (definition.type === "input") {
    return {
      config: {
        commandType: "player.text",
        prompt: localized("prompt", "What do you say?"),
        saveAs: `${nodeId.replace(/-/g, "_")}_response`,
        multiline: true,
        ...timeline,
      },
      messages,
    };
  }
  if (["background", "video"].includes(definition.type)) {
    return { config: { fit: "cover", ...timeline }, messages };
  }
  if (definition.type === "character_visual") {
    return { config: { fit: "contain", ...timeline }, messages };
  }
  if (["state", "character_state", "relationship", "memory"].includes(definition.type)) {
    return { config: { key: "state_key", op: "set", value: true }, messages };
  }
  if (definition.type === "loop" || definition.type === "for_each") return { config: { maxIterations: 10 }, messages };
  if (definition.timelineCompatible) return { config: timeline, messages };
  return { config: {}, messages };
}

function agentOutputSchema(): Record<string, unknown> {
  return {
    type: "object",
    required: ["dialogue"],
    properties: {
      dialogue: { type: "string" },
      emotion: { type: "string" },
      presentationIntent: { type: "object" },
      stateChanges: {
        type: "array",
        items: {
          type: "object",
          required: ["key", "op", "value"],
          properties: {
            key: { type: "string" },
            op: { enum: ["set", "increment"] },
            value: {},
          },
          additionalProperties: false,
        },
      },
    },
    additionalProperties: false,
  };
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

function uniqueCharacterId(characters: GameCharacter[], seed: string): string {
  const base = seed
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "") || "character";
  const existing = new Set(characters.map((character) => character.id));
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

function trackPriority(trackId: string): number {
  const executionOrder = ["picture", "sound", "story", "cast", "interaction", "cue"];
  const index = executionOrder.indexOf(trackId);
  return index < 0 ? executionOrder.length : index;
}

function defaultDuration(nodeType: string): number {
  if (["scene", "episode", "video"].includes(nodeType)) return 5000;
  if (["dialogue", "agent", "audio", "voice"].includes(nodeType)) return 3000;
  return 1500;
}

function numberValue(value: unknown, fallback: number): number {
  return typeof value === "number" && Number.isFinite(value) ? value : fallback;
}

function subtitleTimestampMs(value: string | undefined): number | null {
  const timestamp = value?.trim().split(/\s+/, 1)[0]?.replace(",", ".");
  if (!timestamp) return null;
  const parts = timestamp.split(":").map(Number);
  if ((parts.length !== 2 && parts.length !== 3) || parts.some((part) => !Number.isFinite(part))) return null;
  const [hours, minutes, seconds] = parts.length === 3 ? parts : [0, parts[0], parts[1]];
  if (minutes < 0 || minutes >= 60 || seconds < 0 || seconds >= 60 || hours < 0) return null;
  return Math.round(((hours * 60 + minutes) * 60 + seconds) * 1000);
}
