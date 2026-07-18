"use client";

import Link from "next/link";
import { useRef, useState, type FormEvent, type ReactNode } from "react";
import { useRouter } from "next/navigation";
import {
  ArrowRight,
  BookOpen,
  Check,
  Copy,
  KeyRound,
  Pencil,
  Plus,
  Search,
  Terminal,
  X,
} from "lucide-react";
import type {
  AgentBinding,
  AgentEndpoint,
  ApiKeyAgentScope,
  ApiKeyPermissions,
  ApiKeyRecord,
  ProjectCanvas,
  RuntimeProject,
} from "../lib/runtime-types";
import { DeleteApiKeyButton, RevokeApiKeyButton } from "./runtime-actions";

type CodeTab = "curl" | "javascript" | "openai";
type ApiKeyFilter = "active" | "revoked";

const CODE_TABS: Array<{ id: CodeTab; label: string }> = [
  { id: "curl", label: "cURL" },
  { id: "javascript", label: "Node.js" },
  { id: "openai", label: "OpenAI SDK" },
];

export function ApiIntegrationsView({
  project,
  projects,
  keys,
  endpoints,
  bindings,
  canvas,
  browserApiBaseUrl,
}: {
  project: RuntimeProject;
  projects: RuntimeProject[];
  keys: ApiKeyRecord[];
  endpoints: AgentEndpoint[];
  bindings: AgentBinding[];
  canvas?: ProjectCanvas;
  browserApiBaseUrl: string;
}) {
  const exposedBindingIds = canvas
    ? new Set(canvas.nodes.filter((node) => node.exposed && node.bindingId).map((node) => node.bindingId as string))
    : new Set(project.bindingIds);
  const enabledEndpoints = endpoints.filter((endpoint) => (
    endpoint.enabled
    && project.bindingIds.includes(endpoint.bindingId)
    && exposedBindingIds.has(endpoint.bindingId)
  ));
  const agentOptions = projectAgentOptions(project, endpoints, bindings);
  const scopedKeys = keys.filter((key) => key.projectId === project.id);
  const [selectedEndpointId, setSelectedEndpointId] = useState(enabledEndpoints[0]?.id ?? "");
  const [codeTab, setCodeTab] = useState<CodeTab>("curl");
  const [apiKeyFilter, setApiKeyFilter] = useState<ApiKeyFilter>("active");
  const selectedEndpoint = enabledEndpoints.find((endpoint) => endpoint.id === selectedEndpointId)
    ?? enabledEndpoints[0];
  const projectBaseUrl = projectApiBaseUrl(project, browserApiBaseUrl);
  const code = buildCodeExample(codeTab, projectBaseUrl, selectedEndpoint?.slug ?? "agent-id");
  const activeKeys = scopedKeys.filter((key) => !key.revokedAt);
  const revokedKeys = scopedKeys.filter((key) => key.revokedAt);
  const visibleKeys = apiKeyFilter === "active" ? activeKeys : revokedKeys;
  return (
    <div className="api-integrations">
      <section className="api-integration-section api-endpoint-section">
        <div className="api-endpoint-heading">
          <div>
            <span>Project endpoint</span>
            <p>One URL for every enabled agent in this project.</p>
          </div>
          <ApiReferenceDialog baseUrl={projectBaseUrl} />
        </div>
        <div className="api-endpoint-value">
          <code>{projectBaseUrl}</code>
          <CopyButton value={projectBaseUrl} label="Copy project endpoint" />
        </div>
      </section>

      {selectedEndpoint ? (
        <div className="api-integration-grid">
          <section className="api-integration-section api-quickstart-section">
            <IntegrationSectionHeading
              title="Quickstart"
              description="Send an OpenAI-compatible chat completion."
              action={<CopyButton key={`${codeTab}:${selectedEndpoint.id}`} value={code} label="Copy code example" />}
            />
            <div className="api-quickstart-controls">
              <label>
                <span>Agent</span>
                <select value={selectedEndpoint.id} onChange={(event) => setSelectedEndpointId(event.target.value)}>
                  {enabledEndpoints.map((endpoint) => (
                    <option key={endpoint.id} value={endpoint.id}>{endpoint.name}</option>
                  ))}
                </select>
              </label>
              <div className="api-code-tabs" role="tablist" aria-label="Code example">
                {CODE_TABS.map((tab) => (
                  <button
                    className={codeTab === tab.id ? "active" : ""}
                    type="button"
                    role="tab"
                    aria-selected={codeTab === tab.id}
                    key={tab.id}
                    onClick={() => setCodeTab(tab.id)}
                  >
                    {tab.label}
                  </button>
                ))}
              </div>
            </div>
            <pre className="api-code-surface"><code>{code}</code></pre>
            <div className="api-code-footnote">
              <Terminal aria-hidden="true" />
              <span>The <code>model</code> value selects an agent inside this project.</span>
            </div>
          </section>

          <section className="api-integration-section api-agents-section">
            <IntegrationSectionHeading
              title="Agents"
              description={`${enabledEndpoints.length} available through this endpoint`}
            />
            <div className="api-agent-list">
              {enabledEndpoints.map((endpoint) => (
                  <button
                    className={selectedEndpoint?.id === endpoint.id ? "selected" : ""}
                    type="button"
                    key={endpoint.id}
                    onClick={() => setSelectedEndpointId(endpoint.id)}
                  >
                    <span className="api-agent-status" aria-label="Enabled" />
                    <span className="api-agent-identity">
                      <strong>{endpoint.name}</strong>
                      <code>{endpoint.slug}</code>
                    </span>
                    <ArrowRight aria-hidden="true" />
                  </button>
              ))}
            </div>
          </section>
        </div>
      ) : (
        <ApiSetupEmpty projectSlug={project.slug} />
      )}

      <section className="api-integration-section api-keys-section">
          <IntegrationSectionHeading
            title="API keys"
            description="Create and manage keys for this project."
            action={(
              <div className="api-key-heading-actions">
                {scopedKeys.length > 0 ? (
                  <div className="api-key-filters" role="tablist" aria-label="API key status">
                    <button
                      className={apiKeyFilter === "active" ? "active" : ""}
                      type="button"
                      role="tab"
                      aria-selected={apiKeyFilter === "active"}
                      onClick={() => setApiKeyFilter("active")}
                    >
                      Active <span>{activeKeys.length}</span>
                    </button>
                    <button
                      className={apiKeyFilter === "revoked" ? "active" : ""}
                      type="button"
                      role="tab"
                      aria-selected={apiKeyFilter === "revoked"}
                      onClick={() => setApiKeyFilter("revoked")}
                    >
                      Revoked <span>{revokedKeys.length}</span>
                    </button>
                  </div>
                ) : null}
                <CreateApiKeyDialog
                  project={project}
                  agentOptions={agentOptions}
                  exampleModel={selectedEndpoint?.slug ?? enabledEndpoints[0]?.slug ?? "agent-slug-or-id"}
                  projectBaseUrl={projectBaseUrl}
                />
              </div>
            )}
          />
          {visibleKeys.length > 0 ? (
            <div className="api-key-table-wrap">
              <table className="api-key-table">
                <thead>
                  <tr>
                    <th>Name</th>
                    <th>Agent access</th>
                    <th>Permissions</th>
                    <th>Key</th>
                    <th>{apiKeyFilter === "active" ? "Created" : "Revoked"}</th>
                    <th><span className="sr-only">Actions</span></th>
                  </tr>
                </thead>
                <tbody>
                  {visibleKeys.map((key) => (
                      <tr key={key.id}>
                        <td><strong>{key.name}</strong></td>
                        <td>
                          <span className="api-key-scope">
                            <span>{formatAgentScope(key.agentScope, agentOptions)}</span>
                            <code>{project.slug}</code>
                          </span>
                        </td>
                        <td><span className="api-key-permission-summary">{formatPermissions(key.permissions)}</span></td>
                        <td><code>{key.keyPrefix}...</code></td>
                        <td><time dateTime={key.revokedAt ?? key.createdAt}>{formatDate(key.revokedAt ?? key.createdAt)}</time></td>
                        <td>
                          <div className="api-key-row-actions">
                            {!key.revokedAt ? (
                              <EditApiKeyDialog apiKey={key} projects={projects} endpoints={endpoints} bindings={bindings} />
                            ) : null}
                            {key.revokedAt
                              ? <DeleteApiKeyButton id={key.id} name={key.name} />
                              : <RevokeApiKeyButton id={key.id} name={key.name} />}
                          </div>
                        </td>
                      </tr>
                  ))}
                </tbody>
              </table>
            </div>
          ) : (
            <div className="api-keys-empty">
              <KeyRound aria-hidden="true" />
              <div>
                <strong>{apiKeyFilter === "active" ? "No active API keys" : "No revoked API keys"}</strong>
                <span>{apiKeyFilter === "active" ? "Create a key when you are ready to call an agent." : "Revoked key records appear here until deleted."}</span>
              </div>
            </div>
          )}
      </section>

    </div>
  );
}

function IntegrationSectionHeading({
  title,
  description,
  action,
}: {
  title: string;
  description: string;
  action?: ReactNode;
}) {
  return (
    <header className="api-integration-heading">
      <div><h2>{title}</h2><p>{description}</p></div>
      {action}
    </header>
  );
}

function ApiSetupEmpty({ projectSlug }: { projectSlug: string }) {
  return (
    <section className="api-integration-section api-setup-empty">
      <Terminal aria-hidden="true" />
      <div><strong>Expose an agent to start</strong><span>Agents added in Gameplay become models on this project endpoint.</span></div>
      <Link className="secondary-button" href={`/project/${projectSlug}/gameplay`}>Open Gameplay<ArrowRight aria-hidden="true" /></Link>
    </section>
  );
}

function CopyButton({ value, label }: { value: string; label: string }) {
  const [copied, setCopied] = useState(false);

  async function copy() {
    try {
      await navigator.clipboard.writeText(value);
      setCopied(true);
    } catch {
      setCopied(false);
    }
  }

  return (
    <button className="icon-button" type="button" onClick={copy} title={copied ? "Copied" : label} aria-label={copied ? "Copied" : label}>
      {copied ? <Check aria-hidden="true" /> : <Copy aria-hidden="true" />}
    </button>
  );
}

function ApiReferenceDialog({ baseUrl }: { baseUrl: string }) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const modelsUrl = `${baseUrl}/models`;
  const chatUrl = `${baseUrl}/chat/completions`;
  return (
    <>
      <button className="secondary-button" type="button" onClick={() => dialogRef.current?.showModal()}>
        <BookOpen aria-hidden="true" />API reference
      </button>
      <dialog className="api-reference-dialog" ref={dialogRef}>
        <div className="api-dialog-shell">
          <header>
            <div><span>Project API</span><h2>API reference</h2></div>
            <form method="dialog"><button className="icon-button" type="submit" aria-label="Close API reference" title="Close"><X aria-hidden="true" /></button></form>
          </header>
          <div className="api-reference-content">
            <section>
              <h3>Authentication</h3>
              <p>Send a key in the bearer authorization header. A project key can call agents in its project by changing <code>model</code>.</p>
              <pre><code>Authorization: Bearer $VIFU_API_KEY</code></pre>
            </section>
            <section>
              <div className="api-reference-route"><span className="get">GET</span><code>{modelsUrl}</code></div>
              <p>Returns the enabled agents available to the supplied key as OpenAI-compatible models.</p>
            </section>
            <section>
              <div className="api-reference-route"><span className="post">POST</span><code>{chatUrl}</code></div>
              <p>Calls one agent. Streaming is not supported yet.</p>
              <pre><code>{JSON.stringify({
                model: "agent-id",
                messages: [{ role: "user", content: "Hello" }],
                stream: false,
              }, null, 2)}</code></pre>
            </section>
            <section>
              <h3>Tracing</h3>
              <p>Every accepted call creates a trace. Use the project Logs page to inspect its request, response, latency, and error details.</p>
            </section>
          </div>
        </div>
      </dialog>
    </>
  );
}

type ApiKeyAgentOption = {
  bindingId: string;
  name: string;
  slug: string;
};

function CreateApiKeyDialog({
  project,
  agentOptions,
  exampleModel,
  projectBaseUrl,
}: {
  project: RuntimeProject;
  agentOptions: ApiKeyAgentOption[];
  exampleModel: string;
  projectBaseUrl: string;
}) {
  const router = useRouter();
  const dialogRef = useRef<HTMLDialogElement>(null);
  const [name, setName] = useState("");
  const [scopeMode, setScopeMode] = useState<ApiKeyAgentScope["mode"]>("all");
  const [selectedBindingIds, setSelectedBindingIds] = useState<string[]>([]);
  const [permissions, setPermissions] = useState<ApiKeyPermissions>(defaultApiKeyPermissions);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [createdKey, setCreatedKey] = useState<string | null>(null);
  const readyCurl = createdKey
    ? buildCurlExample(projectBaseUrl, exampleModel, createdKey)
    : "";

  async function createKey(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setPending(true);
    setError(null);
    try {
      const payload = await runtimeRequest<{ apiKey?: { key?: string } }>("api-keys", "POST", {
        projectId: project.id,
        name: name.trim() || readableDefaultKeyName(),
        agentScope: agentScopePayload(scopeMode, selectedBindingIds),
        permissions,
      });
      const key = payload.apiKey?.key;
      if (!key) throw new Error("The runtime did not return the new key.");
      setCreatedKey(key);
      router.refresh();
    } catch (nextError) {
      setError(nextError instanceof Error ? nextError.message : "Could not create the API key.");
    } finally {
      setPending(false);
    }
  }

  function open() {
    reset();
    setName(readableDefaultKeyName());
    dialogRef.current?.showModal();
  }

  function reset() {
    setName("");
    setScopeMode("all");
    setSelectedBindingIds([]);
    setPermissions(defaultApiKeyPermissions());
    setCreatedKey(null);
    setError(null);
  }

  return (
    <>
      <button className="primary-button" type="button" onClick={open}>
        <Plus aria-hidden="true" />Create key
      </button>
      <dialog className="api-key-dialog" ref={dialogRef} onClose={reset}>
        <div className="api-dialog-shell api-key-dialog-shell">
          <header>
            <div>
              <span>{createdKey ? "Project key" : "Project access"}</span>
              <h2>{createdKey ? "Save your API key" : "Create API key"}</h2>
            </div>
            <form method="dialog"><button className="icon-button" type="submit" aria-label="Close API key dialog" title="Close"><X aria-hidden="true" /></button></form>
          </header>
          {createdKey ? (
            <div className="api-created-key">
              <div className="api-created-key-note"><Check aria-hidden="true" /><div><strong>Shown once</strong><span>Store this key before closing the window.</span></div></div>
              <div className="api-secret-value"><code>{createdKey}</code><CopyButton value={createdKey} label="Copy API key" /></div>
              <div className="api-created-curl">
                <div><span>Ready-to-run request</span><CopyButton value={readyCurl} label="Copy ready-to-run request" /></div>
                <pre><code>{readyCurl}</code></pre>
              </div>
              <form method="dialog" className="api-dialog-actions"><button className="primary-button" type="submit">Done</button></form>
            </div>
          ) : (
            <form className="api-key-create-form" onSubmit={createKey}>
              <div className="api-key-form-content">
                <label className="api-key-name-field">
                  <span>Name</span>
                  <input
                    name="name"
                    maxLength={128}
                    value={name}
                    placeholder="Project key"
                    onChange={(event) => setName(event.target.value)}
                    autoFocus
                  />
                </label>

                <div className="api-key-scope-note">
                  <strong>Project key</strong>
                  <span>Each request must provide a <code>model</code> that resolves to an exposed agent in <b>{project.name}</b>.</span>
                </div>
                <ApiKeyAgentScopeFields
                  mode={scopeMode}
                  selectedBindingIds={selectedBindingIds}
                  options={agentOptions}
                  onModeChange={setScopeMode}
                  onSelectedBindingIdsChange={setSelectedBindingIds}
                />
                <ApiKeyPermissionsFields permissions={permissions} onChange={setPermissions} />
                {error ? <span className="action-message error" role="alert">{error}</span> : null}
              </div>
              <div className="api-dialog-actions">
                <button className="secondary-button" type="button" onClick={() => dialogRef.current?.close()}>Cancel</button>
                <button className="primary-button" type="submit" disabled={pending || !name.trim() || (scopeMode === "selected" && selectedBindingIds.length === 0)}>
                  <KeyRound aria-hidden="true" />{pending ? "Creating" : "Create key"}
                </button>
              </div>
            </form>
          )}
        </div>
      </dialog>
    </>
  );
}

function EditApiKeyDialog({
  apiKey,
  projects,
  endpoints,
  bindings,
}: {
  apiKey: ApiKeyRecord;
  projects: RuntimeProject[];
  endpoints: AgentEndpoint[];
  bindings: AgentBinding[];
}) {
  const router = useRouter();
  const dialogRef = useRef<HTMLDialogElement>(null);
  const [name, setName] = useState(apiKey.name);
  const [projectId, setProjectId] = useState(apiKey.projectId);
  const [scopeMode, setScopeMode] = useState<ApiKeyAgentScope["mode"]>(apiKey.agentScope.mode);
  const [selectedBindingIds, setSelectedBindingIds] = useState<string[]>(selectedScopeBindingIds(apiKey.agentScope));
  const [permissions, setPermissions] = useState<ApiKeyPermissions>(apiKey.permissions);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const selectedProject = projects.find((project) => project.id === projectId);
  const agentOptions = selectedProject ? projectAgentOptions(selectedProject, endpoints, bindings) : [];
  const scopeChanged = projectId !== apiKey.projectId;

  function reset() {
    setName(apiKey.name);
    setProjectId(apiKey.projectId);
    setScopeMode(apiKey.agentScope.mode);
    setSelectedBindingIds(selectedScopeBindingIds(apiKey.agentScope));
    setPermissions(apiKey.permissions);
    setError(null);
  }

  function open() {
    reset();
    dialogRef.current?.showModal();
  }

  function changeProject(nextProjectId: string) {
    setProjectId(nextProjectId);
    setScopeMode("all");
    setSelectedBindingIds([]);
  }

  async function save(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setPending(true);
    setError(null);
    try {
      await runtimeRequest(`api-keys/${apiKey.id}`, "PATCH", {
        projectId,
        name: name.trim(),
        agentScope: agentScopePayload(scopeMode, selectedBindingIds),
        permissions,
      });
      dialogRef.current?.close();
      router.refresh();
    } catch (nextError) {
      setError(nextError instanceof Error ? nextError.message : "Could not update the API key.");
    } finally {
      setPending(false);
    }
  }

  return (
    <>
      <button className="icon-button" type="button" onClick={open} title={`Edit ${apiKey.name}`} aria-label={`Edit ${apiKey.name}`}>
        <Pencil aria-hidden="true" />
      </button>
      <dialog className="api-key-dialog" ref={dialogRef} onClose={reset}>
        <div className="api-dialog-shell api-key-dialog-shell">
          <header>
            <div><span>Project access</span><h2>Edit API key</h2></div>
            <form method="dialog"><button className="icon-button" type="submit" aria-label="Close API key dialog" title="Close"><X aria-hidden="true" /></button></form>
          </header>
          <form className="api-key-create-form" onSubmit={save}>
            <div className="api-key-form-content">
              <label className="api-key-name-field">
                <span>Name</span>
                <input
                  name="name"
                  maxLength={128}
                  value={name}
                  onChange={(event) => setName(event.target.value)}
                  autoFocus
                />
              </label>
              <label>
                <span>Project scope</span>
                <select name="projectId" value={projectId} onChange={(event) => changeProject(event.target.value)}>
                  {projects.map((project) => <option value={project.id} key={project.id}>{project.name}</option>)}
                </select>
              </label>
              <div className="api-key-scope-note">
                <strong>{scopeChanged ? `Move access to ${selectedProject?.name ?? "this project"}` : `Scoped to ${selectedProject?.name ?? "this project"}`}</strong>
                <span>{scopeChanged
                  ? "Saving moves this key to the selected project. Its agent access has been reset so it cannot retain bindings from the previous project."
                  : "Choose whether this key follows every exposed agent or only an explicit set."}</span>
              </div>
              <ApiKeyAgentScopeFields
                mode={scopeMode}
                selectedBindingIds={selectedBindingIds}
                options={agentOptions}
                onModeChange={setScopeMode}
                onSelectedBindingIdsChange={setSelectedBindingIds}
              />
              <ApiKeyPermissionsFields permissions={permissions} onChange={setPermissions} />
              {error ? <span className="action-message error" role="alert">{error}</span> : null}
            </div>
            <div className="api-dialog-actions">
              <button className="secondary-button" type="button" onClick={() => dialogRef.current?.close()}>Cancel</button>
              <button className="primary-button" type="submit" disabled={pending || !name.trim() || !projectId || (scopeMode === "selected" && selectedBindingIds.length === 0)}>
                <Check aria-hidden="true" />{pending ? "Saving" : "Save changes"}
              </button>
            </div>
          </form>
        </div>
      </dialog>
    </>
  );
}

function ApiKeyAgentScopeFields({
  mode,
  selectedBindingIds,
  options,
  onModeChange,
  onSelectedBindingIdsChange,
}: {
  mode: ApiKeyAgentScope["mode"];
  selectedBindingIds: string[];
  options: ApiKeyAgentOption[];
  onModeChange: (mode: ApiKeyAgentScope["mode"]) => void;
  onSelectedBindingIdsChange: (bindingIds: string[]) => void;
}) {
  const [query, setQuery] = useState("");
  const normalizedQuery = query.trim().toLocaleLowerCase();
  const visibleOptions = normalizedQuery
    ? options.filter((option) => `${option.name} ${option.slug}`.toLocaleLowerCase().includes(normalizedQuery))
    : options;

  function toggleBinding(bindingId: string) {
    onSelectedBindingIdsChange(selectedBindingIds.includes(bindingId)
      ? selectedBindingIds.filter((id) => id !== bindingId)
      : [...selectedBindingIds, bindingId]);
  }

  return (
    <fieldset className="api-key-fieldset api-key-agent-access">
      <legend>Agent access</legend>
      <div className="api-key-segmented" role="group" aria-label="Agent access">
        <button type="button" aria-pressed={mode === "all"} onClick={() => onModeChange("all")}>All agents</button>
        <button type="button" aria-pressed={mode === "selected"} onClick={() => onModeChange("selected")}>Selected agents</button>
      </div>
      <p className="api-key-agent-access-help">
        {mode === "all"
          ? "Includes agents exposed now and agents exposed later in this project."
          : "Only the selected project bindings can be invoked with this key."}
      </p>
      {mode === "selected" ? (
        <div className="api-key-agent-picker">
          <label className="api-key-agent-search">
            <Search aria-hidden="true" />
            <input type="search" value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search agents" />
          </label>
          <div className="api-key-agent-options">
            {visibleOptions.length > 0 ? visibleOptions.map((option) => (
              <label key={option.bindingId}>
                <input
                  type="checkbox"
                  checked={selectedBindingIds.includes(option.bindingId)}
                  onChange={() => toggleBinding(option.bindingId)}
                />
                <span><strong>{option.name}</strong><code>{option.slug}</code></span>
              </label>
            )) : <p>{options.length > 0 ? "No matching agents." : "This project has no agent bindings yet."}</p>}
          </div>
          <span className="api-key-agent-count">{selectedBindingIds.length} selected</span>
        </div>
      ) : null}
    </fieldset>
  );
}

function ApiKeyPermissionsFields({
  permissions,
  onChange,
}: {
  permissions: ApiKeyPermissions;
  onChange: (permissions: ApiKeyPermissions) => void;
}) {
  return (
    <fieldset className="api-key-fieldset api-key-permissions">
      <legend>Permissions</legend>
      <ApiKeyPermissionRow
        label="Chat Completions"
        value={permissions.chatCompletions}
        options={[
          { value: "none", label: "No access" },
          { value: "access", label: "Access" },
        ]}
        onChange={(chatCompletions) => onChange({ ...permissions, chatCompletions })}
      />
      <ApiKeyPermissionRow
        label="Agents"
        value={permissions.agents}
        options={[
          { value: "none", label: "No access" },
          { value: "read", label: "Read" },
          { value: "write", label: "Write" },
        ]}
        onChange={(agents) => onChange({ ...permissions, agents })}
      />
      <ApiKeyPermissionRow
        label="Project"
        value={permissions.project}
        options={[
          { value: "none", label: "No access" },
          { value: "read", label: "Read" },
          { value: "write", label: "Write" },
        ]}
        onChange={(project) => onChange({ ...permissions, project })}
      />
    </fieldset>
  );
}

function ApiKeyPermissionRow<T extends string>({
  label,
  value,
  options,
  onChange,
}: {
  label: string;
  value: T;
  options: Array<{ value: T; label: string }>;
  onChange: (value: T) => void;
}) {
  return (
    <div className="api-key-permission-row">
      <span>{label}</span>
      <div
        className="api-key-segmented"
        role="group"
        aria-label={`${label} permission`}
        data-option-count={options.length}
      >
        {options.map((option) => (
          <button
            type="button"
            key={option.value}
            aria-pressed={value === option.value}
            onClick={() => onChange(option.value)}
          >
            {option.label}
          </button>
        ))}
      </div>
    </div>
  );
}

function buildCodeExample(tab: CodeTab, baseUrl: string, model: string): string {
  if (tab === "javascript") {
    return [
      `const response = await fetch(${JSON.stringify(`${baseUrl}/chat/completions`)}, {`,
      '  method: "POST",',
      "  headers: {",
      '    Authorization: `Bearer ${process.env.VIFU_API_KEY}`,',
      '    "Content-Type": "application/json",',
      "  },",
      `  body: JSON.stringify(${JSON.stringify({
        model,
        messages: [{ role: "user", content: "Hello" }],
      }, null, 2).replaceAll("\n", "\n  ")}),`,
      "});",
      "",
      "const completion = await response.json();",
    ].join("\n");
  }
  if (tab === "openai") {
    return [
      'import OpenAI from "openai";',
      "",
      "const client = new OpenAI({",
      "  apiKey: process.env.VIFU_API_KEY,",
      `  baseURL: ${JSON.stringify(baseUrl)},`,
      "});",
      "",
      "const completion = await client.chat.completions.create({",
      `  model: ${JSON.stringify(model)},`,
      '  messages: [{ role: "user", content: "Hello" }],',
      "});",
    ].join("\n");
  }
  return buildCurlExample(baseUrl, model);
}

function buildCurlExample(baseUrl: string, model: string, apiKey?: string): string {
  const authHeader = apiKey
    ? shellQuote(`Authorization: Bearer ${apiKey}`)
    : '"Authorization: Bearer $VIFU_API_KEY"';
  return [
    `curl ${shellQuote(`${baseUrl}/chat/completions`)} \\`,
    "  --request POST \\",
    `  --header ${authHeader} \\`,
    `  --header ${shellQuote("Content-Type: application/json")} \\`,
    `  --data ${shellQuote(JSON.stringify({
      model,
      messages: [{ role: "user", content: "Hello" }],
    }))}`,
  ].join("\n");
}

function projectApiBaseUrl(project: RuntimeProject, browserApiBaseUrl: string): string {
  const url = new URL(browserApiBaseUrl);
  const basePath = url.pathname.replace(/\/+$/, "");
  url.pathname = `${basePath}/${encodeURIComponent(project.slug)}/v1`;
  url.search = "";
  return url.toString().replace(/\/$/, "");
}

function shellQuote(value: string): string {
  return `'${value.replaceAll("'", "'\\''")}'`;
}

function readableDefaultKeyName(): string {
  const date = new Intl.DateTimeFormat("en-US", {
    month: "short",
    day: "numeric",
    year: "numeric",
  }).format(new Date());
  return `Project key - ${date}`;
}

function projectAgentOptions(
  project: RuntimeProject,
  endpoints: AgentEndpoint[],
  bindings: AgentBinding[],
): ApiKeyAgentOption[] {
  const projectBindings = new Set(project.bindingIds);
  return bindings
    .filter((binding) => projectBindings.has(binding.id))
    .map((binding) => {
      const endpoint = endpoints.find((item) => item.bindingId === binding.id);
      return {
        bindingId: binding.id,
        name: endpoint?.name ?? binding.agentId,
        slug: endpoint?.slug ?? binding.agentId,
      };
    })
    .sort((left, right) => left.name.localeCompare(right.name));
}

function agentScopePayload(mode: ApiKeyAgentScope["mode"], bindingIds: string[]): ApiKeyAgentScope {
  return mode === "all" ? { mode: "all" } : { mode: "selected", bindingIds };
}

function selectedScopeBindingIds(scope: ApiKeyAgentScope): string[] {
  return scope.mode === "selected" ? scope.bindingIds : [];
}

function defaultApiKeyPermissions(): ApiKeyPermissions {
  return {
    chatCompletions: "access",
    agents: "none",
    project: "none",
  };
}

function formatPermissions(permissions: ApiKeyPermissions): string {
  const enabled: string[] = [];
  if (permissions.chatCompletions === "access") enabled.push("Chat completions");
  if (permissions.agents !== "none") enabled.push(`Agents ${permissions.agents}`);
  if (permissions.project !== "none") enabled.push(`Project ${permissions.project}`);
  return enabled.length > 0 ? enabled.join(", ") : "No access";
}

function formatAgentScope(scope: ApiKeyAgentScope, options: ApiKeyAgentOption[]): string {
  if (scope.mode === "all") return "All agents";
  if (scope.bindingIds.length === 1) {
    return options.find((option) => option.bindingId === scope.bindingIds[0])?.name ?? "1 selected agent";
  }
  return `${scope.bindingIds.length} selected agents`;
}

function formatDate(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.valueOf())) return value;
  return new Intl.DateTimeFormat("en-US", {
    month: "short",
    day: "numeric",
    year: "numeric",
    timeZone: "UTC",
  }).format(date);
}

async function runtimeRequest<T>(path: string, method: string, body: unknown): Promise<T> {
  const response = await fetch(`/api/runtime/${path}`, {
    method,
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  const payload = await response.json().catch(() => null) as T | { error?: { message?: unknown } } | null;
  if (!response.ok) {
    const message = payload && typeof payload === "object" && "error" in payload
      ? (payload as { error?: { message?: unknown } }).error?.message
      : null;
    throw new Error(typeof message === "string" ? message : "Runtime request failed.");
  }
  return (payload ?? {}) as T;
}
