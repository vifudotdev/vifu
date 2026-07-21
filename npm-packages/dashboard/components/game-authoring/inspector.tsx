"use client";

import { useEffect, useMemo, useState } from "react";
import { Braces, FormInput, Trash2, Volume2, VolumeX, X } from "lucide-react";
import { definitionForNode, objectValue } from "../../lib/game-authoring";
import type { AgentProfile, GameAgentReference, GameNodeDefinition, GameSourceNode } from "../../lib/runtime-types";
import { useGameAuthoring } from "./store";

type InspectorMode = "form" | "json";

export function GameNodeInspector({
  definitions,
  profiles,
}: {
  definitions: GameNodeDefinition[];
  profiles: AgentProfile[];
}) {
  const source = useGameAuthoring((state) => state.source);
  const selectedNodeId = useGameAuthoring((state) => state.selectedNodeId);
  const setSelectedNode = useGameAuthoring((state) => state.setSelectedNode);
  const updateNode = useGameAuthoring((state) => state.updateNode);
  const deleteNode = useGameAuthoring((state) => state.deleteNode);
  const issues = useGameAuthoring((state) => state.validationIssues);
  const selected = source.graph.nodes.find((node) => node.id === selectedNodeId) ?? null;
  const definition = selected ? definitionForNode(definitions, selected) : undefined;
  const [mode, setMode] = useState<InspectorMode>("form");
  const [draft, setDraft] = useState<GameSourceNode | null>(selected);
  const [jsonText, setJsonText] = useState("");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setDraft(selected);
    setJsonText(selected ? JSON.stringify(selected, null, 2) : "");
    setError(null);
    setMode("form");
  }, [selected]);

  const nodeIssues = useMemo(
    () => selected ? issues.filter((issue) => issue.nodeId === selected.id || issue.path?.includes(selected.id)) : [],
    [issues, selected],
  );

  if (!selected || !draft) return null;

  function save() {
    try {
      const next = mode === "json" ? JSON.parse(jsonText) as GameSourceNode : draft;
      if (!next || next.id !== selected?.id || typeof next.type !== "string" || typeof next.version !== "number") {
        throw new Error("Node JSON must preserve its id, type, and version.");
      }
      if (!next.config || typeof next.config !== "object" || Array.isArray(next.config)) {
        throw new Error("Node config must be a JSON object.");
      }
      updateNode(next);
      setDraft(next);
      setJsonText(JSON.stringify(next, null, 2));
      setError(null);
    } catch (nextError) {
      setError(nextError instanceof Error ? nextError.message : "Node JSON is invalid.");
    }
  }

  return (
    <aside className="game-node-inspector">
      <header>
        <div>
          <span>{definition?.category ?? "Node"}</span>
          <strong>{draft.label || definition?.title || draft.type}</strong>
          <code>{draft.id}</code>
        </div>
        <button type="button" className="icon-button" onClick={() => setSelectedNode(null)} aria-label="Close inspector"><X aria-hidden="true" /></button>
      </header>
      <div className="inspector-mode-switch" role="tablist" aria-label="Node editor">
        <button type="button" role="tab" aria-selected={mode === "form"} onClick={() => setMode("form")}><FormInput aria-hidden="true" />Fields</button>
        <button type="button" role="tab" aria-selected={mode === "json"} onClick={() => {
          setJsonText(JSON.stringify(draft, null, 2));
          setMode("json");
        }}><Braces aria-hidden="true" />JSON</button>
      </div>
      <div className="game-inspector-body">
        {mode === "form" ? (
          <NodeFields
            node={draft}
            definition={definition}
            profiles={profiles}
            agentReferences={source.agents}
            onChange={setDraft}
          />
        ) : (
          <label className="editor-field json-field">
            <span>Node document</span>
            <textarea value={jsonText} onChange={(event) => setJsonText(event.target.value)} spellCheck={false} />
          </label>
        )}
        {nodeIssues.length > 0 ? (
          <div className="node-validation-list">
            {nodeIssues.map((issue, index) => <p className={issue.severity} key={`${issue.code}-${index}`}><strong>{issue.code}</strong>{issue.message}</p>)}
          </div>
        ) : null}
        {error ? <p className="inline-error" role="alert">{error}</p> : null}
      </div>
      <footer>
        <button
          type="button"
          className="editor-danger-action"
          disabled={selected.id === source.entryNodeId}
          onClick={() => deleteNode(selected.id)}
          title={selected.id === source.entryNodeId ? "The entry node cannot be removed" : "Delete node"}
        >
          <Trash2 aria-hidden="true" />Delete
        </button>
        <button type="button" className="primary-button" onClick={save}>Save changes</button>
      </footer>
    </aside>
  );
}

function NodeFields({
  node,
  definition,
  profiles,
  agentReferences,
  onChange,
}: {
  node: GameSourceNode;
  definition?: GameNodeDefinition;
  profiles: AgentProfile[];
  agentReferences: GameAgentReference[];
  onChange: (node: GameSourceNode) => void;
}) {
  const properties = objectValue(definition?.configSchema.properties);
  const configFields = Object.entries(properties);
  const agentReference = node.type === "agent" && typeof node.config.agentId === "string"
    ? node.config.agentId
    : null;
  return (
    <div className="node-field-stack">
      <label className="editor-field"><span>Name</span><input value={node.label ?? ""} onChange={(event) => onChange({ ...node, label: event.target.value })} /></label>
      <label className="editor-field"><span>Notes</span><textarea value={node.notes ?? ""} onChange={(event) => onChange({ ...node, notes: event.target.value || null })} rows={3} /></label>
      {agentReference ? (
        <div className="editor-readonly-field"><span>Agent reference</span><strong>{agentReference}</strong><small>{profileNameForReference(agentReference, profiles)}</small></div>
      ) : null}
      {configFields.map(([key, schema]) => key === "agentId" && node.type === "tool" ? (
        <label className="editor-field" key={key}>
          <span>Agent</span>
          <select
            value={typeof node.config.agentId === "string" ? node.config.agentId : ""}
            onChange={(event) => onChange({
              ...node,
              config: { ...node.config, agentId: event.target.value },
            })}
          >
            <option value="">Select an Agent</option>
            {agentReferences.map((agent) => (
              <option value={agent.id} key={agent.id}>
                {profileNameForReference(agent.id, profiles)}
              </option>
            ))}
          </select>
        </label>
      ) : (
        <ConfigField
          key={key}
          fieldKey={key}
          schema={objectValue(schema)}
          value={node.config[key]}
          onChange={(value) => onChange({ ...node, config: { ...node.config, [key]: value } })}
        />
      ))}
      {definition?.timelineCompatible ? (
        <div className="timeline-config-grid">
          <ConfigField fieldKey="sequenceId" schema={{ type: "string", title: "Sequence" }} value={node.config.sequenceId ?? "main"} onChange={(value) => onChange({ ...node, config: { ...node.config, sequenceId: value } })} />
          <ConfigField fieldKey="startMs" schema={{ type: "integer", title: "Start (ms)" }} value={node.config.startMs ?? 0} onChange={(value) => onChange({ ...node, config: { ...node.config, startMs: value } })} />
          <ConfigField fieldKey="durationMs" schema={{ type: "integer", title: "Duration (ms)" }} value={node.config.durationMs ?? 1500} onChange={(value) => onChange({ ...node, config: { ...node.config, durationMs: value } })} />
        </div>
      ) : null}
      {isMediaNode(node.type) ? <TimelineMediaFields node={node} onChange={onChange} /> : null}
      {configFields.length === 0 && !definition?.timelineCompatible ? (
        <div className="editor-empty-note">This node has no configurable fields.</div>
      ) : null}
    </div>
  );
}

function TimelineMediaFields({ node, onChange }: { node: GameSourceNode; onChange: (node: GameSourceNode) => void }) {
  const volume = typeof node.config.volume === "number" ? node.config.volume : 1;
  const muted = Boolean(node.config.muted);
  const updateConfig = (values: Record<string, unknown>) => onChange({
    ...node,
    config: { ...node.config, ...values },
  });
  return (
    <section className="timeline-media-fields">
      <header><span>Clip</span><strong>Non-destructive media settings</strong></header>
      <div className="timeline-config-grid">
        <ConfigField fieldKey="inMs" schema={{ type: "integer", title: "Trim in (ms)" }} value={node.config.inMs ?? 0} onChange={(value) => updateConfig({ inMs: value })} />
        <ConfigField fieldKey="outMs" schema={{ type: "integer", title: "Trim out (ms)" }} value={node.config.outMs ?? 0} onChange={(value) => updateConfig({ outMs: value })} />
        <ConfigField fieldKey="transitionMs" schema={{ type: "integer", title: "Transition (ms)" }} value={node.config.transitionMs ?? 0} onChange={(value) => updateConfig({ transitionMs: value })} />
      </div>
      {supportsVolume(node.type) ? (
        <div className="timeline-volume-control">
          <span>{muted ? <VolumeX aria-hidden="true" /> : <Volume2 aria-hidden="true" />}Volume</span>
          <input type="range" min="0" max="1" step="0.05" value={volume} disabled={muted} onChange={(event) => updateConfig({ volume: Number(event.target.value) })} />
          <output>{muted ? "Muted" : `${Math.round(volume * 100)}%`}</output>
          <label><input type="checkbox" checked={muted} onChange={(event) => updateConfig({ muted: event.target.checked })} /><span>Mute</span></label>
        </div>
      ) : null}
    </section>
  );
}

function isMediaNode(type: string): boolean {
  return ["video", "audio", "voice", "subtitle", "background", "character_visual", "asset"].includes(type);
}

function supportsVolume(type: string): boolean {
  return ["video", "audio", "voice"].includes(type);
}

function ConfigField({
  fieldKey,
  schema,
  value,
  onChange,
}: {
  fieldKey: string;
  schema: Record<string, unknown>;
  value: unknown;
  onChange: (value: unknown) => void;
}) {
  const label = typeof schema.title === "string" ? schema.title : humanize(fieldKey);
  const type = typeof schema.type === "string" ? schema.type : inferType(value);
  const options = Array.isArray(schema.enum) ? schema.enum : null;
  if (options) {
    return (
      <label className="editor-field"><span>{label}</span><select value={String(value ?? "")} onChange={(event) => onChange(event.target.value)}>
        {options.map((option) => <option key={String(option)} value={String(option)}>{humanize(String(option))}</option>)}
      </select></label>
    );
  }
  if (type === "boolean") {
    return <label className="editor-toggle"><input type="checkbox" checked={Boolean(value)} onChange={(event) => onChange(event.target.checked)} /><span>{label}</span></label>;
  }
  if (type === "integer" || type === "number") {
    return <label className="editor-field"><span>{label}</span><input type="number" value={typeof value === "number" ? value : 0} onChange={(event) => onChange(Number(event.target.value))} /></label>;
  }
  if (type === "array" || type === "object" || (value !== null && typeof value === "object")) {
    return (
      <label className="editor-field json-field"><span>{label}</span><textarea value={JSON.stringify(value ?? (type === "array" ? [] : {}), null, 2)} onChange={(event) => {
        try { onChange(JSON.parse(event.target.value)); } catch { /* Keep the last valid structured value. */ }
      }} spellCheck={false} rows={6} /></label>
    );
  }
  return <label className="editor-field"><span>{label}</span><input value={typeof value === "string" ? value : ""} onChange={(event) => onChange(event.target.value)} /></label>;
}

function profileNameForReference(agentReference: string, profiles: AgentProfile[]): string {
  const slug = agentReference.replace(/^agent\./, "");
  return profiles.find((profile) => profile.slug === slug)?.name ?? "Pinned when published";
}

function inferType(value: unknown): string {
  if (Array.isArray(value)) return "array";
  if (value === null) return "string";
  return typeof value;
}

function humanize(value: string): string {
  return value.replace(/_/g, " ").replace(/([a-z])([A-Z])/g, "$1 $2").replace(/^./, (letter) => letter.toUpperCase());
}
