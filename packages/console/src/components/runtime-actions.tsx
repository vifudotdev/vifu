"use client";

import type { FormEvent, ReactNode } from "react";
import { useState } from "react";
import { Play, Plus, Save, Trash2, XCircle } from "lucide-react";
import type {
  AgentBinding,
  AgentEndpoint,
  AgentGateway,
  AgentProfile,
  AvailableAgent,
  RuntimeProject,
} from "../types";
import { useRuntimeConsoleHost, useRuntimeConsoleRouter } from "../host";

type ActionState = { tone: "error" | "success"; message: string } | null;

export function ProjectCreateForm({
  variant = "full",
}: {
  variant?: "full" | "menu";
}) {
  const host = useRuntimeConsoleHost();
  const router = useRuntimeConsoleRouter();
  const [state, setState] = useState<ActionState>(null);
  const [pending, setPending] = useState(false);
  const [projectName, setProjectName] = useState("");
  const isMenu = variant === "menu";

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setPending(true);
    setState(null);
    const form = new FormData(event.currentTarget);
    try {
      const payload = await host.request<{ app?: { slug?: string; appId?: string } }>("apps", "POST", {
        name: value(form, "name"),
        description: optionalValue(form, "description"),
      });
      const slug = payload.app?.slug ?? null;
      setState({ tone: "success", message: "App created." });
      if (slug) router.push(host.projectHref(slug));
    } catch (error) {
      setState({ tone: "error", message: errorMessage(error) });
    } finally {
      setPending(false);
    }
  }

  return (
    <form className={`runtime-form project-create-form${isMenu ? " menu" : ""}`} onSubmit={submit}>
      <div className="form-grid">
        <Field label="Name">
          <input
            name="name"
            required
            maxLength={128}
            placeholder="My game"
            value={projectName}
            onChange={(event) => setProjectName(event.target.value)}
          />
        </Field>
        {isMenu ? null : <Field label="Description" wide><input name="description" maxLength={4096} placeholder="Agent runtime for your game" /></Field>}
      </div>
      <div className="form-actions">
        <button className="primary-button" type="submit" disabled={pending}><Plus aria-hidden="true" />{pending ? "Creating" : "Create app"}</button>
        <ActionMessage state={state} />
      </div>
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
      <Field label="Idle timeout (ms)"><input name="requestTimeoutMs" type="number" min={500} max={120000} step={500} defaultValue={30000} title="Cancel only after the provider stops reporting activity for this long." /></Field>
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
      <Field label="Idle timeout (ms)"><input name="requestTimeoutMs" type="number" min={500} max={120000} step={500} defaultValue={endpoint.requestTimeoutMs} title="Cancel only after the provider stops reporting activity for this long." /></Field>
      <label className="check-field"><input name="enabled" type="checkbox" defaultChecked={endpoint.enabled} />Enabled</label>
    </RuntimeForm>
  );
}

export function DeleteResourceButton({
  path,
  label,
  redirectTo,
}: {
  path: string;
  label: string;
  redirectTo?: string;
}) {
  const host = useRuntimeConsoleHost();
  const router = useRuntimeConsoleRouter();
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function remove() {
    if (!window.confirm(`Delete ${label}?`)) return;
    setPending(true);
    setError(null);
    try {
      await host.request(path, "DELETE");
      if (redirectTo) router.push(redirectTo);
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

export function RevokeApiKeyButton({ projectSlug, id, name }: { projectSlug: string; id: string; name: string }) {
  const host = useRuntimeConsoleHost();
  const router = useRuntimeConsoleRouter();
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function revoke() {
    if (!window.confirm(`Revoke ${name}?`)) return;
    setPending(true);
    setError(null);
    try {
      await host.request(`apps/${projectSlug}/api-keys/${id}/revoke`, "POST");
      router.refresh();
    } catch (nextError) {
      setError(errorMessage(nextError));
    } finally {
      setPending(false);
    }
  }

  return (
    <span className="row-action-wrap">
      <button className="icon-button danger" type="button" onClick={revoke} disabled={pending} title={`Revoke ${name}`} aria-label={`Revoke ${name}`}>
        <XCircle aria-hidden="true" />
      </button>
      {error ? <span className="inline-error" role="alert">{error}</span> : null}
    </span>
  );
}

export function DeleteApiKeyButton({ projectSlug, id, name }: { projectSlug: string; id: string; name: string }) {
  const host = useRuntimeConsoleHost();
  const router = useRuntimeConsoleRouter();
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function remove() {
    if (!window.confirm(`Permanently delete the revoked API key record ${name}? This cannot be undone.`)) return;
    setPending(true);
    setError(null);
    try {
      await host.request(`apps/${projectSlug}/api-keys/${id}`, "DELETE");
      router.refresh();
    } catch (nextError) {
      setError(errorMessage(nextError));
    } finally {
      setPending(false);
    }
  }

  return (
    <span className="row-action-wrap">
      <button className="icon-button danger" type="button" onClick={remove} disabled={pending} title={`Delete ${name} record`} aria-label={`Delete ${name} record`}>
        <Trash2 aria-hidden="true" />
      </button>
      {error ? <span className="inline-error" role="alert">{error}</span> : null}
    </span>
  );
}

export function ChatCompletionForm({ endpoint }: { endpoint: AgentEndpoint }) {
  const host = useRuntimeConsoleHost();
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
      const payload = await host.request("chat/completions", "POST", {
        model: endpoint.slug,
        messages: [{ role: "user", content: value(form, "message") }],
        stream: false,
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
  method?: "POST" | "PUT" | "PATCH";
  submitLabel: string;
  successMessage: string;
  icon: ReactNode;
  payload: (form: FormData) => Record<string, unknown>;
  compact?: boolean;
  children: ReactNode;
}) {
  const host = useRuntimeConsoleHost();
  const router = useRuntimeConsoleRouter();
  const [pending, setPending] = useState(false);
  const [state, setState] = useState<ActionState>(null);

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const formElement = event.currentTarget;
    setPending(true);
    setState(null);
    try {
      await host.request(path, method, payload(new FormData(formElement)));
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
