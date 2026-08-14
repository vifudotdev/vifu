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
import { useEffect, useMemo, useRef, useState, type FormEvent } from "react";
import { useRuntimeConsoleHost, useRuntimeConsoleRouter } from "../host";
import type {
  AvailableAgent,
  CustomProvider,
  ProjectProvider,
  ProviderAdapter,
  ProviderCatalog,
  ProviderAdapterField,
  RuntimeProject,
} from "../types";
import { providerSettingsRequestBody } from "../provider-request";
import { RuntimeConfirmDialog } from "./runtime-confirm-dialog";

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
          <span>Connections available to agents in this app.</span>
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
              adapter={catalog.registry.find((adapter) => adapter.id === provider.providerType) ?? fallbackAdapter(provider.providerType)}
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
  const host = useRuntimeConsoleHost();
  const router = useRuntimeConsoleRouter();
  const [pending, setPending] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function remove() {
    setPending(true);
    setError(null);
    try {
      await host.request(`apps/${project.slug}/providers/${provider.providerKey}`, "DELETE");
      router.refresh();
    } catch (nextError) {
      setError(errorMessage(nextError));
    } finally {
      setPending(false);
      setConfirming(false);
    }
  }

  const status = online ? "Online" : provider.status === "configured" ? "Configured" : titleCase(provider.status);
  const configuration = providerConfigurationSummary(provider);
  return (
    <article className="provider-resource-card project-provider-card">
      <button className="provider-resource-main" type="button" onClick={onConfigure}>
        <ProviderMark type={provider.providerType} />
        <div>
          <strong>{provider.name}</strong>
          <p>{configuration ?? adapter?.description ?? provider.baseUrl}</p>
        </div>
        <span className={`provider-health ${online ? "online" : provider.status === "configured" ? "configured" : "offline"}`}>
          <i />{status}
        </span>
      </button>
      <footer>
        <span>{capabilitySummary(providerCapabilities(provider, adapter?.capabilities ?? []))}</span>
        <button className="icon-button danger" type="button" disabled={pending} onClick={() => setConfirming(true)} title="Remove provider" aria-label={`Remove ${provider.name}`}>
          <Trash2 aria-hidden="true" />
        </button>
      </footer>
      {error ? <p className="provider-card-error" role="alert">{error}</p> : null}
      {confirming ? (
        <RuntimeConfirmDialog
          title="Remove provider?"
          description={`${provider.name} will be disconnected from ${project.name}. Remove or move any agents using it first.`}
          confirmLabel="Remove provider"
          pending={pending}
          onCancel={() => setConfirming(false)}
          onConfirm={remove}
        />
      ) : null}
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
  const host = useRuntimeConsoleHost();
  const router = useRuntimeConsoleRouter();
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
    setPending(true);
    setError(null);
    setNotice(null);
    try {
      const body = providerSettingsRequestBody(provider, choice, form);
      const result = await host.request<{ message?: string; addedAgents?: number }>(
        provider ? `apps/${project.slug}/providers/${provider.providerKey}` : `apps/${project.slug}/providers`,
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
      const result = await host.request<{ message?: string; addedAgents?: number }>(
        `apps/${project.slug}/providers/${provider.providerKey}/test`,
        "POST",
        {},
      );
      setNotice(result.message ?? `${provider.name} is reachable.`);
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
              <ProviderMark type={choice.providerType} />
              <div>
                <span>{provider ? "Provider settings" : "Add provider"}</span>
                <h2>{choice.name}</h2>
                <p>{choice.description}</p>
              </div>
            </div>
            <button className="icon-button" type="button" onClick={() => dialogRef.current?.close()} aria-label="Close"><X aria-hidden="true" /></button>
          </header>
          <div className="resource-dialog-fields">
            <label><span>Name</span><input name="name" required maxLength={128} defaultValue={provider?.name ?? choice.name} autoFocus /></label>
            {!provider && choice.source.kind === "custom" ? (
              <p>This provider is already available from the connected runtime. Adding it only makes it available to this app.</p>
            ) : choice.fields.map((field) => (
              <label key={field.key}><span>{field.label}</span>{field.kind === "json" ? (
                <textarea
                  name={field.key}
                  required={field.required}
                  defaultValue={providerJsonFieldValue(provider?.config[field.key] ?? choice.config[field.key])}
                  spellCheck={false}
                />
              ) : (
                <input
                  name={field.key}
                  type={field.secret ? "password" : field.kind === "url" ? "url" : "text"}
                  required={field.required && !(field.secret && (provider?.secretKeys.includes(field.key) || choice.secretKeys.includes(field.key)))}
                  defaultValue={field.secret ? "" : field.key === "baseUrl" ? provider?.baseUrl ?? choice.baseUrl : stringValue(provider?.config[field.key] ?? choice.config[field.key])}
                  placeholder={field.secret && (provider?.secretKeys.includes(field.key) || choice.secretKeys.includes(field.key)) ? "Leave blank to keep the configured value" : undefined}
                  autoComplete={field.secret ? "off" : undefined}
                />
              )}</label>
            ))}
          </div>
          {error ? <p className="inline-error" role="alert">{error}</p> : null}
          {notice ? <p className="inline-success" role="status">{notice}</p> : null}
          <footer className="provider-dialog-actions">
            {provider ? <button className="secondary-button provider-test-button" type="button" disabled={testing || pending} onClick={test}><RefreshCw aria-hidden="true" />{testing ? "Testing" : "Test connection"}</button> : <span />}
            <span><button className="secondary-button" type="button" onClick={() => dialogRef.current?.close()}>Cancel</button><button className="primary-button" type="submit" disabled={pending}>{pending ? "Saving" : provider ? "Save changes" : "Add provider"}</button></span>
          </footer>
        </form>
      ) : (
        <div className="resource-dialog-shell provider-picker-shell">
          <header><div><span>Add provider</span><h2>Choose a provider</h2></div><button className="icon-button" type="button" onClick={() => dialogRef.current?.close()} aria-label="Close"><X aria-hidden="true" /></button></header>
          <label className="resource-picker-search"><Search aria-hidden="true" /><input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search providers" autoFocus /></label>
          <div className="provider-picker-list">
            <ProviderChoiceGroup title="Provider templates" items={filtered.filter((item) => item.source.kind === "registry")} onSelect={setChoice} />
            <ProviderChoiceGroup title="Available providers" items={filtered.filter((item) => item.source.kind === "custom")} onSelect={setChoice} />
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
    description: `Available ${adapter.name} provider`,
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
  if (type === "vifu-runtime") {
    return {
      id: type,
      category: "local",
      name: "Vifu Runtime",
      description: "Provider reported by a connected Vifu Agent Gateway. Put maxTokens, temperature, or topP under settings.generation to define its model-call defaults.",
      capabilities: ["chat", "embedding", "transcription"],
      executionModes: ["gateway"],
      supportsDiscovery: false,
      fields: [
        { key: "settings", label: "Runtime settings (JSON)", kind: "json", required: true, secret: false },
        { key: "resources", label: "Runtime resources (JSON)", kind: "json", required: true, secret: false },
      ],
    };
  }
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
  const gatewayId = stringValue(provider.config.gatewayId);
  if (gatewayId) {
    return agents.some((agent) => (
      agent.gatewayId === gatewayId
      && agent.status === "connected"
      && stringValue(agent.metadata.providerKey) === provider.providerKey
    ));
  }
  return provider.status === "online";
}

function capabilitySummary(capabilities: string[]): string {
  if (capabilities.length === 0) return "Custom provider";
  return capabilities.slice(0, 3).map((value) => value === "chat" ? "Conversation" : titleCase(value)).join(" · ");
}

function providerCapabilities(provider: ProjectProvider, fallback: string[]): string[] {
  const declared = Array.isArray(provider.config.capabilities)
    ? provider.config.capabilities.filter((value): value is string => typeof value === "string")
    : [];
  return declared.length > 0 ? declared : fallback;
}

function providerConfigurationSummary(provider: ProjectProvider): string | null {
  const settings = recordValue(provider.config.settings);
  const model = stringValue(settings?.model);
  if (model) return `Model ${model}`;
  const sources = Array.isArray(settings?.sources)
    ? settings.sources.filter((value): value is string => typeof value === "string")
    : [];
  if (sources.length > 0) return sources.join(" · ");
  const runtime = stringValue(provider.config.runtimeProviderType);
  return runtime ? `${titleCase(runtime)} runtime` : null;
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : "The request failed.";
}

function stringValue(value: unknown): string {
  return typeof value === "string" ? value : "";
}

function recordValue(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

function providerJsonFieldValue(value: unknown): string {
  return JSON.stringify(value && typeof value === "object" && !Array.isArray(value) ? value : {}, null, 2);
}

function titleCase(value: string): string {
  return value.replace(/[-_]/g, " ").replace(/\b\w/g, (letter) => letter.toUpperCase());
}
