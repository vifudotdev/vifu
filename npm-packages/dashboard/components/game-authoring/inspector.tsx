"use client";

import { useEffect, useMemo, useRef, useState, type ChangeEvent } from "react";
import { Braces, FormInput, Plus, Trash2, Volume2, VolumeX, X } from "lucide-react";
import {
  definitionForNode,
  localizedMessage,
  messageReference,
  messageReferenceId,
  objectValue,
  setLocalizedMessage,
} from "../../lib/game-authoring";
import type { AgentProfile, GameAgentReference, GameNodeDefinition, GameSource, GameSourceNode } from "../../lib/runtime-types";
import { useGameAuthoring } from "./store";

type InspectorMode = "form" | "json";

export function GameNodeInspector({
  definitions,
  profiles,
  locale,
}: {
  definitions: GameNodeDefinition[];
  profiles: AgentProfile[];
  locale?: string;
}) {
  const source = useGameAuthoring((state) => state.source);
  const selectedNodeId = useGameAuthoring((state) => state.selectedNodeId);
  const setSelectedNode = useGameAuthoring((state) => state.setSelectedNode);
  const updateNode = useGameAuthoring((state) => state.updateNode);
  const setSource = useGameAuthoring((state) => state.setSource);
  const deleteNode = useGameAuthoring((state) => state.deleteNode);
  const issues = useGameAuthoring((state) => state.validationIssues);
  const selected = source.graph.nodes.find((node) => node.id === selectedNodeId) ?? null;
  const definition = selected ? definitionForNode(definitions, selected) : undefined;
  const [mode, setMode] = useState<InspectorMode>("form");
  const [draft, setDraft] = useState<GameSourceNode | null>(selected);
  const [jsonText, setJsonText] = useState("");
  const [error, setError] = useState<string | null>(null);
  const sourceRef = useRef(source);
  sourceRef.current = source;

  useEffect(() => {
    const nextSelected = sourceRef.current.graph.nodes.find((node) => node.id === selectedNodeId) ?? null;
    setDraft(nextSelected);
    setJsonText(nextSelected ? JSON.stringify(nextSelected, null, 2) : "");
    setError(null);
    setMode("form");
  }, [selectedNodeId]);

  const nodeIssues = useMemo(
    () => selected ? issues.filter((issue) => issue.nodeId === selected.id || issue.path?.includes(selected.id)) : [],
    [issues, selected],
  );
  const editorLocale = locale && [source.localization.sourceLocale, ...source.localization.targetLocales].includes(locale)
    ? locale
    : source.localization.defaultLocale;

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
            source={source}
            locale={editorLocale}
            onSourceChange={setSource}
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
  source,
  locale,
  onSourceChange,
  onChange,
}: {
  node: GameSourceNode;
  definition?: GameNodeDefinition;
  profiles: AgentProfile[];
  agentReferences: GameAgentReference[];
  source: GameSource;
  locale: string;
  onSourceChange: (source: GameSource) => void;
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
      {node.type === "choice" ? (
        <ChoiceFields node={node} source={source} locale={locale} onSourceChange={onSourceChange} onChange={onChange} />
      ) : node.type === "agent" ? (
        <AgentBeatFields node={node} source={source} locale={locale} onSourceChange={onSourceChange} onChange={onChange} />
      ) : node.type === "dialogue" ? (
        <DialogueFields node={node} source={source} locale={locale} onSourceChange={onSourceChange} onChange={onChange} />
      ) : configFields.filter(([key]) => !hiddenConfigField(node.type, key)).map(([key, schema]) => key === "characterId" && node.type === "character_visual" ? (
        <label className="editor-field" key={key}>
          <span>Character</span>
          <select
            value={typeof node.config.characterId === "string" ? node.config.characterId : ""}
            onChange={(event) => onChange({
              ...node,
              config: { ...node.config, characterId: event.target.value },
            })}
          >
            <option value="">Select a character</option>
            {source.characters.map((character) => (
              <option value={character.id} key={character.id}>
                {localizedMessage(source, character.nameMessageId, locale) || character.id}
              </option>
            ))}
          </select>
        </label>
      ) : key === "agentId" && node.type === "tool" ? (
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
        <LocalizedOrConfigField
          key={key}
          fieldKey={key}
          schema={objectValue(schema)}
          value={node.config[key]}
          source={source}
          locale={locale}
          onSourceChange={onSourceChange}
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

function LocalizedOrConfigField({
  fieldKey,
  schema,
  value,
  source,
  locale,
  onSourceChange,
  onChange,
}: {
  fieldKey: string;
  schema: Record<string, unknown>;
  value: unknown;
  source: GameSource;
  locale: string;
  onSourceChange: (source: GameSource) => void;
  onChange: (value: unknown) => void;
}) {
  const messageId = messageReferenceId(value);
  if (messageId) {
    return (
      <LocalizedTextField
        label={typeof schema.title === "string" ? schema.title : humanize(fieldKey)}
        messageId={messageId}
        source={source}
        locale={locale}
        onSourceChange={onSourceChange}
        multiline={isLongTextField(fieldKey)}
      />
    );
  }
  return <ConfigField fieldKey={fieldKey} schema={schema} value={value} onChange={onChange} />;
}

function LocalizedTextField({
  label,
  messageId,
  source,
  locale,
  onSourceChange,
  multiline = true,
  onReference,
  fallbackValue = "",
}: {
  label: string;
  messageId: string;
  source: GameSource;
  locale: string;
  onSourceChange: (source: GameSource) => void;
  multiline?: boolean;
  onReference?: (messageId: string) => void;
  fallbackValue?: string;
}) {
  const value = localizedMessage(source, messageId, locale) || fallbackValue;
  const sourceLocale = locale === source.localization.sourceLocale;
  const common = {
    value,
    onChange: (event: ChangeEvent<HTMLInputElement | HTMLTextAreaElement>) => {
      onReference?.(messageId);
      onSourceChange(setLocalizedMessage(source, messageId, locale, event.target.value));
    },
  };
  return (
    <label className="editor-field localized-editor-field">
      <span>{label}<small>{sourceLocale ? "Source" : locale}</small></span>
      {multiline ? <textarea {...common} rows={4} /> : <input {...common} />}
    </label>
  );
}

function DialogueFields({
  node,
  source,
  locale,
  onSourceChange,
  onChange,
}: {
  node: GameSourceNode;
  source: GameSource;
  locale: string;
  onSourceChange: (source: GameSource) => void;
  onChange: (node: GameSourceNode) => void;
}) {
  const messageId = ensureMessageId(node, "text");
  return (
    <section className="inspector-section">
      <header><strong>Dialogue</strong><span>What the player sees</span></header>
      <label className="editor-field">
        <span>Speaker</span>
        <select
          value={typeof node.config.speakerId === "string" ? node.config.speakerId : ""}
          onChange={(event) => onChange({ ...node, config: { ...node.config, speakerId: event.target.value } })}
        >
          <option value="">Narrator</option>
          {source.characters.map((character) => (
            <option value={character.id} key={character.id}>{localizedMessage(source, character.nameMessageId, locale) || character.id}</option>
          ))}
        </select>
      </label>
      <LocalizedTextField label="Line" messageId={messageId} source={source} locale={locale} onSourceChange={onSourceChange} fallbackValue={typeof node.config.text === "string" ? node.config.text : ""} onReference={(nextMessageId) => onChange({ ...node, config: { ...node.config, text: messageReference(nextMessageId) } })} />
      <label className="editor-toggle"><input type="checkbox" checked={node.config.blocking !== false} onChange={(event) => onChange({ ...node, config: { ...node.config, blocking: event.target.checked } })} /><span>Wait for the player to continue</span></label>
    </section>
  );
}

function AgentBeatFields({
  node,
  source,
  locale,
  onSourceChange,
  onChange,
}: {
  node: GameSourceNode;
  source: GameSource;
  locale: string;
  onSourceChange: (source: GameSource) => void;
  onChange: (node: GameSourceNode) => void;
}) {
  const promptId = ensureMessageId(node, "prompt");
  const fallback = objectValue(node.config.fallback);
  const fallbackId = messageReferenceId(fallback.dialogue) ?? `node.${node.id}.fallback`;
  const allowed = Array.isArray(node.config.allowedStateChanges)
    ? node.config.allowedStateChanges.filter((value): value is string => typeof value === "string")
    : [];
  const setAllowed = (next: string[]) => onChange({ ...node, config: { ...node.config, allowedStateChanges: next } });
  return (
    <>
      <section className="inspector-section">
        <header><strong>Agent beat</strong><span>Keep story facts in the prompt</span></header>
        <LocalizedTextField label="Direction" messageId={promptId} source={source} locale={locale} onSourceChange={onSourceChange} fallbackValue={typeof node.config.prompt === "string" ? node.config.prompt : ""} onReference={(nextMessageId) => onChange({ ...node, config: { ...node.config, prompt: messageReference(nextMessageId) } })} />
        <label className="editor-toggle"><input type="checkbox" checked={node.config.blocking !== false} onChange={(event) => onChange({ ...node, config: { ...node.config, blocking: event.target.checked } })} /><span>Wait for the player&apos;s response</span></label>
      </section>
      <section className="inspector-section">
        <header><strong>State access</strong><span>The Agent can only change these values</span></header>
        <EditableStringList values={allowed} placeholder="mizuki_trust" addLabel="Allow state value" onChange={setAllowed} />
      </section>
      <section className="inspector-section">
        <header><strong>Fallback</strong><span>Shown when the provider cannot answer</span></header>
        <LocalizedTextField label="Fallback line" messageId={fallbackId} source={source} locale={locale} onSourceChange={onSourceChange} fallbackValue={typeof fallback.dialogue === "string" ? fallback.dialogue : ""} onReference={(nextMessageId) => onChange({ ...node, config: { ...node.config, fallback: { ...fallback, dialogue: messageReference(nextMessageId), stateChanges: Array.isArray(fallback.stateChanges) ? fallback.stateChanges : [] } } })} />
      </section>
    </>
  );
}

function ChoiceFields({
  node,
  source,
  locale,
  onSourceChange,
  onChange,
}: {
  node: GameSourceNode;
  source: GameSource;
  locale: string;
  onSourceChange: (source: GameSource) => void;
  onChange: (node: GameSourceNode) => void;
}) {
  const promptId = ensureMessageId(node, "prompt");
  const options = Array.isArray(node.config.options)
    ? node.config.options.map((option) => objectValue(option))
    : [];
  const updateOptions = (next: Record<string, unknown>[]) => onChange({ ...node, config: { ...node.config, options: next } });
  const targets = source.graph.nodes.filter((candidate) => ![node.id, source.entryNodeId, "short-drama-end"].includes(candidate.id));

  function addOption() {
    const id = uniqueOptionId(options);
    const messageId = `node.${node.id}.${id}`;
    onSourceChange(setLocalizedMessage(source, messageId, source.localization.sourceLocale, `Option ${options.length + 1}`));
    updateOptions([...options, { id, label: messageReference(messageId), mutations: [] }]);
  }

  return (
    <>
      <section className="inspector-section">
        <header><strong>Choice</strong><span>Ask one clear question</span></header>
        <LocalizedTextField label="Prompt" messageId={promptId} source={source} locale={locale} onSourceChange={onSourceChange} fallbackValue={typeof node.config.prompt === "string" ? node.config.prompt : ""} onReference={(nextMessageId) => onChange({ ...node, config: { ...node.config, prompt: messageReference(nextMessageId) } })} />
      </section>
      <section className="inspector-section choice-option-section">
        <header><strong>Player options</strong><button type="button" className="editor-inline-action" onClick={addOption}><Plus aria-hidden="true" />Add option</button></header>
        {options.map((option, index) => (
          <ChoiceOptionEditor
            key={String(option.id ?? index)}
            choiceId={node.id}
            option={option}
            index={index}
            source={source}
            locale={locale}
            targets={targets}
            onSourceChange={onSourceChange}
            onChange={(next) => updateOptions(options.map((current, currentIndex) => currentIndex === index ? next : current))}
            onDelete={() => updateOptions(options.filter((_current, currentIndex) => currentIndex !== index))}
          />
        ))}
        {options.length === 0 ? <div className="editor-empty-note">Add at least one player option.</div> : null}
      </section>
    </>
  );
}

function ChoiceOptionEditor({
  choiceId,
  option,
  index,
  source,
  locale,
  targets,
  onSourceChange,
  onChange,
  onDelete,
}: {
  choiceId: string;
  option: Record<string, unknown>;
  index: number;
  source: GameSource;
  locale: string;
  targets: GameSourceNode[];
  onSourceChange: (source: GameSource) => void;
  onChange: (option: Record<string, unknown>) => void;
  onDelete: () => void;
}) {
  const id = typeof option.id === "string" && option.id ? option.id : `option-${index + 1}`;
  const labelId = messageReferenceId(option.label) ?? `node.${choiceId}.${id}`;
  const condition = conditionGroup(option.condition);
  const mutations = Array.isArray(option.mutations) ? option.mutations.map((mutation) => objectValue(mutation)) : [];
  const [rulesOpen, setRulesOpen] = useState(Boolean(option.condition) || mutations.length > 0);
  const updateMutations = (next: Record<string, unknown>[]) => onChange({ ...option, mutations: next });
  return (
    <article className="choice-option-card">
      <header><span>{index + 1}</span><strong>{localizedMessage(source, labelId, locale) || id}</strong><button type="button" className="icon-button" onClick={onDelete} aria-label={`Remove option ${index + 1}`}><Trash2 aria-hidden="true" /></button></header>
      <LocalizedTextField label="Label" messageId={labelId} source={source} locale={locale} onSourceChange={onSourceChange} multiline={false} fallbackValue={typeof option.label === "string" ? option.label : ""} onReference={(nextMessageId) => onChange({ ...option, label: messageReference(nextMessageId) })} />
      <label className="editor-field"><span>Go to</span><select value={typeof option.targetNodeId === "string" ? option.targetNodeId : ""} onChange={(event) => onChange({ ...option, targetNodeId: event.target.value || undefined })}><option value="">Next beat</option>{targets.map((target) => <option value={target.id} key={target.id}>{target.label || target.id}</option>)}</select></label>
      <details className="choice-option-details" open={rulesOpen} onToggle={(event) => setRulesOpen(event.currentTarget.open)}>
        <summary>Rules and state changes</summary>
        <div>
          <label className="editor-toggle"><input type="checkbox" checked={Boolean(option.condition)} onChange={(event) => onChange({ ...option, condition: event.target.checked ? defaultCondition() : undefined })} /><span>Only available when</span></label>
          {option.condition ? (
            <div className="condition-builder">
              <div className="condition-builder-heading"><label><span>Match</span><select value={condition.mode} onChange={(event) => onChange({ ...option, condition: serializeCondition({ ...condition, mode: event.target.value === "any" ? "any" : "all" }) })}><option value="all">All rules</option><option value="any">Any rule</option></select></label><button type="button" className="editor-inline-action" onClick={() => onChange({ ...option, condition: serializeCondition({ ...condition, rules: [...condition.rules, { key: "state_key", op: "eq", value: true }] }) })}><Plus aria-hidden="true" />Add rule</button></div>
              {condition.rules.map((rule, ruleIndex) => (
                <div className="condition-editor" key={ruleIndex}>
                  <input aria-label="State value" value={rule.key} placeholder="state_key" onChange={(event) => onChange({ ...option, condition: serializeCondition({ ...condition, rules: condition.rules.map((current, currentIndex) => currentIndex === ruleIndex ? { ...current, key: event.target.value } : current) }) })} />
                  <select aria-label="Comparison" value={rule.op} onChange={(event) => onChange({ ...option, condition: serializeCondition({ ...condition, rules: condition.rules.map((current, currentIndex) => currentIndex === ruleIndex ? { ...current, op: event.target.value } : current) }) })}><option value="eq">is</option><option value="ne">is not</option><option value="gte">at least</option><option value="gt">greater than</option><option value="lte">at most</option><option value="lt">less than</option></select>
                  <input aria-label="Expected value" value={scalarText(rule.value)} onChange={(event) => onChange({ ...option, condition: serializeCondition({ ...condition, rules: condition.rules.map((current, currentIndex) => currentIndex === ruleIndex ? { ...current, value: parseScalar(event.target.value) } : current) }) })} />
                  <button type="button" className="icon-button" disabled={condition.rules.length === 1} onClick={() => onChange({ ...option, condition: serializeCondition({ ...condition, rules: condition.rules.filter((_current, currentIndex) => currentIndex !== ruleIndex) }) })} aria-label="Remove condition"><X aria-hidden="true" /></button>
                </div>
              ))}
            </div>
          ) : null}
          {option.condition ? <LocalizedChoiceLockReason choiceId={choiceId} optionId={id} option={option} source={source} locale={locale} onSourceChange={onSourceChange} onChange={onChange} /> : null}
          <div className="mutation-editor-list">
            <div className="mutation-editor-heading"><strong>After selection</strong><button type="button" className="editor-inline-action" onClick={() => updateMutations([...mutations, { key: "state_key", op: "set", value: true }])}><Plus aria-hidden="true" />Change state</button></div>
            {mutations.map((mutation, mutationIndex) => (
              <div className="mutation-editor" key={mutationIndex}>
                <input aria-label="State value" value={typeof mutation.key === "string" ? mutation.key : ""} onChange={(event) => updateMutations(mutations.map((current, currentIndex) => currentIndex === mutationIndex ? { ...current, key: event.target.value } : current))} />
                <select aria-label="Operation" value={mutation.op === "increment" ? "increment" : "set"} onChange={(event) => updateMutations(mutations.map((current, currentIndex) => currentIndex === mutationIndex ? { ...current, op: event.target.value } : current))}><option value="set">Set to</option><option value="increment">Add</option></select>
                <input aria-label="Value" value={scalarText(mutation.value)} onChange={(event) => updateMutations(mutations.map((current, currentIndex) => currentIndex === mutationIndex ? { ...current, value: parseScalar(event.target.value) } : current))} />
                <button type="button" className="icon-button" onClick={() => updateMutations(mutations.filter((_current, currentIndex) => currentIndex !== mutationIndex))} aria-label="Remove state change"><X aria-hidden="true" /></button>
              </div>
            ))}
          </div>
        </div>
      </details>
    </article>
  );
}

function LocalizedChoiceLockReason({
  choiceId,
  optionId,
  option,
  source,
  locale,
  onSourceChange,
  onChange,
}: {
  choiceId: string;
  optionId: string;
  option: Record<string, unknown>;
  source: GameSource;
  locale: string;
  onSourceChange: (source: GameSource) => void;
  onChange: (option: Record<string, unknown>) => void;
}) {
  const messageId = messageReferenceId(option.lockedReason) ?? `node.${choiceId}.${optionId}.locked`;
  return <LocalizedTextField label="Locked message" messageId={messageId} source={source} locale={locale} onSourceChange={onSourceChange} multiline={false} fallbackValue={typeof option.lockedReason === "string" ? option.lockedReason : ""} onReference={(nextMessageId) => onChange({ ...option, lockedReason: messageReference(nextMessageId) })} />;
}

function EditableStringList({ values, placeholder, addLabel, onChange }: { values: string[]; placeholder: string; addLabel: string; onChange: (values: string[]) => void }) {
  return (
    <div className="editor-string-list">
      {values.map((value, index) => (
        <div key={index}><input value={value} placeholder={placeholder} onChange={(event) => onChange(values.map((current, currentIndex) => currentIndex === index ? event.target.value : current))} /><button type="button" className="icon-button" onClick={() => onChange(values.filter((_current, currentIndex) => currentIndex !== index))} aria-label={`Remove ${value || "state value"}`}><X aria-hidden="true" /></button></div>
      ))}
      <button type="button" className="editor-inline-action" onClick={() => onChange([...values, ""])}><Plus aria-hidden="true" />{addLabel}</button>
    </div>
  );
}

function ensureMessageId(node: GameSourceNode, field: string): string {
  return messageReferenceId(node.config[field]) ?? `node.${node.id}.${field}`;
}

function hiddenConfigField(nodeType: string, fieldKey: string): boolean {
  if (["sequenceId", "startMs", "durationMs", "inMs", "outMs", "transitionMs", "volume", "muted"].includes(fieldKey)) return true;
  if (nodeType === "choice") return ["prompt", "options"].includes(fieldKey);
  if (nodeType === "agent") return ["agentId", "prompt", "input", "allowedStateChanges", "fallback", "blocking", "outputSchema"].includes(fieldKey);
  if (nodeType === "dialogue") return ["speakerId", "text", "blocking"].includes(fieldKey);
  return false;
}

function isLongTextField(fieldKey: string): boolean {
  return ["prompt", "text", "description", "direction", "fallback"].includes(fieldKey);
}

function uniqueOptionId(options: Record<string, unknown>[]): string {
  const existing = new Set(options.map((option) => String(option.id ?? "")));
  let index = options.length + 1;
  while (existing.has(`option-${index}`)) index += 1;
  return `option-${index}`;
}

function defaultCondition(): Record<string, unknown> {
  return conditionValue("state_key", "eq", true);
}

function conditionValue(key: string, op: string, value: unknown): Record<string, unknown> {
  return {
    op,
    left: { pointer: `/state/${key.trim().replace(/^\/+/, "")}` },
    right: { value },
  };
}

type SimpleCondition = { key: string; op: string; value: unknown };
type ConditionGroup = { mode: "all" | "any"; rules: SimpleCondition[] };

function simpleCondition(value: unknown): SimpleCondition {
  const condition = objectValue(value);
  const left = objectValue(condition.left);
  const right = objectValue(condition.right);
  const pointer = typeof left.pointer === "string" ? left.pointer : "/state/state_key";
  return {
    key: pointer.replace(/^\/state\//, ""),
    op: typeof condition.op === "string" ? condition.op : "eq",
    value: "value" in right ? right.value : true,
  };
}

function conditionGroup(value: unknown): ConditionGroup {
  const condition = objectValue(value);
  const mode = condition.op === "or" ? "any" : "all";
  if (["and", "or"].includes(String(condition.op)) && Array.isArray(condition.conditions)) {
    const rules = condition.conditions.map(simpleCondition);
    return { mode, rules: rules.length > 0 ? rules : [simpleCondition(defaultCondition())] };
  }
  return { mode: "all", rules: [simpleCondition(value)] };
}

function serializeCondition(group: ConditionGroup): Record<string, unknown> {
  const conditions = group.rules.map((rule) => conditionValue(rule.key, rule.op, rule.value));
  if (conditions.length === 1) return conditions[0] ?? defaultCondition();
  return { op: group.mode === "any" ? "or" : "and", conditions };
}

function parseScalar(value: string): unknown {
  const trimmed = value.trim();
  if (trimmed === "true") return true;
  if (trimmed === "false") return false;
  if (trimmed === "null") return null;
  if (trimmed !== "" && Number.isFinite(Number(trimmed))) return Number(trimmed);
  if ((trimmed.startsWith("{") && trimmed.endsWith("}")) || (trimmed.startsWith("[") && trimmed.endsWith("]"))) {
    try { return JSON.parse(trimmed); } catch { return value; }
  }
  return value;
}

function scalarText(value: unknown): string {
  return typeof value === "string" ? value : JSON.stringify(value ?? "");
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
        {options.map((option) => <option key={String(option)} value={String(option)}>{optionLabel(fieldKey, String(option))}</option>)}
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

function optionLabel(fieldKey: string, value: string): string {
  if (fieldKey === "fit") return value === "contain" ? "Fit whole image" : "Fill frame";
  return humanize(value);
}
