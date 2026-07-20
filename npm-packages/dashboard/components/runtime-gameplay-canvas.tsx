"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useRouter } from "next/navigation";
import {
  Background,
  Controls,
  Handle,
  MarkerType,
  Position,
  ReactFlow,
  ReactFlowProvider,
  type Edge,
  type Node,
  type NodeProps,
  useEdgesState,
  useNodesState,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { AlertCircle, ArrowRight, Bot, CheckCircle2, Gamepad2, Plus, RadioTower, X } from "lucide-react";
import type {
  AgentBinding,
  AgentGateway,
  AgentProfile,
  EndpointTrace,
  ProjectCanvas,
  ProjectCanvasNode,
  ProviderAdapter,
  ProjectProvider,
  RuntimeProject,
} from "../lib/runtime-types";
import { RuntimeProfileWorkbench } from "./runtime-profile-workbench";

type RuntimeNodeData = {
  title: string;
  subtitle: string;
  kind: "endpoint" | "agent" | "gateway";
  status: "ready" | "pending" | "off";
  meta: string;
  detail?: string;
  canvasNode?: ProjectCanvasNode;
};

type GameplayCanvasProps = {
  project: RuntimeProject;
  canvas?: ProjectCanvas;
  profiles: AgentProfile[];
  bindings: AgentBinding[];
  agentGateways: AgentGateway[];
  traces: EndpointTrace[];
  providerAdapters: ProviderAdapter[];
  providerConnections: ProjectProvider[];
  browserApiBaseUrl: string;
};

const nodeTypes = {
  runtime: RuntimeNode,
};

export function RuntimeGameplayCanvas(props: GameplayCanvasProps) {
  return (
    <ReactFlowProvider>
      <RuntimeGameplayCanvasInner {...props} />
    </ReactFlowProvider>
  );
}

function RuntimeGameplayCanvasInner({
  project,
  canvas,
  profiles,
  bindings,
  agentGateways,
  traces,
  providerAdapters,
  providerConnections,
  browserApiBaseUrl,
}: GameplayCanvasProps) {
  const graph = useMemo(
    () => buildGraph({ project, canvas, profiles, bindings, agentGateways, providerAdapters, providerConnections, browserApiBaseUrl }),
    [project, canvas, profiles, bindings, agentGateways, providerAdapters, providerConnections, browserApiBaseUrl],
  );
  const [nodes, setNodes, onNodesChange] = useNodesState<Node<RuntimeNodeData>>(graph.nodes);
  const [edges, setEdges, onEdgesChange] = useEdgesState(graph.edges);
  const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null);

  useEffect(() => {
    setNodes(graph.nodes);
    setEdges(graph.edges);
    setSelectedNodeId((current) => current && graph.nodes.some((node) => node.id === current) ? current : null);
  }, [graph.nodes, graph.edges, setEdges, setNodes]);

  const selected = selectedNodeId ? nodes.find((node) => node.id === selectedNodeId) ?? null : null;
  const selectedCanvasNode = selected?.data.canvasNode;
  const selectedProfile = selectedCanvasNode?.profileId
    ? profiles.find((profile) => profile.id === selectedCanvasNode.profileId) ?? null
    : null;
  const latestFailure = traces.find((trace) => trace.status !== "completed" && trace.status !== "pending");
  const gameAgentCount = canvas?.nodes.length ?? 0;
  const readyAgentCount = graph.nodes.filter((node) => node.data.kind === "agent" && node.data.status === "ready").length;
  const onlineGatewayCount = connectedGatewayCount(agentGateways);

  const onNodeDragStop = useCallback(async (_event: unknown, node: Node<RuntimeNodeData>) => {
    const canvasNode = node.data.canvasNode;
    if (!canvasNode) return;
    await runtimeRequest(`project/${project.slug}/canvas/nodes/${canvasNode.id}`, "PATCH", {
      position: { x: Math.round(node.position.x), y: Math.round(node.position.y) },
    });
  }, [project.slug]);

  return (
    <section className={`gameplay-workspace ${selectedProfile ? "has-profile-workbench" : ""}`}>
      <div className="gameplay-canvas">
        <header className="canvas-command-bar">
          <div className="canvas-runtime-path" aria-label="Runtime path">
            <span><Gamepad2 aria-hidden="true" />Game API</span>
            <ArrowRight aria-hidden="true" />
            <span><Bot aria-hidden="true" />Agents</span>
            <ArrowRight aria-hidden="true" />
            <span><RadioTower aria-hidden="true" />Providers</span>
          </div>
          <div className="canvas-command-actions">
            <span className={`canvas-ready-count ${readyAgentCount < gameAgentCount ? "warning" : ""}`}>
              {readyAgentCount < gameAgentCount ? <AlertCircle aria-hidden="true" /> : <CheckCircle2 aria-hidden="true" />}
              {gameAgentCount > 0 ? `${readyAgentCount}/${gameAgentCount} ready` : "No agents yet"}
            </span>
            <span className="canvas-provider-count">{onlineGatewayCount} connected</span>
            <AddProfileDialog
              project={project}
              availableProfiles={graph.profilePalette}
              nextPosition={() => nextPalettePosition(nodes)}
            />
          </div>
        </header>
        <ReactFlow
          nodes={nodes}
          edges={edges}
          nodeTypes={nodeTypes}
          onNodesChange={onNodesChange}
          onEdgesChange={onEdgesChange}
          onNodeClick={(_event, node) => setSelectedNodeId(node.id)}
          onPaneClick={() => setSelectedNodeId(null)}
          onNodeDragStop={onNodeDragStop}
          fitView
          fitViewOptions={{ padding: 0.12, minZoom: 0.68, maxZoom: 0.92 }}
          minZoom={0.25}
          maxZoom={1.8}
          nodesConnectable={false}
          edgesFocusable={false}
          proOptions={{ hideAttribution: true }}
        >
          <Background gap={18} size={1.2} color="#d8dde6" />
          <Controls showInteractive={false} />
        </ReactFlow>
        {nodes.length <= 2 ? (
          <div className="canvas-empty-hint">
            <strong>Place a project agent</strong>
            <span>Choose an Agent to include in this gameplay layout.</span>
          </div>
        ) : null}
      </div>

      {selectedProfile && selectedCanvasNode ? (
        <RuntimeProfileWorkbench
          project={project}
          profile={selectedProfile}
          providerAdapters={providerAdapters}
          providerConnections={providerConnections}
          onClose={() => setSelectedNodeId(null)}
        />
      ) : (
        <NodeInspector selected={selected} latestFailure={latestFailure} onClose={() => setSelectedNodeId(null)} />
      )}
    </section>
  );
}

function RuntimeNode({ data }: NodeProps<Node<RuntimeNodeData>>) {
  const KindIcon = data.kind === "endpoint" ? Gamepad2 : data.kind === "gateway" ? RadioTower : Bot;
  const statusLabel = data.status === "ready" ? "Ready" : data.status === "pending" ? "Setup needed" : "Offline";
  return (
    <div className={`gameplay-node ${data.kind} ${data.status}`}>
      <Handle type="target" position={Position.Left} />
      <div className="gameplay-node-header">
        <span><KindIcon aria-hidden="true" />{nodeKindLabel(data.kind)}</span>
        <small className={`gameplay-node-status ${data.status}`}>
          {data.status === "off" ? <AlertCircle aria-hidden="true" /> : <CheckCircle2 aria-hidden="true" />}
          {statusLabel}
        </small>
      </div>
      <div className="gameplay-node-identity">
        {data.kind === "agent" ? <span className="gameplay-agent-mark" aria-hidden="true">{agentInitials(data.title)}</span> : null}
        <div><strong>{data.title}</strong><p>{data.subtitle}</p></div>
      </div>
      <footer>
        <span>{data.meta}</span>
      </footer>
      <Handle type="source" position={Position.Right} />
    </div>
  );
}

function AddProfileDialog({
  project,
  availableProfiles,
  nextPosition,
}: {
  project: RuntimeProject;
  availableProfiles: AgentProfile[];
  nextPosition: () => { x: number; y: number };
}) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const router = useRouter();

  function open() {
    setError(null);
    dialogRef.current?.showModal();
  }

  async function addExistingProfile(profile: AgentProfile) {
    setPending(true);
    setError(null);
    try {
      await runtimeRequest(`project/${project.slug}/canvas/nodes`, "POST", {
        kind: "agent",
        profileId: profile.id,
        position: nextPosition(),
        config: { source: "profile" },
        inputs: {},
        outputs: {},
      });
      dialogRef.current?.close();
      router.refresh();
    } catch (nextError) {
      setError(errorMessage(nextError));
    } finally {
      setPending(false);
    }
  }

  return (
    <>
      <button className="canvas-add-agent-button" type="button" onClick={open}><Plus aria-hidden="true" /><span>Add agent</span></button>
      <dialog
        className="canvas-add-dialog"
        ref={dialogRef}
        onClick={(event) => { if (event.target === event.currentTarget) event.currentTarget.close(); }}
      >
        <div className="canvas-add-dialog-shell">
          <header>
            <div><span>Project Agents</span><h2>Place an Agent</h2></div>
            <button className="icon-button" type="button" onClick={() => dialogRef.current?.close()} title="Close" aria-label="Close"><X aria-hidden="true" /></button>
          </header>
          <div className="canvas-detected-list">
            {availableProfiles.map((profile) => (
              <button type="button" key={profile.id} disabled={pending} onClick={() => void addExistingProfile(profile)}>
                <span><strong>{profile.name}</strong><code>{profile.slug}</code></span>
                <small>Project Agent</small>
                <Plus aria-hidden="true" />
              </button>
            ))}
            {availableProfiles.length === 0 ? (
              <div className="canvas-dialog-empty">
                <Bot aria-hidden="true" />
                <strong>No unplaced Agents</strong>
                <span>Add or detect Agents from the project library first.</span>
                <button className="secondary-button" type="button" onClick={() => router.push(`/project/${project.slug}/agents`)}>Open Agents</button>
              </div>
            ) : null}
          </div>
          {error ? <p className="inline-error" role="alert">{error}</p> : null}
        </div>
      </dialog>
    </>
  );
}

function NodeInspector({
  selected,
  latestFailure,
  onClose,
}: {
  selected: Node<RuntimeNodeData> | null;
  latestFailure?: EndpointTrace;
  onClose: () => void;
}) {
  if (!selected) {
    return latestFailure ? <p className="inspector-alert floating-alert">Latest issue: {latestFailure.error ?? latestFailure.status}</p> : null;
  }

  return (
    <aside className="node-inspector">
      <header>
        <div>
          <span>{nodeKindLabel(selected.data.kind)}</span>
          <strong>{selected.data.title}</strong>
        </div>
        <button type="button" onClick={onClose} aria-label="Close inspector"><X aria-hidden="true" /></button>
      </header>
      <dl>
        <div><dt>Status</dt><dd>{selected.data.status === "ready" ? "Ready" : selected.data.status === "pending" ? "Setup needed" : "Offline"}</dd></div>
        <div><dt>{selected.data.kind === "endpoint" ? "Agents" : "Connection"}</dt><dd>{selected.data.meta}</dd></div>
        <div><dt>{selected.data.kind === "endpoint" ? "Address" : "Runtime"}</dt><dd>{selected.data.detail ?? selected.data.subtitle}</dd></div>
      </dl>
      <p>{selected.data.kind === "endpoint" ? "Every active project Agent is available through this API." : "This provider supplies the Agents connected to the project."}</p>
    </aside>
  );
}

function buildGraph({
  project,
  canvas,
  profiles,
  bindings,
  agentGateways,
  providerConnections,
  browserApiBaseUrl,
}: Omit<GameplayCanvasProps, "traces">): {
  nodes: Node<RuntimeNodeData>[];
  edges: Edge[];
  profilePalette: AgentProfile[];
} {
  const profileById = new Map(profiles.map((profile) => [profile.id, profile]));
  const bindingById = new Map(bindings.map((binding) => [binding.id, binding]));
  const gatewayById = gatewayStatusMap(agentGateways);
  const canvasNodes = canvas?.nodes ?? [];
  const agentStatusByNodeId = new Map<string, RuntimeNodeData["status"]>();
  const providerNodeIdByCanvasNodeId = new Map<string, string>();
  const providerGroups = new Map<string, {
    provider?: ProjectProvider;
    gatewayIds: Set<string>;
    canvasNodeIds: string[];
  }>();
  const canvasProfileIds = new Set(canvasNodes.flatMap((node) => node.profileId ? [node.profileId] : []));
  const profilePalette = profiles.filter((profile) => (
    profile.projectId === project.id
    && !profile.archivedAt
    && !canvasProfileIds.has(profile.id)
  ));
  const nodes: Node<RuntimeNodeData>[] = [
    {
      id: `endpoint:${project.id}`,
      type: "runtime",
      position: { x: 20, y: 230 },
      data: {
        title: "Game API",
        subtitle: project.name,
        kind: "endpoint",
        status: project.enabled ? "ready" : "off",
        meta: `${profiles.length} agent${profiles.length === 1 ? "" : "s"} available`,
        detail: projectChatCompletionsUrl(project, browserApiBaseUrl),
      },
      draggable: false,
    },
  ];

  for (const node of canvasNodes) {
    const profile = node.profileId ? profileById.get(node.profileId) : null;
    const binding = node.bindingId ? bindingById.get(node.bindingId) : null;
    const gateway = node.gatewayId ? gatewayById.get(node.gatewayId) : null;
    const provider = providerForCanvasNode(node, providerConnections);
    const status: RuntimeNodeData["status"] = gateway?.status === "connected" || provider?.status === "online"
      ? "ready"
      : profile?.activeVersionId
        ? "pending"
        : "off";
    agentStatusByNodeId.set(node.id, status);
    const providerNodeId = graphProviderNodeId(node, provider);
    if (providerNodeId) {
      providerNodeIdByCanvasNodeId.set(node.id, providerNodeId);
      const group = providerGroups.get(providerNodeId) ?? {
        provider,
        gatewayIds: new Set<string>(),
        canvasNodeIds: [],
      };
      if (node.gatewayId) group.gatewayIds.add(node.gatewayId);
      group.canvasNodeIds.push(node.id);
      providerGroups.set(providerNodeId, group);
    }
    nodes.push({
      id: node.id,
      type: "runtime",
      position: readPosition(node.position, nodes.length),
      data: {
        title: profile?.name ?? String(node.config.agentName ?? node.resourceId ?? "Agent"),
        subtitle: profile?.description ?? "Ready to shape and playtest",
        kind: "agent",
        status,
        meta: binding ? provider?.name ?? "OpenClaw" : provider?.name ?? "Provider not connected",
        canvasNode: node,
      },
    });
  }

  let providerIndex = 0;
  for (const [providerNodeId, group] of providerGroups) {
    const gateways = [...group.gatewayIds].flatMap((gatewayId) => {
      const gateway = gatewayById.get(gatewayId);
      return gateway ? [gateway] : [];
    });
    const readyAgents = group.canvasNodeIds.filter((nodeId) => agentStatusByNodeId.get(nodeId) === "ready").length;
    const connected = gateways.some((gateway) => gateway.status === "connected") || group.provider?.status === "online";
    nodes.push({
      id: providerNodeId,
      type: "runtime",
      position: { x: 880, y: 230 + providerIndex * 210 },
      data: {
        title: group.provider?.name ?? "Agent provider",
        subtitle: `${readyAgents}/${group.canvasNodeIds.length} agent${group.canvasNodeIds.length === 1 ? "" : "s"} ready`,
        kind: "gateway",
        status: connected ? "ready" : "off",
        meta: connected ? "Connected" : "Needs connection",
        detail: group.provider?.providerType ?? "Agent provider",
      },
      draggable: false,
    });
    providerIndex += 1;
  }

  const edges: Edge[] = [];
  for (const node of canvasNodes) {
    edges.push({
      id: `endpoint:${node.id}`,
      source: `endpoint:${project.id}`,
      target: node.id,
      animated: true,
      className: "gameplay-edge runtime",
      markerEnd: { type: MarkerType.ArrowClosed, color: "#5367e8" },
    });
    const providerNodeId = providerNodeIdByCanvasNodeId.get(node.id);
    if (providerNodeId) {
      edges.push({
        id: `gateway:${node.id}`,
        source: node.id,
        target: providerNodeId,
        className: "gameplay-edge runtime",
        markerEnd: { type: MarkerType.ArrowClosed, color: "#7d8796" },
      });
    }
  }
  for (const edge of canvas?.edges ?? []) {
    edges.push({
      id: edge.id,
      source: edge.sourceNodeId,
      target: edge.targetNodeId,
      sourceHandle: edge.sourceHandle ?? undefined,
      targetHandle: edge.targetHandle ?? undefined,
      className: "gameplay-edge custom",
      markerEnd: { type: MarkerType.ArrowClosed, color: "#687284" },
    });
  }

  return { nodes, edges, profilePalette };
}

function readPosition(value: Record<string, unknown>, index: number): { x: number; y: number } {
  const x = typeof value.x === "number" ? value.x : 360 + (index % 3) * 280;
  const y = typeof value.y === "number" ? value.y : 160 + Math.floor(index / 3) * 190;
  return { x, y };
}

function nextPalettePosition(nodes: Node[]): { x: number; y: number } {
  return { x: 360 + (nodes.length % 3) * 280, y: 160 + Math.floor(nodes.length / 3) * 190 };
}

function projectChatCompletionsUrl(project: RuntimeProject, browserApiBaseUrl: string): string {
  const url = new URL(browserApiBaseUrl);
  const basePath = url.pathname.replace(/\/+$/, "");
  url.pathname = `${basePath}/${encodeURIComponent(project.slug)}/v1/chat/completions`;
  url.search = "";
  return url.toString().replace(/\/$/, "");
}

async function runtimeRequest<T = unknown>(path: string, method: string, body?: unknown): Promise<T> {
  const response = await fetch(`/api/runtime/${path}`, {
    method,
    headers: body === undefined ? undefined : { "content-type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (response.status === 204) return undefined as T;
  const payload = await response.json().catch(() => null) as T | { error?: { message?: unknown } } | null;
  if (!response.ok) {
    const message = payload && typeof payload === "object" && "error" in payload
      ? (payload as { error?: { message?: unknown } }).error?.message
      : null;
    throw new Error(typeof message === "string" ? message : "Runtime request failed.");
  }
  return (payload ?? {}) as T;
}

function metadataString(value: Record<string, unknown>, key: string): string {
  return typeof value[key] === "string" ? value[key] : "";
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : "Request failed.";
}

function gatewayStatusMap(gateways: AgentGateway[]): Map<string, AgentGateway> {
  const byId = new Map<string, AgentGateway>();
  for (const gateway of gateways) {
    const current = byId.get(gateway.gatewayId);
    if (!current || (current.status !== "connected" && gateway.status === "connected")) {
      byId.set(gateway.gatewayId, gateway);
    }
  }
  return byId;
}

function providerForCanvasNode(node: ProjectCanvasNode, connections: ProjectProvider[]): ProjectProvider | undefined {
  const providerKey = metadataString(node.config, "providerKey");
  const providerType = metadataString(node.config, "providerType");
  return connections.find((connection) => connection.providerKey === providerKey)
    ?? connections.find((connection) => providerType && connection.providerType === providerType)
    ?? (connections.length === 1 ? connections[0] : undefined);
}

function graphProviderNodeId(node: ProjectCanvasNode, provider?: ProjectProvider): string | null {
  if (provider) return `provider:${provider.id}`;
  const providerKey = metadataString(node.config, "providerKey");
  if (providerKey) return `provider-key:${providerKey}`;
  return node.gatewayId ? `gateway:${node.gatewayId}` : null;
}

function connectedGatewayCount(gateways: AgentGateway[]): number {
  return [...gatewayStatusMap(gateways).values()].filter((gateway) => gateway.status === "connected").length;
}

function nodeKindLabel(kind: RuntimeNodeData["kind"]): string {
  if (kind === "endpoint") return "Game entry";
  if (kind === "gateway") return "Provider";
  return "Agent";
}

function agentInitials(name: string): string {
  const words = name.trim().split(/[\s_-]+/).filter(Boolean);
  return words.slice(0, 2).map((word) => word[0]?.toUpperCase() ?? "").join("") || "AI";
}
