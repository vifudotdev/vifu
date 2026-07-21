"use client";

import { useRef, useState } from "react";
import { useRouter } from "next/navigation";
import {
  AlertTriangle,
  Braces,
  Check,
  CloudUpload,
  Download,
  MonitorSmartphone,
  Play,
  Redo2,
  RotateCcw,
  Undo2,
  X,
} from "lucide-react";
import { runtimeBrowserRequest } from "../../lib/runtime-browser-client";
import {
  DEFAULT_GAME_VIEWPORT,
  DEFAULT_SHORT_DRAMA_VIEWPORT,
  managedPresentationFromSource,
  presentationViewport,
  setPresentationViewport,
  type GameViewport,
} from "../../lib/game-authoring";
import type { GameDraft, GameRelease, GameSource, GameValidationIssue } from "../../lib/runtime-types";
import { useGameAuthoring, useGameAuthoringStore, useGameHistory, type GameSyncStatus } from "./store";

const VIEWPORT_PRESETS: GameViewport[] = [
  { width: 1080, height: 1920, aspectRatio: "9:16" },
  { width: 1920, height: 1080, aspectRatio: "16:9" },
  { width: 1080, height: 1080, aspectRatio: "1:1" },
  { width: 1080, height: 1350, aspectRatio: "4:5" },
];

export function GameAuthoringToolbar({ projectSlug, viewLabel }: { projectSlug: string; viewLabel: string }) {
  const revision = useGameAuthoring((state) => state.revision);
  const source = useGameAuthoring((state) => state.source);
  const syncStatus = useGameAuthoring((state) => state.syncStatus);
  const syncError = useGameAuthoring((state) => state.syncError);
  const setValidationIssues = useGameAuthoring((state) => state.setValidationIssues);
  const replaceFromServer = useGameAuthoring((state) => state.replaceFromServer);
  const setSource = useGameAuthoring((state) => state.setSource);
  const history = useGameHistory();
  const store = useGameAuthoringStore();
  const router = useRouter();
  const sourceDialog = useRef<HTMLDialogElement>(null);
  const publishDialog = useRef<HTMLDialogElement>(null);
  const formatDialog = useRef<HTMLDialogElement>(null);
  const [sourceText, setSourceText] = useState("");
  const [sourceError, setSourceError] = useState<string | null>(null);
  const [changeSummary, setChangeSummary] = useState("");
  const [formatWidth, setFormatWidth] = useState(DEFAULT_GAME_VIEWPORT.width);
  const [formatHeight, setFormatHeight] = useState(DEFAULT_GAME_VIEWPORT.height);
  const [formatError, setFormatError] = useState<string | null>(null);
  const [actionStatus, setActionStatus] = useState<"idle" | "validating" | "publishing" | "reloading">("idle");
  const [actionMessage, setActionMessage] = useState<string | null>(null);

  async function validate() {
    setActionStatus("validating");
    setActionMessage(null);
    try {
      const result = await runtimeBrowserRequest<{ valid: boolean; issues: GameValidationIssue[] }>(
        `project/${encodeURIComponent(projectSlug)}/game/validate`,
        "POST",
        { source: store.getState().source },
      );
      setValidationIssues(result.issues);
      setActionMessage(result.valid ? "Ready to publish" : `${result.issues.length} issue${result.issues.length === 1 ? "" : "s"} found`);
    } catch (error) {
      setActionMessage(error instanceof Error ? error.message : "Validation failed.");
    } finally {
      setActionStatus("idle");
    }
  }

  async function publish() {
    setActionStatus("publishing");
    setActionMessage(null);
    try {
      const result = await runtimeBrowserRequest<{ release: GameRelease }>(
        `project/${encodeURIComponent(projectSlug)}/game/publish`,
        "POST",
        { expectedRevision: store.getState().revision, changeSummary: changeSummary.trim() || null },
      );
      const presentation = managedPresentationFromSource(store.getState().source);
      if (presentation) {
        await runtimeBrowserRequest(
          `project/${encodeURIComponent(projectSlug)}/game/presentations`,
          "POST",
          {
            gameReleaseId: result.release.id,
            bindingManifest: {
              schemaVersion: 1,
              engine: "web",
              adapterVersion: "vifu-reference-v1",
              bindings: presentation.bindings,
            },
            assetVersionIds: presentation.assetVersionIds,
          },
        );
      }
      setActionMessage(`Release ${result.release.releaseNumber} is active${presentation ? " with its web presentation" : ""}`);
      setChangeSummary("");
      publishDialog.current?.close();
      router.refresh();
    } catch (error) {
      setActionMessage(error instanceof Error ? error.message : "Publish failed.");
    } finally {
      setActionStatus("idle");
    }
  }

  async function reloadDraft() {
    setActionStatus("reloading");
    try {
      const result = await runtimeBrowserRequest<{ draft: GameDraft }>(
        `project/${encodeURIComponent(projectSlug)}/game/source`,
      );
      replaceFromServer(result.draft);
      history.clear();
    } catch (error) {
      setActionMessage(error instanceof Error ? error.message : "Draft reload failed.");
    } finally {
      setActionStatus("idle");
    }
  }

  function openSource() {
    setSourceText(JSON.stringify(source, null, 2));
    setSourceError(null);
    sourceDialog.current?.showModal();
  }

  function applySource() {
    try {
      const parsed = JSON.parse(sourceText) as GameSource;
      if (!parsed || parsed.schemaVersion !== 1 || !parsed.graph || !Array.isArray(parsed.graph.nodes)) {
        throw new Error("The document is not a GameSourceV1 object.");
      }
      setSource(parsed);
      sourceDialog.current?.close();
    } catch (error) {
      setSourceError(error instanceof Error ? error.message : "Source JSON is invalid.");
    }
  }

  function downloadSource() {
    const blob = new Blob([JSON.stringify(source, null, 2)], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const anchor = document.createElement("a");
    anchor.href = url;
    anchor.download = `${projectSlug}.vifu.json`;
    anchor.click();
    URL.revokeObjectURL(url);
  }

  function openFormat() {
    const current = presentationViewport(
      store.getState().source,
      viewLabel === "Short Drama" ? DEFAULT_SHORT_DRAMA_VIEWPORT : DEFAULT_GAME_VIEWPORT,
    );
    setFormatWidth(current.width);
    setFormatHeight(current.height);
    setFormatError(null);
    formatDialog.current?.showModal();
  }

  function applyFormat() {
    if (!Number.isInteger(formatWidth) || !Number.isInteger(formatHeight) || formatWidth < 240 || formatHeight < 240 || formatWidth > 8192 || formatHeight > 8192) {
      setFormatError("Width and height must be whole numbers from 240 to 8192.");
      return;
    }
    setSource(setPresentationViewport(store.getState().source, {
      width: formatWidth,
      height: formatHeight,
      aspectRatio: `${formatWidth}:${formatHeight}`,
    }));
    formatDialog.current?.close();
  }

  const publishDisabled = syncStatus !== "saved" || actionStatus !== "idle";
  const viewport = presentationViewport(
    source,
    viewLabel === "Short Drama" ? DEFAULT_SHORT_DRAMA_VIEWPORT : DEFAULT_GAME_VIEWPORT,
  );
  return (
    <header className="game-editor-toolbar">
      <div className="game-editor-identity">
        <strong>{viewLabel}</strong>
        <span>Draft {revision}</span>
        <SyncState status={syncStatus} />
      </div>
      <div className="game-editor-actions">
        {syncStatus === "conflict" ? (
          <button type="button" className="editor-text-action warning" onClick={() => void reloadDraft()} disabled={actionStatus !== "idle"}>
            <RotateCcw aria-hidden="true" />Reload server draft
          </button>
        ) : null}
        <button type="button" className="editor-icon-action" onClick={history.undo} disabled={!history.canUndo} title="Undo" aria-label="Undo">
          <Undo2 aria-hidden="true" />
        </button>
        <button type="button" className="editor-icon-action" onClick={history.redo} disabled={!history.canRedo} title="Redo" aria-label="Redo">
          <Redo2 aria-hidden="true" />
        </button>
        <span className="editor-action-divider" />
        <button type="button" className="editor-text-action editor-format-action" onClick={openFormat} title="Frame size">
          <MonitorSmartphone aria-hidden="true" />{viewport.aspectRatio}
        </button>
        <button type="button" className="editor-icon-action" onClick={openSource} title="Edit source JSON" aria-label="Edit source JSON">
          <Braces aria-hidden="true" />
        </button>
        <button type="button" className="editor-icon-action" onClick={downloadSource} title="Export source" aria-label="Export source">
          <Download aria-hidden="true" />
        </button>
        <button type="button" className="editor-text-action" onClick={() => void validate()} disabled={actionStatus !== "idle"}>
          <Check aria-hidden="true" />Validate
        </button>
        <button type="button" className="editor-text-action" onClick={() => router.push(`/project/${projectSlug}/preview`)}>
          <Play aria-hidden="true" />Test
        </button>
        <button type="button" className="editor-publish-action" onClick={() => publishDialog.current?.showModal()} disabled={publishDisabled}>
          <CloudUpload aria-hidden="true" />Publish
        </button>
      </div>
      {syncError || actionMessage ? (
        <div className={`game-editor-message ${syncError ? "error" : ""}`} role="status">
          {syncError ? <AlertTriangle aria-hidden="true" /> : null}
          <span>{syncError ?? actionMessage}</span>
        </div>
      ) : null}

      <dialog className="game-source-dialog" ref={sourceDialog} onClick={(event) => {
        if (event.target === event.currentTarget) event.currentTarget.close();
      }}>
        <div className="game-dialog-shell">
          <header>
            <div><span>Portable source</span><h2>GameSourceV1</h2></div>
            <button type="button" className="icon-button" onClick={() => sourceDialog.current?.close()} aria-label="Close"><X aria-hidden="true" /></button>
          </header>
          <textarea value={sourceText} onChange={(event) => setSourceText(event.target.value)} spellCheck={false} aria-label="Game source JSON" />
          {sourceError ? <p className="inline-error" role="alert">{sourceError}</p> : null}
          <footer><button type="button" className="secondary-button" onClick={() => sourceDialog.current?.close()}>Cancel</button><button type="button" className="primary-button" onClick={applySource}>Apply source</button></footer>
        </div>
      </dialog>

      <dialog className="game-format-dialog" ref={formatDialog} onClick={(event) => {
        if (event.target === event.currentTarget) event.currentTarget.close();
      }}>
        <form className="game-dialog-shell compact" onSubmit={(event) => { event.preventDefault(); applyFormat(); }}>
          <header>
            <div><span>Output</span><h2>Frame size</h2></div>
            <button type="button" className="icon-button" onClick={() => formatDialog.current?.close()} aria-label="Close"><X aria-hidden="true" /></button>
          </header>
          <div className="game-format-body">
            <div className="game-format-presets" role="radiogroup" aria-label="Frame size presets">
              {VIEWPORT_PRESETS.map((preset) => {
                const selected = formatWidth === preset.width && formatHeight === preset.height;
                return (
                  <button
                    type="button"
                    role="radio"
                    aria-checked={selected}
                    className={selected ? "selected" : ""}
                    key={`${preset.width}x${preset.height}`}
                    onClick={() => { setFormatWidth(preset.width); setFormatHeight(preset.height); setFormatError(null); }}
                  >
                    <i style={{ aspectRatio: `${preset.width} / ${preset.height}` }} />
                    <span><strong>{preset.aspectRatio}</strong><small>{preset.width} × {preset.height}</small></span>
                  </button>
                );
              })}
            </div>
            <div className="game-format-custom">
              <label className="editor-field"><span>Width</span><input type="number" min="240" max="8192" step="1" value={formatWidth} onChange={(event) => setFormatWidth(Number(event.target.value))} /></label>
              <span aria-hidden="true">×</span>
              <label className="editor-field"><span>Height</span><input type="number" min="240" max="8192" step="1" value={formatHeight} onChange={(event) => setFormatHeight(Number(event.target.value))} /></label>
            </div>
            {formatError ? <p className="inline-error" role="alert">{formatError}</p> : null}
          </div>
          <footer><button type="button" className="secondary-button" onClick={() => formatDialog.current?.close()}>Cancel</button><button type="submit" className="primary-button">Apply</button></footer>
        </form>
      </dialog>

      <dialog className="game-publish-dialog" ref={publishDialog} onClick={(event) => {
        if (event.target === event.currentTarget) event.currentTarget.close();
      }}>
        <form className="game-dialog-shell compact" onSubmit={(event) => { event.preventDefault(); void publish(); }}>
          <header>
            <div><span>Immutable release</span><h2>Publish this draft</h2></div>
            <button type="button" className="icon-button" onClick={() => publishDialog.current?.close()} aria-label="Close"><X aria-hidden="true" /></button>
          </header>
          <label className="editor-field"><span>Release note</span><input value={changeSummary} onChange={(event) => setChangeSummary(event.target.value)} placeholder="What changed?" maxLength={500} /></label>
          <p>Existing sessions stay pinned to their current release. New sessions use this one.</p>
          <footer><button type="button" className="secondary-button" onClick={() => publishDialog.current?.close()}>Cancel</button><button type="submit" className="primary-button" disabled={actionStatus !== "idle"}>{actionStatus === "publishing" ? "Publishing..." : "Publish release"}</button></footer>
        </form>
      </dialog>
    </header>
  );
}

function SyncState({ status }: { status: GameSyncStatus }) {
  const label = status === "saved"
    ? "Saved"
    : status === "saving"
      ? "Saving"
      : status === "dirty"
        ? "Unsaved"
        : status === "conflict"
          ? "Conflict"
          : "Save failed";
  return <span className={`game-sync-state ${status}`}><i aria-hidden="true" />{label}</span>;
}
