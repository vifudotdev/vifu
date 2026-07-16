"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { useRouter } from "next/navigation";
import {
  Background,
  Controls,
  Handle,
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
import { Download, Plus, Upload, X } from "lucide-react";
import type {
  AgentBinding,
  AgentEndpoint,
  AgentGateway,
  AgentProfile,
  AvailableAgent,
  EndpointTrace,
  ProjectCanvas,
  ProjectCanvasNode,
  RuntimeProject,
} from "../lib/runtime-types";

type RuntimeNodeData = {
  title: string;
  subtitle: string;
  kind: "endpoint" | "agent" | "gateway";
  status: "ready" | "pending" | "off";
  meta: string;
  exposed?: boolean;
  canvasNode?: ProjectCanvasNode;
};

type GameplayCanvasProps = {
  project: RuntimeProject;
  canvas?: ProjectCanvas;
  profiles: AgentProfile[];
  bindings: AgentBinding[];
  agentGateways: AgentGateway[];
  availableAgents: AvailableAgent[];
  endpoints: AgentEndpoint[];
  traces: EndpointTrace[];
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
  availableAgents,
  endpoints,
  traces,
  browserApiBaseUrl,
}: GameplayCanvasProps) {
  const graph = useMemo(
    () => buildGraph({ project, canvas, profiles, bindings, agentGateways, availableAgents, endpoints, browserApiBaseUrl }),
    [project, canvas, profiles, bindings, agentGateways, availableAgents, endpoints, browserApiBaseUrl],
  );
  const [nodes, setNodes, onNodesChange] = useNodesState<Node<RuntimeNodeData>>(graph.nodes);
  const [edges, setEdges, onEdgesChange] = useEdgesState(graph.edges);
  const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [pendingResource, setPendingResource] = useState<string | null>(null);
  const router = useRouter();

  useEffect(() => {
    setNodes(graph.nodes);
    setEdges(graph.edges);
    setSelectedNodeId((current) => current && graph.nodes.some((node) => node.id === current) ? current : null);
  }, [graph.nodes, graph.edges, setEdges, setNodes]);

  const selected = selectedNodeId ? nodes.find((node) => node.id === selectedNodeId) ?? null : null;
  const latestFailure = traces.find((trace) => trace.status !== "completed" && trace.status !== "pending");

  const onNodeDragStop = useCallback(async (_event: unknown, node: Node<RuntimeNodeData>) => {
    const canvasNode = node.data.canvasNode;
    if (!canvasNode) return;
    await runtimeRequest(`project/${project.slug}/canvas/nodes/${canvasNode.id}`, "PATCH", {
      position: { x: Math.round(node.position.x), y: Math.round(node.position.y) },
    });
  }, [project.slug]);

  const addResource = useCallback(async (agent: AvailableAgent) => {
    setActionError(null);
    setPendingResource(`${agent.gatewayId}/${agent.id}`);
    try {
      await runtimeRequest(`project/${project.slug}/canvas/nodes`, "POST", {
        kind: "agent",
        gatewayId: agent.gatewayId,
        resourceId: agent.id,
        position: nextPalettePosition(nodes),
        config: {
          source: "detected-resource",
          provider: "openclaw",
          agentName: agent.name,
          metadata: agent.metadata,
        },
        inputs: {},
        outputs: {},
        exposed: true,
      });
      router.refresh();
    } catch (error) {
      setActionError(errorMessage(error));
    } finally {
      setPendingResource(null);
    }
  }, [nodes, project.slug, router]);

  return (
    <section className="gameplay-workspace">
      <div className="gameplay-canvas">
        <div className="canvas-tool-panel" aria-label="Canvas tools">
          {graph.palette.length === 0 ? (
            <button type="button" disabled title="All detected agents are on this canvas" aria-label="All detected agents are on this canvas">
              <Plus aria-hidden="true" />
            </button>
          ) : (
            graph.palette.map((agent) => {
              const key = `${agent.gatewayId}/${agent.id}`;
              return (
                <button key={key} type="button" onClick={() => addResource(agent)} disabled={pendingResource === key} title={`Add ${agent.name}`} aria-label={`Add ${agent.name}`}>
                  <Plus aria-hidden="true" />
                  <span className="sr-only">Add {agent.name}</span>
                </button>
              );
            })
          )}
        </div>
        {actionError ? <p className="canvas-action-error" role="alert">{actionError}</p> : null}
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
          fitViewOptions={{ padding: 0.16, minZoom: 0.42, maxZoom: 1 }}
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
            <strong>Build this project from agent nodes.</strong>
            <span>Start Vifu Gateway or add detected agents from the resource palette.</span>
          </div>
        ) : null}
      </div>

      <NodeInspector
        project={project}
        selected={selected}
        latestFailure={latestFailure}
        onClose={() => setSelectedNodeId(null)}
      />
    </section>
  );
}

function RuntimeNode({ data }: NodeProps<Node<RuntimeNodeData>>) {
  return (
    <div className={`gameplay-node ${data.kind} ${data.status}`}>
      <Handle type="target" position={Position.Left} />
      <div className="gameplay-node-header">
        <span>{data.kind}</span>
        {data.exposed !== undefined ? <small>{data.exposed ? "Exposed" : "Hidden"}</small> : null}
      </div>
      <strong>{data.title}</strong>
      <p>{data.subtitle}</p>
      <code>{data.meta}</code>
      <Handle type="source" position={Position.Right} />
    </div>
  );
}

function NodeInspector({
  project,
  selected,
  latestFailure,
  onClose,
}: {
  project: RuntimeProject;
  selected: Node<RuntimeNodeData> | null;
  latestFailure?: EndpointTrace;
  onClose: () => void;
}) {
  const [pending, setPending] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const router = useRouter();
  const canvasNode = selected?.data.canvasNode;

  async function patchNode(body: Record<string, unknown>) {
    if (!canvasNode) return;
    setPending(true);
    setMessage(null);
    try {
      await runtimeRequest(`project/${project.slug}/canvas/nodes/${canvasNode.id}`, "PATCH", body);
      router.refresh();
    } catch (error) {
      setMessage(errorMessage(error));
    } finally {
      setPending(false);
    }
  }

  async function removeNode() {
    if (!canvasNode || !window.confirm(`Remove ${selected?.data.title ?? "node"} from this project?`)) return;
    setPending(true);
    setMessage(null);
    try {
      await runtimeRequest(`project/${project.slug}/canvas/nodes/${canvasNode.id}`, "DELETE");
      router.refresh();
    } catch (error) {
      setMessage(errorMessage(error));
    } finally {
      setPending(false);
    }
  }

  async function importProfile(file: File | null) {
    if (!file || !canvasNode) return;
    try {
      const text = await file.text();
      const profile = JSON.parse(text) as unknown;
      await patchNode({ config: { ...canvasNode.config, importedProfile: profile } });
    } catch (error) {
      setMessage(errorMessage(error));
    }
  }

  function exportProfile() {
    if (!canvasNode || !selected) return;
    const profile = {
      profile: {
        id: canvasNode.profileId,
        name: selected.data.title,
        kind: canvasNode.kind,
        provider: "openclaw",
        gatewayId: canvasNode.gatewayId,
        resourceId: canvasNode.resourceId,
        inputs: canvasNode.inputs,
        outputs: canvasNode.outputs,
        capabilities: canvasNode.config.capabilities ?? [],
        metadata: canvasNode.config,
      },
    };
    const blob = new Blob([JSON.stringify(profile, null, 2)], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = `${slugify(selected.data.title)}.vifu-profile.json`;
    link.click();
    URL.revokeObjectURL(url);
  }

  if (!selected) {
    return latestFailure ? <p className="inspector-alert floating-alert">Latest issue: {latestFailure.error ?? latestFailure.status}</p> : null;
  }

  return (
    <aside className="node-inspector">
      <header>
        <div>
          <span>{selected.data.kind}</span>
          <strong>{selected.data.title}</strong>
        </div>
        <button type="button" onClick={onClose} aria-label="Close inspector"><X aria-hidden="true" /></button>
      </header>
      <dl>
        <div><dt>Status</dt><dd>{selected.data.status}</dd></div>
        <div><dt>Resource</dt><dd>{selected.data.meta}</dd></div>
        <div><dt>Details</dt><dd>{selected.data.subtitle}</dd></div>
      </dl>
      {canvasNode ? (
        <div className="inspector-actions">
          <button type="button" className="secondary-button" disabled={pending} onClick={() => patchNode({ exposed: !canvasNode.exposed })}>
            {canvasNode.exposed ? "Hide from endpoint" : "Expose to endpoint"}
          </button>
          <label className="file-action">
            <Upload aria-hidden="true" /> Import profile
            <input type="file" accept="application/json" onChange={(event) => importProfile(event.currentTarget.files?.[0] ?? null)} />
          </label>
          <button type="button" className="secondary-button" onClick={exportProfile}><Download aria-hidden="true" /> Export profile</button>
          <button type="button" className="danger-text-button" disabled={pending} onClick={removeNode}>Remove node</button>
          {message ? <p className="inline-error" role="alert">{message}</p> : null}
        </div>
      ) : (
        <p>This node is generated from the project runtime and cannot be edited directly.</p>
      )}
    </aside>
  );
}

function buildGraph({
  project,
  canvas,
  profiles,
  bindings,
  agentGateways,
  availableAgents,
  endpoints,
  browserApiBaseUrl,
}: Omit<GameplayCanvasProps, "traces">): {
  nodes: Node<RuntimeNodeData>[];
  edges: Edge[];
  palette: AvailableAgent[];
} {
  const profileById = new Map(profiles.map((profile) => [profile.id, profile]));
  const bindingById = new Map(bindings.map((binding) => [binding.id, binding]));
  const gatewayById = new Map(agentGateways.map((gateway) => [gateway.gatewayId, gateway]));
  const canvasNodes = canvas?.nodes ?? [];
  const primaryEndpoint = endpoints[0];
  const resourceKeys = new Set(canvasNodes.map((node) => `${node.gatewayId ?? ""}/${node.resourceId ?? ""}`));
  const palette = availableAgents.filter((agent) => agent.status === "connected" && !resourceKeys.has(`${agent.gatewayId}/${agent.id}`));
  const nodes: Node<RuntimeNodeData>[] = [
    {
      id: `endpoint:${project.id}`,
      type: "runtime",
      position: { x: 20, y: 230 },
      data: {
        title: project.name,
        subtitle: "Project endpoint",
        kind: "endpoint",
        status: project.enabled ? "ready" : "off",
        meta: primaryEndpoint ? endpointInvokeUrl(primaryEndpoint, browserApiBaseUrl) : "No endpoint yet",
      },
      draggable: false,
    },
  ];

  for (const node of canvasNodes) {
    const profile = node.profileId ? profileById.get(node.profileId) : null;
    const binding = node.bindingId ? bindingById.get(node.bindingId) : null;
    const gateway = node.gatewayId ? gatewayById.get(node.gatewayId) : null;
    nodes.push({
      id: node.id,
      type: "runtime",
      position: readPosition(node.position, nodes.length),
      data: {
        title: profile?.name ?? String(node.config.agentName ?? node.resourceId ?? "Agent"),
        subtitle: profile?.description ?? "Agent profile",
        kind: "agent",
        status: gateway?.status === "connected" ? "ready" : "off",
        meta: binding ? `${gatewayDisplayLabel(binding.gatewayId)} / ${binding.agentId}` : node.resourceId ?? "unbound",
        exposed: node.exposed,
        canvasNode: node,
      },
    });
  }

  const usedGateways = new Set(canvasNodes.flatMap((node) => node.gatewayId ? [node.gatewayId] : []));
  for (const gatewayId of usedGateways) {
    const gateway = gatewayById.get(gatewayId);
    nodes.push({
      id: `gateway:${gatewayId}`,
      type: "runtime",
      position: { x: 880, y: 150 + nodes.filter((node) => node.id.startsWith("gateway:")).length * 210 },
      data: {
        title: gatewayDisplayLabel(gatewayId),
        subtitle: gateway ? `${gateway.agents.length} detected agents` : "Agent Gateway",
        kind: "gateway",
        status: gateway?.status === "connected" ? "ready" : "off",
        meta: gateway ? `Last seen ${formatTime(gateway.lastSeenAt)}` : "not connected",
      },
      draggable: false,
    });
  }

  const edges: Edge[] = [];
  for (const node of canvasNodes) {
    if (node.exposed) {
      edges.push({
        id: `endpoint:${node.id}`,
        source: `endpoint:${project.id}`,
        target: node.id,
        animated: true,
        className: "gameplay-edge exposed",
      });
    }
    if (node.gatewayId) {
      edges.push({
        id: `gateway:${node.id}`,
        source: node.id,
        target: `gateway:${node.gatewayId}`,
        className: "gameplay-edge runtime",
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
    });
  }

  return { nodes, edges, palette };
}

function readPosition(value: Record<string, unknown>, index: number): { x: number; y: number } {
  const x = typeof value.x === "number" ? value.x : 360 + (index % 3) * 280;
  const y = typeof value.y === "number" ? value.y : 160 + Math.floor(index / 3) * 190;
  return { x, y };
}

function nextPalettePosition(nodes: Node[]): { x: number; y: number } {
  return { x: 360 + (nodes.length % 3) * 280, y: 160 + Math.floor(nodes.length / 3) * 190 };
}

function endpointInvokeUrl(endpoint: AgentEndpoint, browserApiBaseUrl: string): string {
  const url = new URL(browserApiBaseUrl);
  url.pathname = `/v1/endpoints/${encodeURIComponent(endpoint.slug || endpoint.id)}/invoke`;
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

function formatTime(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.valueOf()) ? "-" : new Intl.DateTimeFormat("en", { dateStyle: "medium", timeStyle: "short" }).format(date);
}

function slugify(value: string): string {
  return value.trim().toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "") || "agent";
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : "Request failed.";
}

function shortId(value: string): string {
  return value.length > 12 ? `${value.slice(0, 8)}...` : value;
}

function gatewayDisplayLabel(value: string): string {
  return value.startsWith("gateway-") ? `Gateway ${shortId(value.replace(/^gateway-/, ""))}` : shortId(value);
}
