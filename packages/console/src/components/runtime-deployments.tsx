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
import { useEffect, useMemo, useState, type ChangeEvent, type FormEvent } from "react";
import { useRuntimeConsoleHost, useRuntimeConsoleRouter } from "../host";
import type {
  AgentGateway,
  ProjectSettings,
  ProjectRuntimeRelease,
  RuntimeDeployment,
  RuntimeProject,
} from "../types";

export type GatewayPairing = {
  serverUrl: string;
  certificateDer?: string | null;
  certificateSha256?: string | null;
  pairingUri: string;
  pairingDeepLink: string;
  pairingQrSvg?: string | null;
};

type Enrollment = {
  enrollmentId: string;
  deployment: string;
  enrollmentToken: string;
  expiresAt: string;
  pairing?: GatewayPairing | null;
};

export const MAX_APPLY_POLL_ATTEMPTS = 6;
export const ENROLLMENT_REFRESH_MS = 2_000;
export const GATEWAY_STATUS_REFRESH_MS = 5_000;

export function nativeGatewayPairingCode(pairing: GatewayPairing): string {
  return pairing.pairingDeepLink;
}

export function runtimeApplyPollDelay(attempt: number): number {
  return Math.min(2_000 * (2 ** Math.max(0, attempt)), 30_000);
}

export function runtimeApplyTarget(deployments: RuntimeDeployment[]): string {
  return deployments.map((deployment) => [
    deployment.id,
    deployment.activeReleaseVersion ?? "none",
    [...deployment.gatewayIds].sort().join(","),
    [...(deployment.applyStates ?? [])]
      .sort((left, right) => left.gatewayId.localeCompare(right.gatewayId))
      .map((state) => `${state.gatewayId}:${state.releaseVersion}:${state.contentHash}`)
      .join(","),
  ].join("|")).sort().join(";");
}

export function latestGatewaySession(
  gatewayId: string,
  gateways: AgentGateway[],
): AgentGateway | undefined {
  return gateways
    .filter((gateway) => gateway.gatewayId === gatewayId)
    .sort((left, right) => {
      const connectionOrder = Number(right.status === "connected") - Number(left.status === "connected");
      if (connectionOrder !== 0) return connectionOrder;
      return Date.parse(right.lastSeenAt) - Date.parse(left.lastSeenAt);
    })[0];
}

export function RuntimeDeploymentsView({
  project,
  deployments,
  releases,
  agentGateways,
}: {
  project: RuntimeProject;
  deployments: RuntimeDeployment[];
  releases: ProjectRuntimeRelease[];
  agentGateways: AgentGateway[];
}) {
  const host = useRuntimeConsoleHost();
  const router = useRuntimeConsoleRouter();
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
  const supportsApplyState = deployments.some((deployment) => deployment.applyStates !== undefined);
  const waitingForApply = supportsApplyState && deployments.some((deployment) =>
    deployment.activeReleaseVersion !== null
    && deployment.gatewayIds.some((gatewayId) => !(deployment.applyStates ?? []).some(
      (state) => state.gatewayId === gatewayId
        && state.releaseVersion === deployment.activeReleaseVersion,
    )),
  );
  const applyTarget = useMemo(() => runtimeApplyTarget(deployments), [deployments]);
  const [applyPoll, setApplyPoll] = useState({ target: applyTarget, attempt: 0 });
  const hasAssignedGateways = deployments.some((deployment) => deployment.gatewayIds.length > 0);
  const enrollmentIsActive = Boolean(
    enrollment && Date.parse(enrollment.expiresAt) > Date.now(),
  );

  useEffect(() => {
    if (!waitingForApply) {
      if (applyPoll.attempt !== 0 || applyPoll.target !== applyTarget) {
        setApplyPoll({ target: applyTarget, attempt: 0 });
      }
      return;
    }
    if (applyPoll.target !== applyTarget) {
      setApplyPoll({ target: applyTarget, attempt: 0 });
      return;
    }
    if (applyPoll.attempt >= MAX_APPLY_POLL_ATTEMPTS) return;
    const timer = window.setTimeout(() => {
      setApplyPoll((current) => current.target === applyTarget
        ? { ...current, attempt: current.attempt + 1 }
        : { target: applyTarget, attempt: 1 });
      router.refresh();
    }, runtimeApplyPollDelay(applyPoll.attempt));
    return () => window.clearTimeout(timer);
  }, [router, waitingForApply, applyTarget, applyPoll]);

  useEffect(() => {
    if (!enrollmentIsActive && !hasAssignedGateways) return;
    const timer = window.setInterval(
      () => router.refresh(),
      enrollmentIsActive ? ENROLLMENT_REFRESH_MS : GATEWAY_STATUS_REFRESH_MS,
    );
    return () => window.clearInterval(timer);
  }, [enrollmentIsActive, hasAssignedGateways, router]);

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
      () => host.request(`apps/${project.slug}/deployments`, "POST", {
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
      setMessage({ tone: "error", text: "App settings JSON is not valid." });
      return;
    }
    const result = await action(
      "import-settings",
      () => host.request<{ release: ProjectRuntimeRelease }>(
        `apps/${project.slug}/runtime-releases`,
        "POST",
        { settings },
      ),
      "App settings imported.",
    );
    if (result?.release) setSettingsSource(formatProjectSettings(result.release.manifest));
  }

  function exportProjectSettings() {
    const settings = latestRelease?.manifest;
    if (!settings) {
      setMessage({ tone: "error", text: "There are no saved app settings to export." });
      return;
    }
    const blob = new Blob([formatProjectSettings(settings)], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const anchor = document.createElement("a");
    anchor.href = url;
    anchor.download = `${project.slug}.app-settings.vifu.json`;
    document.body.append(anchor);
    anchor.click();
    anchor.remove();
    window.setTimeout(() => URL.revokeObjectURL(url), 0);
    setMessage({ tone: "success", text: "App settings exported." });
  }

  async function loadProjectSettingsFile(event: ChangeEvent<HTMLInputElement>) {
    const file = event.currentTarget.files?.[0];
    event.currentTarget.value = "";
    if (!file) return;
    try {
      setSettingsSource(await file.text());
      setMessage({ tone: "success", text: "App settings file loaded." });
    } catch {
      setMessage({ tone: "error", text: "App settings file could not be read." });
    }
  }

  async function pairGateway(deployment: RuntimeDeployment) {
    const result = await action(
      `pair-${deployment.id}`,
      () => host.request<Enrollment>(
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
      () => host.request(
        `apps/${project.slug}/deployments/${deployment.name}/runtime-releases/${version}/activate`,
        "POST",
      ),
      `${deployment.name} now uses settings version ${version}.`,
    );
  }

  async function detachGateway(deployment: RuntimeDeployment, gatewayId: string) {
    await action(
      `detach-${deployment.id}-${gatewayId}`,
      () => host.request(
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
      () => host.request(`agent-gateways/${gatewayId}/revoke`, "POST"),
      "Gateway access revoked.",
    );
  }

  async function updatePolicies(deployment: RuntimeDeployment, event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const data = new FormData(event.currentTarget);
    await action(
      `settings-${deployment.id}`,
      () => host.request(
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
      () => host.request(
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
                    <GatewayConnectionStatus gateway={latestGatewaySession(gatewayId, agentGateways)} />
                    <GatewayApplyStatus deployment={deployment} gatewayId={gatewayId} />
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
          <div><h2>App settings</h2><p>Database-backed provider, agent, and endpoint settings.</p></div>
          <div className="settings-artifact-actions">
            <label className="secondary-button settings-file-button">
              <CloudUpload aria-hidden="true" />Load JSON
              <input type="file" accept="application/json,.json" onChange={loadProjectSettingsFile} />
            </label>
            <button className="secondary-button" type="button" onClick={exportProjectSettings} disabled={!latestRelease}><Download aria-hidden="true" />Export</button>
            <button className="primary-button" type="button" onClick={importProjectSettings} disabled={pending === "import-settings"}><CloudUpload aria-hidden="true" />{pending === "import-settings" ? "Importing" : "Import"}</button>
          </div>
        </header>
        <textarea value={settingsSource} onChange={(event) => setSettingsSource(event.target.value)} spellCheck={false} aria-label="App settings JSON" />
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
          )) : <div className="deployment-empty">No saved app settings yet.</div>}
        </div>
      </section>
    </div>
  );
}

function GatewayConnectionStatus({ gateway }: { gateway?: AgentGateway }) {
  const connected = gateway?.status === "connected";
  return (
    <span className={`deployment-gateway-connection ${connected ? "online" : "offline"}`}>
      <strong><i aria-hidden="true" />{connected ? "Online" : "Offline"}</strong>
      <small>
        {gateway
          ? connected
            ? `${gateway.agents.length} ${gateway.agents.length === 1 ? "agent" : "agents"}`
            : `Last seen ${formatDate(gateway.lastSeenAt)}`
          : "No session yet"}
      </small>
    </span>
  );
}

function GatewayApplyStatus({ deployment, gatewayId }: { deployment: RuntimeDeployment; gatewayId: string }) {
  const applied = (deployment.applyStates ?? []).find((state) => state.gatewayId === gatewayId);
  const current = deployment.activeReleaseVersion;
  const isCurrent = current !== null && applied?.releaseVersion === current;
  return (
    <span className={`gateway-apply-status ${isCurrent ? "applied" : "pending"}`}>
      {isCurrent ? `Applied v${current}` : current ? `Waiting for v${current}` : "No settings"}
    </span>
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
  async function copyPairingCode() {
    if (!enrollment.pairing) return;
    try {
      await navigator.clipboard.writeText(nativeGatewayPairingCode(enrollment.pairing));
      setCopied(true);
      setCopyFailed(false);
    } catch {
      setCopyFailed(true);
    }
  }
  const qrSource = enrollment.pairing?.pairingQrSvg
    ? `data:image/svg+xml,${encodeURIComponent(enrollment.pairing.pairingQrSvg)}`
    : null;
  return (
    <section className={`enrollment-panel${qrSource ? " has-qr" : ""}`} role="status">
      <div><strong>Pair with {enrollment.deployment}</strong><p>Copy this one-time code into the mobile Starter within five minutes. The QR supports configured application-link bridges.</p></div>
      {qrSource ? <img className="enrollment-qr" src={qrSource} alt="Vifu Server pairing code" /> : null}
      <code>{enrollment.pairing ? nativeGatewayPairingCode(enrollment.pairing) : enrollment.enrollmentToken}</code>
      <div><button className="secondary-button" type="button" onClick={enrollment.pairing ? copyPairingCode : copyToken}>{copied ? <Check aria-hidden="true" /> : <Clipboard aria-hidden="true" />}{copied ? "Copied" : enrollment.pairing ? "Copy pairing code" : "Copy token"}</button><button className="quiet-button" type="button" onClick={onClose}>Done</button></div>
      {enrollment.pairing?.certificateSha256 ? <p className="enrollment-fingerprint">TLS {enrollment.pairing.certificateSha256}</p> : enrollment.pairing ? <p>HTTPS uses the device system trust store.</p> : <p>Enter this token together with the Vifu Server URL.</p>}
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
