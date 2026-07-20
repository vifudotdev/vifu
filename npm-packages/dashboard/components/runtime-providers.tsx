"use client";

import {
  Bot,
  Cloud,
  Cpu,
  Mic2,
  MoreHorizontal,
  Plus,
  RefreshCw,
  Trash2,
  Volume2,
  X,
  type LucideIcon,
} from "lucide-react";
import { useRouter } from "next/navigation";
import { useEffect, useMemo, useRef, useState, type FormEvent } from "react";
import type {
  AvailableAgent,
  ProviderAdapter,
  ProviderStockItem,
  RuntimeProject,
} from "../lib/runtime-types";

type ProvidersViewProps = {
  project: RuntimeProject;
  adapters: ProviderAdapter[];
  stock: ProviderStockItem[];
  assigned: ProviderStockItem[];
  availableAgents: AvailableAgent[];
};

type ConfigureTarget = {
  adapter: ProviderAdapter;
  provider?: ProviderStockItem;
} | null;

export function RuntimeProvidersView({ project, adapters, stock, assigned, availableAgents }: ProvidersViewProps) {
  const assignedKeys = useMemo(() => new Set(assigned.map((provider) => provider.providerKey)), [assigned]);
  const [configure, setConfigure] = useState<ConfigureTarget>(null);
  return (
    <div className="providers-page">
      <ProviderSection title="Project providers" count={assigned.length} description="Providers this project can use for agents and abilities.">
        {assigned.length > 0 ? (
          <div className="provider-card-grid">
            {assigned.map((provider) => (
              <ProjectProviderCard
                key={provider.id}
                project={project}
                provider={provider}
                adapter={adapters.find((adapter) => adapter.id === provider.providerType)}
                online={providerOnline(provider, availableAgents)}
                onConfigure={() => setConfigure({ adapter: adapterFor(provider, adapters), provider })}
              />
            ))}
          </div>
        ) : <ProviderEmpty title="No providers assigned" detail="Add a configured provider from Provider Stock below." />}
      </ProviderSection>

      <ProviderSection title="Provider Stock" count={stock.length} description="Your configured providers, shared across projects on this Vifu deployment.">
        {stock.length > 0 ? (
          <div className="provider-card-grid">
            {stock.map((provider) => (
              <StockProviderCard
                key={provider.id}
                project={project}
                provider={provider}
                adapter={adapters.find((adapter) => adapter.id === provider.providerType)}
                assigned={assignedKeys.has(provider.providerKey)}
                online={providerOnline(provider, availableAgents)}
                onConfigure={() => setConfigure({ adapter: adapterFor(provider, adapters), provider })}
              />
            ))}
          </div>
        ) : <ProviderEmpty title="Provider Stock is empty" detail="Configure a provider from the registry to make it available to projects." />}
      </ProviderSection>

      <ProviderSection title="Provider Registry" count={adapters.length} description="Supported provider templates for local agents, models, voice, and realtime abilities.">
        <div className="provider-card-grid registry">
          {adapters.map((adapter) => (
            <button className="provider-registry-card" type="button" key={adapter.id} onClick={() => setConfigure({ adapter })}>
              <ProviderMark type={adapter.id} />
              <div><strong>{adapter.name}</strong><p>{adapter.description}</p></div>
              <footer>{adapter.capabilities.map((capability) => <span key={capability}>{capabilityLabel(capability)}</span>)}</footer>
              <Plus aria-hidden="true" />
            </button>
          ))}
        </div>
      </ProviderSection>

      {configure ? (
        <ProviderDialog
          target={configure}
          stock={stock}
          onClose={() => setConfigure(null)}
        />
      ) : null}
    </div>
  );
}

function ProviderSection({
  title,
  count,
  description,
  children,
}: {
  title: string;
  count: number;
  description: string;
  children: React.ReactNode;
}) {
  return (
    <section className="providers-section">
      <header><div><h2>{title}</h2><span>{count}</span></div><p>{description}</p></header>
      {children}
    </section>
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
  provider: ProviderStockItem;
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

  return (
    <article className="provider-resource-card">
      <button className="provider-resource-main" type="button" onClick={onConfigure}>
        <ProviderMark type={provider.providerType} />
        <div><strong>{provider.name}</strong><code>{provider.providerKey}</code><p>{adapter?.description ?? provider.baseUrl}</p></div>
        <span className={`provider-health ${online ? "online" : "offline"}`}><i />{online ? "Online" : provider.status}</span>
      </button>
      <footer>
        <span>{provider.secretKeys.length > 0 ? "Credentials configured" : "No credentials"}</span>
        <button className="icon-text-button" type="button" disabled={pending} onClick={remove}><X aria-hidden="true" />Remove</button>
      </footer>
      {error ? <p className="provider-card-error" role="alert">{error}</p> : null}
    </article>
  );
}

function StockProviderCard({
  project,
  provider,
  adapter,
  assigned,
  online,
  onConfigure,
}: {
  project: RuntimeProject;
  provider: ProviderStockItem;
  adapter?: ProviderAdapter;
  assigned: boolean;
  online: boolean;
  onConfigure: () => void;
}) {
  const router = useRouter();
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function assign() {
    setPending(true);
    setError(null);
    try {
      await runtimeRequest(`project/${project.slug}/providers/${provider.providerKey}`, "PUT", {});
      router.refresh();
    } catch (nextError) {
      setError(errorMessage(nextError));
    } finally {
      setPending(false);
    }
  }

  async function removeFromStock() {
    if (!window.confirm(`Delete ${provider.name} from Provider Stock?`)) return;
    setPending(true);
    setError(null);
    try {
      await runtimeRequest(`providers/${provider.providerKey}`, "DELETE");
      router.refresh();
    } catch (nextError) {
      setError(errorMessage(nextError));
    } finally {
      setPending(false);
    }
  }

  return (
    <article className={`provider-resource-card stock${assigned ? " assigned" : ""}`}>
      <button className="provider-resource-main" type="button" onClick={onConfigure}>
        <ProviderMark type={provider.providerType} />
        <div><strong>{provider.name}</strong><code>{provider.providerKey}</code><p>{adapter?.description ?? provider.baseUrl}</p></div>
        <span className={`provider-health ${online ? "online" : "offline"}`}><i />{online ? "Online" : provider.status}</span>
      </button>
      <footer>
        <span>{assigned ? `Assigned to ${project.name}` : "Available to projects"}</span>
        <span className="provider-stock-actions">
          {!assigned ? <button className="provider-assign-button" type="button" disabled={pending} onClick={assign} title={`Add ${provider.name} to project`} aria-label={`Add ${provider.name} to project`}><Plus aria-hidden="true" /></button> : null}
          {!assigned ? <button className="icon-button danger" type="button" disabled={pending} onClick={removeFromStock} title="Delete from Provider Stock" aria-label="Delete from Provider Stock"><Trash2 aria-hidden="true" /></button> : null}
        </span>
      </footer>
      {error ? <p className="provider-card-error" role="alert">{error}</p> : null}
    </article>
  );
}

function ProviderDialog({ target, stock, onClose }: { target: NonNullable<ConfigureTarget>; stock: ProviderStockItem[]; onClose: () => void }) {
  const router = useRouter();
  const dialogRef = useRef<HTMLDialogElement>(null);
  const [pending, setPending] = useState(false);
  const [testing, setTesting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const providerKey = target.provider?.providerKey ?? uniqueProviderKey(target.adapter.id, stock);

  useEffect(() => {
    dialogRef.current?.showModal();
  }, []);

  async function save(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const form = new FormData(event.currentTarget);
    const config: Record<string, unknown> = { ...(target.provider?.config ?? {}) };
    const secrets: Record<string, string> = {};
    for (const field of target.adapter.fields) {
      const fieldValue = String(form.get(field.key) ?? "").trim();
      if (!fieldValue || field.key === "baseUrl") continue;
      if (field.secret) secrets[field.key] = fieldValue;
      else config[field.key] = fieldValue;
    }
    const key = target.provider?.providerKey ?? String(form.get("providerKey") ?? providerKey).trim();
    const baseUrl = String(form.get("baseUrl") ?? target.provider?.baseUrl ?? `local://${target.adapter.id}`).trim();
    setPending(true);
    setError(null);
    try {
      await runtimeRequest(`providers/${key}`, "PUT", {
        name: String(form.get("name") ?? target.adapter.name).trim(),
        providerType: target.adapter.id,
        baseUrl,
        config,
        secrets,
      });
      router.refresh();
      onClose();
    } catch (nextError) {
      setError(errorMessage(nextError));
    } finally {
      setPending(false);
    }
  }

  async function test() {
    if (!target.provider) return;
    setTesting(true);
    setError(null);
    try {
      await runtimeRequest(`providers/${target.provider.providerKey}/test`, "POST", {});
      router.refresh();
    } catch (nextError) {
      setError(errorMessage(nextError));
    } finally {
      setTesting(false);
    }
  }

  return (
    <dialog className="resource-dialog provider-config-dialog" ref={dialogRef} onClose={onClose} onClick={(event) => { if (event.target === event.currentTarget) event.currentTarget.close(); }}>
      <form className="resource-dialog-shell" onSubmit={save}>
        <header><div><span>{target.provider ? "Provider Stock" : "Provider Registry"}</span><h2>{target.provider ? `Configure ${target.provider.name}` : `Add ${target.adapter.name}`}</h2></div><button className="icon-button" type="button" onClick={() => dialogRef.current?.close()} aria-label="Close"><X aria-hidden="true" /></button></header>
        <div className="provider-dialog-identity"><ProviderMark type={target.adapter.id} /><div><strong>{target.adapter.name}</strong><p>{target.adapter.description}</p></div></div>
        <div className="resource-dialog-fields">
          <label><span>Provider key</span>{target.provider ? <code className="provider-key-readonly">{target.provider.providerKey}</code> : <input name="providerKey" required maxLength={64} defaultValue={providerKey} />}</label>
          <label><span>Name</span><input name="name" required maxLength={128} defaultValue={target.provider?.name ?? target.adapter.name} /></label>
          {target.adapter.fields.map((field) => (
            <label key={field.key}><span>{field.label}</span><input
              name={field.key}
              type={field.secret ? "password" : field.kind === "url" ? "url" : "text"}
              required={field.required && !(target.provider && field.secret && target.provider.secretKeys.includes(field.key))}
              defaultValue={field.secret ? "" : field.key === "baseUrl" ? target.provider?.baseUrl ?? "" : stringValue(target.provider?.config[field.key])}
              placeholder={field.secret && target.provider?.secretKeys.includes(field.key) ? "Leave blank to keep current value" : undefined}
              autoComplete={field.secret ? "off" : undefined}
            /></label>
          ))}
        </div>
        {error ? <p className="inline-error" role="alert">{error}</p> : null}
        <footer>
          <span>{target.provider ? <button className="secondary-button" type="button" disabled={testing || pending} onClick={test}><RefreshCw aria-hidden="true" />{testing ? "Testing" : "Test connection"}</button> : null}</span>
          <span><button className="secondary-button" type="button" onClick={() => dialogRef.current?.close()}>Cancel</button><button className="primary-button" type="submit" disabled={pending}>{pending ? "Saving" : "Save provider"}</button></span>
        </footer>
      </form>
    </dialog>
  );
}

function ProviderMark({ type }: { type: string }) {
  const Icon = providerIcon(type);
  return <span className={`provider-mark ${providerTone(type)}`}><Icon aria-hidden="true" /></span>;
}

function ProviderEmpty({ title, detail }: { title: string; detail: string }) {
  return <div className="provider-empty"><MoreHorizontal aria-hidden="true" /><strong>{title}</strong><span>{detail}</span></div>;
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

function providerOnline(provider: ProviderStockItem, agents: AvailableAgent[]): boolean {
  if (provider.providerType === "openclaw") {
    return agents.some((agent) => agent.status === "connected" && stringValue(agent.metadata.providerKey) === provider.providerKey);
  }
  return provider.status === "online";
}

function adapterFor(provider: ProviderStockItem, adapters: ProviderAdapter[]): ProviderAdapter {
  return adapters.find((adapter) => adapter.id === provider.providerType) ?? {
    id: provider.providerType,
    category: "custom",
    name: provider.providerType,
    description: "Custom provider configuration.",
    capabilities: [],
    executionModes: ["server"],
    supportsDiscovery: false,
    fields: [
      { key: "baseUrl", label: "Base URL", kind: "url", required: true, secret: false },
      { key: "token", label: "API key", kind: "password", required: false, secret: true },
    ],
  };
}

function uniqueProviderKey(base: string, providers: ProviderStockItem[]): string {
  const keys = new Set(providers.map((provider) => provider.providerKey));
  if (!keys.has(base)) return base;
  for (let suffix = 2; suffix < 1000; suffix += 1) {
    const candidate = `${base}-${suffix}`;
    if (!keys.has(candidate)) return candidate;
  }
  return `${base}-new`;
}

function capabilityLabel(value: string): string {
  if (value === "chat") return "Conversation";
  if (value === "speech") return "Voice";
  if (value === "transcription") return "Listening";
  if (value === "realtime") return "Realtime";
  if (value === "tool") return "Tools";
  return value;
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
