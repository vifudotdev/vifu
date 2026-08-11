"use client";

import {
  Check,
  Clipboard,
  CloudUpload,
  Download,
  Link2,
  Plus,
  RotateCcw,
  Settings2,
  ShieldOff,
  Star,
  Unplug,
} from "lucide-react";
import { useMemo, useState, type ChangeEvent, type FormEvent } from "react";
import { useRouter } from "next/navigation";
import { runtimeBrowserRequest } from "../lib/runtime-browser-client";
import type {
  ProjectSettings,
  ProjectRuntimeRelease,
  RuntimeDeployment,
  RuntimeProject,
} from "../lib/runtime-types";

type Enrollment = {
  deployment: string;
  enrollmentToken: string;
  expiresAt: string;
};

export function RuntimeDeploymentsView({
  project,
  deployments,
  releases,
}: {
  project: RuntimeProject;
  deployments: RuntimeDeployment[];
  releases: ProjectRuntimeRelease[];
}) {
  const router = useRouter();
  const latestRelease = useMemo(
    () => [...releases].sort((left, right) => right.version - left.version)[0],
    [releases],
  );
  const [settingsSource, setSettingsSource] = useState(() => formatProjectSettings(
    latestRelease?.manifest ?? emptyProjectSettings(project.slug),
  ));
  const [enrollment, setEnrollment] = useState<Enrollment | null>(null);
  const [pending, setPending] = useState<string | null>(null);
  const [message, setMessage] = useState<{ tone: "error" | "success"; text: string } | null>(null);

  async function action<T>(key: string, work: () => Promise<T>, success: string): Promise<T | null> {
    setPending(key);
    setMessage(null);
    try {
      const result = await work();
      setMessage({ tone: "success", text: success });
      router.refresh();
      return result;
    } catch (error) {
      setMessage({ tone: "error", text: error instanceof Error ? error.message : "Request failed." });
      return null;
    } finally {
      setPending(null);
    }
  }

  async function createDeployment(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const form = event.currentTarget;
    const data = new FormData(form);
    const name = String(data.get("name") ?? "").trim();
    const created = await action(
      "create",
      () => runtimeBrowserRequest(`apps/${project.slug}/deployments`, "POST", {
        name,
        configSyncEnabled: true,
        traceMode: "summary",
        remoteInvocationEnabled: false,
      }),
      `Deployment ${name} created.`,
    );
    if (created) form.reset();
  }

  async function importProjectSettings() {
    let settings: ProjectSettings;
    try {
      settings = JSON.parse(settingsSource) as ProjectSettings;
    } catch {
      setMessage({ tone: "error", text: "project settings JSON is not valid." });
      return;
    }
    const result = await action(
      "import-settings",
      () => runtimeBrowserRequest<{ release: ProjectRuntimeRelease }>(
        `apps/${project.slug}/runtime-releases`,
        "POST",
        { settings },
      ),
      "project settings imported.",
    );
    if (result?.release) setSettingsSource(formatProjectSettings(result.release.manifest));
  }

  function exportProjectSettings() {
    const settings = latestRelease?.manifest;
    if (!settings) {
      setMessage({ tone: "error", text: "There are no saved project settings to export." });
      return;
    }
    const blob = new Blob([formatProjectSettings(settings)], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const anchor = document.createElement("a");
    anchor.href = url;
    anchor.download = `${project.slug}.project-settings.vifu.json`;
    document.body.append(anchor);
    anchor.click();
    anchor.remove();
    window.setTimeout(() => URL.revokeObjectURL(url), 0);
    setMessage({ tone: "success", text: "project settings exported." });
  }

  async function loadProjectSettingsFile(event: ChangeEvent<HTMLInputElement>) {
    const file = event.currentTarget.files?.[0];
    event.currentTarget.value = "";
    if (!file) return;
    try {
      setSettingsSource(await file.text());
      setMessage({ tone: "success", text: "project settings file loaded." });
    } catch {
      setMessage({ tone: "error", text: "project settings file could not be read." });
    }
  }

  async function pairGateway(deployment: RuntimeDeployment) {
    const result = await action(
      `pair-${deployment.id}`,
      () => runtimeBrowserRequest<Enrollment>(
        `apps/${project.slug}/deployments/${deployment.name}/agent-gateway-enrollments`,
        "POST",
      ),
      "Pairing token created.",
    );
    if (result) setEnrollment(result);
  }

  async function activate(deployment: RuntimeDeployment, version: number) {
    await action(
      `activate-${deployment.id}-${version}`,
      () => runtimeBrowserRequest(
        `apps/${project.slug}/deployments/${deployment.name}/runtime-releases/${version}/activate`,
        "POST",
      ),
      `${deployment.name} now uses settings version ${version}.`,
    );
  }

  async function detachGateway(deployment: RuntimeDeployment, gatewayId: string) {
    await action(
      `detach-${deployment.id}-${gatewayId}`,
      () => runtimeBrowserRequest(
        `apps/${project.slug}/deployments/${deployment.name}/agent-gateways/${gatewayId}`,
        "DELETE",
      ),
      "Gateway detached from this deployment.",
    );
  }

  async function revokeGateway(gatewayId: string) {
    if (!window.confirm("Revoke this Gateway? It will disconnect and require approval before it can reconnect.")) return;
    await action(
      `revoke-${gatewayId}`,
      () => runtimeBrowserRequest(`agent-gateways/${gatewayId}/revoke`, "POST"),
      "Gateway access revoked.",
    );
  }

  async function updatePolicies(deployment: RuntimeDeployment, event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const data = new FormData(event.currentTarget);
    await action(
      `settings-${deployment.id}`,
      () => runtimeBrowserRequest(
        `apps/${project.slug}/deployments/${deployment.name}`,
        "PATCH",
        {
          configSyncEnabled: data.get("configSyncEnabled") === "on",
          traceMode: String(data.get("traceMode") ?? "summary"),
          remoteInvocationEnabled: data.get("remoteInvocationEnabled") === "on",
        },
      ),
      `${deployment.name} settings updated.`,
    );
  }

  async function promote(deployment: RuntimeDeployment) {
    await action(
      `promote-${deployment.id}`,
      () => runtimeBrowserRequest(
        `apps/${project.slug}/deployments/${deployment.name}/promote`,
        "POST",
      ),
      `${deployment.name} is now the primary deployment.`,
    );
  }

  return (
    <div className="deployment-workbench">
      <section className="deployment-toolbar">
        <form onSubmit={createDeployment}>
          <label><span>New deployment</span><input name="name" required maxLength={64} placeholder="staging" /></label>
          <button className="secondary-button" type="submit" disabled={pending === "create"}>
            <Plus aria-hidden="true" />{pending === "create" ? "Creating" : "Create"}
          </button>
        </form>
        <div className="deployment-summary">
          <span><strong>{deployments.length}</strong> environments</span>
          <span><strong>{releases.length}</strong> settings versions</span>
        </div>
      </section>

      {message ? <div className={`action-message deployment-message ${message.tone}`} role={message.tone === "error" ? "alert" : "status"}>{message.text}</div> : null}

      <section className="deployment-grid" aria-label="Runtime deployments">
        {deployments.map((deployment) => (
          <article className="deployment-card" key={deployment.id}>
            <header>
              <div>
                <span className="deployment-icon"><Settings2 aria-hidden="true" /></span>
                <div><strong>{deployment.name}</strong><small>{deployment.gatewayIds.length} paired gateways</small></div>
              </div>
              {deployment.isPrimary ? <span className="deployment-primary"><Star aria-hidden="true" />Primary</span> : null}
            </header>
            <dl>
              <div><dt>Settings</dt><dd>{deployment.activeReleaseVersion ? `v${deployment.activeReleaseVersion}` : "Not set"}</dd></div>
              <div><dt>Config</dt><dd>{deployment.configSyncEnabled ? "Synced" : "Local"}</dd></div>
              <div><dt>Traces</dt><dd>{deployment.traceMode}</dd></div>
              <div><dt>Remote calls</dt><dd>{deployment.remoteInvocationEnabled ? "Allowed" : "Blocked"}</dd></div>
            </dl>
            {deployment.gatewayIds.length > 0 ? (
              <div className="deployment-gateways">
                {deployment.gatewayIds.map((gatewayId) => (
                  <div key={gatewayId}>
                    <code>{gatewayId}</code>
                    <button className="icon-button" type="button" title="Detach from deployment" aria-label={`Detach ${gatewayId}`} onClick={() => detachGateway(deployment, gatewayId)} disabled={pending !== null}><Unplug aria-hidden="true" /></button>
                    <button className="icon-button danger" type="button" title="Revoke gateway" aria-label={`Revoke ${gatewayId}`} onClick={() => revokeGateway(gatewayId)} disabled={pending !== null}><ShieldOff aria-hidden="true" /></button>
                  </div>
                ))}
              </div>
            ) : null}
            <form className="deployment-policy-form" onSubmit={(event) => updatePolicies(deployment, event)}>
              <label><input type="checkbox" name="configSyncEnabled" defaultChecked={deployment.configSyncEnabled} />Sync settings</label>
              <label><input type="checkbox" name="remoteInvocationEnabled" defaultChecked={deployment.remoteInvocationEnabled} />Allow remote calls</label>
              <label><span>Trace upload</span><select name="traceMode" defaultValue={deployment.traceMode === "full" ? "summary" : deployment.traceMode}><option value="off">Off</option><option value="summary">Summary</option></select></label>
              <button className="icon-text-button" type="submit" disabled={pending === `settings-${deployment.id}`}><Check aria-hidden="true" />Save</button>
            </form>
            <footer>
              <button className="secondary-button" type="button" onClick={() => pairGateway(deployment)} disabled={pending === `pair-${deployment.id}`}><Link2 aria-hidden="true" />Pair gateway</button>
              {!deployment.isPrimary ? <button className="quiet-button" type="button" onClick={() => promote(deployment)} disabled={pending === `promote-${deployment.id}`}><Star aria-hidden="true" />Make primary</button> : null}
            </footer>
          </article>
        ))}
      </section>

      {enrollment ? <EnrollmentPanel enrollment={enrollment} onClose={() => setEnrollment(null)} /> : null}

      <section className="release-workbench">
        <header>
          <div><h2>project settings</h2><p>Database-backed provider, agent, and endpoint settings.</p></div>
          <div className="settings-artifact-actions">
            <label className="secondary-button settings-file-button">
              <CloudUpload aria-hidden="true" />Load JSON
              <input type="file" accept="application/json,.json" onChange={loadProjectSettingsFile} />
            </label>
            <button className="secondary-button" type="button" onClick={exportProjectSettings} disabled={!latestRelease}><Download aria-hidden="true" />Export</button>
            <button className="primary-button" type="button" onClick={importProjectSettings} disabled={pending === "import-settings"}><CloudUpload aria-hidden="true" />{pending === "import-settings" ? "Importing" : "Import"}</button>
          </div>
        </header>
        <textarea value={settingsSource} onChange={(event) => setSettingsSource(event.target.value)} spellCheck={false} aria-label="project settings JSON" />
        <div className="release-list">
          {releases.length > 0 ? releases.map((release) => (
            <article key={release.id}>
              <div><strong>Settings version {release.version}</strong><code>{shortHash(release.contentHash)}</code><time dateTime={release.createdAt}>{formatDate(release.createdAt)}</time></div>
              <div className="release-targets">
                {deployments.map((deployment) => deployment.activeReleaseVersion === release.version ? (
                  <span key={deployment.id}><Check aria-hidden="true" />{deployment.name}</span>
                ) : (
                  <button key={deployment.id} type="button" onClick={() => activate(deployment, release.version)} disabled={pending === `activate-${deployment.id}-${release.version}`}><RotateCcw aria-hidden="true" />Use in {deployment.name}</button>
                ))}
              </div>
            </article>
          )) : <div className="deployment-empty">No saved project settings yet.</div>}
        </div>
      </section>
    </div>
  );
}

function EnrollmentPanel({ enrollment, onClose }: { enrollment: Enrollment; onClose: () => void }) {
  const [copied, setCopied] = useState(false);
  const [copyFailed, setCopyFailed] = useState(false);
  async function copyToken() {
    try {
      await navigator.clipboard.writeText(enrollment.enrollmentToken);
      setCopied(true);
      setCopyFailed(false);
    } catch {
      setCopyFailed(true);
    }
  }
  return (
    <section className="enrollment-panel" role="status">
      <div><strong>Pair with {enrollment.deployment}</strong><p>Use this one-time token on the device within five minutes.</p></div>
      <code>{enrollment.enrollmentToken}</code>
      <div><button className="secondary-button" type="button" onClick={copyToken}>{copied ? <Check aria-hidden="true" /> : <Clipboard aria-hidden="true" />}{copied ? "Copied" : "Copy token"}</button><button className="quiet-button" type="button" onClick={onClose}>Done</button></div>
      {copyFailed ? <p role="alert">Copy failed. Select the token above and copy it manually.</p> : null}
    </section>
  );
}

function emptyProjectSettings(projectId: string): ProjectSettings {
  return { schemaVersion: 1, projectId, providers: [], agents: [], endpoints: [], metadata: {} };
}

function formatProjectSettings(settings: ProjectSettings): string {
  return `${JSON.stringify(settings, null, 2)}\n`;
}

function shortHash(value: string): string {
  return value.length > 22 ? `${value.slice(0, 18)}...` : value;
}

function formatDate(value: string): string {
  const date = new Date(value);
  if (!Number.isFinite(date.getTime())) return "Recently";
  return new Intl.DateTimeFormat("en", { month: "short", day: "numeric", year: "numeric", timeZone: "UTC" }).format(date);
}
