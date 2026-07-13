"use client";

import type { FormEvent, ReactNode } from "react";
import { useState } from "react";
import { useRouter } from "next/navigation";
import { Copy, KeyRound, Play, Plus, Save, Trash2, XCircle } from "lucide-react";
import type { AgentBinding, AgentEndpoint, AgentGateway, AgentProfile, AvailableAgent } from "../lib/runtime-types";

type ActionState = { tone: "error" | "success"; message: string } | null;

export function ProjectCreateForm({ availableAgents, agentGateways }: { availableAgents: AvailableAgent[]; agentGateways: AgentGateway[] }) {
  const router = useRouter();
  const [state, setState] = useState<ActionState>(null);
  const [rawKey, setRawKey] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const [gatewayId, setGatewayId] = useState("");
  const connectedGateways = new Set(agentGateways
    .filter((gateway) => gateway.status === "connected")
    .map((gateway) => gateway.gatewayId));
  const selectableAgents = availableAgents.filter((agent) => agent.status === "connected" && connectedGateways.has(agent.gatewayId));
  const gateways = Array.from(new Set(selectableAgents.map((agent) => agent.gatewayId)));
  const gatewayAgents = selectableAgents.filter((agent) => agent.gatewayId === gatewayId);

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setPending(true);
    setState(null);
    setRawKey(null);
    const form = new FormData(event.currentTarget);
    try {
      const payload = await runtimeRequest<{ project?: { publishableKey?: string } }>("projects", "POST", {
        name: value(form, "name"),
        slug: optionalValue(form, "slug"),
        description: optionalValue(form, "description"),
        gatewayId: value(form, "gatewayId"),
        agentIds: form.getAll("agentIds").map(String),
      });
      const key = payload.project?.publishableKey;
      if (!key) throw new Error("The runtime did not return the new publishable project key.");
      setRawKey(key);
      setState({ tone: "success", message: "Project created." });
      router.refresh();
    } catch (error) {
      setState({ tone: "error", message: errorMessage(error) });
    } finally {
      setPending(false);
    }
  }

  return (
    <form className="runtime-form" onSubmit={submit}>
      <div className="form-grid">
        <Field label="Name"><input name="name" required maxLength={128} placeholder="Agentshire" /></Field>
        <Field label="Slug"><input name="slug" maxLength={64} placeholder="agentshire" /></Field>
        <Field label="Agent Gateway">
          <select name="gatewayId" required value={gatewayId} onChange={(event) => setGatewayId(event.target.value)}>
            <option value="" disabled>{gateways.length > 0 ? "Select Agent Gateway" : "No Agent Gateways connected"}</option>
            {gateways.map((gatewayId) => <option key={gatewayId} value={gatewayId}>{gatewayId}</option>)}
          </select>
        </Field>
        <Field label="OpenClaw agents" wide>
          <select name="agentIds" multiple required size={Math.min(Math.max(gatewayAgents.length, 2), 8)}>
            {gatewayAgents.map((agent) => <option key={`${agent.gatewayId}/${agent.id}`} value={agent.id}>{agent.name}</option>)}
          </select>
        </Field>
        <Field label="Description" wide><input name="description" maxLength={4096} placeholder="Agent runtime for Agentshire" /></Field>
      </div>
      <div className="form-actions">
        <button className="primary-button" type="submit" disabled={pending || selectableAgents.length === 0}><Plus aria-hidden="true" />{pending ? "Creating" : "Create project"}</button>
        <ActionMessage state={state} />
      </div>
      {rawKey ? <KeyReveal value={rawKey} /> : null}
    </form>
  );
}

export function ProfileCreateForm() {
  return (
    <RuntimeForm
      submitLabel="Create profile"
      successMessage="Profile created."
      path="profiles"
      icon={<Plus aria-hidden="true" />}
      payload={(form) => ({
        name: value(form, "name"),
        slug: optionalValue(form, "slug"),
        description: optionalValue(form, "description"),
      })}
    >
      <Field label="Name"><input name="name" required maxLength={128} placeholder="Town guide" /></Field>
      <Field label="Slug"><input name="slug" maxLength={64} placeholder="town-guide" /></Field>
      <Field label="Description" wide><input name="description" maxLength={4096} placeholder="Guides players through the town" /></Field>
    </RuntimeForm>
  );
}

export function ProfileEditForm({ profile }: { profile: AgentProfile }) {
  return (
    <RuntimeForm
      submitLabel="Save profile"
      successMessage="Profile updated."
      path={`profiles/${profile.id}`}
      method="PATCH"
      icon={<Save aria-hidden="true" />}
      compact
      payload={(form) => ({
        name: value(form, "name"),
        slug: value(form, "slug"),
        description: value(form, "description"),
      })}
    >
      <Field label="Name"><input name="name" required defaultValue={profile.name} maxLength={128} /></Field>
      <Field label="Slug"><input name="slug" required defaultValue={profile.slug} maxLength={64} /></Field>
      <Field label="Description" wide><input name="description" defaultValue={profile.description ?? ""} maxLength={4096} /></Field>
    </RuntimeForm>
  );
}

export function BindingCreateForm({ profiles, agentGateways }: { profiles: AgentProfile[]; agentGateways: AgentGateway[] }) {
  const agents = agentGateways.flatMap((gateway) => gateway.status === "connected"
    ? gateway.agents.flatMap((agent) => {
      const agentId = typeof agent.id === "string" ? agent.id : "";
      if (!agentId) return [];
      return [{
        value: `${gateway.gatewayId}/${agentId}`,
        label: `${typeof agent.name === "string" ? agent.name : agentId} - ${gateway.gatewayId}`,
      }];
    })
    : []);
  return (
    <RuntimeForm
      submitLabel="Create binding"
      successMessage="Binding created."
      path="bindings"
      icon={<Plus aria-hidden="true" />}
      payload={(form) => {
        const [gatewayId, agentId] = value(form, "gatewayAgent").split("/", 2);
        return {
          profileId: value(form, "profileId"),
          provider: "openclaw",
          gatewayId,
          agentId,
          config: {},
        };
      }}
    >
      <Field label="Profile">
        <select name="profileId" required defaultValue="">
          <option value="" disabled>Select profile</option>
          {profiles.map((profile) => <option key={profile.id} value={profile.id}>{profile.name}</option>)}
        </select>
      </Field>
      <Field label="OpenClaw agent">
        <select name="gatewayAgent" required defaultValue="">
          <option value="" disabled>{agents.length > 0 ? "Select connected agent" : "No connected agents"}</option>
          {agents.map((agent) => <option key={agent.value} value={agent.value}>{agent.label}</option>)}
        </select>
      </Field>
    </RuntimeForm>
  );
}

export function BindingEditForm({ binding }: { binding: AgentBinding }) {
  return (
    <RuntimeForm
      submitLabel="Save binding"
      successMessage="Binding updated."
      path={`bindings/${binding.id}`}
      method="PATCH"
      icon={<Save aria-hidden="true" />}
      compact
      payload={(form) => ({
        gatewayId: value(form, "gatewayId"),
        agentId: value(form, "agentId"),
        config: {},
      })}
    >
      <Field label="Agent Gateway ID"><input name="gatewayId" required defaultValue={binding.gatewayId} /></Field>
      <Field label="OpenClaw agent ID"><input name="agentId" required defaultValue={binding.agentId} /></Field>
    </RuntimeForm>
  );
}

export function EndpointCreateForm({ profiles, bindings }: { profiles: AgentProfile[]; bindings: AgentBinding[] }) {
  return (
    <RuntimeForm
      submitLabel="Create endpoint"
      successMessage="Endpoint created."
      path="endpoints"
      icon={<Plus aria-hidden="true" />}
      payload={(form) => ({
        name: value(form, "name"),
        slug: optionalValue(form, "slug"),
        profileId: value(form, "profileId"),
        bindingId: value(form, "bindingId"),
        enabled: true,
        requestTimeoutMs: numberValue(form, "requestTimeoutMs", 30000),
      })}
    >
      <Field label="Name"><input name="name" required maxLength={128} placeholder="Town guide" /></Field>
      <Field label="Slug"><input name="slug" maxLength={64} placeholder="town-guide" /></Field>
      <Field label="Profile">
        <select name="profileId" required defaultValue="">
          <option value="" disabled>Select profile</option>
          {profiles.map((profile) => <option key={profile.id} value={profile.id}>{profile.name}</option>)}
        </select>
      </Field>
      <Field label="Binding">
        <select name="bindingId" required defaultValue="">
          <option value="" disabled>Select binding</option>
          {bindings.map((binding) => <option key={binding.id} value={binding.id}>{binding.gatewayId} / {binding.agentId}</option>)}
        </select>
      </Field>
      <Field label="Timeout (ms)"><input name="requestTimeoutMs" type="number" min={500} max={120000} step={500} defaultValue={30000} /></Field>
    </RuntimeForm>
  );
}

export function EndpointEditForm({ endpoint }: { endpoint: AgentEndpoint }) {
  return (
    <RuntimeForm
      submitLabel="Save endpoint"
      successMessage="Endpoint updated."
      path={`endpoints/${endpoint.id}`}
      method="PATCH"
      icon={<Save aria-hidden="true" />}
      compact
      payload={(form) => ({
        name: value(form, "name"),
        slug: value(form, "slug"),
        enabled: form.get("enabled") === "on",
        requestTimeoutMs: numberValue(form, "requestTimeoutMs", endpoint.requestTimeoutMs),
      })}
    >
      <Field label="Name"><input name="name" required defaultValue={endpoint.name} maxLength={128} /></Field>
      <Field label="Slug"><input name="slug" required defaultValue={endpoint.slug} maxLength={64} /></Field>
      <Field label="Timeout (ms)"><input name="requestTimeoutMs" type="number" min={500} max={120000} step={500} defaultValue={endpoint.requestTimeoutMs} /></Field>
      <label className="check-field"><input name="enabled" type="checkbox" defaultChecked={endpoint.enabled} />Enabled</label>
    </RuntimeForm>
  );
}

export function ApiKeyCreateForm({ endpoints }: { endpoints: AgentEndpoint[] }) {
  const router = useRouter();
  const [state, setState] = useState<ActionState>(null);
  const [rawKey, setRawKey] = useState<string | null>(null);
  const [pending, setPending] = useState(false);

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setPending(true);
    setState(null);
    setRawKey(null);
    const form = new FormData(event.currentTarget);
    try {
      const payload = await runtimeRequest<{ apiKey?: { key?: string } }>("api-keys", "POST", {
        endpointId: value(form, "endpointId"),
        name: optionalValue(form, "name"),
      });
      const key = payload.apiKey?.key;
      if (!key) throw new Error("The runtime did not return the new key.");
      setRawKey(key);
      setState({ tone: "success", message: "API key created." });
      router.refresh();
    } catch (error) {
      setState({ tone: "error", message: errorMessage(error) });
    } finally {
      setPending(false);
    }
  }

  return (
    <form className="runtime-form" onSubmit={submit}>
      <div className="form-grid">
        <Field label="Endpoint">
          <select name="endpointId" required defaultValue="">
            <option value="" disabled>Select endpoint</option>
            {endpoints.map((endpoint) => <option key={endpoint.id} value={endpoint.id}>{endpoint.name}</option>)}
          </select>
        </Field>
        <Field label="Key name"><input name="name" placeholder="Game client" maxLength={128} /></Field>
      </div>
      <div className="form-actions">
        <button className="primary-button" type="submit" disabled={pending}><KeyRound aria-hidden="true" />{pending ? "Creating" : "Create API key"}</button>
        <ActionMessage state={state} />
      </div>
      {rawKey ? <KeyReveal value={rawKey} /> : null}
    </form>
  );
}

export function DeleteResourceButton({ path, label }: { path: string; label: string }) {
  const router = useRouter();
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function remove() {
    if (!window.confirm(`Delete ${label}?`)) return;
    setPending(true);
    setError(null);
    try {
      await runtimeRequest(path, "DELETE");
      router.refresh();
    } catch (nextError) {
      setError(errorMessage(nextError));
    } finally {
      setPending(false);
    }
  }

  return (
    <span className="row-action-wrap">
      <button className="icon-button danger" type="button" onClick={remove} disabled={pending} title={`Delete ${label}`} aria-label={`Delete ${label}`}>
        <Trash2 aria-hidden="true" />
      </button>
      {error ? <span className="inline-error" role="alert">{error}</span> : null}
    </span>
  );
}

export function RevokeApiKeyButton({ id, name }: { id: string; name: string }) {
  const router = useRouter();
  const [pending, setPending] = useState(false);

  async function revoke() {
    if (!window.confirm(`Revoke ${name}?`)) return;
    setPending(true);
    try {
      await runtimeRequest(`api-keys/${id}`, "DELETE");
      router.refresh();
    } finally {
      setPending(false);
    }
  }

  return <button className="icon-button danger" type="button" onClick={revoke} disabled={pending} title={`Revoke ${name}`} aria-label={`Revoke ${name}`}><XCircle aria-hidden="true" /></button>;
}

export function InvokeEndpointForm({ endpoint }: { endpoint: AgentEndpoint }) {
  const [pending, setPending] = useState(false);
  const [result, setResult] = useState<string | null>(null);
  const [state, setState] = useState<ActionState>(null);

  async function invoke(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setPending(true);
    setState(null);
    setResult(null);
    const form = new FormData(event.currentTarget);
    try {
      const payload = await runtimeRequest(`endpoints/${endpoint.id}/invoke`, "POST", {
        message: value(form, "message"),
      });
      setResult(JSON.stringify(payload, null, 2));
      setState({ tone: "success", message: "Call completed." });
    } catch (error) {
      setState({ tone: "error", message: errorMessage(error) });
    } finally {
      setPending(false);
    }
  }

  return (
    <form className="invoke-form" onSubmit={invoke}>
      <input name="message" required placeholder="Ask this agent" aria-label="Message" />
      <button className="icon-text-button" type="submit" disabled={pending}><Play aria-hidden="true" />{pending ? "Running" : "Run"}</button>
      <ActionMessage state={state} />
      {result ? <pre>{result}</pre> : null}
    </form>
  );
}

function RuntimeForm({
  path,
  method = "POST",
  submitLabel,
  successMessage,
  icon,
  payload,
  compact = false,
  children,
}: {
  path: string;
  method?: "POST" | "PATCH";
  submitLabel: string;
  successMessage: string;
  icon: ReactNode;
  payload: (form: FormData) => Record<string, unknown>;
  compact?: boolean;
  children: ReactNode;
}) {
  const router = useRouter();
  const [pending, setPending] = useState(false);
  const [state, setState] = useState<ActionState>(null);

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const formElement = event.currentTarget;
    setPending(true);
    setState(null);
    try {
      await runtimeRequest(path, method, payload(new FormData(formElement)));
      setState({ tone: "success", message: successMessage });
      if (method === "POST") formElement.reset();
      router.refresh();
    } catch (error) {
      setState({ tone: "error", message: errorMessage(error) });
    } finally {
      setPending(false);
    }
  }

  return (
    <form className={`runtime-form${compact ? " compact" : ""}`} onSubmit={submit}>
      <div className="form-grid">{children}</div>
      <div className="form-actions">
        <button className={compact ? "secondary-button" : "primary-button"} type="submit" disabled={pending}>{icon}{pending ? "Saving" : submitLabel}</button>
        <ActionMessage state={state} />
      </div>
    </form>
  );
}

function Field({ label, wide = false, children }: { label: string; wide?: boolean; children: ReactNode }) {
  return <label className={wide ? "wide-field" : undefined}><span>{label}</span>{children}</label>;
}

function ActionMessage({ state }: { state: ActionState }) {
  return state ? <span className={`action-message ${state.tone}`} role={state.tone === "error" ? "alert" : "status"}>{state.message}</span> : null;
}

function KeyReveal({ value: key }: { value: string }) {
  const [copied, setCopied] = useState(false);
  async function copy() {
    await navigator.clipboard.writeText(key);
    setCopied(true);
  }
  return (
    <div className="key-reveal">
      <code>{key}</code>
      <button className="icon-button" type="button" onClick={copy} title="Copy key" aria-label="Copy key"><Copy aria-hidden="true" /></button>
      <span>{copied ? "Copied" : "Shown once"}</span>
    </div>
  );
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

function value(form: FormData, name: string): string {
  return String(form.get(name) ?? "").trim();
}

function optionalValue(form: FormData, name: string): string | undefined {
  return value(form, name) || undefined;
}

function numberValue(form: FormData, name: string, fallback: number): number {
  const parsed = Number(value(form, name));
  return Number.isFinite(parsed) ? parsed : fallback;
}


function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : "Request failed.";
}
