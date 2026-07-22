"use client";

import Link from "next/link";
import {
  AlertTriangle,
  CheckCircle2,
  ChevronRight,
  CircleStop,
  Hammer,
  Languages,
  Maximize2,
  Minimize2,
  Play,
  RefreshCw,
  Send,
  Square,
  TerminalSquare,
  Video,
} from "lucide-react";
import { useEffect, useRef, useState, type CSSProperties, type FormEvent } from "react";
import { runtimeBrowserRequest } from "../lib/runtime-browser-client";
import { DEFAULT_GAME_VIEWPORT, presentationViewport } from "../lib/game-authoring";
import type {
  GameAdvance,
  GameAsset,
  GameBuild,
  GameDraft,
  GameEvent,
  GameOverview,
  GameQa,
  GameSession,
  PublicGameAdvance,
  RuntimeProject,
} from "../lib/runtime-types";

type PreviewState = {
  sessionId: string;
  draftRevision: number;
  advance: GameAdvance;
  events: GameEvent[];
};

export function RuntimeGamePreview({
  project,
  overview,
  qa,
  recentSessions,
  draft,
  assets,
}: {
  project: RuntimeProject;
  overview?: GameOverview;
  qa?: GameQa;
  recentSessions: GameSession[];
  draft?: GameDraft;
  assets: GameAsset[];
}) {
  const [preview, setPreview] = useState<PreviewState | null>(null);
  const [pending, setPending] = useState<"start" | "command" | "build" | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [build, setBuild] = useState<GameBuild | null>(null);
  const [focused, setFocused] = useState(false);
  const [recording, setRecording] = useState(false);
  const [recordingSeconds, setRecordingSeconds] = useState(0);
  const recorder = useRef<MediaRecorder | null>(null);
  const captureStream = useRef<MediaStream | null>(null);
  const recordingChunks = useRef<Blob[]>([]);
  const discardRecording = useRef(false);
  const locales = draft ? [...new Set([draft.source.localization.sourceLocale, ...draft.source.localization.targetLocales])] : ["en"];
  const [locale, setLocale] = useState(draft?.source.localization.defaultLocale ?? locales[0] ?? "en");
  const requiredCapabilities = qa?.requiredHostCapabilities ?? [];
  const previewSessions = recentSessions.filter((session) => session.preview);
  const viewport = draft ? presentationViewport(draft.source) : DEFAULT_GAME_VIEWPORT;
  const orientation = viewport.width === viewport.height
    ? "square"
    : viewport.width < viewport.height
      ? "portrait"
      : "landscape";
  const playerStyle = {
    "--game-preview-ratio": `${viewport.width} / ${viewport.height}`,
    "--game-preview-ratio-value": viewport.width / viewport.height,
  } as CSSProperties;

  useEffect(() => {
    if (!preview || preview.advance.snapshot.status !== "waiting_effect") return;
    let cancelled = false;
    let timer: number | undefined;

    async function refreshEffect() {
      if (!preview) return;
      try {
        const detail = await runtimeBrowserRequest<{ session: GameSession; events: GameEvent[] }>(
          `project/${encodeURIComponent(project.slug)}/game/sessions/${preview.sessionId}`,
        );
        if (cancelled) return;
        setPreview((current) => current && current.sessionId === preview.sessionId ? {
          ...current,
          advance: {
            snapshot: detail.session.snapshot,
            events: detail.events,
            effects: [],
          },
          events: mergeEvents(current.events, detail.events),
        } : current);
        if (detail.session.snapshot.status === "waiting_effect") {
          timer = window.setTimeout(refreshEffect, 500);
        }
      } catch (error) {
        if (!cancelled) setMessage(errorMessage(error));
      }
    }

    timer = window.setTimeout(refreshEffect, 250);
    return () => {
      cancelled = true;
      if (timer !== undefined) window.clearTimeout(timer);
    };
  }, [preview?.advance.snapshot.status, preview?.sessionId, project.slug]);

  useEffect(() => {
    if (!focused) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") setFocused(false);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [focused]);

  useEffect(() => {
    if (!recording) return;
    const startedAt = Date.now();
    const timer = window.setInterval(() => setRecordingSeconds(Math.floor((Date.now() - startedAt) / 1000)), 1000);
    return () => window.clearInterval(timer);
  }, [recording]);

  useEffect(() => () => {
    discardRecording.current = true;
    if (recorder.current?.state === "recording") recorder.current.stop();
    captureStream.current?.getTracks().forEach((track) => track.stop());
  }, []);

  async function start() {
    setPending("start");
    setMessage(null);
    try {
      const result = await runtimeBrowserRequest<{
        sessionId: string;
        draftRevision: number;
        advance: PublicGameAdvance;
      }>(`project/${encodeURIComponent(project.slug)}/game/preview`, "POST", {
        host: {
          engine: "web",
          adapterVersion: "vifu-dashboard-v1",
          capabilities: requiredCapabilities,
          locale,
        },
        input: {},
      });
      const detail = await runtimeBrowserRequest<{ session: GameSession }>(
        `project/${encodeURIComponent(project.slug)}/game/sessions/${result.sessionId}`,
      );
      setPreview({
        sessionId: result.sessionId,
        draftRevision: result.draftRevision,
        advance: {
          snapshot: detail.session.snapshot,
          events: result.advance.events,
          effects: [],
        },
        events: result.advance.events,
      });
      setFocused(true);
    } catch (error) {
      setMessage(errorMessage(error));
    } finally {
      setPending(null);
    }
  }

  async function command(type: string, data: unknown) {
    if (!preview) return;
    setPending("command");
    setMessage(null);
    try {
      const result = await runtimeBrowserRequest<{ advance: PublicGameAdvance }>(
        `${encodeURIComponent(project.slug)}/v1/game/sessions/${preview.sessionId}/commands`,
        "POST",
        {
          idempotencyKey: `dashboard:${crypto.randomUUID()}`,
          expectedRevision: preview.advance.snapshot.revision,
          type,
          data,
        },
      );
      const detail = await runtimeBrowserRequest<{ session: GameSession }>(
        `project/${encodeURIComponent(project.slug)}/game/sessions/${preview.sessionId}`,
      );
      setPreview((current) => current ? {
        ...current,
        advance: {
          snapshot: detail.session.snapshot,
          events: result.advance.events,
          effects: [],
        },
        events: [...current.events, ...result.advance.events],
      } : current);
    } catch (error) {
      setMessage(errorMessage(error));
    } finally {
      setPending(null);
    }
  }

  async function runBuild() {
    setPending("build");
    setMessage(null);
    try {
      const result = await runtimeBrowserRequest<{ build: GameBuild }>(
        `project/${encodeURIComponent(project.slug)}/game/builds`,
        "POST",
        { kind: "compile" },
      );
      setBuild(result.build);
    } catch (error) {
      setMessage(errorMessage(error));
    } finally {
      setPending(null);
    }
  }

  async function startVideoExport() {
    if (!preview || !navigator.mediaDevices?.getDisplayMedia || typeof MediaRecorder === "undefined") {
      setMessage("This browser cannot export a playthrough video.");
      return;
    }
    setMessage("Choose this browser tab and enable tab audio to export the playthrough.");
    setFocused(true);
    discardRecording.current = false;
    await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
    try {
      const stream = await navigator.mediaDevices.getDisplayMedia({
        video: { displaySurface: "browser" },
        audio: true,
        preferCurrentTab: true,
        selfBrowserSurface: "include",
        surfaceSwitching: "exclude",
      } as DisplayMediaStreamOptions);
      if (discardRecording.current) {
        stream.getTracks().forEach((track) => track.stop());
        return;
      }
      captureStream.current = stream;
      const surface = document.querySelector<HTMLElement>(".reference-player-surface");
      const videoTrack = stream.getVideoTracks()[0];
      const cropTarget = (window as Window & {
        CropTarget?: { fromElement(element: Element): Promise<unknown> };
      }).CropTarget;
      const cropTrack = videoTrack as MediaStreamTrack & { cropTo?(target: unknown): Promise<void> };
      if (surface && cropTarget && cropTrack.cropTo) {
        try {
          await cropTrack.cropTo(await cropTarget.fromElement(surface));
        } catch {
          // Element Capture is optional; full-tab capture remains a valid fallback.
        }
      }
      const mimeType = supportedRecordingType();
      const nextRecorder = new MediaRecorder(stream, mimeType ? { mimeType } : undefined);
      recordingChunks.current = [];
      nextRecorder.addEventListener("dataavailable", (event) => {
        if (event.data.size > 0) recordingChunks.current.push(event.data);
      });
      nextRecorder.addEventListener("stop", () => finishVideoExport(nextRecorder.mimeType));
      stream.getVideoTracks()[0]?.addEventListener("ended", () => {
        if (nextRecorder.state === "recording") nextRecorder.stop();
      });
      recorder.current = nextRecorder;
      nextRecorder.start(1000);
      setRecordingSeconds(0);
      setRecording(true);
      setMessage(null);
    } catch (error) {
      captureStream.current?.getTracks().forEach((track) => track.stop());
      captureStream.current = null;
      if (discardRecording.current) return;
      setFocused(false);
      setMessage(error instanceof Error && error.name !== "NotAllowedError" ? error.message : "Video export was cancelled.");
    }
  }

  function stopVideoExport() {
    if (recorder.current?.state === "recording") recorder.current.stop();
  }

  function finishVideoExport(mimeType: string) {
    captureStream.current?.getTracks().forEach((track) => track.stop());
    const chunks = recordingChunks.current;
    recorder.current = null;
    captureStream.current = null;
    recordingChunks.current = [];
    if (discardRecording.current) return;
    setRecording(false);
    setRecordingSeconds(0);
    setFocused(false);
    if (chunks.length === 0) {
      setMessage("The browser did not produce a video file.");
      return;
    }
    const effectiveType = mimeType || chunks[0]?.type || "video/webm";
    const extension = effectiveType.includes("mp4") ? "mp4" : "webm";
    const url = URL.createObjectURL(new Blob(chunks, { type: effectiveType }));
    const anchor = document.createElement("a");
    anchor.href = url;
    anchor.download = `${project.slug}-playthrough.${extension}`;
    anchor.click();
    window.setTimeout(() => URL.revokeObjectURL(url), 0);
    setMessage(`Playthrough exported as ${extension.toUpperCase()}.`);
  }

  return (
    <div className={`game-preview-page ${focused ? "focused" : ""}`}>
      <section className="game-qa-summary">
        <div className={`qa-readiness ${qa?.ready ? "ready" : "blocked"}`}>
          {qa?.ready ? <CheckCircle2 aria-hidden="true" /> : <AlertTriangle aria-hidden="true" />}
          <div><strong>{qa?.ready ? "Ready to build" : "Needs attention"}</strong><span>Draft {qa?.draftRevision ?? overview?.draftRevision ?? "-"}</span></div>
        </div>
        <dl>
          <div><dt>Blockers</dt><dd>{qa?.blockerCount ?? 0}</dd></div>
          <div><dt>Warnings</dt><dd>{qa?.warningCount ?? 0}</dd></div>
          <div><dt>Active release</dt><dd>{overview?.activeRelease ? `R${overview.activeRelease.releaseNumber}` : "None"}</dd></div>
          <div><dt>Recent sessions</dt><dd>{recentSessions.length}</dd></div>
        </dl>
        <button className="secondary-button compact" type="button" disabled={pending !== null || !qa?.ready} onClick={() => void runBuild()}>
          <Hammer aria-hidden="true" />{pending === "build" ? "Building..." : "Build draft"}
        </button>
        <label className="preview-locale-select"><Languages aria-hidden="true" /><span className="sr-only">Preview language</span><select value={locale} onChange={(event) => setLocale(event.target.value)}>{locales.map((item) => <option value={item} key={item}>{localeName(item)}</option>)}</select></label>
      </section>

      {message ? <p className="inline-error game-preview-error" role="alert">{message}</p> : null}
      {build ? <p className="game-build-result"><CheckCircle2 aria-hidden="true" />Build {build.status} for draft {build.sourceRevision}</p> : null}

      <div className="game-preview-workspace">
        <section className={`reference-player formatted ${orientation}`} style={playerStyle}>
          <header><div><span>{focused ? project.name : "Reference host"}</span><strong>{recording ? `Recording ${formatDuration(recordingSeconds)}` : preview ? statusLabel(preview.advance.snapshot.status) : "Not running"}</strong></div><nav className="reference-player-actions" aria-label="Preview controls"><button className="icon-button" type="button" disabled={pending !== null || !qa?.ready || recording} onClick={() => void start()} title={preview ? "Restart preview" : "Start preview"} aria-label={preview ? "Restart preview" : "Start preview"}>{preview ? <RefreshCw aria-hidden="true" /> : <Play aria-hidden="true" />}</button>{preview ? <button className={`icon-button${recording ? " recording" : ""}`} type="button" onClick={recording ? stopVideoExport : () => void startVideoExport()} title={recording ? "Finish video export" : "Export playthrough video"} aria-label={recording ? "Finish video export" : "Export playthrough video"}>{recording ? <Square aria-hidden="true" /> : <Video aria-hidden="true" />}</button> : null}{preview ? <button className="icon-button" type="button" disabled={recording} onClick={() => setFocused((value) => !value)} title={focused ? "Exit play mode" : "Enter play mode"} aria-label={focused ? "Exit play mode" : "Enter play mode"}>{focused ? <Minimize2 aria-hidden="true" /> : <Maximize2 aria-hidden="true" />}</button> : null}</nav></header>
          {preview ? (
            <div className="reference-player-canvas">
              <PlayerSurface projectSlug={project.slug} draft={draft} assets={assets} preview={preview} pending={pending === "command"} onCommand={command} />
            </div>
          ) : (
            <div className="reference-player-empty">
              <Play aria-hidden="true" />
              <strong>{qa?.ready ? "Run the current draft" : "Resolve draft blockers"}</strong>
              <span>The reference host executes the current draft with the same runtime used by published releases.</span>
              {qa?.ready ? <button className="primary-button" type="button" disabled={pending !== null} onClick={() => void start()}>{pending === "start" ? "Starting..." : "Start preview"}</button> : <Link className="primary-button" href={`/project/${project.slug}/canvas`}>Open Canvas</Link>}
            </div>
          )}
        </section>

        <aside className="game-qa-panel">
          <header><span>QA checks</span><strong>{qa?.issues.length ?? 0} findings</strong></header>
          {qa && qa.issues.length > 0 ? (
            <div className="qa-issue-list">
              {qa.issues.map((issue, index) => (
                <article className={issue.severity} key={`${issue.code}-${issue.nodeId ?? issue.path ?? index}`}>
                  {issue.severity === "error" ? <AlertTriangle aria-hidden="true" /> : <CircleStop aria-hidden="true" />}
                  <div><strong>{issue.message}</strong><span>{issue.nodeId ? `Node ${issue.nodeId}` : issue.path ?? issue.code}</span></div>
                </article>
              ))}
            </div>
          ) : (
            <div className="qa-empty"><CheckCircle2 aria-hidden="true" /><strong>No structural issues</strong><span>Build and run each branch before publishing.</span></div>
          )}
        </aside>
      </div>

      {previewSessions.length > 0 ? (
        <section className="preview-session-history">
          <header><TerminalSquare aria-hidden="true" /><strong>Recent test sessions</strong></header>
          <div>
            {previewSessions.slice(0, 8).map((session) => (
              <article key={session.id}><code>{session.id.slice(0, 8)}</code><span className={`session-state ${session.status}`}>{statusLabel(session.status)}</span><time>{formatTime(session.createdAt)}</time><ChevronRight aria-hidden="true" /></article>
            ))}
          </div>
        </section>
      ) : null}
    </div>
  );
}

function PlayerSurface({
  projectSlug,
  draft,
  assets,
  preview,
  pending,
  onCommand,
}: {
  projectSlug: string;
  draft?: GameDraft;
  assets: GameAsset[];
  preview: PreviewState;
  pending: boolean;
  onCommand: (type: string, data: unknown) => Promise<void>;
}) {
  const [text, setText] = useState("");
  const latest = latestPresentationEvent(preview.events);
  const waiting = latestWaiting(preview.events);
  const choice = waiting.commandType === "player.choice" ? [...preview.events].reverse().find((event) => event.type === "choice.presented") : undefined;
  const choices = choiceOptions(choice?.data);
  const input = waiting.commandType && !["player.choice", "player.continue"].includes(waiting.commandType)
    ? [...preview.events].reverse().find((event) => event.type === "player.input.requested")
    : undefined;
  const snapshot = preview.advance.snapshot;
  const stageMediaEvent = [...preview.events].reverse().find((event) => ["background.changed", "video.play"].includes(event.type));
  const characterEvent = [...preview.events].reverse().find((event) => event.type === "character.visual.changed");
  const audioEvents = [...preview.events].reverse().filter((event) => event.type === "audio.play");
  const scoreEvent = audioEvents.find((event) => objectBoolean(event.data, "loop"));
  const effectEvent = audioEvents.find((event) => !objectBoolean(event.data, "loop"));
  const voiceEvent = [...preview.events].reverse().find((event) => event.type === "voice.play");
  const stageMediaUrl = assetUrl(projectSlug, draft, assets, objectString(stageMediaEvent?.data, "logicalResourceId"));
  const characterUrl = assetUrl(projectSlug, draft, assets, objectString(characterEvent?.data, "logicalResourceId"));
  const scoreUrl = assetUrl(projectSlug, draft, assets, objectString(scoreEvent?.data, "logicalResourceId"));
  const effectUrl = assetUrl(projectSlug, draft, assets, objectString(effectEvent?.data, "logicalResourceId"));
  const voiceUrl = assetUrl(projectSlug, draft, assets, objectString(voiceEvent?.data, "logicalResourceId"));
  const backgroundFit = objectString(stageMediaEvent?.data, "fit") === "contain" ? "contain" : "cover";

  function submitText(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const value = text.trim();
    if (!value) return;
    setText("");
    void onCommand(waiting.commandType || "player.text", { text: value });
  }

  return (
    <div className="reference-player-surface">
      <div className="reference-stage-media" aria-hidden="true">
        {stageMediaUrl && stageMediaEvent?.type === "video.play" ? (
          <RuntimeVideo event={stageMediaEvent} src={stageMediaUrl} fit={backgroundFit} />
        ) : stageMediaUrl ? (
          <img className="reference-stage-background" src={stageMediaUrl} alt="" style={{ objectFit: backgroundFit }} />
        ) : <div className="reference-stage-fallback" />}
        {characterUrl ? <img className="reference-stage-character" src={characterUrl} alt="" /> : null}
        <div className="reference-stage-vignette" />
      </div>
      <div className="reference-stage-copy">
        <span>{latest ? eventLabel(latest.type) : "Session started"}</span>
        <strong>{eventHeadline(latest)}</strong>
        <p>{eventDetail(latest)}</p>
      </div>
      {snapshot.status === "waiting_host" && snapshot.pendingHostAction ? (
        snapshot.pendingHostAction.action === "break_moon_control"
          ? <BreakControlAction pending={pending} onComplete={() => onCommand("host.action.completed", { actionId: snapshot.pendingHostAction?.actionId })} />
          : <button className="player-host-action" type="button" disabled={pending} onClick={() => void onCommand("host.action.completed", { actionId: snapshot.pendingHostAction?.actionId })}>Complete {snapshot.pendingHostAction.action}</button>
      ) : null}
      {snapshot.status === "waiting_input" && waiting.commandType === "player.choice" ? (
        <div className="player-choice-list">
          {choices.map((option) => <button type="button" disabled={pending || !option.available} key={option.id} title={!option.available ? option.lockedReason || "This path is locked" : undefined} onClick={() => void onCommand("player.choice", { optionId: option.id })}><span>{option.label}{!option.available && option.lockedReason ? <small>{option.lockedReason}</small> : null}</span><ChevronRight aria-hidden="true" /></button>)}
        </div>
      ) : null}
      {snapshot.status === "waiting_input" && waiting.commandType === "player.continue" ? <button type="button" className="player-continue-button" disabled={pending} onClick={() => void onCommand("player.continue", {})}>Continue<ChevronRight aria-hidden="true" /></button> : null}
      {snapshot.status === "waiting_input" && waiting.commandType && !["player.choice", "player.continue"].includes(waiting.commandType) ? (
        <form className="player-text-input" onSubmit={submitText}><label><span>{objectString(input?.data, "prompt") || "What do you say?"}</span><input value={text} onChange={(event) => setText(event.target.value)} placeholder="Your response" aria-label="Player response" /></label><button className="icon-button" type="submit" disabled={pending || !text.trim()} aria-label="Send response"><Send aria-hidden="true" /></button></form>
      ) : null}
      {scoreUrl || effectUrl || voiceUrl ? <div className="reference-player-audio-stack">
        {scoreUrl ? <audio className="reference-player-audio" key={scoreEvent?.id} src={scoreUrl} controls autoPlay loop /> : null}
        {effectUrl ? <audio className="reference-player-audio" key={effectEvent?.id} src={effectUrl} controls autoPlay /> : null}
        {voiceUrl ? <audio className="reference-player-audio" key={voiceEvent?.id} src={voiceUrl} controls autoPlay /> : null}
      </div> : null}
      <details className="player-debug-state"><summary>Runtime state</summary><pre>{JSON.stringify(snapshot.state, null, 2)}</pre></details>
      <div className="player-event-strip">
        {preview.events.slice(-6).map((event) => <span key={event.id}><i />{event.type}</span>)}
      </div>
    </div>
  );
}

function RuntimeVideo({ event, src, fit }: { event: GameEvent; src: string; fit: "contain" | "cover" }) {
  const videoRef = useRef<HTMLVideoElement>(null);
  const muted = objectBoolean(event.data, "muted");
  const loop = objectBoolean(event.data, "loop");
  const volume = objectNumber(event.data, "volume", 1);
  const inMs = objectNumber(event.data, "inMs", 0);

  useEffect(() => {
    const video = videoRef.current;
    if (!video) return;
    video.volume = Math.max(0, Math.min(1, volume));
    video.currentTime = Math.max(0, inMs) / 1000;
    void video.play().catch(() => undefined);
    return () => video.pause();
  }, [event.id, inMs, src, volume]);

  return (
    <video
      className="reference-stage-background"
      ref={videoRef}
      src={src}
      style={{ objectFit: fit }}
      autoPlay
      loop={loop}
      muted={muted}
      playsInline
      preload="auto"
    />
  );
}

function BreakControlAction({ pending, onComplete }: { pending: boolean; onComplete: () => Promise<void> }) {
  const [progress, setProgress] = useState(0);
  const timer = useRef<number | null>(null);
  const started = useRef(0);

  useEffect(() => () => stop(false), []);

  function start() {
    if (pending || timer.current !== null) return;
    started.current = performance.now();
    timer.current = window.setInterval(() => {
      const next = Math.min(100, ((performance.now() - started.current) / 1200) * 100);
      setProgress(next);
      if (next >= 100) {
        stop(false);
        void onComplete();
      }
    }, 32);
  }

  function stop(reset = true) {
    if (timer.current !== null) window.clearInterval(timer.current);
    timer.current = null;
    if (reset) setProgress(0);
  }

  return (
    <button className="player-host-action break-control-action" type="button" disabled={pending} onPointerDown={start} onPointerUp={() => stop()} onPointerCancel={() => stop()} onPointerLeave={() => stop()} onKeyDown={(event) => { if (["Enter", " "].includes(event.key)) { event.preventDefault(); start(); } }} onKeyUp={(event) => { if (["Enter", " "].includes(event.key)) stop(); }}>
      <i style={{ width: `${progress}%` }} /><span>Hold to break the moon&apos;s control</span>
    </button>
  );
}

function choiceOptions(value: unknown): Array<{ id: string; label: string; available: boolean; lockedReason: string }> {
  if (!value || typeof value !== "object") return [];
  const options = (value as { options?: unknown }).options;
  if (!Array.isArray(options)) return [];
  return options.flatMap((option) => {
    if (!option || typeof option !== "object") return [];
    const id = String((option as { id?: unknown }).id ?? "");
    if (!id) return [];
    const label = String((option as { label?: unknown; text?: unknown }).label ?? (option as { text?: unknown }).text ?? id);
    const available = (option as { available?: unknown }).available !== false;
    const lockedReason = String((option as { lockedReason?: unknown }).lockedReason ?? "");
    return [{ id, label, available, lockedReason }];
  });
}

function mergeEvents(current: GameEvent[], incoming: GameEvent[]): GameEvent[] {
  const events = new Map(current.map((event) => [event.id, event]));
  for (const event of incoming) events.set(event.id, event);
  return [...events.values()].sort((left, right) => left.sequence - right.sequence);
}

function latestPresentationEvent(events: GameEvent[]): GameEvent | undefined {
  const visible = new Set(["scene.entered", "dialogue.started", "agent.completed", "ending.reached", "choice.presented", "player.input.requested", "host.action.requested"]);
  return [...events].reverse().find((event) => visible.has(event.type)) ?? events.at(-1);
}

function latestWaiting(events: GameEvent[]): { commandType: string; for: string } {
  const event = [...events].reverse().find((item) => item.type === "game.session.waiting");
  const data = objectValue(event?.data);
  return { commandType: stringValue(data.commandType), for: stringValue(data.for) };
}

function assetUrl(projectSlug: string, draft: GameDraft | undefined, assets: GameAsset[], logicalResourceId: string): string | null {
  if (!draft || !logicalResourceId) return null;
  const presentation = objectValue(draft.source.views.presentation);
  const binding = objectValue(objectValue(presentation.bindings)[logicalResourceId]);
  const versionId = binding.kind === "managed-asset-version" && typeof binding.value === "string" ? binding.value : "";
  if (!versionId) return null;
  const asset = assets.find((item) => item.versions.some((version) => version.id === versionId));
  if (!asset) return null;
  return `/api/runtime/project/${encodeURIComponent(projectSlug)}/game/assets/${encodeURIComponent(asset.id)}/versions/${encodeURIComponent(versionId)}/content`;
}

function objectString(value: unknown, key: string): string {
  return stringValue(objectValue(value)[key]);
}

function objectBoolean(value: unknown, key: string): boolean {
  return objectValue(value)[key] === true;
}

function objectNumber(value: unknown, key: string, fallback: number): number {
  const candidate = objectValue(value)[key];
  return typeof candidate === "number" && Number.isFinite(candidate) ? candidate : fallback;
}

function eventHeadline(event?: GameEvent): string {
  if (!event) return "The game is ready.";
  const data = objectValue(event.data);
  return stringValue(data.prompt) || stringValue(data.title) || stringValue(data.name) || stringValue(data.text) || eventLabel(event.type);
}

function eventDetail(event?: GameEvent): string {
  if (!event) return "Runtime events will appear here as the session advances.";
  const data = objectValue(event.data);
  const options = Array.isArray(data.options) ? data.options.length : 0;
  return stringValue(data.description)
    || stringValue(data.dialogue)
    || stringValue(data.message)
    || stringValue(data.text)
    || (options > 0 ? `Choose one of ${options} paths.` : `Event ${event.sequence}`);
}

function eventLabel(value: string): string {
  return value.replaceAll(".", " · ").replaceAll("_", " ");
}

function objectValue(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : {};
}

function stringValue(value: unknown): string {
  return typeof value === "string" ? value : "";
}

function statusLabel(value: string): string {
  return value.replaceAll("_", " ").replace(/^./, (letter) => letter.toUpperCase());
}

function localeName(value: string): string {
  const labels: Record<string, string> = { "zh-CN": "简体中文", ja: "日本語", en: "English", ko: "한국어", es: "Español", fr: "Français", de: "Deutsch" };
  return labels[value] ?? value;
}

function formatTime(value: string): string {
  return new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" }).format(new Date(value));
}

function formatDuration(seconds: number): string {
  const minutes = Math.floor(seconds / 60).toString().padStart(2, "0");
  const remainder = (seconds % 60).toString().padStart(2, "0");
  return `${minutes}:${remainder}`;
}

function supportedRecordingType(): string {
  return [
    "video/mp4;codecs=avc1.42E01E,mp4a.40.2",
    "video/webm;codecs=vp9,opus",
    "video/webm;codecs=vp8,opus",
    "video/webm",
  ].find((type) => MediaRecorder.isTypeSupported(type)) ?? "";
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : "Preview request failed.";
}
