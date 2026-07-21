"use client";

import { useMemo, useState } from "react";
import { useRouter } from "next/navigation";
import {
  DndContext,
  KeyboardSensor,
  PointerSensor,
  closestCenter,
  useSensor,
  useSensors,
  type DragEndEvent,
} from "@dnd-kit/core";
import {
  SortableContext,
  arrayMove,
  horizontalListSortingStrategy,
  sortableKeyboardCoordinates,
  useSortable,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import {
  Bot,
  Boxes,
  ChevronRight,
  Clapperboard,
  GitBranch,
  Image as ImageIcon,
  Library,
  ListTree,
  Play,
  Plus,
  Scissors,
  Search,
  Sparkles,
  Volume2,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import {
  SHORT_DRAMA_TRACKS,
  bindManagedPresentationAsset,
  definitionForNode,
  isShortDramaStoryDefinition,
  nextCanvasNodePosition,
  shortDramaTrack,
  timelineDuration,
  timelineStart,
  nodeInputPorts,
  nodeOutputPorts,
} from "../../lib/game-authoring";
import type {
  AgentProfile,
  GameAsset,
  GameDraft,
  GameNodeDefinition,
  GameResource,
  GameSourceNode,
  RuntimeProject,
} from "../../lib/runtime-types";
import { GameNodeInspector } from "./inspector";
import {
  GameAuthoringProvider,
  useGameAuthoring,
  useGameAuthoringStore,
  useGameDraftSync,
} from "./store";
import { GameAuthoringToolbar } from "./toolbar";

const PIXELS_PER_SECOND = 18;
export function RuntimeShortDrama({
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
      <ShortDramaWorkspace project={project} definitions={definitions} profiles={profiles} resources={resources} assets={assets} />
    </GameAuthoringProvider>
  );
}

function ShortDramaWorkspace({
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
  const selectedNodeId = useGameAuthoring((state) => state.selectedNodeId);
  return (
    <section className={`game-authoring-workspace short-drama-workspace ${selectedNodeId ? "inspector-open" : ""}`}>
      <GameAuthoringToolbar projectSlug={project.slug} viewLabel="Short Drama" />
      <div className="short-drama-layout">
        <DramaLibrary definitions={definitions} profiles={profiles} resources={resources} assets={assets} />
        <DramaStage projectSlug={project.slug} />
        <GameNodeInspector definitions={definitions} profiles={profiles} />
        <DramaTimeline definitions={definitions} profiles={profiles} />
      </div>
    </section>
  );
}

function DramaLibrary({ definitions, profiles, resources, assets }: { definitions: GameNodeDefinition[]; profiles: AgentProfile[]; resources: GameResource[]; assets: GameAsset[] }) {
  const source = useGameAuthoring((state) => state.source);
  const store = useGameAuthoringStore();
  const [query, setQuery] = useState("");
  const [tab, setTab] = useState<"story" | "agents" | "resources">("story");
  const normalized = query.trim().toLowerCase();
  const timelineDefinitions = definitions.filter((definition) => (
    isShortDramaStoryDefinition(definition, normalized)
  ));
  const agentDefinition = definitions.find((definition) => definition.type === "agent");

  function add(definition: GameNodeDefinition, profile?: AgentProfile, config?: Record<string, unknown>, label?: string) {
    const before = new Set(store.getState().source.graph.nodes.map((node) => node.id));
    store.getState().addNode(
      definition,
      nextCanvasNodePosition(store.getState().source.graph.nodes.length),
      profile,
    );
    const added = store.getState().source.graph.nodes.find((node) => !before.has(node.id));
    if (!added) return;
    if (config || label) {
      store.getState().updateNode({ ...added, label: label ?? added.label, config: { ...added.config, ...config } });
    }
    const current = store.getState().source.graph.nodes.find((node) => node.id === added.id) ?? added;
    const trackId = shortDramaTrack(store.getState().source, current);
    const trackNodes = store.getState().source.graph.nodes.filter((node) => shortDramaTrack(store.getState().source, node) === trackId);
    const endMs = trackNodes.reduce((maximum, node) => node.id === current.id ? maximum : Math.max(maximum, timelineStart(node) + timelineDuration(node)), 0);
    store.getState().placeTimelineNode(current.id, trackId, endMs);
    store.getState().setSelectedNode(current.id);
  }

  function addAsset(asset: GameAsset) {
    const version = asset.versions.find((item) => item.approvalStatus === "approved");
    if (!version) return;
    const preferredType = asset.kind === "video"
      ? "video"
      : asset.kind === "audio"
        ? "audio"
        : asset.kind === "subtitle"
          ? "subtitle"
          : asset.kind === "image"
            ? "background"
            : "asset";
    const definition = definitions.find((item) => item.type === preferredType)
      ?? definitions.find((item) => item.type === "asset");
    if (!definition) return;
    add(definition, undefined, {
      logicalResourceId: asset.assetKey,
      kind: asset.kind,
      inMs: 0,
      volume: 1,
      muted: false,
    }, asset.name);
    store.getState().setSource(bindManagedPresentationAsset(store.getState().source, asset, version));
  }

  return (
    <aside className="drama-library">
      <header><Library aria-hidden="true" /><strong>Library</strong></header>
      <div className="drama-library-tabs" role="tablist">
        <button type="button" role="tab" aria-selected={tab === "story"} onClick={() => setTab("story")}>Story</button>
        <button type="button" role="tab" aria-selected={tab === "agents"} onClick={() => setTab("agents")}>Agents</button>
        <button type="button" role="tab" aria-selected={tab === "resources"} onClick={() => setTab("resources")}>Media</button>
      </div>
      <label className="drama-library-search"><Search aria-hidden="true" /><input type="search" value={query} onChange={(event) => setQuery(event.target.value)} placeholder={`Search ${tab}`} /></label>
      <div className="drama-library-list">
        {tab === "story" ? timelineDefinitions.map((definition) => (
          <DramaLibraryItem key={definition.type} icon={libraryIcon(definition.type)} title={definition.title} detail={definition.category} onAdd={() => add(definition)} />
        )) : null}
        {tab === "agents" && agentDefinition ? profiles.filter((profile) => !profile.archivedAt && (!normalized || `${profile.name} ${profile.slug}`.toLowerCase().includes(normalized))).map((profile) => (
          <DramaLibraryItem key={profile.id} icon={Bot} title={profile.name} detail={profile.slug} onAdd={() => add(agentDefinition, profile)} />
        )) : null}
        {tab === "resources" ? (
          <>
            {assets.filter((asset) => asset.versions.some((version) => version.approvalStatus === "approved") && (!normalized || `${asset.name} ${asset.assetKey} ${asset.kind}`.toLowerCase().includes(normalized))).map((asset) => <DramaLibraryItem key={asset.id} icon={libraryIcon(asset.kind)} title={asset.name} detail={asset.kind} onAdd={() => addAsset(asset)} />)}
            {source.presentationResources.map((resource) => <DramaLibraryItem key={resource.id} icon={ImageIcon} title={resource.id} detail="Logical slot" />)}
            {resources.filter((resource) => source.resources.some((reference) => reference.id === resource.resourceKey)).map((resource) => <DramaLibraryItem key={resource.id} icon={Boxes} title={resource.name} detail="Game data" />)}
            {assets.length + source.presentationResources.length + source.resources.length === 0 ? <div className="drama-library-empty"><ImageIcon aria-hidden="true" /><strong>No media yet</strong><span>Import media from the Resources page.</span></div> : null}
          </>
        ) : null}
      </div>
    </aside>
  );
}

function DramaLibraryItem({ icon: Icon, title, detail, onAdd }: { icon: LucideIcon; title: string; detail: string; onAdd?: () => void }) {
  return (
    <button type="button" className="drama-library-item" onClick={onAdd} disabled={!onAdd}>
      <span><Icon aria-hidden="true" /></span><div><strong>{title}</strong><small>{detail}</small></div>{onAdd ? <Plus aria-hidden="true" /> : null}
    </button>
  );
}

function DramaStage({ projectSlug }: { projectSlug: string }) {
  const router = useRouter();
  const source = useGameAuthoring((state) => state.source);
  const selectedNodeId = useGameAuthoring((state) => state.selectedNodeId);
  const selected = source.graph.nodes.find((node) => node.id === selectedNodeId)
    ?? source.graph.nodes.find((node) => node.type === "scene")
    ?? source.graph.nodes[0];
  const choiceCount = source.graph.nodes.filter((node) => node.type === "choice").length;
  const endingCount = source.graph.nodes.filter((node) => node.type === "ending").length;
  return (
    <main className="drama-stage-area">
      <div className="drama-stage-meta"><span>Sequence <strong>main</strong></span><span>{choiceCount} branches</span><span>{endingCount} endings</span></div>
      <div className="drama-player-frame">
        <div className="drama-player-scene">
          <span className="drama-scene-kicker">{selected?.type ?? "scene"}</span>
          <strong>{selected?.label || source.metadata.name}</strong>
          <p>{selected ? stageDescription(selected) : "Add a scene or Agent beat to begin."}</p>
          {selected?.type === "choice" ? <ChoicePreview node={selected} /> : null}
        </div>
        <div className="drama-player-controls">
          <button type="button" className="primary" aria-label="Open live preview" title="Open live preview" onClick={() => router.push(`/project/${projectSlug}/preview`)}><Play aria-hidden="true" /></button>
          <span>Draft sequence</span><div><i /></div><span>{formatTime(dramaDuration(source.graph.nodes))}</span>
        </div>
      </div>
      <BranchNavigator nodes={source.graph.nodes} />
    </main>
  );
}

function DramaTimeline({ definitions, profiles }: { definitions: GameNodeDefinition[]; profiles: AgentProfile[] }) {
  const source = useGameAuthoring((state) => state.source);
  const reorderTimeline = useGameAuthoring((state) => state.reorderTimeline);
  const placeTimelineNode = useGameAuthoring((state) => state.placeTimelineNode);
  const setSelectedNode = useGameAuthoring((state) => state.setSelectedNode);
  const selectedNodeId = useGameAuthoring((state) => state.selectedNodeId);
  const splitTimelineNode = useGameAuthoring((state) => state.splitTimelineNode);
  const timelineNodes = source.graph.nodes.filter((node) => definitionForNode(definitions, node)?.timelineCompatible);
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 5 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
  );
  const maxDuration = Math.max(30_000, dramaDuration(timelineNodes));
  const timelineWidth = Math.max(760, (maxDuration / 1000) * PIXELS_PER_SECOND + 80);
  const selected = timelineNodes.find((node) => node.id === selectedNodeId);
  const selectedDefinition = selected ? definitionForNode(definitions, selected) : undefined;

  function splitSelected() {
    if (!selected) return;
    const outputPort = nodeOutputPorts(selectedDefinition, selected)[0];
    const inputPort = nodeInputPorts(selectedDefinition)[0];
    if (!outputPort || !inputPort) return;
    splitTimelineNode(selected.id, outputPort, inputPort);
  }

  function onDragEnd(event: DragEndEvent) {
    const activeId = String(event.active.id);
    const overId = event.over ? String(event.over.id) : null;
    if (!overId || activeId === overId) return;
    const active = timelineNodes.find((node) => node.id === activeId);
    const over = timelineNodes.find((node) => node.id === overId);
    if (!active || !over) return;
    const activeTrack = shortDramaTrack(source, active);
    const targetTrack = shortDramaTrack(source, over);
    const targetNodes = timelineNodes
      .filter((node) => shortDramaTrack(source, node) === targetTrack)
      .sort((left, right) => timelineStart(left) - timelineStart(right));
    const oldIndex = targetNodes.findIndex((node) => node.id === activeId);
    const overIndex = targetNodes.findIndex((node) => node.id === overId);
    if (activeTrack === targetTrack && oldIndex >= 0 && overIndex >= 0) {
      reorderTimeline(targetTrack, arrayMove(targetNodes.map((node) => node.id), oldIndex, overIndex));
      return;
    }
    const insertionIndex = Math.max(0, overIndex);
    const nextTargetIds = targetNodes.map((node) => node.id);
    nextTargetIds.splice(insertionIndex, 0, activeId);
    placeTimelineNode(activeId, targetTrack, timelineStart(over));
    reorderTimeline(targetTrack, nextTargetIds);
    const oldTrackIds = timelineNodes
      .filter((node) => node.id !== activeId && shortDramaTrack(source, node) === activeTrack)
      .sort((left, right) => timelineStart(left) - timelineStart(right))
      .map((node) => node.id);
    reorderTimeline(activeTrack, oldTrackIds);
  }

  return (
    <section className="drama-timeline">
      <header><div><Scissors aria-hidden="true" /><strong>Timeline</strong><span>{timelineNodes.length} items</span></div><div><button type="button" title="Split selected" aria-label="Split selected" disabled={!selected || timelineDuration(selected) < 500} onClick={splitSelected}><Scissors aria-hidden="true" /></button><button type="button" title="Timeline settings" aria-label="Timeline settings"><Sparkles aria-hidden="true" /></button></div></header>
      <div className="timeline-scroll">
        <div className="timeline-ruler-row"><span className="timeline-track-label">Tracks</span><div className="timeline-ruler" style={{ width: timelineWidth }}>{timelineTicks(maxDuration).map((tick) => <span key={tick} style={{ left: (tick / 1000) * PIXELS_PER_SECOND }}>{formatTime(tick)}</span>)}</div></div>
        <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={onDragEnd}>
          {SHORT_DRAMA_TRACKS.map((track) => {
            const nodes = timelineNodes.filter((node) => shortDramaTrack(source, node) === track.id).sort((left, right) => timelineStart(left) - timelineStart(right));
            return (
              <div className="timeline-track" key={track.id}>
                <div className="timeline-track-label"><TrackIcon kind={track.kind} /><span>{track.label}</span></div>
                <SortableContext items={nodes.map((node) => node.id)} strategy={horizontalListSortingStrategy}>
                  <div className="timeline-track-lane" style={{ width: timelineWidth }}>
                    {nodes.map((node) => <SortableTimelineClip key={node.id} node={node} selected={selectedNodeId === node.id} profile={profileForNode(node, source.agents, profiles)} onSelect={() => setSelectedNode(node.id)} />)}
                  </div>
                </SortableContext>
              </div>
            );
          })}
        </DndContext>
      </div>
    </section>
  );
}

function SortableTimelineClip({ node, selected, profile, onSelect }: { node: GameSourceNode; selected: boolean; profile?: AgentProfile; onSelect: () => void }) {
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({ id: node.id });
  const width = Math.max(74, (timelineDuration(node) / 1000) * PIXELS_PER_SECOND);
  const left = (timelineStart(node) / 1000) * PIXELS_PER_SECOND;
  return (
    <button
      type="button"
      ref={setNodeRef}
      className={`timeline-clip clip-${clipKind(node.type)} ${selected ? "selected" : ""} ${isDragging ? "dragging" : ""}`}
      style={{ width, left, transform: CSS.Transform.toString(transform), transition }}
      onClick={onSelect}
      {...attributes}
      {...listeners}
    >
      <span>{profile ? <Bot aria-hidden="true" /> : <ClipIcon type={node.type} />}</span>
      <div><strong>{node.label || profile?.name || node.type}</strong><small>{formatTime(timelineDuration(node))}</small></div>
    </button>
  );
}

function BranchNavigator({ nodes }: { nodes: GameSourceNode[] }) {
  const choices = nodes.filter((node) => node.type === "choice");
  const setSelectedNode = useGameAuthoring((state) => state.setSelectedNode);
  return (
    <section className="drama-branch-navigator">
      <header><ListTree aria-hidden="true" /><strong>Branches</strong></header>
      {choices.length > 0 ? choices.map((choice) => (
        <button type="button" key={choice.id} onClick={() => setSelectedNode(choice.id)}><GitBranch aria-hidden="true" /><span><strong>{choice.label || "Choice"}</strong><small>{Array.isArray(choice.config.options) ? `${choice.config.options.length} paths` : "Configure paths"}</small></span><ChevronRight aria-hidden="true" /></button>
      )) : <p>No branches in this sequence.</p>}
    </section>
  );
}

function ChoicePreview({ node }: { node: GameSourceNode }) {
  const options = Array.isArray(node.config.options) ? node.config.options : [];
  return <div className="drama-choice-preview">{options.map((option, index) => {
    const value = option && typeof option === "object" ? option as Record<string, unknown> : {};
    return <span key={String(value.id ?? index)}>{String(value.label ?? value.id ?? `Option ${index + 1}`)}</span>;
  })}</div>;
}

function TrackIcon({ kind }: { kind: string }) {
  const Icon = kind === "agent" ? Bot : kind === "interaction" ? GitBranch : kind === "media" ? ImageIcon : kind === "scene" ? Clapperboard : Sparkles;
  return <Icon aria-hidden="true" />;
}

function ClipIcon({ type }: { type: string }) {
  const Icon = libraryIcon(type);
  return <Icon aria-hidden="true" />;
}

function libraryIcon(type: string): LucideIcon {
  if (type === "agent") return Bot;
  if (["choice", "condition", "input", "event"].includes(type)) return GitBranch;
  if (["image", "video", "background", "character_visual", "asset"].includes(type)) return ImageIcon;
  if (["audio", "voice", "subtitle"].includes(type)) return Volume2;
  if (["scene", "episode", "dialogue", "ending"].includes(type)) return Clapperboard;
  return Sparkles;
}

function clipKind(type: string): string {
  if (type === "agent") return "agent";
  if (["choice", "condition", "input", "event"].includes(type)) return "interaction";
  if (["video", "background", "character_visual", "asset"].includes(type)) return "visual";
  if (["audio", "voice", "subtitle"].includes(type)) return "audio";
  return "story";
}

function profileForNode(
  node: GameSourceNode,
  agentReferences: Array<{ id: string; profileId: string }>,
  profiles: AgentProfile[],
): AgentProfile | undefined {
  const agentId = typeof node.config.agentId === "string" ? node.config.agentId : "";
  const reference = agentReferences.find((agent) => agent.id === agentId);
  return reference ? profiles.find((profile) => profile.id === reference.profileId) : undefined;
}

function stageDescription(node: GameSourceNode): string {
  for (const key of ["prompt", "text", "description", "title", "action"]) {
    if (typeof node.config[key] === "string" && node.config[key]) return String(node.config[key]);
  }
  return `Configure this ${node.type.replace(/_/g, " ")} in the inspector.`;
}

function dramaDuration(nodes: GameSourceNode[]): number {
  return nodes.reduce((maximum, node) => Math.max(maximum, timelineStart(node) + timelineDuration(node)), 0);
}

function timelineTicks(durationMs: number): number[] {
  const ticks: number[] = [];
  for (let tick = 0; tick <= durationMs; tick += 5000) ticks.push(tick);
  return ticks;
}

function formatTime(valueMs: number): string {
  const seconds = Math.max(0, valueMs) / 1000;
  const minutes = Math.floor(seconds / 60);
  const remainder = seconds - minutes * 60;
  return `${String(minutes).padStart(2, "0")}:${remainder.toFixed(remainder % 1 ? 1 : 0).padStart(2, "0")}`;
}
