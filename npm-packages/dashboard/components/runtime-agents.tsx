"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { Bot, ChevronRight, Plus, Sparkles, X } from "lucide-react";
import { useMemo, useRef, useState, type FormEvent } from "react";
import type {
  AgentBinding,
  AgentProfile,
  AgentProfileDetail,
  AvailableAgent,
  ProfileCapabilityKind,
  ProjectAgentCandidate,
  ProviderAdapter,
  ProviderStockItem,
  RuntimeProject,
} from "../lib/runtime-types";
import { RuntimeProfileWorkbench } from "./runtime-profile-workbench";

type AgentsViewProps = {
  project: RuntimeProject;
  profiles: AgentProfile[];
  profileDetails: AgentProfileDetail[];
  bindings: AgentBinding[];
  availableAgents: AvailableAgent[];
  candidates: ProjectAgentCandidate[];
  providerAdapters: ProviderAdapter[];
  projectProviders: ProviderStockItem[];
};

export function RuntimeAgentsView(props: AgentsViewProps) {
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const selected = props.profiles.find((profile) => profile.id === selectedId) ?? null;
  return (
    <div className={`agents-management${selected ? " has-workbench" : ""}`}>
      <main className="agents-library">
        <section className="agents-section">
          <header className="agents-section-heading">
            <div><h2>Agents</h2><span>{props.profiles.length}</span></div>
            <CreateAgentButton
              project={props.project}
              providers={props.projectProviders}
              adapters={props.providerAdapters}
            />
          </header>
          {props.profiles.length > 0 ? (
            <div className="agent-card-grid">
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
            <div className="agents-empty-state">
              <Bot aria-hidden="true" />
              <strong>Add the first agent to this project</strong>
              <span>Choose a detected agent below or create one from an assigned provider.</span>
            </div>
          )}
        </section>

        <section className="agents-section detected-agents-section">
          <header className="agents-section-heading">
            <div><h2>Detected agents</h2><span>{props.candidates.length}</span></div>
            <p>Available from assigned Gateway providers</p>
          </header>
          {props.candidates.length > 0 ? (
            <div className="agent-card-grid detected">
              {props.candidates.map((candidate) => (
                <DetectedAgentCard project={props.project} candidate={candidate} key={`${candidate.gatewayId}:${candidate.providerKey}:${candidate.id}`} />
              ))}
            </div>
          ) : (
            <div className="detected-agents-empty">
              <span>No new agents detected.</span>
              {props.projectProviders.length === 0
                ? <Link href={`/project/${props.project.slug}/providers`}>Assign a provider</Link>
                : <small>New Gateway agents will appear here when they are online.</small>}
            </div>
          )}
        </section>
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
  providers: ProviderStockItem[];
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
  const availability = provider?.providerType === "openclaw"
    ? (gatewayOnline ? { label: "Online", className: "online" } : { label: "Unavailable", className: "offline" })
    : provider?.status === "online"
      ? { label: "Online", className: "online" }
      : provider?.status === "configured"
        ? { label: "Configured", className: "configured" }
        : { label: "Unavailable", className: "offline" };
  const capabilities = active?.capabilities.map((capability) => capability.kind) ?? [];
  return (
    <button className="agent-library-card" type="button" onClick={onSelect}>
      <header>
        <span className="agent-card-avatar">{initials(profile.name)}</span>
        <span className={`agent-card-status ${availability.className}`}><i />{availability.label}</span>
      </header>
      <div className="agent-card-copy">
        <strong>{profile.name}</strong>
        <p>{profile.description || "No role description yet."}</p>
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

function DetectedAgentCard({ project, candidate }: { project: RuntimeProject; candidate: ProjectAgentCandidate }) {
  const router = useRouter();
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function add() {
    setPending(true);
    setError(null);
    try {
      await runtimeRequest(`project/${project.slug}/agents/import`, "POST", {
        gatewayId: candidate.gatewayId,
        agentId: candidate.id,
        providerKey: candidate.providerKey,
      });
      router.refresh();
    } catch (nextError) {
      setError(errorMessage(nextError));
    } finally {
      setPending(false);
    }
  }

  return (
    <article className="detected-agent-card">
      <div className="detected-agent-card-content">
        <span className="agent-card-avatar muted">{initials(candidate.name)}</span>
        <div><strong>{candidate.name}</strong><code>{candidate.id}</code></div>
        <span className="agent-card-provider">{candidate.providerKey}</span>
      </div>
      <button type="button" disabled={pending} onClick={add} aria-label={`Add ${candidate.name} to project`} title={`Add ${candidate.name}`}>
        <Plus aria-hidden="true" />
      </button>
      {error ? <span className="detected-agent-error" role="alert">{error}</span> : null}
    </article>
  );
}

function CreateAgentButton({
  project,
  providers,
  adapters,
}: {
  project: RuntimeProject;
  providers: ProviderStockItem[];
  adapters: ProviderAdapter[];
}) {
  const router = useRouter();
  const dialogRef = useRef<HTMLDialogElement>(null);
  const directProviders = providers.filter((provider) => provider.providerType !== "openclaw");
  const [providerKey, setProviderKey] = useState(directProviders[0]?.providerKey ?? "");
  const selectedProvider = providers.find((provider) => provider.providerKey === providerKey);
  const adapter = adapters.find((item) => item.id === selectedProvider?.providerType);
  const supported = adapter?.capabilities ?? [];
  const [capability, setCapability] = useState<ProfileCapabilityKind>(supported[0] ?? "chat");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  function selectProvider(nextKey: string) {
    setProviderKey(nextKey);
    const next = providers.find((provider) => provider.providerKey === nextKey);
    const nextAdapter = adapters.find((item) => item.id === next?.providerType);
    setCapability(nextAdapter?.capabilities[0] ?? "chat");
  }

  async function create(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const formElement = event.currentTarget;
    const form = new FormData(formElement);
    const name = String(form.get("name") ?? "").trim();
    const description = String(form.get("description") ?? "").trim();
    const resourceId = String(form.get("resourceId") ?? "").trim();
    if (!selectedProvider || !name || !resourceId) return;
    setPending(true);
    setError(null);
    try {
      await runtimeRequest(`project/${project.slug}/profiles`, "POST", {
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
      dialogRef.current?.close();
      formElement.reset();
      router.refresh();
    } catch (nextError) {
      setError(errorMessage(nextError));
    } finally {
      setPending(false);
    }
  }

  return (
    <>
      <button className="primary-button compact" type="button" onClick={() => dialogRef.current?.showModal()}><Plus aria-hidden="true" />Add agent</button>
      <dialog className="resource-dialog" ref={dialogRef} onClick={(event) => { if (event.target === event.currentTarget) event.currentTarget.close(); }}>
        <form className="resource-dialog-shell" onSubmit={create}>
          <header><div><span>New agent</span><h2>Add an agent</h2></div><button className="icon-button" type="button" onClick={() => dialogRef.current?.close()} aria-label="Close"><X aria-hidden="true" /></button></header>
          {directProviders.length > 0 ? (
            <div className="resource-dialog-fields">
              <label><span>Name</span><input name="name" required maxLength={128} autoFocus placeholder="Town guide" /></label>
              <label><span>Role</span><textarea name="description" maxLength={4096} placeholder="How this agent appears in the game" /></label>
              <label><span>Provider</span><select value={providerKey} onChange={(event) => selectProvider(event.target.value)}>{directProviders.map((provider) => <option key={provider.id} value={provider.providerKey}>{provider.name}</option>)}</select></label>
              <label><span>Ability</span><select value={capability} onChange={(event) => setCapability(event.target.value as ProfileCapabilityKind)}>{supported.map((item) => <option key={item} value={item}>{capabilityLabel(item)}</option>)}</select></label>
              <label><span>{resourceLabel(capability)}</span><input name="resourceId" required placeholder={resourcePlaceholder(capability)} /></label>
            </div>
          ) : (
            <div className="resource-dialog-empty"><Sparkles aria-hidden="true" /><strong>Assign a model provider first</strong><span>Gateway agents are added from the Detected agents section.</span><Link href={`/project/${project.slug}/providers`} onClick={() => dialogRef.current?.close()}>Open Providers</Link></div>
          )}
          {error ? <p className="inline-error" role="alert">{error}</p> : null}
          <footer><button className="secondary-button" type="button" onClick={() => dialogRef.current?.close()}>Cancel</button>{directProviders.length > 0 ? <button className="primary-button" type="submit" disabled={pending}>{pending ? "Adding" : "Add agent"}</button> : null}</footer>
        </form>
      </dialog>
    </>
  );
}

async function runtimeRequest<T = unknown>(path: string, method: string, body?: unknown): Promise<T> {
  const response = await fetch(`/api/runtime/${path}`, {
    method,
    headers: body === undefined ? undefined : { "content-type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const payload = await response.json().catch(() => null) as T | { error?: { message?: string } | string } | null;
  if (!response.ok) throw new Error(readError(payload));
  return (payload ?? {}) as T;
}

function readError(payload: unknown): string {
  if (!payload || typeof payload !== "object") return "The request failed.";
  const error = (payload as { error?: unknown }).error;
  if (typeof error === "string") return error;
  if (error && typeof error === "object" && typeof (error as { message?: unknown }).message === "string") return String((error as { message: string }).message);
  return "The request failed.";
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

function capabilityLabel(kind: string): string {
  if (kind === "chat") return "Conversation";
  if (kind === "speech") return "Voice";
  if (kind === "transcription") return "Listening";
  if (kind === "realtime") return "Live voice";
  if (kind === "tool") return "Tools";
  return kind;
}

function resourceLabel(kind: ProfileCapabilityKind): string {
  if (kind === "speech") return "Voice ID";
  if (kind === "tool") return "Tool set";
  return "Model";
}

function resourcePlaceholder(kind: ProfileCapabilityKind): string {
  if (kind === "speech") return "voice-id";
  if (kind === "transcription") return "whisper-1";
  if (kind === "tool") return "tool-set";
  return "model-name";
}
