"use client";

import {
  ArrowLeft,
  Bot,
  Cloud,
  Cpu,
  Mic2,
  Plus,
  RefreshCw,
  Search,
  Trash2,
  Volume2,
  X,
  type LucideIcon,
} from "lucide-react";
import { useRouter } from "next/navigation";
import { useEffect, useMemo, useRef, useState, type FormEvent } from "react";
import type {
  AvailableAgent,
  CustomProvider,
  ProjectProvider,
  ProviderAdapter,
  ProviderCatalog,
  ProviderAdapterField,
  RuntimeProject,
} from "../lib/runtime-types";

type ProvidersViewProps = {
  project: RuntimeProject;
  catalog: ProviderCatalog;
  providers: ProjectProvider[];
  availableAgents: AvailableAgent[];
};

type ProviderChoice = {
  source: { kind: "registry" | "custom"; key: string };
  name: string;
  providerType: string;
  description: string;
  capabilities: string[];
  fields: ProviderAdapterField[];
  baseUrl: string;
  config: Record<string, unknown>;
  secretKeys: string[];
};

export function RuntimeProvidersView({ project, catalog, providers, availableAgents }: ProvidersViewProps) {
  const [dialog, setDialog] = useState<{ provider?: ProjectProvider } | null>(null);
  return (
    <div className="providers-page providers-page-simple">
      <header className="resource-page-heading">
        <div className="resource-page-summary">
          <strong>{providers.length} {providers.length === 1 ? "provider" : "providers"}</strong>
          <span>Connections available to agents in this project.</span>
        </div>
        <button className="primary-button compact" type="button" onClick={() => setDialog({})}>
          <Plus aria-hidden="true" />Add provider
        </button>
      </header>

      {providers.length > 0 ? (
        <div className="provider-card-grid project-provider-grid">
          {providers.map((provider) => (
            <ProjectProviderCard
              key={provider.id}
              project={project}
              provider={provider}
              adapter={catalog.registry.find((adapter) => adapter.id === provider.providerType)}
              online={providerOnline(provider, availableAgents)}
              onConfigure={() => setDialog({ provider })}
            />
          ))}
        </div>
      ) : (
        <button className="resource-empty-action" type="button" onClick={() => setDialog({})}>
          <span className="resource-empty-icon"><Plus aria-hidden="true" /></span>
          <strong>Add your first provider</strong>
          <span>Connect an agent runtime, model, voice, or transcription provider.</span>
        </button>
      )}

      {dialog ? (
        <ProviderDialog
          project={project}
          catalog={catalog}
          providers={providers}
          provider={dialog.provider}
          onClose={() => setDialog(null)}
        />
      ) : null}
    </div>
  );
}

function ProjectProviderCard({
  project,
  provider,
  adapter,
  online,
  onConfigure,
}: {
  project: RuntimeProject;
  provider: ProjectProvider;
  adapter?: ProviderAdapter;
  online: boolean;
  onConfigure: () => void;
}) {
  const router = useRouter();
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function remove() {
    if (!window.confirm(`Remove ${provider.name} from ${project.name}?`)) return;
    setPending(true);
    setError(null);
    try {
      await runtimeRequest(`project/${project.slug}/providers/${provider.providerKey}`, "DELETE");
      router.refresh();
    } catch (nextError) {
      setError(errorMessage(nextError));
    } finally {
      setPending(false);
    }
  }

  const status = online ? "Online" : provider.status === "configured" ? "Configured" : titleCase(provider.status);
  return (
    <article className="provider-resource-card project-provider-card">
      <button className="provider-resource-main" type="button" onClick={onConfigure}>
        <ProviderMark type={provider.providerType} />
        <div>
          <strong>{provider.name}</strong>
          <p>{adapter?.description ?? provider.baseUrl}</p>
        </div>
        <span className={`provider-health ${online ? "online" : provider.status === "configured" ? "configured" : "offline"}`}>
          <i />{status}
        </span>
      </button>
      <footer>
        <span>{capabilitySummary(adapter?.capabilities ?? [])}</span>
        <button className="icon-button danger" type="button" disabled={pending} onClick={remove} title="Remove provider" aria-label={`Remove ${provider.name}`}>
          <Trash2 aria-hidden="true" />
        </button>
      </footer>
      {error ? <p className="provider-card-error" role="alert">{error}</p> : null}
    </article>
  );
}

function ProviderDialog({
  project,
  catalog,
  providers,
  provider,
  onClose,
}: {
  project: RuntimeProject;
  catalog: ProviderCatalog;
  providers: ProjectProvider[];
  provider?: ProjectProvider;
  onClose: () => void;
}) {
  const router = useRouter();
  const dialogRef = useRef<HTMLDialogElement>(null);
  const choices = useMemo(() => catalogChoices(catalog).filter((item) => (
    item.source.kind === "registry"
      || !providers.some((current) => current.sourceKind === "custom" && current.sourceKey === item.source.key)
  )), [catalog, providers]);
  const initial = provider ? choiceForProvider(provider, catalog) : null;
  const [choice, setChoice] = useState<ProviderChoice | null>(initial);
  const [query, setQuery] = useState("");
  const [pending, setPending] = useState(false);
  const [testing, setTesting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  useEffect(() => {
    dialogRef.current?.showModal();
  }, []);

  const filtered = choices.filter((item) => {
    const needle = query.trim().toLowerCase();
    return !needle || `${item.name} ${item.providerType} ${item.description}`.toLowerCase().includes(needle);
  });

  async function save(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!choice) return;
    const form = new FormData(event.currentTarget);
    const config: Record<string, unknown> = {};
    const secrets: Record<string, string> = {};
    for (const field of choice.fields) {
      if (field.key === "baseUrl") continue;
      const value = String(form.get(field.key) ?? "").trim();
      if (!value) continue;
      if (field.secret) secrets[field.key] = value;
      else config[field.key] = value;
    }
    const body = {
      ...(provider ? {} : { source: choice.source }),
      name: String(form.get("name") ?? choice.name).trim(),
      baseUrl: String(form.get("baseUrl") ?? choice.baseUrl).trim(),
      config,
      secrets,
    };
    setPending(true);
    setError(null);
    setNotice(null);
    try {
      const result = await runtimeRequest<{ message?: string; addedAgents?: number }>(
        provider ? `project/${project.slug}/providers/${provider.providerKey}` : `project/${project.slug}/providers`,
        provider ? "PATCH" : "POST",
        body,
      );
      const discovered = result.addedAgents ? ` ${result.addedAgents} agents added.` : "";
      if (result.message) setNotice(`${result.message}.${discovered}`);
      router.refresh();
      dialogRef.current?.close();
    } catch (nextError) {
      setError(errorMessage(nextError));
    } finally {
      setPending(false);
    }
  }

  async function test() {
    if (!provider) return;
    setTesting(true);
    setError(null);
    setNotice(null);
    try {
      const result = await runtimeRequest<{ message?: string; addedAgents?: number }>(
        `project/${project.slug}/providers/${provider.providerKey}/test`,
        "POST",
        {},
      );
      setNotice(result.message ?? "Connection test complete.");
      router.refresh();
    } catch (nextError) {
      setError(errorMessage(nextError));
    } finally {
      setTesting(false);
    }
  }

  return (
    <dialog className="resource-dialog provider-config-dialog provider-picker-dialog" ref={dialogRef} onClose={onClose} onClick={(event) => { if (event.target === event.currentTarget) event.currentTarget.close(); }}>
      {choice ? (
        <form className="resource-dialog-shell" onSubmit={save}>
          <header>
            <div className="dialog-title-with-back">
              {!provider ? <button className="icon-button" type="button" onClick={() => { setChoice(null); setError(null); }} aria-label="Back to providers"><ArrowLeft aria-hidden="true" /></button> : null}
              <div><span>{provider ? "Provider settings" : "Add provider"}</span><h2>{choice.name}</h2></div>
            </div>
            <button className="icon-button" type="button" onClick={() => dialogRef.current?.close()} aria-label="Close"><X aria-hidden="true" /></button>
          </header>
          <div className="provider-dialog-identity"><ProviderMark type={choice.providerType} /><div><strong>{choice.name}</strong><p>{choice.description}</p></div></div>
          <div className="resource-dialog-fields">
            <label><span>Name</span><input name="name" required maxLength={128} defaultValue={provider?.name ?? choice.name} autoFocus /></label>
            {choice.fields.map((field) => (
              <label key={field.key}><span>{field.label}</span><input
                name={field.key}
                type={field.secret ? "password" : field.kind === "url" ? "url" : "text"}
                required={field.required && !(field.secret && (provider?.secretKeys.includes(field.key) || choice.secretKeys.includes(field.key)))}
                defaultValue={field.secret ? "" : field.key === "baseUrl" ? provider?.baseUrl ?? choice.baseUrl : stringValue(provider?.config[field.key] ?? choice.config[field.key])}
                placeholder={field.secret && (provider?.secretKeys.includes(field.key) || choice.secretKeys.includes(field.key)) ? "Leave blank to keep the configured value" : undefined}
                autoComplete={field.secret ? "off" : undefined}
              /></label>
            ))}
          </div>
          {error ? <p className="inline-error" role="alert">{error}</p> : null}
          {notice ? <p className="inline-success" role="status">{notice}</p> : null}
          <footer>
            <span>{provider ? <button className="secondary-button" type="button" disabled={testing || pending} onClick={test}><RefreshCw aria-hidden="true" />{testing ? "Testing" : "Test connection"}</button> : null}</span>
            <span><button className="secondary-button" type="button" onClick={() => dialogRef.current?.close()}>Cancel</button><button className="primary-button" type="submit" disabled={pending}>{pending ? "Saving" : provider ? "Save changes" : "Add provider"}</button></span>
          </footer>
        </form>
      ) : (
        <div className="resource-dialog-shell provider-picker-shell">
          <header><div><span>Add provider</span><h2>Choose a provider</h2></div><button className="icon-button" type="button" onClick={() => dialogRef.current?.close()} aria-label="Close"><X aria-hidden="true" /></button></header>
          <label className="resource-picker-search"><Search aria-hidden="true" /><input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search providers" autoFocus /></label>
          <div className="provider-picker-list">
            <ProviderChoiceGroup title="Vifu Registry" items={filtered.filter((item) => item.source.kind === "registry")} onSelect={setChoice} />
            <ProviderChoiceGroup title="Custom Providers" items={filtered.filter((item) => item.source.kind === "custom")} onSelect={setChoice} />
            {filtered.length === 0 ? <div className="resource-picker-empty">No providers match your search.</div> : null}
          </div>
          <footer><span /><button className="secondary-button" type="button" onClick={() => dialogRef.current?.close()}>Cancel</button></footer>
        </div>
      )}
    </dialog>
  );
}

function ProviderChoiceGroup({ title, items, onSelect }: { title: string; items: ProviderChoice[]; onSelect: (choice: ProviderChoice) => void }) {
  if (items.length === 0) return null;
  return (
    <section className="provider-choice-group">
      <h3>{title}</h3>
      {items.map((item) => (
        <button type="button" key={`${item.source.kind}:${item.source.key}`} onClick={() => onSelect(item)}>
          <ProviderMark type={item.providerType} />
          <span><strong>{item.name}</strong><small>{item.description}</small></span>
          <Plus aria-hidden="true" />
        </button>
      ))}
    </section>
  );
}

function catalogChoices(catalog: ProviderCatalog): ProviderChoice[] {
  const registry = catalog.registry.map((adapter) => ({
    source: { kind: "registry" as const, key: adapter.id },
    name: adapter.name,
    providerType: adapter.id,
    description: adapter.description,
    capabilities: adapter.capabilities,
    fields: adapter.fields,
    baseUrl: "",
    config: {},
    secretKeys: [],
  }));
  const custom = catalog.custom.map((provider) => customChoice(provider, catalog.registry));
  return [...registry, ...custom];
}

function customChoice(provider: CustomProvider, adapters: ProviderAdapter[]): ProviderChoice {
  const adapter = adapters.find((item) => item.id === provider.providerType) ?? fallbackAdapter(provider.providerType);
  return {
    source: { kind: "custom", key: provider.providerKey },
    name: provider.name,
    providerType: provider.providerType,
    description: `Custom ${adapter.name} configuration`,
    capabilities: adapter.capabilities,
    fields: adapter.fields,
    baseUrl: provider.baseUrl,
    config: provider.config,
    secretKeys: provider.secretKeys,
  };
}

function choiceForProvider(provider: ProjectProvider, catalog: ProviderCatalog): ProviderChoice {
  if (provider.sourceKind === "custom") {
    const source = catalog.custom.find((item) => item.providerKey === provider.sourceKey);
    if (source) return customChoice(source, catalog.registry);
  }
  const adapter = catalog.registry.find((item) => item.id === provider.providerType) ?? fallbackAdapter(provider.providerType);
  return {
    source: { kind: provider.sourceKind, key: provider.sourceKey },
    name: provider.name,
    providerType: provider.providerType,
    description: adapter.description,
    capabilities: adapter.capabilities,
    fields: adapter.fields,
    baseUrl: provider.baseUrl,
    config: provider.config,
    secretKeys: provider.secretKeys,
  };
}

function fallbackAdapter(type: string): ProviderAdapter {
  return {
    id: type,
    category: "custom",
    name: titleCase(type),
    description: "Custom provider configuration.",
    capabilities: ["chat"],
    executionModes: ["server"],
    supportsDiscovery: false,
    fields: [
      { key: "baseUrl", label: "Base URL", kind: "url", required: true, secret: false },
      { key: "token", label: "API key", kind: "password", required: false, secret: true },
    ],
  };
}

function ProviderMark({ type }: { type: string }) {
  const Icon = providerIcon(type);
  return <span className={`provider-mark ${providerTone(type)}`}><Icon aria-hidden="true" /></span>;
}

function providerIcon(type: string): LucideIcon {
  if (type === "openclaw") return Bot;
  if (type === "elevenlabs") return Volume2;
  if (type === "local-whisper") return Mic2;
  if (type === "openai-compatible") return Cloud;
  return Cpu;
}

function providerTone(type: string): string {
  if (type === "openclaw") return "coral";
  if (type === "elevenlabs") return "mint";
  if (type === "local-whisper") return "amber";
  return "blue";
}

function providerOnline(provider: ProjectProvider, agents: AvailableAgent[]): boolean {
  if (provider.providerType === "openclaw") {
    return agents.some((agent) => agent.status === "connected" && stringValue(agent.metadata.providerKey) === provider.providerKey);
  }
  return provider.status === "online";
}

function capabilitySummary(capabilities: string[]): string {
  if (capabilities.length === 0) return "Custom provider";
  return capabilities.slice(0, 3).map((value) => value === "chat" ? "Conversation" : titleCase(value)).join(" · ");
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

function titleCase(value: string): string {
  return value.replace(/[-_]/g, " ").replace(/\b\w/g, (letter) => letter.toUpperCase());
}
