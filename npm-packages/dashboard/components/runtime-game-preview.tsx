"use client";

import Link from "next/link";
import {
  AlertTriangle,
  CheckCircle2,
  ChevronRight,
  CircleStop,
  Hammer,
  Play,
  RefreshCw,
  Send,
  TerminalSquare,
} from "lucide-react";
import { useState, type FormEvent } from "react";
import { runtimeBrowserRequest } from "../lib/runtime-browser-client";
import type {
  GameAdvance,
  GameBuild,
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
}: {
  project: RuntimeProject;
  overview?: GameOverview;
  qa?: GameQa;
  recentSessions: GameSession[];
}) {
  const [preview, setPreview] = useState<PreviewState | null>(null);
  const [pending, setPending] = useState<"start" | "command" | "build" | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [build, setBuild] = useState<GameBuild | null>(null);
  const requiredCapabilities = qa?.requiredHostCapabilities ?? [];
  const previewSessions = recentSessions.filter((session) => session.preview);

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
          locale: "en",
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

  return (
    <div className="game-preview-page">
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
      </section>

      {message ? <p className="inline-error game-preview-error" role="alert">{message}</p> : null}
      {build ? <p className="game-build-result"><CheckCircle2 aria-hidden="true" />Build {build.status} for draft {build.sourceRevision}</p> : null}

      <div className="game-preview-workspace">
        <section className="reference-player">
          <header><div><span>Reference host</span><strong>{preview ? statusLabel(preview.advance.snapshot.status) : "Not running"}</strong></div><button className="icon-button" type="button" disabled={pending !== null || !qa?.ready} onClick={() => void start()} title={preview ? "Restart preview" : "Start preview"} aria-label={preview ? "Restart preview" : "Start preview"}>{preview ? <RefreshCw aria-hidden="true" /> : <Play aria-hidden="true" />}</button></header>
          {preview ? (
            <PlayerSurface preview={preview} pending={pending === "command"} onCommand={command} />
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

function PlayerSurface({ preview, pending, onCommand }: { preview: PreviewState; pending: boolean; onCommand: (type: string, data: unknown) => Promise<void> }) {
  const [text, setText] = useState("");
  const latest = preview.events.at(-1);
  const choice = [...preview.events].reverse().find((event) => event.type === "choice.presented");
  const choices = choiceOptions(choice?.data);
  const snapshot = preview.advance.snapshot;

  function submitText(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const value = text.trim();
    if (!value) return;
    setText("");
    void onCommand("player.text", { text: value });
  }

  return (
    <div className="reference-player-surface">
      <div className="reference-stage-copy">
        <span>{latest ? eventLabel(latest.type) : "Session started"}</span>
        <strong>{eventHeadline(latest)}</strong>
        <p>{eventDetail(latest)}</p>
      </div>
      {snapshot.status === "waiting_host" && snapshot.pendingHostAction ? (
        <button className="player-host-action" type="button" disabled={pending} onClick={() => void onCommand("host.action.completed", { actionId: snapshot.pendingHostAction?.actionId })}>
          Complete {snapshot.pendingHostAction.action} on {snapshot.pendingHostAction.target}
        </button>
      ) : null}
      {snapshot.status === "waiting_input" && choices.length > 0 ? (
        <div className="player-choice-list">
          {choices.map((option) => <button type="button" disabled={pending} key={option.id} onClick={() => void onCommand("player.choice", { optionId: option.id })}>{option.label}<ChevronRight aria-hidden="true" /></button>)}
        </div>
      ) : null}
      {snapshot.status === "waiting_input" && choices.length === 0 ? (
        <form className="player-text-input" onSubmit={submitText}><input value={text} onChange={(event) => setText(event.target.value)} placeholder="Player response" aria-label="Player response" /><button className="icon-button" type="submit" disabled={pending || !text.trim()} aria-label="Send response"><Send aria-hidden="true" /></button></form>
      ) : null}
      <details className="player-debug-state"><summary>Runtime state</summary><pre>{JSON.stringify(snapshot.state, null, 2)}</pre></details>
      <div className="player-event-strip">
        {preview.events.slice(-6).map((event) => <span key={event.id}><i />{event.type}</span>)}
      </div>
    </div>
  );
}

function choiceOptions(value: unknown): Array<{ id: string; label: string }> {
  if (!value || typeof value !== "object") return [];
  const options = (value as { options?: unknown }).options;
  if (!Array.isArray(options)) return [];
  return options.flatMap((option) => {
    if (!option || typeof option !== "object") return [];
    const id = String((option as { id?: unknown }).id ?? "");
    if (!id) return [];
    const label = String((option as { label?: unknown; text?: unknown }).label ?? (option as { text?: unknown }).text ?? id);
    return [{ id, label }];
  });
}

function eventHeadline(event?: GameEvent): string {
  if (!event) return "The game is ready.";
  const data = objectValue(event.data);
  return stringValue(data.text) || stringValue(data.name) || stringValue(data.title) || eventLabel(event.type);
}

function eventDetail(event?: GameEvent): string {
  if (!event) return "Runtime events will appear here as the session advances.";
  const data = objectValue(event.data);
  return stringValue(data.description) || stringValue(data.dialogue) || stringValue(data.message) || `Event ${event.sequence}`;
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

function formatTime(value: string): string {
  return new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" }).format(new Date(value));
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : "Preview request failed.";
}
