"use client";

import {
  Archive,
  Bot,
  BrainCircuit,
  Check,
  Download,
  Gamepad2,
  History,
  Braces,
  Play,
  Plus,
  RefreshCw,
  Save,
  Sparkles,
  Trash2,
  Upload,
  X,
} from "lucide-react";
import { useCallback, useEffect, useRef, useState, type ChangeEvent } from "react";
import { useRouter } from "next/navigation";
import type {
  AgentProfile,
  AgentProfileCapability,
  AgentProfileDetail,
  AgentProfileVersion,
  ProfileCapabilityKind,
  ProfileVersionWithCapabilities,
  ProviderAdapter,
  ProjectProvider,
  RuntimeProject,
} from "../lib/runtime-types";
import { RuntimeConfirmDialog } from "./runtime-confirm-dialog";

type ProfileWorkbenchProps = {
  project: RuntimeProject;
  profile: AgentProfile;
  providerAdapters: ProviderAdapter[];
  providerConnections: ProjectProvider[];
  onClose: () => void;
};

type WorkbenchTab = "overview" | "persona" | "capabilities" | "versions" | "test" | "json";

type CapabilityDraft = Omit<AgentProfileCapability, "id" | "profileVersionId" | "createdAt">;

type VersionDraft = {
  persona: Record<string, unknown>;
  runtime: Record<string, unknown>;
  presentation: Record<string, unknown>;
  source: Record<string, unknown>;
  capabilities: CapabilityDraft[];
};

type TestResult = {
  output?: unknown;
  profileId?: string;
  versionId?: string;
  version?: number;
  providerKey?: string;
  latencyMs?: number;
  previewMode?: string | null;
};

type TestExecution = {
  result?: TestResult;
  error?: string;
};

const TABS = [
  { id: "overview", label: "Agent", icon: Bot },
  { id: "persona", label: "Behavior", icon: BrainCircuit },
  { id: "capabilities", label: "Abilities", icon: Sparkles },
  { id: "versions", label: "Versions", icon: History },
  { id: "test", label: "Playtest", icon: Gamepad2 },
  { id: "json", label: "JSON", icon: Braces },
] satisfies Array<{ id: WorkbenchTab; label: string; icon: typeof Bot }>;

const CAPABILITY_LABELS: Record<ProfileCapabilityKind, string> = {
  chat: "Conversation",
  embedding: "Embeddings",
  speech: "Voice",
  transcription: "Listening",
  realtime: "Live voice",
  tool: "Tools",
};

export function RuntimeProfileWorkbench({
  project,
  profile,
  providerAdapters,
  providerConnections,
  onClose,
}: ProfileWorkbenchProps) {
  const [tab, setTab] = useState<WorkbenchTab>("overview");
  const [detail, setDetail] = useState<AgentProfileDetail | null>(null);
  const [baseVersionId, setBaseVersionId] = useState<string | null>(null);
  const [draft, setDraft] = useState<VersionDraft | null>(null);
  const [initialDraftSignature, setInitialDraftSignature] = useState("");
  const [selectedFile, setSelectedFile] = useState<string | null>(null);
  const [changeSummary, setChangeSummary] = useState("");
  const [loading, setLoading] = useState(true);
  const [pending, setPending] = useState<string | null>(null);
  const [confirmingRemoval, setConfirmingRemoval] = useState(false);
  const [message, setMessage] = useState<{ tone: "error" | "success"; text: string } | null>(null);
  const router = useRouter();
  const importRef = useRef<HTMLInputElement>(null);

  const load = useCallback(async (preferredVersionId?: string) => {
    setLoading(true);
    setMessage(null);
    try {
      const next = await runtimeRequest<AgentProfileDetail>(
        `project/${project.slug}/profiles/${profile.id}`,
        "GET",
      );
      setDetail(next);
      const preferred = preferredVersionId
        ? next.versions.find((item) => item.version.id === preferredVersionId)
        : undefined;
      const active = preferred
        ?? next.versions.find((item) => item.version.id === next.profile.activeVersionId)
        ?? next.versions[0];
      if (active) applyVersionDraft(active, setDraft, setInitialDraftSignature, setBaseVersionId, setSelectedFile);
    } catch (error) {
      setMessage({ tone: "error", text: errorMessage(error) });
    } finally {
      setLoading(false);
    }
  }, [profile.id, project.slug]);

  useEffect(() => {
    void load();
  }, [load]);

  const dirty = draft ? draftSignature(draft) !== initialDraftSignature : false;
  const activeVersion = detail?.versions.find((item) => item.version.id === detail.profile.activeVersionId);
  const sourceManaged = draft?.source.type === "openclaw" && draft.source.managed !== false;

  async function saveVersion() {
    if (!draft) return;
    setPending("save-version");
    setMessage(null);
    try {
      const payload = await runtimeRequest<ProfileVersionWithCapabilities>(
        `project/${project.slug}/profiles/${profile.id}/versions`,
        "POST",
        { ...draft, changeSummary: changeSummary.trim() || undefined },
      );
      const nextVersionId = payload.version.id;
      setChangeSummary("");
      await load(nextVersionId);
      setTab("test");
      setMessage({ tone: "success", text: `Version ${payload.version.versionNumber} saved. Playtest it, then make it live when ready.` });
    } catch (error) {
      setMessage({ tone: "error", text: errorMessage(error) });
    } finally {
      setPending(null);
    }
  }

  async function syncSource() {
    setPending("sync-source");
    setMessage(null);
    try {
      const payload = await runtimeRequest<{ version?: ProfileVersionWithCapabilities }>(
        `project/${project.slug}/profiles/${profile.id}/source/sync`,
        "POST",
        { changeSummary: "Synced from provider" },
      );
      await load(payload.version?.version.id);
      setTab("persona");
      setMessage({ tone: "success", text: "Provider behavior and tools synced into a new version." });
    } catch (error) {
      setMessage({ tone: "error", text: errorMessage(error) });
    } finally {
      setPending(null);
    }
  }

  async function activateVersion(versionId: string) {
    const version = detail?.versions.find((item) => item.version.id === versionId)?.version;
    if (!window.confirm(`Make version ${version?.versionNumber ?? ""} live for this game?`)) return;
    setPending(`activate:${versionId}`);
    setMessage(null);
    try {
      await runtimeRequest(
        `project/${project.slug}/profiles/${profile.id}/versions/${versionId}/activate`,
        "POST",
        {},
      );
      await load(versionId);
      router.refresh();
      setMessage({ tone: "success", text: "Version is now live in the game." });
    } catch (error) {
      setMessage({ tone: "error", text: errorMessage(error) });
    } finally {
      setPending(null);
    }
  }

  async function archiveVersion(versionId: string) {
    setPending(`archive:${versionId}`);
    setMessage(null);
    try {
      await runtimeRequest(
        `project/${project.slug}/profiles/${profile.id}/versions/${versionId}/archive`,
        "POST",
        {},
      );
      await load();
      setMessage({ tone: "success", text: "Version archived." });
    } catch (error) {
      setMessage({ tone: "error", text: errorMessage(error) });
    } finally {
      setPending(null);
    }
  }

  function editFromVersion(version: ProfileVersionWithCapabilities) {
    applyVersionDraft(version, setDraft, setInitialDraftSignature, setBaseVersionId, setSelectedFile);
    setTab("persona");
    setMessage({ tone: "success", text: `Editing from version ${version.version.versionNumber}.` });
  }

  async function updateProfile(name: string, description: string) {
    setPending("profile");
    setMessage(null);
    try {
      await runtimeRequest(
        `project/${project.slug}/profiles/${profile.id}`,
        "PATCH",
        { name: name.trim(), description: description.trim() },
      );
      await load(baseVersionId ?? undefined);
      router.refresh();
      setMessage({ tone: "success", text: "Agent details updated." });
    } catch (error) {
      setMessage({ tone: "error", text: errorMessage(error) });
    } finally {
      setPending(null);
    }
  }

  async function deleteProfile() {
    setPending("delete-profile");
    setMessage(null);
    try {
      await runtimeRequest(`project/${project.slug}/profiles/${profile.id}`, "DELETE");
      router.refresh();
      onClose();
    } catch (error) {
      setMessage({ tone: "error", text: errorMessage(error) });
    } finally {
      setPending(null);
      setConfirmingRemoval(false);
    }
  }

  function exportVersion() {
    if (!detail || !draft) return;
    const payload = {
      format: "vifu-agent-profile",
      version: 1,
      profile: {
        slug: detail.profile.slug,
        name: detail.profile.name,
        description: detail.profile.description,
      },
      definition: draft,
    };
    downloadJson(`${detail.profile.slug}.vifu-profile.json`, payload);
  }

  async function importVersion(event: ChangeEvent<HTMLInputElement>) {
    const file = event.currentTarget.files?.[0];
    event.currentTarget.value = "";
    if (!file) return;
    try {
      const payload = JSON.parse(await file.text()) as unknown;
      const nextDraft = importedDraft(payload);
      setDraft(nextDraft);
      setBaseVersionId(null);
      setSelectedFile(firstPersonaFile(nextDraft));
      setMessage({ tone: "success", text: "Agent file loaded as an unsaved version." });
    } catch (error) {
      setMessage({ tone: "error", text: errorMessage(error) });
    }
  }

  return (
    <>
      <aside className="profile-workbench" aria-label={`${profile.name} profile`}>
      <header className="profile-workbench-header">
        <div className="profile-workbench-title">
          <span>Selected agent</span>
          <strong>{detail?.profile.name ?? profile.name}</strong>
          <code>{detail?.profile.slug ?? profile.slug}</code>
        </div>
        <div className="profile-workbench-header-actions">
          <input ref={importRef} className="sr-only" type="file" accept="application/json" onChange={importVersion} />
          <button type="button" onClick={() => importRef.current?.click()} title="Import profile" aria-label="Import profile"><Upload aria-hidden="true" /></button>
          <button type="button" onClick={exportVersion} disabled={!draft} title="Export profile" aria-label="Export profile"><Download aria-hidden="true" /></button>
          <button type="button" onClick={onClose} title="Close profile" aria-label="Close profile"><X aria-hidden="true" /></button>
        </div>
      </header>

      <nav className="profile-workbench-tabs" aria-label="Profile sections">
        {TABS.map((item) => {
          const Icon = item.icon;
          return (
            <button
              type="button"
              key={item.id}
              className={tab === item.id ? "active" : ""}
              aria-current={tab === item.id ? "page" : undefined}
              onClick={() => setTab(item.id)}
            >
              <Icon aria-hidden="true" />{item.label}
            </button>
          );
        })}
      </nav>

      <div className="profile-workbench-body">
        {loading ? <ProfileLoading /> : null}
        {!loading && !draft ? <ProfileEmpty message={message?.text ?? "This agent has no editable version yet."} /> : null}
        {!loading && draft && detail ? (
          <>
            {tab === "overview" ? (
              <OverviewPanel
                project={project}
                detail={detail}
                activeVersion={activeVersion}
                pending={pending}
                onUpdateProfile={updateProfile}
                onRemove={() => setConfirmingRemoval(true)}
                onSync={sourceManaged ? syncSource : undefined}
                presentation={draft.presentation}
                onPresentationChange={(presentation) => setDraft({ ...draft, presentation })}
              />
            ) : null}
            {tab === "persona" ? (
              <PersonaPanel
                draft={draft}
                sourceManaged={sourceManaged}
                selectedFile={selectedFile}
                onSelectedFile={setSelectedFile}
                onDraft={setDraft}
                onSync={sourceManaged ? syncSource : undefined}
                syncing={pending === "sync-source"}
              />
            ) : null}
            {tab === "capabilities" ? (
              <CapabilitiesPanel
                capabilities={draft.capabilities}
                providerAdapters={providerAdapters}
                providerConnections={providerConnections}
                sourceManaged={sourceManaged}
                settingsHref={`/project/${project.slug}/providers`}
                onChange={(capabilities) => setDraft({ ...draft, capabilities })}
              />
            ) : null}
            {tab === "versions" ? (
              <VersionsPanel
                detail={detail}
                baseVersionId={baseVersionId}
                pending={pending}
                onActivate={activateVersion}
                onArchive={archiveVersion}
                onEdit={editFromVersion}
              />
            ) : null}
            {tab === "test" ? (
              <TestPanel project={project} profile={detail.profile} versions={detail.versions} baseVersionId={baseVersionId} />
            ) : null}
            {tab === "json" ? (
              <JsonPanel
                draft={draft}
                onDraft={setDraft}
                onApplied={() => setMessage({
                  tone: "success",
                  text: "JSON applied to the editor. Save a new version to keep your changes.",
                })}
              />
            ) : null}
          </>
        ) : null}
      </div>

      {message ? <div className={`profile-workbench-message ${message.tone}`} role={message.tone === "error" ? "alert" : "status"}>{message.text}</div> : null}
      {draft && dirty ? (
        <footer className="profile-workbench-footer">
          <label>
            <span>Version note</span>
            <input value={changeSummary} maxLength={1024} onChange={(event) => setChangeSummary(event.target.value)} placeholder="What changed?" />
          </label>
          <button className="primary-button" type="button" disabled={!dirty || pending !== null} onClick={saveVersion}>
            <Save aria-hidden="true" />{pending === "save-version" ? "Saving" : "Save as new version"}
          </button>
        </footer>
      ) : null}
      </aside>
      {confirmingRemoval ? (
        <RuntimeConfirmDialog
          title="Remove agent?"
          description={`${profile.name} will be removed from ${project.name}. You can add it again from the agent picker.`}
          confirmLabel="Remove agent"
          pending={pending === "delete-profile"}
          onCancel={() => setConfirmingRemoval(false)}
          onConfirm={deleteProfile}
        />
      ) : null}
    </>
  );
}

function OverviewPanel({
  project,
  detail,
  activeVersion,
  pending,
  onUpdateProfile,
  onRemove,
  onSync,
  presentation,
  onPresentationChange,
}: {
  project: RuntimeProject;
  detail: AgentProfileDetail;
  activeVersion?: ProfileVersionWithCapabilities;
  pending: string | null;
  onUpdateProfile: (name: string, description: string) => void;
  onRemove: () => void;
  onSync?: () => void;
  presentation: Record<string, unknown>;
  onPresentationChange: (presentation: Record<string, unknown>) => void;
}) {
  const source = activeVersion?.version.source ?? {};
  const sourceName = source.type === "openclaw" ? "OpenClaw" : stringValue(source.providerKey) || "Custom provider";
  const [name, setName] = useState(detail.profile.name);
  const [description, setDescription] = useState(detail.profile.description ?? "");
  useEffect(() => {
    setName(detail.profile.name);
    setDescription(detail.profile.description ?? "");
  }, [detail.profile.description, detail.profile.name, detail.profile.updatedAt]);
  const metadataDirty = name.trim() !== detail.profile.name || description.trim() !== (detail.profile.description ?? "");
  return (
    <div className="profile-panel-stack">
      <section className="profile-overview-status">
        <div><span>Project access</span><strong><i className="ready" />Available</strong></div>
        <div><span>Live version</span><strong>{activeVersion ? `v${activeVersion.version.versionNumber}` : "Not set"}</strong></div>
        <div><span>Runs on</span><strong>{sourceName}</strong></div>
      </section>
      <section className="profile-definition-list">
        <div className="profile-section-heading"><h3>Agent details</h3><p>The name and role used while building this project.</p></div>
        <div className="profile-identity-fields">
          <label><span>Agent name</span><input value={name} maxLength={128} onChange={(event) => setName(event.target.value)} /></label>
          <label><span>Role in your game</span><textarea value={description} maxLength={4096} onChange={(event) => setDescription(event.target.value)} placeholder="Welcomes players and helps them explore the world" /></label>
        </div>
        <dl><div><dt>Agent ID</dt><dd><code>{detail.profile.slug}</code></dd></div><div><dt>Game project</dt><dd>{project.name}</dd></div></dl>
        <button className="secondary-button" type="button" disabled={!metadataDirty || !name.trim() || pending !== null} onClick={() => onUpdateProfile(name, description)}>{pending === "profile" ? "Saving" : "Save agent details"}</button>
      </section>
      <GameIdentityPanel presentation={presentation} onChange={onPresentationChange} />
      <details className="profile-provider-details">
        <summary><span>Provider details</span><strong>{sourceName}</strong></summary>
        <div className="profile-definition-list">
          <dl>
            <div><dt>Connection</dt><dd><code>{stringValue(source.providerKey) || sourceName}</code></dd></div>
            <div><dt>Provider agent</dt><dd><code>{stringValue(source.resourceId) || "Not bound"}</code></dd></div>
            <div><dt>Last synced</dt><dd>{formatDate(stringValue(source.syncedAt))}</dd></div>
          </dl>
          {onSync ? <button className="secondary-button" type="button" disabled={pending !== null} onClick={onSync}><RefreshCw aria-hidden="true" />{pending === "sync-source" ? "Syncing" : "Sync from provider"}</button> : null}
        </div>
      </details>
      <section className="profile-game-access">
        <div><strong>Available through the project API</strong><span>Requests can select this agent using its Agent ID.</span></div>
        <button className="danger-text-button" type="button" disabled={pending !== null} onClick={onRemove}><Trash2 aria-hidden="true" />Remove agent</button>
      </section>
    </div>
  );
}

function PersonaPanel({
  draft,
  sourceManaged,
  selectedFile,
  onSelectedFile,
  onDraft,
  onSync,
  syncing,
}: {
  draft: VersionDraft;
  sourceManaged: boolean;
  selectedFile: string | null;
  onSelectedFile: (name: string) => void;
  onDraft: (draft: VersionDraft) => void;
  onSync?: () => void;
  syncing: boolean;
}) {
  const files = recordValue(draft.persona.files);
  const names = Object.keys(files);
  const currentName = selectedFile && names.includes(selectedFile) ? selectedFile : names[0] ?? null;

  function updateFile(name: string, content: string) {
    onDraft({ ...draft, persona: { ...draft.persona, files: { ...files, [name]: content } } });
  }

  if (sourceManaged) {
    return (
      <div className="profile-persona-editor">
        <header>
          <div><h3>Agent behavior</h3><p>Edit the behavior files supplied by this agent runtime. Changes stay in a new version until you make it live.</p></div>
          {onSync ? <button className="secondary-button" type="button" onClick={onSync} disabled={syncing}><RefreshCw aria-hidden="true" />{syncing ? "Syncing" : "Sync from provider"}</button> : null}
        </header>
        {currentName ? (
          <div className="profile-file-editor">
            <nav aria-label="Persona files">
              {names.map((name) => <button type="button" key={name} className={name === currentName ? "active" : ""} onClick={() => onSelectedFile(name)}>{name}</button>)}
            </nav>
            <label>
              <span>{currentName}</span>
              <textarea value={stringValue(files[currentName])} onChange={(event) => updateFile(currentName, event.target.value)} spellCheck={false} />
            </label>
          </div>
        ) : (
          <ProfileEmpty message="Sync from the provider to load this agent's behavior files." />
        )}
      </div>
    );
  }

  return (
    <div className="profile-panel-stack">
      <section className="profile-text-editor">
        <div><h3>Behavior instructions</h3><p>Define how this agent speaks, decides, and responds inside the game.</p></div>
        <textarea
          value={stringValue(draft.persona.systemPrompt)}
          onChange={(event) => onDraft({ ...draft, persona: { ...draft.persona, systemPrompt: event.target.value } })}
          placeholder="Describe the agent's role, voice, goals, and boundaries."
        />
      </section>
    </div>
  );
}

function CapabilitiesPanel({
  capabilities,
  providerAdapters,
  providerConnections,
  sourceManaged,
  settingsHref,
  onChange,
}: {
  capabilities: CapabilityDraft[];
  providerAdapters: ProviderAdapter[];
  providerConnections: ProjectProvider[];
  sourceManaged: boolean;
  settingsHref: string;
  onChange: (capabilities: CapabilityDraft[]) => void;
}) {
  const availableKinds = (Object.keys(CAPABILITY_LABELS) as ProfileCapabilityKind[])
    .filter((kind) => !capabilities.some((capability) => capability.kind === kind));
  const addableKinds = availableKinds.filter((kind) => providerConnections.some((connection) => providerSupportsCapability(connection.providerType, kind)));
  const [newKind, setNewKind] = useState<ProfileCapabilityKind>(addableKinds[0] ?? "chat");
  const compatibleConnections = providerConnections.filter((connection) => providerSupportsCapability(connection.providerType, newKind));
  const [newProviderKey, setNewProviderKey] = useState(compatibleConnections[0]?.providerKey ?? "");

  useEffect(() => {
    if (!addableKinds.includes(newKind)) setNewKind(addableKinds[0] ?? "chat");
  }, [addableKinds, newKind]);

  useEffect(() => {
    if (!compatibleConnections.some((connection) => connection.providerKey === newProviderKey)) {
      setNewProviderKey(compatibleConnections[0]?.providerKey ?? "");
    }
  }, [compatibleConnections, newProviderKey]);

  function addCapability() {
    const connection = compatibleConnections.find((item) => item.providerKey === newProviderKey);
    if (!addableKinds.includes(newKind) || !connection) return;
    onChange([...capabilities, {
      kind: newKind,
      providerType: capabilityProviderType(connection.providerType),
      providerKey: connection.providerKey,
      resourceId: null,
      config: {},
      inputSchema: {},
      outputSchema: {},
    }]);
  }

  function updateCapability(index: number, patch: Partial<CapabilityDraft>) {
    onChange(capabilities.map((capability, itemIndex) => itemIndex === index ? { ...capability, ...patch } : capability));
  }

  return (
    <div className="profile-capabilities">
      <header><div><h3>Agent abilities</h3><p>Choose what this agent can do and which provider performs each ability.</p></div></header>
      <div className="profile-capability-list">
        {capabilities.map((capability, index) => {
          const connection = providerConnections.find((item) => item.providerKey === capability.providerKey);
          const capabilityConnections = providerConnections.filter((item) => providerSupportsCapability(item.providerType, capability.kind));
          const locked = sourceManaged && capability.providerType === "openclaw" && (capability.kind === "chat" || capability.kind === "tool");
          return (
            <section key={`${capability.kind}:${index}`}>
              <header>
                <div><strong>{CAPABILITY_LABELS[capability.kind]}</strong><span>{connection?.name ?? capability.providerKey}</span></div>
                <button type="button" disabled={locked} onClick={() => onChange(capabilities.filter((_, itemIndex) => itemIndex !== index))} title={`Remove ${CAPABILITY_LABELS[capability.kind]}`} aria-label={`Remove ${CAPABILITY_LABELS[capability.kind]}`}><X aria-hidden="true" /></button>
              </header>
              <div className="profile-capability-fields">
                <label><span>Runs on</span>
                  <select
                    value={capability.providerKey}
                    disabled={locked}
                    onChange={(event) => {
                      const next = providerConnections.find((item) => item.providerKey === event.target.value);
                      if (next) updateCapability(index, { providerKey: next.providerKey, providerType: capabilityProviderType(next.providerType), resourceId: null });
                    }}
                  >
                    {!connection ? <option value={capability.providerKey}>{capability.providerKey}</option> : null}
                    {capabilityConnections.map((item) => <option value={item.providerKey} key={item.id}>{item.name}</option>)}
                  </select>
                </label>
                <label><span>{resourceLabel(capability.kind, capability.providerType)}</span>
                  <input
                    value={capability.resourceId ?? ""}
                    disabled={locked}
                    onChange={(event) => updateCapability(index, { resourceId: event.target.value || null })}
                    placeholder={resourcePlaceholder(capability.kind, capability.providerType)}
                  />
                </label>
              </div>
              {capability.kind === "tool" ? <ToolSummary capability={capability} /> : null}
            </section>
          );
        })}
      </div>
      <div className="profile-capability-footer">
        {addableKinds.length > 0 ? (
          <div className="profile-capability-add">
            <select value={newKind} onChange={(event) => setNewKind(event.target.value as ProfileCapabilityKind)} aria-label="Ability type">
              {addableKinds.map((kind) => <option value={kind} key={kind}>{CAPABILITY_LABELS[kind]}</option>)}
            </select>
            <select value={newProviderKey} onChange={(event) => setNewProviderKey(event.target.value)} aria-label="Ability provider">
              {compatibleConnections.map((connection) => <option value={connection.providerKey} key={connection.id}>{connection.name}</option>)}
            </select>
            <button className="secondary-button" type="button" disabled={!newProviderKey} onClick={addCapability}><Plus aria-hidden="true" />Add ability</button>
          </div>
        ) : (
          <div className="profile-capability-next">
            <span>{availableKinds.length === 0 ? "All available abilities are already configured." : "Connect another provider to add voice, listening, or realtime abilities."}</span>
            {availableKinds.length > 0 ? <a className="secondary-button" href={settingsHref}>Connect provider</a> : null}
          </div>
        )}
      </div>
      <ProviderLegend adapters={providerAdapters} connections={providerConnections} />
    </div>
  );
}

function VersionsPanel({
  detail,
  baseVersionId,
  pending,
  onActivate,
  onArchive,
  onEdit,
}: {
  detail: AgentProfileDetail;
  baseVersionId: string | null;
  pending: string | null;
  onActivate: (versionId: string) => void;
  onArchive: (versionId: string) => void;
  onEdit: (version: ProfileVersionWithCapabilities) => void;
}) {
  const active = detail.versions.find((item) => item.version.id === detail.profile.activeVersionId);
  return (
    <div className="profile-versions">
      <header><div><h3>Version history</h3><p>Playtest any saved version. Only the live version answers new game requests.</p></div></header>
      <div className="profile-version-list">
        {detail.versions.map((item) => {
          const isActive = item.version.id === detail.profile.activeVersionId;
          const changed = active && active.version.id !== item.version.id ? changedSections(active, item) : [];
          return (
            <section key={item.version.id} className={isActive ? "active" : ""}>
              <header>
                <div><strong>v{item.version.versionNumber}</strong>{isActive ? <span><Check aria-hidden="true" />Live</span> : null}{item.version.archivedAt ? <span>Archived</span> : null}</div>
                <time dateTime={item.version.createdAt}>{formatDate(item.version.createdAt)}</time>
              </header>
              <p>{item.version.changeSummary || "No version note"}</p>
              <div className="profile-version-meta"><code>{item.version.contentHash.slice(0, 10)}</code><span>{item.capabilities.length} capabilities</span>{changed.length > 0 ? <span>Changed: {changed.join(", ")}</span> : null}</div>
              <div className="profile-version-actions">
                {baseVersionId === item.version.id ? <span className="profile-version-current">Editing base</span> : <button className="secondary-button" type="button" disabled={pending !== null} onClick={() => onEdit(item)}>Edit as new version</button>}
                {!isActive && !item.version.archivedAt ? <button className="primary-button" type="button" disabled={pending !== null} onClick={() => onActivate(item.version.id)}>{pending === `activate:${item.version.id}` ? "Going live" : "Make live"}</button> : null}
                {!isActive && !item.version.archivedAt ? <button className="icon-button" type="button" disabled={pending !== null} onClick={() => onArchive(item.version.id)} title="Archive version" aria-label="Archive version"><Archive aria-hidden="true" /></button> : null}
              </div>
            </section>
          );
        })}
      </div>
    </div>
  );
}

function JsonPanel({
  draft,
  onDraft,
  onApplied,
}: {
  draft: VersionDraft;
  onDraft: (draft: VersionDraft) => void;
  onApplied: () => void;
}) {
  const [source, setSource] = useState(() => JSON.stringify(draft, null, 2));
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setSource(JSON.stringify(draft, null, 2));
  }, [draft]);

  function apply() {
    try {
      const next = importedDraft(JSON.parse(source) as unknown);
      onDraft(next);
      setError(null);
      onApplied();
    } catch (nextError) {
      setError(errorMessage(nextError));
    }
  }

  return (
    <div className="profile-json-panel">
      <header><div><h3>Agent JSON</h3><p>This is the same draft shown in the visual editor. Applying valid JSON updates every editor tab.</p></div></header>
      <textarea
        aria-label="Agent profile JSON"
        spellCheck={false}
        value={source}
        onChange={(event) => setSource(event.target.value)}
      />
      <div className="profile-json-actions">
        {error ? <span className="inline-error" role="alert">{error}</span> : <span />}
        <button className="secondary-button" type="button" onClick={apply}><Braces aria-hidden="true" />Apply JSON</button>
      </div>
    </div>
  );
}

function TestPanel({
  project,
  profile,
  versions,
  baseVersionId,
}: {
  project: RuntimeProject;
  profile: AgentProfile;
  versions: ProfileVersionWithCapabilities[];
  baseVersionId: string | null;
}) {
  const defaultVersion = baseVersionId ?? profile.activeVersionId ?? versions[0]?.version.id ?? "";
  const [versionId, setVersionId] = useState(defaultVersion);
  const [compareVersionId, setCompareVersionId] = useState("");
  const [prompt, setPrompt] = useState("");
  const [pending, setPending] = useState(false);
  const [executions, setExecutions] = useState<Array<{ versionId: string; execution: TestExecution }>>([]);

  useEffect(() => {
    setVersionId(defaultVersion);
    setCompareVersionId((current) => current === defaultVersion ? "" : current);
  }, [defaultVersion]);

  async function run() {
    if (!prompt.trim() || !versionId) return;
    setPending(true);
    setExecutions([]);
    const versionIds = compareVersionId && compareVersionId !== versionId
      ? [versionId, compareVersionId]
      : [versionId];
    const settled = await Promise.allSettled(versionIds.map((targetVersionId) => runtimeRequest<TestResult>(
      `project/${project.slug}/profiles/${profile.id}/test`,
      "POST",
      {
        versionId: targetVersionId,
        capability: "chat",
        input: { messages: [{ role: "user", content: prompt.trim() }] },
        user: "dashboard-profile-test",
      },
    )));
    setExecutions(settled.map((execution, index) => ({
      versionId: versionIds[index],
      execution: execution.status === "fulfilled"
        ? { result: execution.value }
        : { error: errorMessage(execution.reason) },
    })));
    setPending(false);
  }

  const selectableVersions = versions.filter((item) => !item.version.archivedAt);
  return (
    <div className="profile-test-panel">
      <header><div><h3>Playtest this agent</h3><p>Talk to a saved version before making it live in your game.</p></div></header>
      <div className="profile-test-version-fields">
        <label><span>Version</span><select value={versionId} onChange={(event) => { setVersionId(event.target.value); if (compareVersionId === event.target.value) setCompareVersionId(""); }}>{selectableVersions.map((item) => <option value={item.version.id} key={item.version.id}>v{item.version.versionNumber}{item.version.id === profile.activeVersionId ? " - live" : ""}</option>)}</select></label>
        <label><span>Compare with</span><select value={compareVersionId} onChange={(event) => setCompareVersionId(event.target.value)}><option value="">No comparison</option>{selectableVersions.filter((item) => item.version.id !== versionId).map((item) => <option value={item.version.id} key={item.version.id}>v{item.version.versionNumber}{item.version.id === profile.activeVersionId ? " - live" : ""}</option>)}</select></label>
      </div>
      <label className="profile-test-input"><span>Player message</span><textarea value={prompt} onChange={(event) => setPrompt(event.target.value)} placeholder="What would a player say to this agent?" /></label>
      <button className="primary-button" type="button" disabled={pending || !prompt.trim() || !versionId} onClick={run}><Play aria-hidden="true" />{pending ? "Waiting for reply" : compareVersionId ? "Compare replies" : "Send message"}</button>
      {executions.length > 0 ? <div className={executions.length > 1 ? "profile-test-results comparison" : "profile-test-results"}>{executions.map(({ versionId: testedVersionId, execution }) => <ProfileTestResult key={testedVersionId} version={versions.find((item) => item.version.id === testedVersionId)?.version} execution={execution} />)}</div> : null}
    </div>
  );
}

function ProfileTestResult({ version, execution }: { version?: AgentProfileVersion; execution: TestExecution }) {
  if (execution.error) {
    return <section className="profile-test-result error"><header><strong>{version ? `Version ${version.versionNumber}` : "Test"}</strong><span>Failed</span></header><p>{execution.error}</p></section>;
  }
  const result = execution.result;
  if (!result) return null;
  const responseText = testResponseText(result.output);
  return (
    <section className="profile-test-result">
      <header><strong>Agent · v{result.version}</strong><span>{result.latencyMs ?? 0} ms</span></header>
      <p>{responseText || "The provider returned an empty response."}</p>
      <dl><div><dt>Provider</dt><dd>{result.providerKey}</dd></div>{result.previewMode ? <div><dt>Preview</dt><dd>{result.previewMode}</dd></div> : null}</dl>
    </section>
  );
}

function GameIdentityPanel({
  presentation,
  onChange,
}: {
  presentation: Record<string, unknown>;
  onChange: (presentation: Record<string, unknown>) => void;
}) {
  return (
    <section className="profile-presentation-editor">
      <div><h3>In-game identity</h3><p>Optional presentation data your game can use for this agent.</p></div>
      <label><span>Name shown in game</span><input value={stringValue(presentation.displayName)} onChange={(event) => onChange({ ...presentation, displayName: event.target.value })} placeholder="Display name" /></label>
      <label><span>Portrait URL</span><input type="url" value={stringValue(presentation.avatarUrl)} onChange={(event) => onChange({ ...presentation, avatarUrl: event.target.value })} placeholder="https://..." /></label>
      <label><span>Voice direction</span><input value={stringValue(presentation.voiceStyle)} onChange={(event) => onChange({ ...presentation, voiceStyle: event.target.value })} placeholder="Calm, warm, concise" /></label>
      <label><span>Game role</span><input value={stringValue(presentation.uiHint)} onChange={(event) => onChange({ ...presentation, uiHint: event.target.value })} placeholder="Merchant, guide, rival" /></label>
    </section>
  );
}

function ToolSummary({ capability }: { capability: CapabilityDraft }) {
  const tools = Array.isArray(capability.config.tools) ? capability.config.tools : [];
  return <p className="profile-tool-summary">{tools.length > 0 ? `${tools.length} tools detected from the provider.` : "Tool availability is resolved by the provider at runtime."}</p>;
}

function ProviderLegend({ adapters, connections }: { adapters: ProviderAdapter[]; connections: ProjectProvider[] }) {
  const adapterById = new Map(adapters.map((adapter) => [adapter.id, adapter]));
  return (
    <div className="profile-provider-legend">
      {connections.map((connection) => <span key={connection.id}><i className={connection.status === "online" ? "ready" : ""} />{connection.name}<small>{adapterById.get(connection.providerType)?.category ?? connection.providerType}</small></span>)}
    </div>
  );
}

function ProfileLoading() {
  return <div className="profile-loading" aria-label="Loading profile"><i /><i /><i /></div>;
}

function ProfileEmpty({ message }: { message: string }) {
  return <div className="profile-empty"><strong>No profile data</strong><span>{message}</span></div>;
}

function applyVersionDraft(
  item: ProfileVersionWithCapabilities,
  setDraft: (draft: VersionDraft) => void,
  setSignature: (signature: string) => void,
  setBaseVersionId: (id: string) => void,
  setSelectedFile: (name: string | null) => void,
) {
  const next = versionDraft(item);
  setDraft(next);
  setSignature(draftSignature(next));
  setBaseVersionId(item.version.id);
  setSelectedFile(firstPersonaFile(next));
}

function versionDraft(item: ProfileVersionWithCapabilities): VersionDraft {
  return {
    persona: structuredClone(item.version.persona),
    runtime: structuredClone(item.version.runtime),
    presentation: structuredClone(item.version.presentation),
    source: structuredClone(item.version.source),
    capabilities: item.capabilities.map((capability) => ({
      kind: capability.kind,
      providerType: capability.providerType,
      providerKey: capability.providerKey,
      resourceId: capability.resourceId,
      config: structuredClone(capability.config),
      inputSchema: structuredClone(capability.inputSchema),
      outputSchema: structuredClone(capability.outputSchema),
    })),
  };
}

function importedDraft(payload: unknown): VersionDraft {
  if (!payload || typeof payload !== "object") throw new Error("Agent file must contain a JSON object.");
  const object = payload as Record<string, unknown>;
  const definition = object.definition && typeof object.definition === "object"
    ? object.definition as Record<string, unknown>
    : object;
  const capabilities = definition.capabilities;
  if (!Array.isArray(capabilities)) throw new Error("Agent file does not include abilities.");
  return {
    persona: recordValue(definition.persona),
    runtime: recordValue(definition.runtime),
    presentation: recordValue(definition.presentation),
    source: recordValue(definition.source),
    capabilities: capabilities.map((item) => {
      if (!item || typeof item !== "object") throw new Error("Agent ability is invalid.");
      const capability = item as Record<string, unknown>;
      const kind = stringValue(capability.kind) as ProfileCapabilityKind;
      if (!(kind in CAPABILITY_LABELS)) throw new Error(`Unsupported capability ${kind || "type"}.`);
      const providerType = stringValue(capability.providerType);
      const providerKey = stringValue(capability.providerKey);
      if (!providerType || !providerKey) throw new Error("Agent ability is missing its provider.");
      return {
        kind,
        providerType,
        providerKey,
        resourceId: stringValue(capability.resourceId) || null,
        config: recordValue(capability.config),
        inputSchema: recordValue(capability.inputSchema),
        outputSchema: recordValue(capability.outputSchema),
      };
    }),
  };
}

function draftSignature(draft: VersionDraft): string {
  return JSON.stringify(draft);
}

function firstPersonaFile(draft: VersionDraft): string | null {
  return Object.keys(recordValue(draft.persona.files))[0] ?? null;
}

function changedSections(left: ProfileVersionWithCapabilities, right: ProfileVersionWithCapabilities): string[] {
  const sections: string[] = [];
  if (JSON.stringify(left.version.persona) !== JSON.stringify(right.version.persona)) sections.push("behavior");
  if (JSON.stringify(left.capabilities) !== JSON.stringify(right.capabilities)) sections.push("abilities");
  if (JSON.stringify(left.version.runtime) !== JSON.stringify(right.version.runtime)) sections.push("runtime");
  if (JSON.stringify(left.version.presentation) !== JSON.stringify(right.version.presentation)) sections.push("game identity");
  return sections;
}

function resourceLabel(kind: ProfileCapabilityKind, providerType: string): string {
  if (providerType === "openclaw") return "OpenClaw agent";
  if (kind === "speech") return "Voice ID";
  if (kind === "transcription") return "Model";
  if (kind === "chat" || kind === "embedding" || kind === "realtime") return "Model";
  return "Resource ID";
}

function resourcePlaceholder(kind: ProfileCapabilityKind, providerType: string): string {
  if (providerType === "openclaw") return "agent-id";
  if (kind === "speech") return "voice-id";
  if (kind === "transcription") return providerType === "local-whisper" ? "ggml-model.bin" : "whisper-1";
  return "provider-model";
}

function providerSupportsCapability(providerType: string, kind: ProfileCapabilityKind): boolean {
  if (providerType === "openclaw") return kind === "chat" || kind === "tool";
  if (providerType === "openai-compatible") return kind === "chat" || kind === "embedding" || kind === "transcription" || kind === "realtime";
  if (providerType === "llama" || providerType === "vifu-runtime") return kind === "chat" || kind === "embedding";
  if (providerType === "elevenlabs") return kind === "speech";
  if (providerType === "local-whisper") return kind === "transcription";
  return false;
}

function capabilityProviderType(providerType: string): string {
  return providerType === "llama" ? "vifu-runtime" : providerType;
}

function testResponseText(output: unknown): string {
  if (!output || typeof output !== "object") return typeof output === "string" ? output : "";
  const object = output as Record<string, unknown>;
  const choices = Array.isArray(object.choices) ? object.choices : [];
  const message = choices[0] && typeof choices[0] === "object"
    ? (choices[0] as Record<string, unknown>).message
    : null;
  return message && typeof message === "object" ? stringValue((message as Record<string, unknown>).content) : "";
}

function recordValue(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : {};
}

function stringValue(value: unknown): string {
  return typeof value === "string" ? value : "";
}

function formatDate(value: string): string {
  if (!value) return "Never";
  const date = new Date(value);
  if (Number.isNaN(date.valueOf())) return value;
  return new Intl.DateTimeFormat("en", { dateStyle: "medium", timeStyle: "short" }).format(date);
}

function downloadJson(name: string, payload: unknown) {
  const blob = new Blob([JSON.stringify(payload, null, 2)], { type: "application/json" });
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = name;
  link.click();
  URL.revokeObjectURL(url);
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

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : "Request failed.";
}
