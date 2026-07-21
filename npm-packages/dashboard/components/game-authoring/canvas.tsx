"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import {
  Background,
  Controls,
  Handle,
  MarkerType,
  MiniMap,
  Position,
  ReactFlow,
  ReactFlowProvider,
  type Connection,
  type Edge,
  type Node,
  type NodeProps,
  useEdgesState,
  useNodesState,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import {
  Bot,
  Boxes,
  Braces,
  ChevronLeft,
  CircleDot,
  Clapperboard,
  GitBranch,
  Layers3,
  Plus,
  Search,
  Sparkles,
  Workflow,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import {
  bindManagedPresentationAsset,
  canvasLayoutPositions,
  definitionForNode,
  nextCanvasNodePosition,
  nodeInputPorts,
  nodeOutputPorts,
  objectValue,
} from "../../lib/game-authoring";
import type {
  AgentProfile,
  GameDraft,
  GameAsset,
  GameNodeDefinition,
  GameResource,
  GameSourceNode,
  RuntimeProject,
} from "../../lib/runtime-types";
import { GameNodeInspector } from "./inspector";
import { GameAuthoringProvider, useGameAuthoring, useGameAuthoringStore, useGameDraftSync } from "./store";
import { GameAuthoringToolbar } from "./toolbar";

type CanvasNodeData = {
  sourceNode: GameSourceNode;
  definition?: GameNodeDefinition;
  inputPorts: string[];
  outputPorts: string[];
  hasIssue: boolean;
  profile?: AgentProfile;
};

type CanvasGameNode = Node<CanvasNodeData>;

const nodeTypes = { game: GameCanvasNode };

export function RuntimeGameCanvas({
  project,
  draft,
  definitions,
  profiles,
  resources,
  assets,
}: {
  project: RuntimeProject;
  draft: GameDraft;
  definitions: GameNodeDefinition[];
  profiles: AgentProfile[];
  resources: GameResource[];
  assets: GameAsset[];
}) {
  return (
    <GameAuthoringProvider draft={draft}>
      <ReactFlowProvider>
        <GameCanvasWorkspace project={project} definitions={definitions} profiles={profiles} resources={resources} assets={assets} />
      </ReactFlowProvider>
    </GameAuthoringProvider>
  );
}

function GameCanvasWorkspace({
  project,
  definitions,
  profiles,
  resources,
  assets,
}: {
  project: RuntimeProject;
  definitions: GameNodeDefinition[];
  profiles: AgentProfile[];
  resources: GameResource[];
  assets: GameAsset[];
}) {
  useGameDraftSync(project.slug);
  const source = useGameAuthoring((state) => state.source);
  const selectedNodeId = useGameAuthoring((state) => state.selectedNodeId);
  const setSelectedNode = useGameAuthoring((state) => state.setSelectedNode);
  const moveNode = useGameAuthoring((state) => state.moveNode);
  const addEdge = useGameAuthoring((state) => state.addEdge);
  const deleteEdge = useGameAuthoring((state) => state.deleteEdge);
  const deleteNode = useGameAuthoring((state) => state.deleteNode);
  const issues = useGameAuthoring((state) => state.validationIssues);
  const graph = useMemo(() => buildCanvasGraph(source.graph.nodes, source.graph.edges, source, definitions, profiles, issues), [definitions, issues, profiles, source]);
  const [nodes, setNodes, onNodesChange] = useNodesState<CanvasGameNode>(graph.nodes);
  const [edges, setEdges, onEdgesChange] = useEdgesState(graph.edges);
  const [paletteOpen, setPaletteOpen] = useState(false);

  useEffect(() => {
    setNodes(graph.nodes);
    setEdges(graph.edges);
  }, [graph.edges, graph.nodes, setEdges, setNodes]);

  const onConnect = useCallback((connection: Connection) => {
    if (!connection.source || !connection.target) return;
    addEdge({
      id: uniqueEdgeId(source.graph.edges.map((edge) => edge.id), connection.source, connection.target),
      source: { nodeId: connection.source, port: connection.sourceHandle || "next" },
      target: { nodeId: connection.target, port: connection.targetHandle || "in" },
    });
  }, [addEdge, source.graph.edges]);

  return (
    <section className={`game-authoring-workspace canvas-workspace ${selectedNodeId ? "inspector-open" : ""} ${paletteOpen ? "palette-open" : ""}`}>
      <GameAuthoringToolbar projectSlug={project.slug} viewLabel="Canvas" />
      <div className="game-editor-stage">
        <CanvasControlRail onOpenPalette={() => setPaletteOpen(true)} />
        {paletteOpen ? (
          <NodePalette
            definitions={definitions}
            profiles={profiles}
            resources={resources}
            assets={assets}
            onClose={() => setPaletteOpen(false)}
          />
        ) : null}
        <div className="game-flow-surface">
          <ReactFlow
            nodes={nodes}
            edges={edges}
            nodeTypes={nodeTypes}
            onNodesChange={onNodesChange}
            onEdgesChange={onEdgesChange}
            onConnect={onConnect}
            onNodeClick={(_event, node) => setSelectedNode(node.id)}
            onPaneClick={() => setSelectedNode(null)}
            onNodeDragStop={(_event, node) => moveNode(node.id, node.position)}
            onNodesDelete={(removed) => removed.forEach((node) => deleteNode(node.id))}
            onEdgesDelete={(removed) => removed.forEach((edge) => deleteEdge(edge.id))}
            fitView
            fitViewOptions={{ padding: 0.2, minZoom: 0.5, maxZoom: 1 }}
            minZoom={0.2}
            maxZoom={1.8}
            deleteKeyCode={["Backspace", "Delete"]}
            proOptions={{ hideAttribution: true }}
          >
            <Background gap={20} size={1} color="#d5d8de" />
            <Controls showInteractive={false} />
            <MiniMap pannable zoomable nodeStrokeWidth={2} />
          </ReactFlow>
        </div>
        <GameNodeInspector definitions={definitions} profiles={profiles} />
      </div>
    </section>
  );
}

function CanvasControlRail({ onOpenPalette }: { onOpenPalette: () => void }) {
  return (
    <nav className="canvas-control-rail" aria-label="Canvas tools">
      <button type="button" onClick={onOpenPalette} title="Add node" aria-label="Add node"><Plus aria-hidden="true" /></button>
      <button type="button" title="Runtime nodes" aria-label="Runtime nodes"><Workflow aria-hidden="true" /></button>
      <button type="button" title="Agents" aria-label="Agents"><Bot aria-hidden="true" /></button>
      <button type="button" title="Resources" aria-label="Resources"><Boxes aria-hidden="true" /></button>
    </nav>
  );
}

function NodePalette({
  definitions,
  profiles,
  resources,
  assets,
  onClose,
}: {
  definitions: GameNodeDefinition[];
  profiles: AgentProfile[];
  resources: GameResource[];
  assets: GameAsset[];
  onClose: () => void;
}) {
  const source = useGameAuthoring((state) => state.source);
  const addNode = useGameAuthoring((state) => state.addNode);
  const store = useGameAuthoringStore();
  const [query, setQuery] = useState("");
  const [phase, setPhase] = useState<"runtime" | "production">("runtime");
  const normalized = query.trim().toLowerCase();
  const available = definitions.filter((definition) => (
    definition.phase === phase
    && (definition.type !== "start" || !source.graph.nodes.some((node) => node.type === "start"))
    && (!normalized || `${definition.title} ${definition.category} ${definition.type}`.toLowerCase().includes(normalized))
  ));
  const grouped = groupDefinitions(available);
  const agentDefinition = definitions.find((definition) => definition.type === "agent");
  const resourceDefinition = definitions.find((definition) => definition.type === "resource");
  const assetDefinition = definitions.find((definition) => definition.type === "asset");

  function addDefinition(definition: GameNodeDefinition, profile?: AgentProfile) {
    const before = new Set(store.getState().source.graph.nodes.map((node) => node.id));
    addNode(definition, nextCanvasNodePosition(source.graph.nodes.length), profile);
    const added = store.getState().source.graph.nodes.find((node) => !before.has(node.id));
    if (added) store.getState().setSelectedNode(added.id);
    onClose();
  }

  function addProjectResource(definition: GameNodeDefinition, resource: GameResource) {
    const before = new Set(store.getState().source.graph.nodes.map((node) => node.id));
    addNode(definition, nextCanvasNodePosition(source.graph.nodes.length));
    const added = store.getState().source.graph.nodes.find((node) => !before.has(node.id));
    if (added) store.getState().updateNode({
      ...added,
      label: resource.name,
      config: {
        ...added.config,
        resourceId: resource.resourceKey,
        versionId: resource.id,
        kind: resource.kind,
        contentHash: resource.contentHash,
      },
    });
    onClose();
  }

  function addProjectAsset(definition: GameNodeDefinition, asset: GameAsset) {
    const version = asset.versions.find((item) => item.approvalStatus === "approved");
    if (!version) return;
    const before = new Set(store.getState().source.graph.nodes.map((node) => node.id));
    addNode(definition, nextCanvasNodePosition(source.graph.nodes.length));
    const added = store.getState().source.graph.nodes.find((node) => !before.has(node.id));
    if (added) store.getState().updateNode({
      ...added,
      label: asset.name,
      config: {
        ...added.config,
        logicalResourceId: asset.assetKey,
        kind: asset.kind,
        fit: "cover",
      },
    });
    store.getState().setSource(bindManagedPresentationAsset(store.getState().source, asset, version));
    onClose();
  }

  return (
    <aside className="game-node-palette">
      <header><strong>Add node</strong><button type="button" onClick={onClose} aria-label="Close node library"><ChevronLeft aria-hidden="true" /></button></header>
      <label className="node-palette-search"><Search aria-hidden="true" /><input type="search" value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search nodes" /></label>
      <div className="node-palette-tabs" role="tablist">
        <button type="button" role="tab" aria-selected={phase === "runtime"} onClick={() => setPhase("runtime")}>Runtime</button>
        <button type="button" role="tab" aria-selected={phase === "production"} onClick={() => setPhase("production")}>Build</button>
      </div>
      <div className="node-palette-scroll">
        {phase === "runtime" && agentDefinition && profiles.length > 0 ? (
          <PaletteGroup label="Project Agents">
            {profiles.filter((profile) => !profile.archivedAt).map((profile) => (
              <PaletteButton key={profile.id} icon={Bot} title={profile.name} detail={profile.slug} onClick={() => addDefinition(agentDefinition, profile)} />
            ))}
          </PaletteGroup>
        ) : null}
        {phase === "runtime" && resourceDefinition && resources.length > 0 ? (
          <PaletteGroup label="Project Resources">
            {resources.filter((resource) => resource.approved).map((resource) => (
              <PaletteButton key={resource.id} icon={Boxes} title={resource.name} detail={`${resource.kind} · v${resource.version}`} onClick={() => addProjectResource(resourceDefinition, resource)} />
            ))}
          </PaletteGroup>
        ) : null}
        {phase === "runtime" && assetDefinition && assets.some((asset) => asset.versions.some((version) => version.approvalStatus === "approved")) ? (
          <PaletteGroup label="Project Media">
            {assets.filter((asset) => asset.versions.some((version) => version.approvalStatus === "approved")).map((asset) => (
              <PaletteButton key={asset.id} icon={Clapperboard} title={asset.name} detail={asset.kind} onClick={() => addProjectAsset(assetDefinition, asset)} />
            ))}
          </PaletteGroup>
        ) : null}
        {[...grouped].map(([category, items]) => (
          <PaletteGroup label={category} key={category}>
            {items.map((definition) => (
              <PaletteButton key={`${definition.type}:${definition.version}`} icon={categoryIcon(definition.category)} title={definition.title} detail={definition.type} onClick={() => addDefinition(definition)} />
            ))}
          </PaletteGroup>
        ))}
        {available.length === 0 ? <p className="node-palette-empty">No matching nodes.</p> : null}
      </div>
    </aside>
  );
}

function PaletteGroup({ children, label }: { children: React.ReactNode; label: string }) {
  return <section className="node-palette-group"><h3>{label}</h3>{children}</section>;
}

function PaletteButton({ icon: Icon, title, detail, onClick }: { icon: LucideIcon; title: string; detail: string; onClick: () => void }) {
  return <button type="button" onClick={onClick}><span><Icon aria-hidden="true" /></span><div><strong>{title}</strong><small>{detail}</small></div><Plus aria-hidden="true" /></button>;
}

function GameCanvasNode({ data }: NodeProps<CanvasGameNode>) {
  const icon = categoryIcon(data.definition?.category ?? "Flow");
  const Icon = icon;
  const summary = nodeSummary(data.sourceNode, data.profile);
  return (
    <article className={`game-canvas-node category-${categoryClass(data.definition?.category)} ${data.hasIssue ? "invalid" : ""}`}>
      {data.inputPorts.map((port, index) => (
        <Handle
          key={`input:${port}`}
          id={port}
          type="target"
          position={Position.Left}
          style={{ top: handleOffset(index, data.inputPorts.length) }}
          title={port}
        />
      ))}
      <header><span><Icon aria-hidden="true" />{data.definition?.category ?? "Node"}</span><code>v{data.sourceNode.version}</code></header>
      <div className="game-canvas-node-body"><strong>{data.sourceNode.label || data.definition?.title || data.sourceNode.type}</strong><p>{summary}</p></div>
      <footer><code>{data.sourceNode.type}</code>{data.hasIssue ? <span>Needs attention</span> : null}</footer>
      {data.outputPorts.map((port, index) => (
        <Handle
          key={`output:${port}`}
          id={port}
          type="source"
          position={Position.Right}
          style={{ top: handleOffset(index, data.outputPorts.length) }}
          title={port}
        />
      ))}
    </article>
  );
}

function buildCanvasGraph(
  sourceNodes: GameSourceNode[],
  sourceEdges: Array<{ id: string; source: { nodeId: string; port: string }; target: { nodeId: string; port: string } }>,
  source: Parameters<typeof canvasLayoutPositions>[0],
  definitions: GameNodeDefinition[],
  profiles: AgentProfile[],
  issues: Array<{ nodeId?: string | null }>,
): { nodes: CanvasGameNode[]; edges: Edge[] } {
  const profileById = new Map(profiles.map((profile) => [profile.id, profile]));
  const agentReferenceById = new Map(source.agents.map((agent) => [agent.id, agent]));
  const positions = canvasLayoutPositions(source);
  return {
    nodes: sourceNodes.map((node) => {
      const definition = definitionForNode(definitions, node);
      const agentReference = typeof node.config.agentId === "string" ? agentReferenceById.get(node.config.agentId) : undefined;
      return {
        id: node.id,
        type: "game",
        position: positions[node.id] ?? { x: 160, y: 120 },
        data: {
          sourceNode: node,
          definition,
          inputPorts: nodeInputPorts(definition),
          outputPorts: nodeOutputPorts(definition, node),
          hasIssue: issues.some((issue) => issue.nodeId === node.id),
          profile: agentReference ? profileById.get(agentReference.profileId) : undefined,
        },
        deletable: node.id !== source.entryNodeId,
      };
    }),
    edges: sourceEdges.map((edge) => ({
      id: edge.id,
      source: edge.source.nodeId,
      sourceHandle: edge.source.port,
      target: edge.target.nodeId,
      targetHandle: edge.target.port,
      markerEnd: { type: MarkerType.ArrowClosed, color: "#69717d" },
      className: "game-source-edge",
    })),
  };
}

function groupDefinitions(definitions: GameNodeDefinition[]): Map<string, GameNodeDefinition[]> {
  const groups = new Map<string, GameNodeDefinition[]>();
  for (const definition of definitions) {
    const current = groups.get(definition.category) ?? [];
    current.push(definition);
    groups.set(definition.category, current);
  }
  return groups;
}

function categoryIcon(category: string): LucideIcon {
  if (category === "Narrative") return Clapperboard;
  if (category === "Characters") return Bot;
  if (category === "Logic") return GitBranch;
  if (category === "Presentation") return Layers3;
  if (category === "Integration") return Boxes;
  if (category === "Production") return Sparkles;
  return CircleDot;
}

function categoryClass(category?: string): string {
  return (category ?? "flow").toLowerCase().replace(/[^a-z0-9]+/g, "-");
}

function nodeSummary(node: GameSourceNode, profile?: AgentProfile): string {
  if (profile) return profile.description || profile.slug;
  for (const key of ["prompt", "title", "text", "action", "target"]) {
    if (typeof node.config[key] === "string" && node.config[key]) return String(node.config[key]);
  }
  const count = Object.keys(objectValue(node.config)).length;
  return count > 0 ? `${count} configured field${count === 1 ? "" : "s"}` : "Select to configure";
}

function uniqueEdgeId(existing: string[], source: string, target: string): string {
  const base = `edge-${source}-${target}`.replace(/[^a-zA-Z0-9_-]/g, "-");
  if (!existing.includes(base)) return base;
  let suffix = 2;
  while (existing.includes(`${base}-${suffix}`)) suffix += 1;
  return `${base}-${suffix}`;
}

function handleOffset(index: number, count: number): string {
  return `${((index + 1) / (count + 1)) * 100}%`;
}
