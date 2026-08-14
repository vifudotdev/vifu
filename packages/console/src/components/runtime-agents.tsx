"use client";

import { Bot, ChevronRight, Plus, Search, X } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type FormEvent } from "react";
import { useRuntimeConsoleHost, useRuntimeConsoleRouter } from "../host";
import type {
  AgentBinding,
  AgentProfile,
  AgentProfileDetail,
  AvailableAgent,
  ProfileCapabilityKind,
  ProjectAgentCandidate,
  ProjectProvider,
  ProviderAdapter,
  RuntimeProject,
} from "../types";
import { RuntimeProfileWorkbench } from "./runtime-profile-workbench";

type AgentsViewProps = {
  project: RuntimeProject;
  profiles: AgentProfile[];
  profileDetails: AgentProfileDetail[];
  bindings: AgentBinding[];
  availableAgents: AvailableAgent[];
  candidates: ProjectAgentCandidate[];
  providerAdapters: ProviderAdapter[];
  projectProviders: ProjectProvider[];
};

export function RuntimeAgentsView(props: AgentsViewProps) {
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const selected = props.profiles.find((profile) => profile.id === selectedId) ?? null;
  return (
    <div className={`agents-management${selected ? " has-workbench" : ""}`}>
      <main className="agents-library">
        <header className="resource-page-heading agents-page-heading">
          <div className="resource-page-summary">
            <strong>{props.profiles.length} {props.profiles.length === 1 ? "agent" : "agents"}</strong>
            <span>Characters and AI roles available to this app.</span>
          </div>
          <button className="primary-button compact" type="button" onClick={() => setAdding(true)}><Plus aria-hidden="true" />Add agent</button>
        </header>

        {props.profiles.length > 0 ? (
          <div className="agent-card-grid project-agent-grid">
            {props.profiles.map((profile) => (
              <AgentCard
                key={profile.id}
                profile={profile}
                detail={props.profileDetails.find((item) => item.profile.id === profile.id)}
                binding={props.bindings.find((binding) => binding.profileId === profile.id)}
                providers={props.projectProviders}
                availableAgents={props.availableAgents}
                onSelect={() => setSelectedId(profile.id)}
              />
            ))}
          </div>
        ) : (
          <button className="resource-empty-action" type="button" onClick={() => setAdding(true)}>
            <span className="resource-empty-icon"><Bot aria-hidden="true" /></span>
            <strong>Add the first agent</strong>
            <span>Choose an available provider agent or create a new app agent.</span>
          </button>
        )}
      </main>

      {selected ? (
        <RuntimeProfileWorkbench
          project={props.project}
          profile={selected}
          providerAdapters={props.providerAdapters}
          providerConnections={props.projectProviders}
          onClose={() => setSelectedId(null)}
        />
      ) : null}
      {adding ? (
        <AddAgentDialog
          project={props.project}
          candidates={props.candidates}
          providers={props.projectProviders}
          adapters={props.providerAdapters}
          onClose={() => setAdding(false)}
        />
      ) : null}
    </div>
  );
}

function AgentCard({
  profile,
  detail,
  binding,
  providers,
  availableAgents,
  onSelect,
}: {
  profile: AgentProfile;
  detail?: AgentProfileDetail;
  binding?: AgentBinding;
  providers: ProjectProvider[];
  availableAgents: AvailableAgent[];
  onSelect: () => void;
}) {
  const active = detail?.versions.find((item) => item.version.id === profile.activeVersionId) ?? detail?.versions[0];
  const providerKey = stringValue(active?.version.source.providerKey)
    || stringValue(binding?.config.providerKey)
    || active?.capabilities[0]?.providerKey
    || binding?.provider
    || "unassigned";
  const provider = providers.find((item) => item.providerKey === providerKey);
  const gatewayOnline = binding ? availableAgents.some((agent) => (
    agent.gatewayId === binding.gatewayId
    && agent.id === binding.agentId
    && stringValue(agent.metadata.providerKey) === providerKey
    && agent.status === "connected"
  )) : false;
  const gatewayManaged = Boolean(provider && stringValue(provider.config.gatewayId));
  const availability = gatewayManaged
    ? (gatewayOnline ? { label: "Online", className: "online" } : { label: "Unavailable", className: "offline" })
    : provider?.status === "online"
      ? { label: "Online", className: "online" }
      : provider?.status === "configured"
        ? { label: "Configured", className: "configured" }
        : { label: "Unavailable", className: "offline" };
  const capabilities = active?.capabilities.map((capability) => capability.kind) ?? [];
  const prompt = stringValue(active?.version.persona.systemPrompt);
  return (
    <button className="agent-library-card" type="button" onClick={onSelect}>
      <header>
        <span className="agent-card-avatar">{initials(profile.name)}</span>
        <span className={`agent-card-status ${availability.className}`}><i />{availability.label}</span>
      </header>
      <div className="agent-card-copy">
        <strong>{profile.name}</strong>
        <p title={prompt}>{prompt ? `Prompt: ${prompt}` : profile.description || "No prompt yet. Open this agent to add one."}</p>
      </div>
      <div className="agent-card-capabilities">
        {capabilities.length > 0
          ? capabilities.slice(0, 3).map((capability) => <span key={capability}>{capabilityLabel(capability)}</span>)
          : <span>Needs abilities</span>}
      </div>
      <footer>
        <span><small>Provider</small>{provider?.name ?? providerKey}</span>
        <span><small>Live</small>{active ? `v${active.version.versionNumber}` : "Not set"}</span>
        <ChevronRight aria-hidden="true" />
      </footer>
    </button>
  );
}

function AddAgentDialog({
  project,
  candidates,
  providers,
  adapters,
  onClose,
}: {
  project: RuntimeProject;
  candidates: ProjectAgentCandidate[];
  providers: ProjectProvider[];
  adapters: ProviderAdapter[];
  onClose: () => void;
}) {
  const host = useRuntimeConsoleHost();
  const router = useRuntimeConsoleRouter();
  const dialogRef = useRef<HTMLDialogElement>(null);
  const creatableProviders = providers.filter((provider) => !adapters.find((adapter) => adapter.id === provider.providerType)?.supportsDiscovery);
  const [mode, setMode] = useState<"available" | "create">(
    candidates.length > 0 || creatableProviders.length === 0 ? "available" : "create",
  );
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState<ProjectAgentCandidate | null>(null);
  const [providerKey, setProviderKey] = useState(creatableProviders[0]?.providerKey ?? "");
  const selectedProvider = providers.find((provider) => provider.providerKey === providerKey);
  const adapter = adapters.find((item) => item.id === selectedProvider?.providerType);
  const supported = adapter?.capabilities ?? [];
  const [capability, setCapability] = useState<ProfileCapabilityKind>(supported[0] ?? "chat");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    dialogRef.current?.showModal();
  }, []);

  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return candidates.filter((candidate) => !needle || `${candidate.name} ${candidate.id} ${candidate.providerKey}`.toLowerCase().includes(needle));
  }, [candidates, query]);

  function selectProvider(nextKey: string) {
    setProviderKey(nextKey);
    const next = providers.find((provider) => provider.providerKey === nextKey);
    const nextAdapter = adapters.find((item) => item.id === next?.providerType);
    setCapability(nextAdapter?.capabilities[0] ?? "chat");
  }

  async function addAvailable() {
    if (!selected) return;
    setPending(true);
    setError(null);
    try {
      if (selected.profileId) {
        await host.request(`apps/${project.slug}/agents/${selected.profileId}/restore`, "POST", {});
      } else {
        await host.request(`apps/${project.slug}/agents/import`, "POST", {
          gatewayId: selected.gatewayId,
          agentId: selected.id,
          providerKey: selected.providerKey,
        });
      }
      router.refresh();
      dialogRef.current?.close();
    } catch (nextError) {
      setError(errorMessage(nextError));
    } finally {
      setPending(false);
    }
  }

  async function create(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const form = new FormData(event.currentTarget);
    const name = String(form.get("name") ?? "").trim();
    const description = String(form.get("description") ?? "").trim();
    const resourceId = String(form.get("resourceId") ?? "").trim();
    if (!selectedProvider || !name || !resourceId) return;
    setPending(true);
    setError(null);
    try {
      await host.request(`apps/${project.slug}/profiles`, "POST", {
        name,
        description: description || undefined,
        persona: { files: {} },
        runtime: {},
        presentation: {},
        source: {
          type: selectedProvider.providerType,
          providerKey: selectedProvider.providerKey,
          resourceId,
          managed: false,
        },
        capabilities: [{
          kind: capability,
          providerType: selectedProvider.providerType,
          providerKey: selectedProvider.providerKey,
          resourceId,
          config: {},
          inputSchema: {},
          outputSchema: {},
        }],
        changeSummary: "Created in Vifu",
      });
      router.refresh();
      dialogRef.current?.close();
    } catch (nextError) {
      setError(errorMessage(nextError));
    } finally {
      setPending(false);
    }
  }

  return (
    <dialog className="resource-dialog agent-picker-dialog" ref={dialogRef} onClose={onClose} onClick={(event) => { if (event.target === event.currentTarget) event.currentTarget.close(); }}>
      <div className={`resource-dialog-shell agent-picker-shell${creatableProviders.length === 0 ? " single-mode" : ""}`}>
        <header><div><span>Add agent</span><h2>{mode === "available" ? "Choose an agent" : "Create an agent"}</h2></div><button className="icon-button" type="button" onClick={() => dialogRef.current?.close()} aria-label="Close"><X aria-hidden="true" /></button></header>
        {creatableProviders.length > 0 ? (
          <div className="resource-mode-switch" role="tablist" aria-label="Agent source">
            <button type="button" role="tab" aria-selected={mode === "available"} className={mode === "available" ? "active" : ""} onClick={() => { setMode("available"); setError(null); }}>Available</button>
            <button type="button" role="tab" aria-selected={mode === "create"} className={mode === "create" ? "active" : ""} onClick={() => { setMode("create"); setError(null); }}>Create new</button>
          </div>
        ) : null}

        {mode === "available" ? (
          <div className="agent-picker-content">
            <label className="resource-picker-search"><Search aria-hidden="true" /><input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search agents" autoFocus /></label>
            <div className="agent-picker-list" role="listbox" aria-label="Available agents">
              {filtered.map((candidate) => (
                <button
                  type="button"
                  role="option"
                  aria-selected={selected === candidate}
                  className={selected === candidate ? "selected" : ""}
                  key={`${candidate.profileId ?? candidate.gatewayId}:${candidate.providerKey}:${candidate.id}`}
                  onClick={() => setSelected(candidate)}
                >
                  <span className="agent-card-avatar muted">{initials(candidate.name)}</span>
                  <span><strong>{candidate.name}</strong><small>{candidate.id}</small></span>
                  <span className="agent-picker-provider">{candidate.providerKey}</span>
                </button>
              ))}
              {filtered.length === 0 ? (
                <div className="resource-picker-empty">
                  {providers.length === 0
                    ? "Add a provider before adding an agent."
                    : "No agents are waiting to be added. Connected Gateways add newly detected agents automatically."}
                </div>
              ) : null}
            </div>
          </div>
        ) : creatableProviders.length > 0 ? (
          <form id="create-agent-form" className="resource-dialog-fields agent-create-fields" onSubmit={create}>
            <label><span>Name</span><input name="name" required maxLength={128} autoFocus placeholder="Town guide" /></label>
            <label><span>Role</span><textarea name="description" maxLength={4096} placeholder="What this agent does in the app" /></label>
            <label><span>Provider</span><select value={providerKey} onChange={(event) => selectProvider(event.target.value)}>{creatableProviders.map((provider) => <option key={provider.id} value={provider.providerKey}>{provider.name}</option>)}</select></label>
            <label><span>Ability</span><select value={capability} onChange={(event) => setCapability(event.target.value as ProfileCapabilityKind)}>{supported.map((item) => <option key={item} value={item}>{capabilityLabel(item)}</option>)}</select></label>
            <label><span>{resourceLabel(capability)}</span><input name="resourceId" required placeholder={resourcePlaceholder(capability)} /></label>
          </form>
        ) : (
          <div className="resource-dialog-empty"><Bot aria-hidden="true" /><strong>Add a model provider first</strong><span>Provider-managed agents appear automatically when their Gateway connects.</span></div>
        )}

        {error ? <p className="inline-error" role="alert">{error}</p> : null}
        <footer>
          <button className="secondary-button" type="button" onClick={() => dialogRef.current?.close()}>Cancel</button>
          {mode === "available"
            ? <button className="primary-button" type="button" disabled={!selected || pending} onClick={addAvailable}>{pending ? "Adding" : "Add agent"}</button>
            : creatableProviders.length > 0
              ? <button className="primary-button" type="submit" form="create-agent-form" disabled={pending}>{pending ? "Creating" : "Create agent"}</button>
              : null}
        </footer>
      </div>
    </dialog>
  );
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : "The request failed.";
}

function stringValue(value: unknown): string {
  return typeof value === "string" ? value : "";
}

function initials(name: string): string {
  return name.trim().split(/[\s_-]+/).filter(Boolean).slice(0, 2).map((part) => part[0]?.toUpperCase() ?? "").join("") || "AI";
}

function capabilityLabel(kind: string) {
  if (kind === "chat") return "Conversation";
  if (kind === "embedding") return "Embeddings";
  if (kind === "speech") return "Voice";
  if (kind === "transcription") return "Listening";
  if (kind === "realtime") return "Live voice";
  if (kind === "tool") return "Tools";
  return kind;
}

function resourceLabel(kind: ProfileCapabilityKind) {
  if (kind === "speech") return "Voice ID";
  if (kind === "tool") return "Tool set";
  return "Model";
}

function resourcePlaceholder(kind: ProfileCapabilityKind) {
  if (kind === "speech") return "voice-id";
  if (kind === "transcription") return "whisper-1";
  if (kind === "tool") return "tool-set";
  return "model-name";
}
