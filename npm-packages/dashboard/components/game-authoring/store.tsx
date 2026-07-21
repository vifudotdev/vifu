"use client";

import { createContext, useContext, useEffect, useRef } from "react";
import type { ReactNode } from "react";
import type { TemporalState } from "zundo";
import { temporal } from "zundo";
import { useStore } from "zustand";
import type { StoreApi } from "zustand";
import { createStore } from "zustand/vanilla";
import type {
  AgentProfile,
  GameDraft,
  GameNodeDefinition,
  GameSource,
  GameSourceEdge,
  GameSourceNode,
  GameValidationIssue,
} from "../../lib/runtime-types";
import {
  addSourceEdge,
  createSourceNode,
  placeNodeOnTimeline,
  removeNodeFromSource,
  removeSourceEdge,
  reorderTimelineNodes,
  replaceSourceNode,
  setCanvasPosition,
  sourceFingerprint,
  splitTimelineSourceNode,
  type CanvasPosition,
} from "../../lib/game-authoring";
import { RuntimeBrowserError, runtimeBrowserRequest } from "../../lib/runtime-browser-client";

export type GameSyncStatus = "saved" | "dirty" | "saving" | "conflict" | "error";

type HistoryState = Pick<GameAuthoringState, "source">;

type GameAuthoringState = {
  source: GameSource;
  revision: number;
  contentHash: string;
  persistedFingerprint: string;
  selectedNodeId: string | null;
  syncStatus: GameSyncStatus;
  syncError: string | null;
  validationIssues: GameValidationIssue[];
  setSelectedNode: (nodeId: string | null) => void;
  setSource: (source: GameSource) => void;
  addNode: (definition: GameNodeDefinition, position: CanvasPosition, profile?: AgentProfile) => void;
  updateNode: (node: GameSourceNode) => void;
  deleteNode: (nodeId: string) => void;
  addEdge: (edge: GameSourceEdge) => void;
  deleteEdge: (edgeId: string) => void;
  moveNode: (nodeId: string, position: CanvasPosition) => void;
  placeTimelineNode: (nodeId: string, trackId: string, startMs: number, durationMs?: number) => void;
  reorderTimeline: (trackId: string, orderedNodeIds: string[]) => void;
  splitTimelineNode: (nodeId: string, outputPort: string, inputPort: string) => void;
  beginSave: () => void;
  applySave: (savedSourceFingerprint: string, draft: GameDraft) => void;
  failSave: (message: string, conflict: boolean) => void;
  setValidationIssues: (issues: GameValidationIssue[]) => void;
  replaceFromServer: (draft: GameDraft) => void;
};

export type GameAuthoringStore = StoreApi<GameAuthoringState> & {
  temporal: StoreApi<TemporalState<HistoryState>>;
};

const GameAuthoringContext = createContext<GameAuthoringStore | null>(null);

export function GameAuthoringProvider({ children, draft }: { children: ReactNode; draft: GameDraft }) {
  const [store] = useConstant(() => createGameAuthoringStore(draft));
  return <GameAuthoringContext.Provider value={store}>{children}</GameAuthoringContext.Provider>;
}

export function useGameAuthoring<T>(selector: (state: GameAuthoringState) => T): T {
  const store = useGameAuthoringStore();
  return useStore(store, selector);
}

export function useGameAuthoringStore(): GameAuthoringStore {
  const store = useContext(GameAuthoringContext);
  if (!store) throw new Error("GameAuthoringProvider is missing.");
  return store;
}

export function useGameHistory() {
  const store = useGameAuthoringStore();
  const canUndo = useStore(store.temporal, (state) => state.pastStates.length > 0);
  const canRedo = useStore(store.temporal, (state) => state.futureStates.length > 0);
  return {
    canUndo,
    canRedo,
    undo: () => store.temporal.getState().undo(),
    redo: () => store.temporal.getState().redo(),
    clear: () => store.temporal.getState().clear(),
  };
}

export function useGameDraftSync(projectSlug: string) {
  const store = useGameAuthoringStore();
  const source = useStore(store, (state) => state.source);
  const persistedFingerprint = useStore(store, (state) => state.persistedFingerprint);
  const syncStatus = useStore(store, (state) => state.syncStatus);
  const sourceHash = sourceFingerprint(source);
  const saveInFlight = useRef(false);
  const queued = useRef(false);

  useEffect(() => {
    if (sourceHash === persistedFingerprint) return;
    if (syncStatus === "conflict") return;
    const timeout = window.setTimeout(() => void saveCurrentDraft(), 850);
    return () => window.clearTimeout(timeout);

    async function saveCurrentDraft() {
      if (saveInFlight.current) {
        queued.current = true;
        return;
      }
      saveInFlight.current = true;
      const current = store.getState();
      const savingSource = current.source;
      const savingFingerprint = sourceFingerprint(savingSource);
      current.beginSave();
      try {
        const response = await runtimeBrowserRequest<{ draft: GameDraft }>(
          `project/${encodeURIComponent(projectSlug)}/game/source`,
          "PUT",
          {
            source: savingSource,
            expectedRevision: current.revision,
            expectedHash: current.contentHash,
          },
        );
        store.getState().applySave(savingFingerprint, response.draft);
      } catch (error) {
        store.getState().failSave(
          error instanceof Error ? error.message : "The draft could not be saved.",
          error instanceof RuntimeBrowserError && error.status === 409,
        );
      } finally {
        saveInFlight.current = false;
        if (queued.current) {
          queued.current = false;
          if (sourceFingerprint(store.getState().source) !== store.getState().persistedFingerprint) {
            void saveCurrentDraft();
          }
        }
      }
    }
  }, [persistedFingerprint, projectSlug, sourceHash, store, syncStatus]);
}

function createGameAuthoringStore(draft: GameDraft): GameAuthoringStore {
  return createStore<GameAuthoringState>()(
    temporal<GameAuthoringState, [], [], HistoryState>(
      (set, get) => ({
        source: draft.source,
        revision: draft.revision,
        contentHash: draft.contentHash,
        persistedFingerprint: sourceFingerprint(draft.source),
        selectedNodeId: null,
        syncStatus: "saved",
        syncError: null,
        validationIssues: [],
        setSelectedNode: (selectedNodeId) => set({ selectedNodeId }),
        setSource: (source) => set({ source, syncStatus: "dirty", syncError: null }),
        addNode: (definition, position, profile) => set({
          source: createSourceNode(get().source, definition, position, profile),
          syncStatus: "dirty",
          syncError: null,
        }),
        updateNode: (node) => set({
          source: replaceSourceNode(get().source, node),
          syncStatus: "dirty",
          syncError: null,
        }),
        deleteNode: (nodeId) => set({
          source: removeNodeFromSource(get().source, nodeId),
          selectedNodeId: get().selectedNodeId === nodeId ? null : get().selectedNodeId,
          syncStatus: "dirty",
          syncError: null,
        }),
        addEdge: (edge) => set({
          source: addSourceEdge(get().source, edge),
          syncStatus: "dirty",
          syncError: null,
        }),
        deleteEdge: (edgeId) => set({
          source: removeSourceEdge(get().source, edgeId),
          syncStatus: "dirty",
          syncError: null,
        }),
        moveNode: (nodeId, position) => set({
          source: setCanvasPosition(get().source, nodeId, position),
          syncStatus: "dirty",
          syncError: null,
        }),
        placeTimelineNode: (nodeId, trackId, startMs, durationMs) => set({
          source: placeNodeOnTimeline(get().source, nodeId, trackId, startMs, durationMs),
          syncStatus: "dirty",
          syncError: null,
        }),
        reorderTimeline: (trackId, orderedNodeIds) => set({
          source: reorderTimelineNodes(get().source, trackId, orderedNodeIds),
          syncStatus: "dirty",
          syncError: null,
        }),
        splitTimelineNode: (nodeId, outputPort, inputPort) => set((state) => {
          const result = splitTimelineSourceNode(state.source, nodeId, outputPort, inputPort);
          return result ? {
            source: result.source,
            selectedNodeId: result.newNodeId,
            syncStatus: "dirty",
            syncError: null,
          } : state;
        }),
        beginSave: () => set({ syncStatus: "saving", syncError: null }),
        applySave: (savedSourceFingerprint, nextDraft) => {
          const hasNewerLocalChanges = sourceFingerprint(get().source) !== savedSourceFingerprint;
          set({
            source: hasNewerLocalChanges ? get().source : nextDraft.source,
            revision: nextDraft.revision,
            contentHash: nextDraft.contentHash,
            persistedFingerprint: sourceFingerprint(nextDraft.source),
            syncStatus: hasNewerLocalChanges ? "dirty" : "saved",
            syncError: null,
          });
        },
        failSave: (message, conflict) => set({
          syncStatus: conflict ? "conflict" : "error",
          syncError: message,
        }),
        setValidationIssues: (validationIssues) => set({ validationIssues }),
        replaceFromServer: (nextDraft) => set({
          source: nextDraft.source,
          revision: nextDraft.revision,
          contentHash: nextDraft.contentHash,
          persistedFingerprint: sourceFingerprint(nextDraft.source),
          selectedNodeId: null,
          syncStatus: "saved",
          syncError: null,
          validationIssues: [],
        }),
      }),
      {
        partialize: (state) => ({ source: state.source }),
        equality: (past, current) => sourceFingerprint(past.source) === sourceFingerprint(current.source),
        limit: 80,
      },
    ),
  ) as GameAuthoringStore;
}

function useConstant<T>(factory: () => T): [T] {
  const value = useRef<{ current: T } | null>(null);
  if (!value.current) value.current = { current: factory() };
  return [value.current.current];
}
