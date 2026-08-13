"use client";

import {
  Box,
  Check,
  Clipboard,
  CloudUpload,
  Download,
  Link2,
  Monitor,
  Plus,
  RotateCcw,
  Settings2,
  ShieldOff,
  Smartphone,
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
import { DEVICE_STATUS_REFRESH_MS, useRuntimeLiveRefresh } from "./runtime-live-refresh";

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
export const GATEWAY_STATUS_REFRESH_MS = DEVICE_STATUS_REFRESH_MS;

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

export function gatewayDeploymentPresentation(
  gatewayId: string,
  gateways: AgentGateway[],
) {
  const gateway = latestGatewaySession(gatewayId, gateways);
  const metadata = gateway?.metadata ?? {};
  const device = recordValue(metadata.device);
  const application = recordValue(metadata.application);
  const kind = stringValue(metadata.kind).trim();
  const platform = stringValue(metadata.platform).trim();
  const manufacturer = stringValue(device.manufacturer).trim();
  const deviceName = firstString(device.model, device.hostname, device.product);
  const applicationName = stringValue(application.name).trim();
  const applicationVersion = stringValue(application.version).trim();

  return {
    gateway,
    gatewayId,
    name: stringValue(metadata.name).trim() || fallbackGatewayName(gatewayId),
    typeLabel: gateway
      ? [platformLabel(platform), kindLabel(kind)].filter(Boolean).join(" ") || "Gateway device"
      : "Device identity pending",
    deviceLabel: [manufacturer, deviceName].filter(Boolean).join(" ")
      || (gateway ? "Device details not reported" : "Waiting for this device to connect"),
    applicationLabel: applicationName || applicationVersion
      ? [applicationName, applicationVersion ? versionLabel(applicationVersion) : ""].filter(Boolean).join(" · ")
      : null,
    agentLabel: gateway
      ? `${gateway.agents.length} ${gateway.agents.length === 1 ? "agent" : "agents"}`
      : "No agents reported",
    kind,
    platform,
  };
}

export function primaryRuntimeDeployment(
  deployments: RuntimeDeployment[],
): RuntimeDeployment | undefined {
  return deployments.find((deployment) => deployment.isPrimary) ?? deployments[0];
}

export function runtimeDeviceGatewayIds(
  deployments: RuntimeDeployment[],
  gateways: AgentGateway[],
): string[] {
  const gatewayIds = new Set(deployments.flatMap((deployment) => deployment.gatewayIds));
  for (const gateway of gateways) gatewayIds.add(gateway.gatewayId);
  return [...gatewayIds];
}

export function RuntimeDevicesView({
  project,
  deployments,
  agentGateways,
}: {
  project: RuntimeProject;
  deployments: RuntimeDeployment[];
  agentGateways: AgentGateway[];
}) {
  const host = useRuntimeConsoleHost();
  const router = useRuntimeConsoleRouter();
  const [pendingGatewayId, setPendingGatewayId] = useState<string | null>(null);
  const [message, setMessage] = useState<{ tone: "error" | "success"; text: string } | null>(null);
  const gatewayIds = runtimeDeviceGatewayIds(deployments, agentGateways);
  const devices = gatewayIds
    .map((gatewayId) => ({
      presentation: gatewayDeploymentPresentation(gatewayId, agentGateways),
      deployments: deployments.filter((deployment) => deployment.gatewayIds.includes(gatewayId)),
    }))
    .sort((left, right) => gatewayConnectionRank(left.presentation.gateway?.status)
      - gatewayConnectionRank(right.presentation.gateway?.status)
      || left.presentation.name.localeCompare(right.presentation.name));
  const connectedDevices = devices.filter(({ presentation }) => presentation.gateway?.status === "connected").length;

  useRuntimeLiveRefresh(true);

  async function revokeGateway(gatewayId: string, name: string) {
    if (!window.confirm(`Revoke access for ${name}? The device will need to pair again.`)) return;
    setPendingGatewayId(gatewayId);
    setMessage(null);
    try {
      await host.request(`agent-gateways/${gatewayId}/revoke`, "POST");
      setMessage({ tone: "success", text: `${name} access revoked.` });
      router.refresh();
    } catch (error) {
      setMessage({ tone: "error", text: error instanceof Error ? error.message : "Device access could not be revoked." });
    } finally {
      setPendingGatewayId(null);
    }
  }

  return (
    <div className="devices-workbench">
      <section className={`device-connect-rail${connectedDevices > 0 ? " has-devices" : ""}`}>
        <div className="device-connect-copy">
          <span className="device-connect-signal" aria-hidden="true"><i /></span>
          <div>
            <span>{connectedDevices > 0 ? "Device network" : "First connection"}</span>
            <strong>{connectedDevices > 0 ? `${connectedDevices} ${connectedDevices === 1 ? "device" : "devices"} online` : "Pair your first device"}</strong>
            <p>{connectedDevices > 0 ? "Vifu is receiving live runtime status from your paired devices." : "Scan one code from the Android, iOS, desktop, or embedded Starter."}</p>
          </div>
        </div>
        <DevicePairingAction project={project} deployments={deployments} />
      </section>

      {message ? <div className={`action-message deployment-message ${message.tone}`} role={message.tone === "error" ? "alert" : "status"}>{message.text}</div> : null}

      <section className="device-inventory" aria-label="Paired devices">
        <header>
          <div><h2>Paired devices</h2><p>Phones, computers, and embedded Gateways connected to this app.</p></div>
          <span><strong>{connectedDevices}</strong> online · <strong>{devices.length}</strong> total</span>
        </header>
        {devices.length > 0 ? (
          <div className="device-inventory-grid">
            {devices.map(({ presentation, deployments: assignedDeployments }) => (
              <DeviceGatewayCard
                key={presentation.gatewayId}
                presentation={presentation}
                deployments={assignedDeployments}
                showEnvironments={deployments.length > 1}
                pending={pendingGatewayId === presentation.gatewayId}
                onRevoke={() => revokeGateway(presentation.gatewayId, presentation.name)}
              />
            ))}
          </div>
        ) : (
          <div className="device-inventory-empty">
            <Smartphone aria-hidden="true" />
            <strong>No devices paired</strong>
            <span>Use Pair device above. This page will update when the device connects.</span>
          </div>
        )}
      </section>
    </div>
  );
}

export function DevicePairingAction({
  project,
  deployments,
}: {
  project: RuntimeProject;
  deployments: RuntimeDeployment[];
}) {
  const host = useRuntimeConsoleHost();
  const router = useRuntimeConsoleRouter();
  const primaryDeployment = primaryRuntimeDeployment(deployments);
  const [deploymentName, setDeploymentName] = useState(primaryDeployment?.name ?? "");
  const [enrollment, setEnrollment] = useState<Enrollment | null>(null);
  const [pending, setPending] = useState(false);
  const [message, setMessage] = useState<{ tone: "error" | "success"; text: string } | null>(null);
  const multipleEnvironments = deployments.length > 1;

  useEffect(() => {
    if (!deployments.some((deployment) => deployment.name === deploymentName)) {
      setDeploymentName(primaryRuntimeDeployment(deployments)?.name ?? "");
    }
  }, [deploymentName, deployments]);

  useRuntimeLiveRefresh(
    Boolean(enrollment && Date.parse(enrollment.expiresAt) > Date.now()),
    ENROLLMENT_REFRESH_MS,
  );

  async function pairDevice() {
    const deployment = deployments.find((candidate) => candidate.name === deploymentName)
      ?? primaryRuntimeDeployment(deployments);
    if (!deployment) {
      setMessage({ tone: "error", text: "This app has no runtime configuration available for pairing." });
      return;
    }
    setPending(true);
    setMessage(null);
    try {
      const nextEnrollment = await host.request<Enrollment>(
        `apps/${project.slug}/deployments/${deployment.name}/agent-gateway-enrollments`,
        "POST",
      );
      setEnrollment(nextEnrollment);
      setMessage({ tone: "success", text: "Pairing code ready." });
      router.refresh();
    } catch (error) {
      setMessage({ tone: "error", text: error instanceof Error ? error.message : "Pairing code could not be created." });
    } finally {
      setPending(false);
    }
  }

  return (
    <div className="device-pairing-action">
      <div className="device-pairing-controls">
        {multipleEnvironments ? (
          <label>
            <span>Environment</span>
            <select value={deploymentName} onChange={(event) => setDeploymentName(event.target.value)}>
              {deployments.map((deployment) => <option value={deployment.name} key={deployment.id}>{deployment.name}</option>)}
            </select>
          </label>
        ) : null}
        <button className="primary-button" type="button" onClick={pairDevice} disabled={pending || !primaryDeployment}>
          <Link2 aria-hidden="true" />{pending ? "Preparing" : "Pair device"}
        </button>
      </div>
      {message ? <div className={`action-message device-pairing-message ${message.tone}`} role={message.tone === "error" ? "alert" : "status"}>{message.text}</div> : null}
      {enrollment ? <EnrollmentPanel enrollment={enrollment} showEnvironment={multipleEnvironments} onClose={() => setEnrollment(null)} /> : null}
    </div>
  );
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
      `Environment ${name} created.`,
    );
    if (created) form.reset();
  }

  async function importProjectSettings() {
    let settings: ProjectSettings;
    try {
      settings = JSON.parse(settingsSource) as ProjectSettings;
    } catch {
      setMessage({ tone: "error", text: "Configuration release JSON is not valid." });
      return;
    }
    const result = await action(
      "import-settings",
      () => host.request<{ release: ProjectRuntimeRelease }>(
        `apps/${project.slug}/runtime-releases`,
        "POST",
        { settings },
      ),
      "Configuration release imported.",
    );
    if (result?.release) setSettingsSource(formatProjectSettings(result.release.manifest));
  }

  function exportProjectSettings() {
    const settings = latestRelease?.manifest;
    if (!settings) {
      setMessage({ tone: "error", text: "There are no configuration releases to export." });
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
    setMessage({ tone: "success", text: "Configuration release exported." });
  }

  async function loadProjectSettingsFile(event: ChangeEvent<HTMLInputElement>) {
    const file = event.currentTarget.files?.[0];
    event.currentTarget.value = "";
    if (!file) return;
    try {
      setSettingsSource(await file.text());
      setMessage({ tone: "success", text: "Configuration release file loaded." });
    } catch {
      setMessage({ tone: "error", text: "Configuration release file could not be read." });
    }
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
      `${deployment.name} is now the primary environment.`,
    );
  }

  return (
    <div className="deployment-workbench">
      <section className="deployment-toolbar">
        <form onSubmit={createDeployment}>
          <label><span>New environment</span><input name="name" required maxLength={64} placeholder="staging" /></label>
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

      <section className="deployment-grid" aria-label="Advanced runtime environments">
        {deployments.map((deployment) => {
          const gatewayCards = deployment.gatewayIds
            .map((gatewayId) => gatewayDeploymentPresentation(gatewayId, agentGateways))
            .sort((left, right) => gatewayConnectionRank(left.gateway?.status)
              - gatewayConnectionRank(right.gateway?.status)
              || left.name.localeCompare(right.name));
          return (
            <article className="deployment-card" key={deployment.id}>
            <header>
              <div>
                <span className="deployment-icon"><Settings2 aria-hidden="true" /></span>
                <div><strong>{deployment.name}</strong><small>{deployment.gatewayIds.length} paired devices</small></div>
              </div>
              {deployment.isPrimary ? <span className="deployment-primary"><Star aria-hidden="true" />Primary</span> : null}
            </header>
            <dl>
              <div><dt>Settings</dt><dd>{deployment.activeReleaseVersion ? `v${deployment.activeReleaseVersion}` : "Not set"}</dd></div>
              <div><dt>Config</dt><dd>{deployment.configSyncEnabled ? "Synced" : "Local"}</dd></div>
              <div><dt>Traces</dt><dd>{deployment.traceMode}</dd></div>
              <div><dt>Remote calls</dt><dd>{deployment.remoteInvocationEnabled ? "Allowed" : "Blocked"}</dd></div>
            </dl>
            {gatewayCards.length > 0 ? (
              <div className="deployment-gateways">
                {gatewayCards.map((presentation) => (
                  <DeploymentGatewayCard
                    deployment={deployment}
                    presentation={presentation}
                    pending={pending}
                    onDetach={() => detachGateway(deployment, presentation.gatewayId)}
                    onRevoke={() => revokeGateway(presentation.gatewayId)}
                    key={presentation.gatewayId}
                  />
                ))}
              </div>
            ) : (
              <div className="deployment-gateways-empty">
                <Link2 aria-hidden="true" />
                <div><strong>No devices assigned</strong><span>Devices paired to this environment receive its configuration.</span></div>
              </div>
            )}
            <form className="deployment-policy-form" onSubmit={(event) => updatePolicies(deployment, event)}>
              <label><input type="checkbox" name="configSyncEnabled" defaultChecked={deployment.configSyncEnabled} />Sync settings</label>
              <label><input type="checkbox" name="remoteInvocationEnabled" defaultChecked={deployment.remoteInvocationEnabled} />Allow remote calls</label>
              <label><span>Trace upload</span><select name="traceMode" defaultValue={deployment.traceMode === "full" ? "summary" : deployment.traceMode}><option value="off">Off</option><option value="summary">Summary</option></select></label>
              <button className="icon-text-button" type="submit" disabled={pending === `settings-${deployment.id}`}><Check aria-hidden="true" />Save</button>
            </form>
            {!deployment.isPrimary ? (
              <footer>
                <button className="quiet-button" type="button" onClick={() => promote(deployment)} disabled={pending === `promote-${deployment.id}`}><Star aria-hidden="true" />Make primary</button>
              </footer>
            ) : null}
            </article>
          );
        })}
      </section>

      <section className="release-workbench">
        <header>
          <div><h2>Configuration releases</h2><p>Versioned provider, agent, and endpoint configuration for advanced environments.</p></div>
          <div className="settings-artifact-actions">
            <label className="secondary-button settings-file-button">
              <CloudUpload aria-hidden="true" />Load JSON
              <input type="file" accept="application/json,.json" onChange={loadProjectSettingsFile} />
            </label>
            <button className="secondary-button" type="button" onClick={exportProjectSettings} disabled={!latestRelease}><Download aria-hidden="true" />Export</button>
            <button className="primary-button" type="button" onClick={importProjectSettings} disabled={pending === "import-settings"}><CloudUpload aria-hidden="true" />{pending === "import-settings" ? "Importing" : "Import"}</button>
          </div>
        </header>
        <textarea value={settingsSource} onChange={(event) => setSettingsSource(event.target.value)} spellCheck={false} aria-label="Configuration release JSON" />
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
          )) : <div className="deployment-empty">No configuration releases saved yet.</div>}
        </div>
      </section>
    </div>
  );
}

function DeploymentGatewayCard({
  deployment,
  presentation,
  pending,
  onDetach,
  onRevoke,
}: {
  deployment: RuntimeDeployment;
  presentation: ReturnType<typeof gatewayDeploymentPresentation>;
  pending: string | null;
  onDetach: () => void;
  onRevoke: () => void;
}) {
  const connected = presentation.gateway?.status === "connected";
  return (
    <article className={`deployment-gateway-card ${connected ? "online" : "offline"}`}>
      <i className="deployment-gateway-rail" aria-hidden="true" />
      <div className="deployment-gateway-identity">
        <span className="deployment-gateway-device" aria-hidden="true">
          <GatewayDeviceIcon kind={presentation.kind} platform={presentation.platform} />
        </span>
        <div>
          <div className="deployment-gateway-name">
            <strong>{presentation.name}</strong>
            <span>{presentation.typeLabel}</span>
          </div>
          <p>
            <span>{presentation.deviceLabel}</span>
            {presentation.applicationLabel ? <span>{presentation.applicationLabel}</span> : null}
          </p>
        </div>
      </div>
      <div className="deployment-gateway-state">
        <GatewayConnectionStatus gateway={presentation.gateway} agentLabel={presentation.agentLabel} />
        <GatewayApplyStatus deployment={deployment} gatewayId={presentation.gatewayId} />
      </div>
      <div className="deployment-gateway-id">
        <span>Gateway ID</span>
        <code title={presentation.gatewayId}>{presentation.gatewayId}</code>
      </div>
      <div className="deployment-gateway-actions">
        <button className="icon-button" type="button" title="Detach from deployment" aria-label={`Detach ${presentation.name} from ${deployment.name}`} onClick={onDetach} disabled={pending !== null}><Unplug aria-hidden="true" /></button>
        <button className="icon-button danger" type="button" title="Revoke Gateway access" aria-label={`Revoke access for ${presentation.name}`} onClick={onRevoke} disabled={pending !== null}><ShieldOff aria-hidden="true" /></button>
      </div>
    </article>
  );
}

function DeviceGatewayCard({
  presentation,
  deployments,
  showEnvironments,
  pending,
  onRevoke,
}: {
  presentation: ReturnType<typeof gatewayDeploymentPresentation>;
  deployments: RuntimeDeployment[];
  showEnvironments: boolean;
  pending: boolean;
  onRevoke: () => void;
}) {
  const connected = presentation.gateway?.status === "connected";
  return (
    <article className={`deployment-gateway-card device-gateway-card ${connected ? "online" : "offline"}`}>
      <i className="deployment-gateway-rail" aria-hidden="true" />
      <div className="deployment-gateway-identity">
        <span className="deployment-gateway-device" aria-hidden="true">
          <GatewayDeviceIcon kind={presentation.kind} platform={presentation.platform} />
        </span>
        <div>
          <div className="deployment-gateway-name">
            <strong>{presentation.name}</strong>
            <span>{presentation.typeLabel}</span>
          </div>
          <p>
            <span>{presentation.deviceLabel}</span>
            {presentation.applicationLabel ? <span>{presentation.applicationLabel}</span> : null}
          </p>
        </div>
      </div>
      <div className="deployment-gateway-state">
        <GatewayConnectionStatus gateway={presentation.gateway} agentLabel={presentation.agentLabel} />
        {showEnvironments ? (
          <span className="device-environment-count">
            {deployments.length} {deployments.length === 1 ? "environment" : "environments"}
          </span>
        ) : null}
      </div>
      <div className="deployment-gateway-id">
        <span>Gateway ID</span>
        <code title={presentation.gatewayId}>{presentation.gatewayId}</code>
      </div>
      <div className="deployment-gateway-actions">
        <button className="icon-button danger" type="button" title="Revoke device access" aria-label={`Revoke access for ${presentation.name}`} onClick={onRevoke} disabled={pending}><ShieldOff aria-hidden="true" /></button>
      </div>
      {showEnvironments && deployments.length > 0 ? (
        <div className="device-environment-list" aria-label="Assigned environments">
          {deployments.map((deployment) => <span key={deployment.id}>{deployment.name}</span>)}
        </div>
      ) : null}
    </article>
  );
}

function GatewayDeviceIcon({ kind, platform }: { kind: string; platform: string }) {
  const DeviceIcon = platform === "android" || platform === "ios"
    ? Smartphone
    : kind === "computer" || platform === "macos" || platform === "windows"
      ? Monitor
      : Box;
  return <DeviceIcon />;
}

function GatewayConnectionStatus({ gateway, agentLabel }: { gateway?: AgentGateway; agentLabel: string }) {
  const connected = gateway?.status === "connected";
  return (
    <span className={`deployment-gateway-connection ${connected ? "online" : "offline"}`}>
      <strong><i aria-hidden="true" />{connected ? "Online" : "Offline"}</strong>
      <small>
        {gateway
          ? connected
            ? agentLabel
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

function EnrollmentPanel({
  enrollment,
  showEnvironment,
  onClose,
}: {
  enrollment: Enrollment;
  showEnvironment: boolean;
  onClose: () => void;
}) {
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
      <div><strong>Pair a device</strong><p>Scan this one-time code from the Starter. It expires in five minutes.{showEnvironment ? ` The device will join ${enrollment.deployment}.` : ""}</p></div>
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

function recordValue(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : {};
}

function stringValue(value: unknown): string {
  return typeof value === "string" ? value : "";
}

function firstString(...values: unknown[]): string {
  for (const value of values) {
    const text = stringValue(value).trim();
    if (text) return text;
  }
  return "";
}

function fallbackGatewayName(gatewayId: string): string {
  const value = gatewayId.replace(/^gateway-/, "");
  return `Gateway ${value.length > 8 ? `${value.slice(0, 8)}…` : value}`;
}

function platformLabel(platform: string): string {
  const labels: Record<string, string> = {
    android: "Android",
    ios: "iOS",
    linux: "Linux",
    macos: "macOS",
    windows: "Windows",
  };
  return labels[platform.toLowerCase()] ?? titleLabel(platform);
}

function kindLabel(kind: string): string {
  return kind.replace(/[._-]+/g, " ").toLowerCase();
}

function titleLabel(value: string): string {
  if (!value) return "";
  const spaced = value.replace(/[._-]+/g, " ");
  return `${spaced.charAt(0).toUpperCase()}${spaced.slice(1)}`;
}

function versionLabel(version: string): string {
  return version.toLowerCase().startsWith("v") ? version : `v${version}`;
}

function gatewayConnectionRank(status: string | undefined): number {
  return status === "connected" ? 0 : status === "pending" ? 1 : 2;
}
