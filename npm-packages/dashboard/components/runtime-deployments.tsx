"use client";

import {
  Check,
  Clipboard,
  CloudUpload,
  Link2,
  Plus,
  RotateCcw,
  Settings2,
  Star,
} from "lucide-react";
import { useMemo, useState, type FormEvent } from "react";
import { useRouter } from "next/navigation";
import { runtimeBrowserRequest } from "../lib/runtime-browser-client";
import type {
  ProjectRuntimeRelease,
  RuntimeDeployment,
  RuntimeManifest,
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
  const [manifestSource, setManifestSource] = useState(() => JSON.stringify(
    latestRelease?.manifest ?? emptyManifest(project.slug),
    null,
    2,
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
      () => runtimeBrowserRequest(`project/${project.slug}/deployments`, "POST", {
        name,
        configSyncEnabled: true,
        traceMode: "summary",
        remoteInvocationEnabled: false,
      }),
      `Deployment ${name} created.`,
    );
    if (created) form.reset();
  }

  async function publishRelease() {
    let manifest: RuntimeManifest;
    try {
      manifest = JSON.parse(manifestSource) as RuntimeManifest;
    } catch {
      setMessage({ tone: "error", text: "The runtime manifest is not valid JSON." });
      return;
    }
    await action(
      "publish",
      () => runtimeBrowserRequest<{ release: ProjectRuntimeRelease }>(
        `project/${project.slug}/runtime-releases`,
        "POST",
        { manifest },
      ),
      "Runtime release published.",
    );
  }

  async function pairGateway(deployment: RuntimeDeployment) {
    const result = await action(
      `pair-${deployment.id}`,
      () => runtimeBrowserRequest<Enrollment>(
        `project/${project.slug}/deployments/${deployment.name}/agent-gateway-enrollments`,
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
        `project/${project.slug}/deployments/${deployment.name}/runtime-releases/${version}/activate`,
        "POST",
      ),
      `${deployment.name} now uses release ${version}.`,
    );
  }

  async function updatePolicies(deployment: RuntimeDeployment, event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const data = new FormData(event.currentTarget);
    await action(
      `settings-${deployment.id}`,
      () => runtimeBrowserRequest(
        `project/${project.slug}/deployments/${deployment.name}`,
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
        `project/${project.slug}/deployments/${deployment.name}/promote`,
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
          <span><strong>{releases.length}</strong> releases</span>
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
              <div><dt>Release</dt><dd>{deployment.activeReleaseVersion ? `v${deployment.activeReleaseVersion}` : "Not published"}</dd></div>
              <div><dt>Config</dt><dd>{deployment.configSyncEnabled ? "Synced" : "Local"}</dd></div>
              <div><dt>Traces</dt><dd>{deployment.traceMode}</dd></div>
              <div><dt>Remote calls</dt><dd>{deployment.remoteInvocationEnabled ? "Allowed" : "Blocked"}</dd></div>
            </dl>
            {deployment.gatewayIds.length > 0 ? (
              <div className="deployment-gateways">
                {deployment.gatewayIds.map((gatewayId) => <code key={gatewayId}>{gatewayId}</code>)}
              </div>
            ) : null}
            <form className="deployment-policy-form" onSubmit={(event) => updatePolicies(deployment, event)}>
              <label><input type="checkbox" name="configSyncEnabled" defaultChecked={deployment.configSyncEnabled} />Sync releases</label>
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
        <header><div><h2>Runtime releases</h2><p>Publish one portable manifest, then choose the version used by each deployment.</p></div><button className="primary-button" type="button" onClick={publishRelease} disabled={pending === "publish"}><CloudUpload aria-hidden="true" />{pending === "publish" ? "Publishing" : "Publish release"}</button></header>
        <textarea value={manifestSource} onChange={(event) => setManifestSource(event.target.value)} spellCheck={false} aria-label="Runtime manifest JSON" />
        <div className="release-list">
          {releases.length > 0 ? releases.map((release) => (
            <article key={release.id}>
              <div><strong>Release {release.version}</strong><code>{shortHash(release.contentHash)}</code><time dateTime={release.createdAt}>{formatDate(release.createdAt)}</time></div>
              <div className="release-targets">
                {deployments.map((deployment) => deployment.activeReleaseVersion === release.version ? (
                  <span key={deployment.id}><Check aria-hidden="true" />{deployment.name}</span>
                ) : (
                  <button key={deployment.id} type="button" onClick={() => activate(deployment, release.version)} disabled={pending === `activate-${deployment.id}-${release.version}`}><RotateCcw aria-hidden="true" />Use in {deployment.name}</button>
                ))}
              </div>
            </article>
          )) : <div className="deployment-empty">Connect an embedded runtime to import the first release, or publish a manifest here.</div>}
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

function emptyManifest(projectId: string): RuntimeManifest {
  return { schemaVersion: 1, projectId, providers: [], agents: [], endpoints: [], metadata: {} };
}

function shortHash(value: string): string {
  return value.length > 22 ? `${value.slice(0, 18)}...` : value;
}

function formatDate(value: string): string {
  const date = new Date(value);
  if (!Number.isFinite(date.getTime())) return "Recently";
  return new Intl.DateTimeFormat("en", { month: "short", day: "numeric", year: "numeric", timeZone: "UTC" }).format(date);
}
