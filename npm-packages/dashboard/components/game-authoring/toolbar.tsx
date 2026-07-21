"use client";

import { useRef, useState } from "react";
import { useRouter } from "next/navigation";
import {
  AlertTriangle,
  Braces,
  Check,
  CloudUpload,
  Download,
  Play,
  Redo2,
  RotateCcw,
  Undo2,
  X,
} from "lucide-react";
import { runtimeBrowserRequest } from "../../lib/runtime-browser-client";
import { managedPresentationFromSource } from "../../lib/game-authoring";
import type { GameDraft, GameRelease, GameSource, GameValidationIssue } from "../../lib/runtime-types";
import { useGameAuthoring, useGameAuthoringStore, useGameHistory, type GameSyncStatus } from "./store";

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
  const [sourceText, setSourceText] = useState("");
  const [sourceError, setSourceError] = useState<string | null>(null);
  const [changeSummary, setChangeSummary] = useState("");
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

  const publishDisabled = syncStatus !== "saved" || actionStatus !== "idle";
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
