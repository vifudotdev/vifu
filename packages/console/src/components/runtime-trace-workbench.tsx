"use client";

import { Search, X } from "lucide-react";
import {
  memo,
  useCallback,
  useDeferredValue,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import type { CSSProperties } from "react";
import { Group as PanelGroup, Panel, Separator as PanelResizeHandle } from "react-resizable-panels";
import { List } from "react-window";
import { useRuntimeConsoleHost } from "../host";
import {
  buildTraceObservationForest,
  exactTraceLookupPath,
  flattenTraceObservationForest,
  formatMetric,
  observationMetadata,
  observationModel,
  observationTimelineOffsets,
  observationTtftMs,
  observationType,
  observationUsage,
  sameTraceSpanRevision,
  traceListPresentation,
  traceListPath,
  traceIdForInvocation,
  traceEventSpansForSelection,
  traceSearchText,
  traceScoresForSelection,
  traceSelectionFromUrl,
  traceSelectionUrl,
  traceIoValues,
  type TraceListPresentation,
  type TraceListCursor,
  type TraceStatusGroup,
} from "../trace-model";
import { decodeTracePayload } from "../trace-payload";
import { traceDateWindowChanged } from "../trace-window";
import type {
  AgentProfileDetail,
  EndpointTrace,
  ProfileVersionWithCapabilities,
  TraceScore,
  TraceSpan,
  TraceUsage,
} from "../types";

const TRACE_PAGE_SIZE = 100;
const ROW_HEIGHT = 42;
const TRACE_POLL_MS = 2_000;
const TRACE_REQUEST_TIMEOUT_MS = 8_000;

type RuntimeTraceWorkbenchProps = {
  projectId: string;
  projectSlug: string;
  profileDetails?: AgentProfileDetail[];
  traces: EndpointTrace[];
};

type RuntimeTraceResponse = {
  traces?: EndpointTrace[];
  nextCursor?: TraceListCursor | null;
  error?: { message?: string };
};

type RuntimeTraceSpansResponse = {
  spans?: TraceSpan[];
  scores?: TraceScore[];
  error?: { message?: string };
};

type RuntimeTraceScoresResponse = {
  scores?: TraceScore[];
  error?: { message?: string };
};

type TraceListRow = {
  presentation: TraceListPresentation;
  searchText: string;
  trace: EndpointTrace;
};

type ObservationView = "tree" | "timeline";
type DetailTab = "summary" | "io" | "metadata" | "scores" | "events";

export function RuntimeTraceWorkbench({
  profileDetails = [],
  projectId,
  projectSlug,
  traces: initialTraces,
}: RuntimeTraceWorkbenchProps) {
  const { request } = useRuntimeConsoleHost();
  const [traces, setTraces] = useState(() => sortTraces(initialTraces));
  const [pausedTraces, setPausedTraces] = useState<EndpointTrace[]>([]);
  const [paused, setPaused] = useState(false);
  const [dateFrom, setDateFrom] = useState("");
  const [dateTo, setDateTo] = useState("");
  const [nextCursor, setNextCursor] = useState<TraceListCursor | null>(null);
  const [olderLoading, setOlderLoading] = useState(false);
  const [olderError, setOlderError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const deferredQuery = useDeferredValue(query);
  const [agentFilter, setAgentFilter] = useState("all");
  const [gatewayFilter, setGatewayFilter] = useState("all");
  const [statusFilter, setStatusFilter] = useState<"all" | TraceStatusGroup>("all");
  const [selectedTraceId, setSelectedTraceId] = useState<string | null>(null);
  const [selectedObservationId, setSelectedObservationId] = useState<string | null>(null);
  const [requestedTraceId, setRequestedTraceId] = useState<string | null>(null);
  const [requestedInvocationId, setRequestedInvocationId] = useState<string | null>(null);
  const [selectedSpans, setSelectedSpans] = useState<TraceSpan[]>([]);
  const [selectedScores, setSelectedScores] = useState<TraceScore[]>([]);
  const [spansLoading, setSpansLoading] = useState(false);
  const [spansError, setSpansError] = useState<string | null>(null);
  const [pollError, setPollError] = useState<string | null>(null);
  const [selectionError, setSelectionError] = useState<string | null>(null);
  const [loadedProfileDetails, setLoadedProfileDetails] = useState<AgentProfileDetail[]>([]);
  const [traceListLoading, setTraceListLoading] = useState(initialTraces.length === 0);
  const [urlSelectionReady, setUrlSelectionReady] = useState(false);
  const tracesRef = useRef(traces);
  const pausedTracesRef = useRef(pausedTraces);
  const pausedRef = useRef(paused);
  const selectedTraceIdRef = useRef(selectedTraceId);
  const initialPageLoadedRef = useRef(false);
  const dateWindow = useMemo(() => localDateWindow(dateFrom, dateTo), [dateFrom, dateTo]);
  const previousProjectRef = useRef({ projectId, projectSlug });
  const previousDateWindowRef = useRef(dateWindow);

  useEffect(() => {
    const previousProject = previousProjectRef.current;
    previousProjectRef.current = { projectId, projectSlug };
    if (previousProject.projectId === projectId && previousProject.projectSlug === projectSlug) return;

    tracesRef.current = [];
    pausedTracesRef.current = [];
    pausedRef.current = false;
    selectedTraceIdRef.current = null;
    setTraces([]);
    setPausedTraces([]);
    setPaused(false);
    setNextCursor(null);
    setOlderLoading(false);
    setOlderError(null);
    setSelectedTraceId(null);
    setSelectedObservationId(null);
    setRequestedTraceId(null);
    setRequestedInvocationId(null);
    setSelectedSpans([]);
    setSelectedScores([]);
    setSpansError(null);
    setPollError(null);
    setSelectionError(null);
    setLoadedProfileDetails([]);
    setTraceListLoading(true);
    setUrlSelectionReady(false);
    initialPageLoadedRef.current = false;
  }, [projectId, projectSlug]);

  useEffect(() => {
    const scoped = sortTraces(initialTraces.filter((trace) => trace.projectId === projectId));
    setTraces((current) => reconcileTraces(
      scoped,
      current.filter((trace) => trace.projectId === projectId),
    ));
  }, [initialTraces, projectId]);

  useEffect(() => {
    const previousDateWindow = previousDateWindowRef.current;
    previousDateWindowRef.current = dateWindow;
    if (!traceDateWindowChanged(previousDateWindow, dateWindow)) return;

    tracesRef.current = [];
    pausedTracesRef.current = [];
    initialPageLoadedRef.current = false;
    setTraces([]);
    setPausedTraces([]);
    setNextCursor(null);
    setOlderError(null);
    setTraceListLoading(true);
    selectedTraceIdRef.current = null;
    setSelectedTraceId(null);
    setSelectedObservationId(null);
  }, [dateWindow.from, dateWindow.to]);

  useEffect(() => {
    tracesRef.current = traces;
  }, [traces]);

  useEffect(() => {
    selectedTraceIdRef.current = selectedTraceId;
  }, [selectedTraceId]);

  useEffect(() => {
    pausedTracesRef.current = pausedTraces;
  }, [pausedTraces]);

  useEffect(() => {
    pausedRef.current = paused;
  }, [paused]);

  useEffect(() => {
    const applyLocationSelection = () => {
      const selection = readTraceSelection();
      selectedTraceIdRef.current = selection.traceId;
      setSelectedTraceId(selection.traceId);
      setSelectedObservationId(selection.observationId);
      setRequestedTraceId(selection.traceId);
      setRequestedInvocationId(selection.traceId ? null : selection.invocationId);
      setSelectionError(null);
      setUrlSelectionReady(true);
    };
    applyLocationSelection();
    window.addEventListener("popstate", applyLocationSelection);
    return () => window.removeEventListener("popstate", applyLocationSelection);
  }, [projectSlug]);

  useEffect(() => {
    if (!urlSelectionReady || requestedTraceId || requestedInvocationId) return;
    writeTraceSelection(selectedTraceId, selectedObservationId);
  }, [requestedInvocationId, requestedTraceId, selectedObservationId, selectedTraceId, urlSelectionReady]);

  useEffect(() => {
    if (!requestedTraceId) return;
    const traceId = requestedTraceId;
    const localTrace = tracesRef.current.find((trace) => trace.id === traceId && trace.projectId === projectId);
    if (localTrace) {
      selectedTraceIdRef.current = localTrace.id;
      setSelectedTraceId(localTrace.id);
      setRequestedTraceId(null);
      setSelectionError(null);
      return;
    }

    const controller = new AbortController();
    let timedOut = false;
    const timeout = window.setTimeout(() => {
      timedOut = true;
      controller.abort();
    }, TRACE_REQUEST_TIMEOUT_MS);
    async function loadExactTrace() {
      try {
        const payload = await request<RuntimeTraceResponse>(
          exactTraceLookupPath(projectSlug, "traceId", traceId),
          "GET",
          undefined,
          controller.signal,
        );
        if (controller.signal.aborted) return;
        const trace = (payload.traces ?? []).find((candidate) =>
          candidate.projectId === projectId && candidate.id === traceId
        );
        if (!trace) {
          setSelectionError("The requested trace is not available in this app.");
          return;
        }
        selectedTraceIdRef.current = trace.id;
        setTraces((current) => reconcileTraces(
          [trace],
          current.filter((candidate) => candidate.projectId === projectId),
        ));
        setSelectedTraceId(trace.id);
        setRequestedTraceId(null);
        setSelectionError(null);
      } catch (error) {
        if (timedOut) {
          setSelectionError("The requested trace lookup timed out.");
        } else if (!controller.signal.aborted) {
          setSelectionError(error instanceof Error ? error.message : "Failed to load the requested trace.");
        }
      } finally {
        window.clearTimeout(timeout);
      }
    }
    void loadExactTrace();
    return () => {
      window.clearTimeout(timeout);
      controller.abort();
    };
  }, [projectId, projectSlug, request, requestedTraceId]);

  useEffect(() => {
    if (!requestedInvocationId) return;
    const invocationId: string = requestedInvocationId;
    const localTraceId = traceIdForInvocation(tracesRef.current, invocationId);
    if (localTraceId) {
      selectedTraceIdRef.current = localTraceId;
      setSelectedTraceId(localTraceId);
      setRequestedInvocationId(null);
      setSelectionError(null);
      return;
    }

    const controller = new AbortController();
    let timedOut = false;
    const timeout = window.setTimeout(() => {
      timedOut = true;
      controller.abort();
    }, TRACE_REQUEST_TIMEOUT_MS);
    async function loadExactInvocation() {
      try {
        const payload = await request<RuntimeTraceResponse>(
          exactTraceLookupPath(projectSlug, "requestId", invocationId),
          "GET",
          undefined,
          controller.signal,
        );
        if (controller.signal.aborted) return;
        const trace = (payload.traces ?? []).find((candidate) =>
          candidate.projectId === projectId && candidate.requestId === invocationId
        );
        if (!trace) {
          setSelectionError("The requested invocation is not available in this app.");
          return;
        }
        selectedTraceIdRef.current = trace.id;
        setTraces((current) => reconcileTraces(
          [trace],
          current.filter((candidate) => candidate.projectId === projectId),
        ));
        setSelectedTraceId(trace.id);
        setRequestedInvocationId(null);
        setSelectionError(null);
      } catch (error) {
        if (timedOut) {
          setSelectionError("The requested invocation lookup timed out.");
        } else if (!controller.signal.aborted) {
          setSelectionError(error instanceof Error ? error.message : "Failed to load the requested invocation.");
        }
      } finally {
        window.clearTimeout(timeout);
      }
    }
    void loadExactInvocation();
    return () => {
      window.clearTimeout(timeout);
      controller.abort();
    };
  }, [projectId, projectSlug, request, requestedInvocationId]);

  useEffect(() => {
    const controller = new AbortController();
    let timer: number | undefined;

    async function poll() {
      const requestController = new AbortController();
      const abortRequest = () => requestController.abort();
      let timedOut = false;
      const timeout = window.setTimeout(() => {
        timedOut = true;
        requestController.abort();
      }, TRACE_REQUEST_TIMEOUT_MS);
      controller.signal.addEventListener("abort", abortRequest, { once: true });
      try {
        const payload = await request<RuntimeTraceResponse>(
          traceListPath(projectSlug, {
            from: dateWindow.from,
            to: dateWindow.to,
            limit: TRACE_PAGE_SIZE,
          }),
          "GET",
          undefined,
          requestController.signal,
        );
        if (controller.signal.aborted) return;
        const fetched = sortTraces((payload.traces ?? []).filter((trace) => trace.projectId === projectId));
        if (!initialPageLoadedRef.current) {
          initialPageLoadedRef.current = true;
          setNextCursor(payload.nextCursor ?? null);
        }
        setPollError((current) => current === null ? current : null);
        if (pausedRef.current) {
          const knownTraceIds = new Set([...tracesRef.current, ...pausedTracesRef.current].map((trace) => trace.id));
          const newPausedTraces = fetched.filter((trace) => !knownTraceIds.has(trace.id));
          if (newPausedTraces.length > 0) {
            setPausedTraces((current) => reconcileTraces(newPausedTraces, current));
          }
        } else {
          setTraces((current) => reconcileTraces(
            fetched,
            current.filter((trace) => trace.projectId === projectId),
          ));
          setPausedTraces((current) => current.length === 0 ? current : []);
        }
      } catch (error) {
        if (!controller.signal.aborted) {
          const message = timedOut
            ? "Trace polling request timed out."
            : error instanceof Error ? error.message : "Failed to load traces.";
          setPollError((current) => current === message ? current : message);
        }
      } finally {
        window.clearTimeout(timeout);
        controller.signal.removeEventListener("abort", abortRequest);
        if (!controller.signal.aborted) {
          setTraceListLoading(false);
          timer = window.setTimeout(poll, TRACE_POLL_MS);
        }
      }
    }

    void poll();
    return () => {
      controller.abort();
      if (timer !== undefined) window.clearTimeout(timer);
    };
  }, [dateWindow.from, dateWindow.to, projectId, projectSlug, request]);

  const rows = useMemo<TraceListRow[]>(() => traces.map((trace) => ({
    presentation: traceListPresentation(trace),
    searchText: traceSearchText(trace),
    trace,
  })), [traces]);

  const pausedRows = useMemo<TraceListRow[]>(() => pausedTraces.map((trace) => ({
    presentation: traceListPresentation(trace),
    searchText: traceSearchText(trace),
    trace,
  })), [pausedTraces]);

  const agentOptions = useMemo(
    () => uniqueSorted([...rows, ...pausedRows].map((row) => row.presentation.agent)),
    [pausedRows, rows],
  );
  const gatewayOptions = useMemo(
    () => uniqueSorted([...rows, ...pausedRows].map((row) => row.presentation.gateway)),
    [pausedRows, rows],
  );

  const normalizedQuery = deferredQuery.trim().toLowerCase();
  const visibleRows = useMemo(() => rows.filter((row) => {
    if (agentFilter !== "all" && row.presentation.agent !== agentFilter) return false;
    if (gatewayFilter !== "all" && row.presentation.gateway !== gatewayFilter) return false;
    if (statusFilter !== "all" && row.presentation.status !== statusFilter) return false;
    return !normalizedQuery || row.searchText.includes(normalizedQuery);
  }), [agentFilter, gatewayFilter, normalizedQuery, rows, statusFilter]);

  const traceById = useMemo(() => new Map(traces.map((trace) => [trace.id, trace])), [traces]);
  const selectedTrace = selectedTraceId ? traceById.get(selectedTraceId) ?? null : null;
  const selectedProfileVersion = useMemo(() => {
    if (!selectedTrace?.profileId || !selectedTrace.profileVersionId) return null;
    return [...loadedProfileDetails, ...profileDetails]
      .find((detail) => detail.profile.id === selectedTrace.profileId)
      ?.versions.find((item) => item.version.id === selectedTrace.profileVersionId) ?? null;
  }, [loadedProfileDetails, profileDetails, selectedTrace]);

  useEffect(() => {
    if (!selectedTrace?.profileId || !selectedTrace.profileVersionId || selectedProfileVersion) return;
    const controller = new AbortController();
    void request<AgentProfileDetail>(
      `apps/${encodeURIComponent(projectSlug)}/profiles/${encodeURIComponent(selectedTrace.profileId)}`,
      "GET",
      undefined,
      controller.signal,
    ).then((detail) => {
      if (controller.signal.aborted) return;
      setLoadedProfileDetails((current) => [
        detail,
        ...current.filter((item) => item.profile.id !== detail.profile.id),
      ]);
    }).catch(() => {
      // The unavailable state in the configuration card explains archived or removed data.
    });
    return () => controller.abort();
  }, [projectSlug, request, selectedProfileVersion, selectedTrace]);

  useEffect(() => {
    const controller = new AbortController();
    let timer: number | undefined;
    if (!selectedTraceId) {
      setSelectedSpans([]);
      setSelectedScores([]);
      setSpansError(null);
      setSpansLoading(false);
      return () => controller.abort();
    }
    setSelectedSpans([]);
    setSelectedScores([]);
    setSpansError(null);
    setSpansLoading(true);
    let loaded = false;
    const loadObservations = async () => {
      const requestController = new AbortController();
      const abortRequest = () => requestController.abort();
      let timedOut = false;
      const timeout = window.setTimeout(() => {
        timedOut = true;
        requestController.abort();
      }, TRACE_REQUEST_TIMEOUT_MS);
      controller.signal.addEventListener("abort", abortRequest, { once: true });
      try {
        const tracePath = `apps/${encodeURIComponent(projectSlug)}/traces/${encodeURIComponent(selectedTraceId)}`;
        const [spanPayload, scorePayload] = await Promise.all([
          request<RuntimeTraceSpansResponse>(`${tracePath}/spans`, "GET", undefined, requestController.signal),
          request<RuntimeTraceScoresResponse>(`${tracePath}/scores`, "GET", undefined, requestController.signal)
            .catch(() => null),
        ]);
        setSelectedSpans((current) => reconcileSpans(spanPayload.spans ?? [], current));
        const scores = scorePayload?.scores ?? spanPayload.scores;
        if (scores) setSelectedScores((current) => reconcileScores(scores, current));
        setSpansError((current) => current === null ? current : null);
      } catch (error: unknown) {
        if (!controller.signal.aborted) {
          const message = timedOut
            ? "Trace observation polling request timed out."
            : error instanceof Error ? error.message : "Failed to load trace observations.";
          setSpansError((current) => current === message ? current : message);
        }
      } finally {
        window.clearTimeout(timeout);
        controller.signal.removeEventListener("abort", abortRequest);
        if (!controller.signal.aborted && !loaded) {
          loaded = true;
          setSpansLoading(false);
        }
        if (!controller.signal.aborted) {
          timer = window.setTimeout(loadObservations, TRACE_POLL_MS);
        }
      }
    };
    void loadObservations();
    return () => {
      controller.abort();
      if (timer !== undefined) window.clearTimeout(timer);
    };
  }, [projectSlug, request, selectedTraceId]);

  useEffect(() => {
    if (spansLoading || !selectedObservationId || selectedSpans.length === 0) return;
    if (!selectedSpans.some((span) => span.id === selectedObservationId)) setSelectedObservationId(null);
  }, [selectedObservationId, selectedSpans, spansLoading]);

  const selectTrace = useCallback((trace: EndpointTrace) => {
    selectedTraceIdRef.current = trace.id;
    setRequestedTraceId(null);
    setRequestedInvocationId(null);
    setSelectionError(null);
    setSelectedTraceId(trace.id);
    setSelectedObservationId(null);
  }, []);

  const rowProps = useMemo<TraceRowProps>(() => ({
    onSelectTrace: selectTrace,
    rows: visibleRows,
    selectedTraceId,
  }), [selectTrace, selectedTraceId, visibleRows]);

  const statusCounts = useMemo(() => {
    const counts = { passed: 0, problems: 0, running: 0, unknown: 0 };
    for (const row of rows) counts[row.presentation.status] += 1;
    return counts;
  }, [rows]);

  const goLive = useCallback(() => {
    setTraces((current) => reconcileTraces(
      pausedTracesRef.current,
      current,
    ));
    setPausedTraces([]);
    setPaused(false);
  }, []);

  const loadOlder = useCallback(async () => {
    if (!nextCursor || olderLoading) return;
    setOlderLoading(true);
    setOlderError(null);
    try {
      const payload = await request<RuntimeTraceResponse>(
        traceListPath(projectSlug, {
          before: nextCursor,
          from: dateWindow.from,
          to: dateWindow.to,
          limit: TRACE_PAGE_SIZE,
        }),
        "GET",
      );
      const fetched = (payload.traces ?? []).filter((trace) => trace.projectId === projectId);
      setTraces((current) => reconcileTraces(fetched, current));
      setNextCursor(payload.nextCursor ?? null);
    } catch (error) {
      setOlderError(error instanceof Error ? error.message : "Failed to load older traces.");
    } finally {
      setOlderLoading(false);
    }
  }, [dateWindow.from, dateWindow.to, nextCursor, olderLoading, projectId, projectSlug, request]);

  const resetFilters = useCallback(() => {
    setQuery("");
    setAgentFilter("all");
    setGatewayFilter("all");
    setStatusFilter("all");
    setDateFrom("");
    setDateTo("");
  }, []);

  const closeTrace = useCallback(() => {
    selectedTraceIdRef.current = null;
    setRequestedTraceId(null);
    setRequestedInvocationId(null);
    setSelectionError(null);
    setSelectedTraceId(null);
    setSelectedObservationId(null);
  }, []);

  return (
    <div className="trace-explorer">
      <header className="trace-explorer-overview">
        <div>
          <strong>Trace Explorer</strong>
          <span className={paused ? "trace-live-state paused" : "trace-live-state"}>{paused ? "Paused" : "Live"}</span>
        </div>
        <dl>
          <div><dt>Running</dt><dd>{statusCounts.running}</dd></div>
          <div><dt>Passed</dt><dd>{statusCounts.passed}</dd></div>
          <div><dt>Problems</dt><dd>{statusCounts.problems}</dd></div>
          <div><dt>Unknown</dt><dd>{statusCounts.unknown}</dd></div>
        </dl>
      </header>

      <div className="trace-explorer-toolbar">
        <label className="trace-explorer-search">
          <Search aria-hidden="true" />
          <span className="sr-only">Search traces</span>
          <input
            placeholder="Search trace, agent, Gateway, model, or error..."
            type="search"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
          />
        </label>
        <label>
          <span className="sr-only">Filter trace status</span>
          <select value={statusFilter} onChange={(event) => setStatusFilter(event.target.value as typeof statusFilter)}>
            <option value="all">All statuses</option>
            <option value="running">Running</option>
            <option value="passed">Passed</option>
            <option value="problems">Problems</option>
            <option value="unknown">Unknown</option>
          </select>
        </label>
        <label>
          <span className="sr-only">Filter agents</span>
          <select value={agentFilter} onChange={(event) => setAgentFilter(event.target.value)}>
            <option value="all">All agents</option>
            {agentOptions.map((agent) => <option key={agent} value={agent}>{agent}</option>)}
          </select>
        </label>
        <label>
          <span className="sr-only">Filter Gateways</span>
          <select value={gatewayFilter} onChange={(event) => setGatewayFilter(event.target.value)}>
            <option value="all">All Gateways</option>
            {gatewayOptions.map((gateway) => <option key={gateway} value={gateway}>{gateway}</option>)}
          </select>
        </label>
        <label className="trace-date-filter">
          <span>From</span>
          <input
            aria-label="Show traces from date"
            inputMode="numeric"
            pattern="[0-9]{4}-[0-9]{2}-[0-9]{2}"
            placeholder="YYYY-MM-DD"
            type="text"
            value={dateFrom}
            onChange={(event) => setDateFrom(event.target.value)}
          />
        </label>
        <label className="trace-date-filter">
          <span>To</span>
          <input
            aria-label="Show traces through date"
            inputMode="numeric"
            pattern="[0-9]{4}-[0-9]{2}-[0-9]{2}"
            placeholder="YYYY-MM-DD"
            type="text"
            value={dateTo}
            onChange={(event) => setDateTo(event.target.value)}
          />
        </label>
        <button className="secondary-button small" type="button" onClick={paused ? goLive : () => setPaused(true)}>
          {paused ? `Go live${pausedTraces.length > 0 ? ` (${pausedTraces.length})` : ""}` : "Pause"}
        </button>
        <button className="secondary-button small" type="button" onClick={resetFilters}>Reset</button>
      </div>

      <div className="trace-explorer-result-bar">
        <span>{visibleRows.length} shown · {rows.length} loaded</span>
        <div>
          {query !== deferredQuery ? <span>Updating results...</span> : null}
          {nextCursor ? (
            <button type="button" onClick={() => void loadOlder()} disabled={olderLoading}>
              {olderLoading ? "Loading older..." : "Load older"}
            </button>
          ) : rows.length > 0 ? <span>Beginning of available history</span> : null}
        </div>
      </div>
      {selectionError ? <div className="inline-notice error">Trace selection error: {selectionError}</div> : null}
      {pollError ? <div className="inline-notice error">Trace polling error: {pollError}</div> : null}
      {olderError ? <div className="inline-notice error">Older trace error: {olderError}</div> : null}

      <div className="trace-explorer-sheet">
        <PanelGroup orientation="horizontal" id="vifu-traces-content" className="trace-explorer-panels">
          <Panel id="trace-list-panel" minSize={selectedTrace ? "38" : "100"} defaultSize={selectedTrace ? "55" : "100"}>
            <div className="trace-explorer-list" aria-busy={traceListLoading}>
              <div className="trace-explorer-list-header" aria-hidden="true">
                <div>Date / time</div>
                <div>Status</div>
                <div>Agent</div>
                <div>Gateway</div>
                <div>Latency</div>
                <div>TTFT</div>
                <div>Tok/s</div>
                <div>App score</div>
              </div>
              {visibleRows.length === 0 ? (
                <div className="trace-explorer-empty">
                  {rows.length === 0
                    ? traceListLoading
                      ? "Loading traces..."
                      : dateWindow.from || dateWindow.to
                        ? "No traces in this date range."
                        : "Waiting for the first trace..."
                    : "No traces match these filters."}
                </div>
              ) : (
                <List
                  className="trace-explorer-virtual-list"
                  defaultHeight={560}
                  overscanCount={16}
                  rowComponent={TraceRow}
                  rowCount={visibleRows.length}
                  rowHeight={ROW_HEIGHT}
                  rowProps={rowProps}
                  style={{ height: "100%", width: "100%" }}
                />
              )}
            </div>
          </Panel>
          {selectedTrace ? (
            <>
              <PanelResizeHandle className="trace-explorer-resize-handle" />
              <Panel id="trace-detail-panel" minSize="34" defaultSize="45">
                <TraceDetail
                  trace={selectedTrace}
                  spans={selectedSpans}
                  scores={selectedScores}
                  loading={spansLoading}
                  error={spansError}
                  selectedObservationId={selectedObservationId}
                  profileVersion={selectedProfileVersion}
                  onSelectObservation={setSelectedObservationId}
                  onClose={closeTrace}
                />
              </Panel>
            </>
          ) : null}
        </PanelGroup>
      </div>
    </div>
  );
}

type TraceRowProps = {
  onSelectTrace: (trace: EndpointTrace) => void;
  rows: TraceListRow[];
  selectedTraceId: string | null;
};

function TraceRow({
  ariaAttributes,
  index,
  onSelectTrace,
  rows,
  selectedTraceId,
  style,
}: TraceRowProps & {
  ariaAttributes?: React.AriaAttributes & { role: "listitem" };
  index?: number;
  style?: CSSProperties;
}) {
  const row = rows[index ?? 0];
  if (!row) return null;
  const { presentation, trace } = row;
  const selected = trace.id === selectedTraceId;
  return (
    <div {...ariaAttributes} className="trace-explorer-row-wrap" style={style}>
      <button
        className={selected ? "trace-explorer-row selected" : "trace-explorer-row"}
        onClick={() => onSelectTrace(trace)}
        type="button"
        aria-label={`Open ${presentation.agent} trace ${trace.requestId}`}
      >
        <time title={formatDate(trace.createdAt)}>{formatShortDateTime(trace.createdAt)}</time>
        <span className={`trace-status ${presentation.status}`}>{presentation.statusLabel}</span>
        <strong title={presentation.agent}>{presentation.agent}</strong>
        <code title={presentation.gatewayId ?? presentation.gateway}>{presentation.gateway}</code>
        <span>{formatMetric(presentation.latencyMs, "ms")}</span>
        <span>{formatMetric(presentation.ttftMs, "ms")}</span>
        <span>{formatMetric(presentation.tokensPerSecond, "", 1)}</span>
        <span className={appScoreClass(presentation.appScore)}>{presentation.appScore ?? "-"}</span>
      </button>
    </div>
  );
}

const TraceDetail = memo(function TraceDetail({
  error,
  loading,
  onClose,
  onSelectObservation,
  scores,
  selectedObservationId,
  profileVersion,
  spans,
  trace,
}: {
  error: string | null;
  loading: boolean;
  onClose: () => void;
  onSelectObservation: (spanId: string | null) => void;
  scores: TraceScore[];
  selectedObservationId: string | null;
  profileVersion: ProfileVersionWithCapabilities | null;
  spans: TraceSpan[];
  trace: EndpointTrace;
}) {
  const [view, setView] = useState<ObservationView>("tree");
  const [tab, setTab] = useState<DetailTab>("summary");
  const presentation = useMemo(() => traceListPresentation(trace), [trace]);
  const selectedSpan = selectedObservationId
    ? spans.find((span) => span.id === selectedObservationId) ?? null
    : null;

  return (
    <aside className="trace-detail">
      <header className="trace-detail-header">
        <div>
          <span className={`trace-status ${presentation.status}`}>{presentation.statusLabel}</span>
          <span className="trace-detail-identity">
            <strong>{trace.profileName ?? presentation.agent}</strong>
            <small>{trace.profileVersionNumber ? `Version ${trace.profileVersionNumber}` : "Version unavailable"}</small>
          </span>
          <code title={trace.requestId}>{shortId(trace.requestId, 12)}</code>
        </div>
        <button className="icon-button" type="button" aria-label="Close trace details" onClick={onClose}>
          <X aria-hidden="true" />
        </button>
      </header>

      <div className="trace-detail-kpis">
        <div><span>Gateway</span><strong title={presentation.gatewayId ?? undefined}>{presentation.gateway}</strong></div>
        <div><span>Latency</span><strong>{formatMetric(presentation.latencyMs, "ms")}</strong></div>
        <div><span>TTFT</span><strong>{formatMetric(presentation.ttftMs, "ms")}</strong></div>
        <div><span>Tokens/s</span><strong>{formatMetric(presentation.tokensPerSecond, "", 1)}</strong></div>
      </div>

      <div className="trace-detail-workspace">
        <PanelGroup orientation="vertical" id="vifu-trace-detail-content" className="trace-detail-panels">
          <Panel id="trace-summary-panel" minSize="32" defaultSize="72">
            <div className="trace-detail-primary">
              <div className="trace-detail-tabs" role="tablist" aria-label="Trace details">
                {(["summary", "io", "metadata", "scores", "events"] as const).map((item) => (
                  <button
                    aria-selected={tab === item}
                    className={tab === item ? "active" : ""}
                    key={item}
                    onClick={() => setTab(item)}
                    role="tab"
                    type="button"
                  >
                    {detailTabLabel(item)}
                  </button>
                ))}
              </div>
              <div className="trace-detail-tab-panel" role="tabpanel">
                <TraceDetailTab
                  tab={tab}
                  trace={trace}
                  spans={spans}
                  selectedSpan={selectedSpan}
                  scores={scores}
                  profileVersion={profileVersion}
                />
              </div>
            </div>
          </Panel>
          <PanelResizeHandle className="trace-detail-resize-handle" />
          <Panel id="trace-observations-panel" minSize="14" defaultSize="28">
            <section className="observation-browser">
              <header>
                <div><strong>Observations</strong><span>{spans.length}</span></div>
                <div className="trace-segmented-control" role="group" aria-label="Observation view">
                  <button className={view === "tree" ? "active" : ""} type="button" onClick={() => setView("tree")}>Tree</button>
                  <button className={view === "timeline" ? "active" : ""} type="button" onClick={() => setView("timeline")}>Timeline</button>
                </div>
              </header>
              {loading ? <p className="trace-detail-message">Loading observations...</p> : null}
              {error ? <p className="trace-detail-message error" role="alert">{error}</p> : null}
              {!loading && !error ? (
                <ObservationList
                  trace={trace}
                  spans={spans}
                  view={view}
                  selectedObservationId={selectedObservationId}
                  onSelectObservation={onSelectObservation}
                />
              ) : null}
            </section>
          </Panel>
        </PanelGroup>
      </div>
    </aside>
  );
});

function ObservationList({
  onSelectObservation,
  selectedObservationId,
  spans,
  trace,
  view,
}: {
  onSelectObservation: (spanId: string | null) => void;
  selectedObservationId: string | null;
  spans: TraceSpan[];
  trace: EndpointTrace;
  view: ObservationView;
}) {
  const observations = useMemo(
    () => flattenTraceObservationForest(buildTraceObservationForest(spans)),
    [spans],
  );
  const traceWindow = useMemo(() => timelineWindow(trace, spans), [spans, trace]);

  if (spans.length === 0) {
    return <p className="trace-detail-message">No observations recorded for this trace.</p>;
  }
  return (
    <div className={`observation-list ${view}`}>
      <button
        className={selectedObservationId === null ? "observation-row trace-root selected" : "observation-row trace-root"}
        type="button"
        onClick={() => onSelectObservation(null)}
      >
        <span className={`observation-status ${traceListPresentation(trace).status}`} />
        <strong>Trace</strong>
        <small>{trace.operation}</small>
        <time>{formatMetric(trace.latencyMs, "ms")}</time>
      </button>
      {observations.map(({ depth, span }) => {
        const selected = span.id === selectedObservationId;
        const rowStyle = { "--observation-depth": depth } as CSSProperties;
        return (
          <button
            className={selected ? "observation-row selected" : "observation-row"}
            key={span.id}
            style={rowStyle}
            type="button"
            onClick={() => onSelectObservation(span.id)}
          >
            <span className={`observation-status ${statusGroupFromSpan(span)}`} />
            <strong title={span.name}>{span.name}</strong>
            {view === "timeline" ? (
              <span className="observation-waterfall" aria-hidden="true">
                <i style={waterfallStyle(trace, span, traceWindow)} />
              </span>
            ) : <small>{observationType(span)}</small>}
            <time>{formatMetric(span.durationMs, "ms")}</time>
          </button>
        );
      })}
    </div>
  );
}

function TraceDetailTab({
  profileVersion,
  scores,
  selectedSpan,
  spans,
  tab,
  trace,
}: {
  profileVersion: ProfileVersionWithCapabilities | null;
  scores: TraceScore[];
  selectedSpan: TraceSpan | null;
  spans: TraceSpan[];
  tab: DetailTab;
  trace: EndpointTrace;
}) {
  if (tab === "summary") {
    return (
      <TraceSummary
        profileVersion={profileVersion}
        trace={trace}
        spans={spans}
        selectedSpan={selectedSpan}
      />
    );
  }
  if (tab === "io") {
    const io = traceIoValues(trace, spans, selectedSpan);
    return (
      <div className="trace-payload-grid">
        <TracePayload title="Input" value={io.input} />
        <TracePayload title="Output" value={io.output} />
      </div>
    );
  }
  if (tab === "metadata") {
    const metadata = selectedSpan ? observationMetadata(selectedSpan) : traceMetadata(trace);
    return (
      <div className="trace-payload-grid">
        <TracePayload title="Metadata" value={metadata} />
        {!selectedSpan && trace.gatewayMetadata
          ? <TracePayload title="Gateway metadata" value={trace.gatewayMetadata} />
          : null}
      </div>
    );
  }
  if (tab === "scores") return <TraceScores scores={scores} selectedSpan={selectedSpan} />;
  return <TraceEvents spans={spans} selectedSpan={selectedSpan} />;
}

function TraceSummary({ profileVersion, trace, spans, selectedSpan }: {
  profileVersion: ProfileVersionWithCapabilities | null;
  trace: EndpointTrace;
  spans: TraceSpan[];
  selectedSpan: TraceSpan | null;
}) {
  const firstProblem = [...spans].sort(compareSpanTime).find((span) => statusGroupFromSpan(span) === "problems") ?? null;
  const childSpans = spans.filter((span) => span.parentSpanId !== null);
  const durationCandidates = childSpans.length > 0 ? childSpans : spans;
  const longest = durationCandidates.reduce<TraceSpan | null>((current, span) => {
    if (span.durationMs === null) return current;
    return current === null || (current.durationMs ?? -1) < span.durationMs ? span : current;
  }, null);
  const usage = selectedSpan
    ? observationUsage(selectedSpan)
    : trace.usage ?? traceUsageFromResponse(trace.response);
  const io = traceIoValues(trace, spans, selectedSpan);

  return (
    <div className="trace-summary-panel">
      <TraceConfiguration profileVersion={profileVersion} trace={trace} />
      <div className="trace-summary-io">
        <TracePayload title="Input" value={io.input} />
        <TracePayload title="Output" value={io.output} />
      </div>
      <dl className="trace-summary-grid">
        <div><dt>Selected</dt><dd>{selectedSpan?.name ?? "Trace"}</dd></div>
        <div><dt>Type</dt><dd>{selectedSpan ? observationType(selectedSpan) : "trace"}</dd></div>
        <div><dt>Status</dt><dd>{selectedSpan?.status ?? trace.status}</dd></div>
        <div><dt>Duration</dt><dd>{formatMetric(selectedSpan?.durationMs ?? trace.latencyMs, "ms")}</dd></div>
        <div><dt>Provider</dt><dd>{selectedSpan?.providerKey ?? trace.providerKey ?? "-"}</dd></div>
        <div><dt>Gateway</dt><dd title={trace.gatewayId ?? undefined}>{traceListPresentation(trace).gateway}</dd></div>
        <div><dt>Model</dt><dd>{selectedSpan ? observationModel(selectedSpan) ?? "-" : traceListPresentation(trace).model ?? "-"}</dd></div>
        <div><dt>TTFT</dt><dd>{formatMetric(selectedSpan ? observationTtftMs(selectedSpan) : traceListPresentation(trace).ttftMs, "ms")}</dd></div>
        <div><dt>Tokens</dt><dd>{formatUsage(usage)}</dd></div>
      </dl>
      <div className="trace-summary-findings">
        <article>
          <span>First problem</span>
          <strong>{firstProblem?.name ?? (trace.error ? "Trace" : "None")}</strong>
          <p>{firstProblem?.error ?? trace.error ?? "No failed or timed-out observation was recorded."}</p>
        </article>
        <article>
          <span>Longest observation</span>
          <strong>{longest?.name ?? "-"}</strong>
          <p>{longest?.durationMs === null || longest === null ? "No duration data." : `${longest.durationMs} ms for the longest recorded observation.`}</p>
        </article>
      </div>
      {selectedSpan?.error ? <TracePayload title="Error" value={selectedSpan.error} tone="error" /> : null}
      {!selectedSpan && trace.error ? <TracePayload title="Error" value={trace.error} tone="error" /> : null}
    </div>
  );
}

function TraceConfiguration({
  profileVersion,
  trace,
}: {
  profileVersion: ProfileVersionWithCapabilities | null;
  trace: EndpointTrace;
}) {
  const persona = profileVersion?.version.persona;
  const runtime = profileVersion?.version.runtime;
  const generation = isRecord(runtime?.generation) ? runtime.generation : null;
  const systemPrompt = typeof persona?.systemPrompt === "string" && persona.systemPrompt.trim()
    ? persona.systemPrompt
    : null;
  const generationSummary = generation
    ? [
        numberSetting("Max tokens", generation.maxTokens),
        numberSetting("Temperature", generation.temperature),
        numberSetting("Top P", generation.topP),
      ].filter((value): value is string => value !== null)
    : [];

  return (
    <section className="trace-configuration">
      <header>
        <div>
          <span>Configuration used</span>
          <strong>{trace.profileName ?? traceListPresentation(trace).agent}</strong>
        </div>
        <code>{trace.profileVersionNumber ? `Version ${trace.profileVersionNumber}` : "Unattributed"}</code>
      </header>
      {profileVersion ? (
        <>
          <dl>
            <div>
              <dt>Version ID</dt>
              <dd title={profileVersion.version.id}>{shortId(profileVersion.version.id, 12)}</dd>
            </div>
            <div>
              <dt>Provider</dt>
              <dd>{profileVersion.capabilities.map((capability) => capability.providerKey).join(", ") || "Provider default"}</dd>
            </div>
            <div>
              <dt>Generation</dt>
              <dd>{generationSummary.join(" · ") || "Provider defaults"}</dd>
            </div>
          </dl>
          <div className="trace-configuration-prompt">
            <span>System prompt</span>
            <p>{systemPrompt ?? "No system prompt was configured for this version."}</p>
          </div>
        </>
      ) : (
        <p className="trace-configuration-missing">
          {trace.profileVersionId
            ? "The exact Agent Version is no longer available in this app."
            : "This trace was recorded before Agent Version attribution was enabled."}
        </p>
      )}
    </section>
  );
}

function TraceScores({ scores, selectedSpan }: { scores: TraceScore[]; selectedSpan: TraceSpan | null }) {
  const visible = traceScoresForSelection(scores, selectedSpan?.id ?? null);
  if (visible.length === 0) return <p className="trace-detail-message">No scores recorded for this selection.</p>;
  return (
    <div className="trace-score-list">
      {visible.map((score) => (
        <article key={score.id}>
          <div><strong>{score.name}</strong><span>{score.dataType}</span></div>
          <code>{formatScoreValue(score.value)}</code>
          <small>{score.source}{selectedSpan && score.spanId === selectedSpan.id ? " · selected observation" : ""}</small>
        </article>
      ))}
    </div>
  );
}

function TraceEvents({ spans, selectedSpan }: { spans: TraceSpan[]; selectedSpan: TraceSpan | null }) {
  const eventSpans = traceEventSpansForSelection(spans, selectedSpan?.id ?? null);
  const rawEvents = selectedSpan && observationType(selectedSpan) !== "event"
    ? eventPayloads(selectedSpan.attributes)
    : [];
  if (eventSpans.length === 0 && rawEvents.length === 0) {
    return <p className="trace-detail-message">No events or linked logs recorded.</p>;
  }
  return (
    <div className="trace-event-list">
      {eventSpans.map((span) => (
        <article key={span.id}>
          <header><strong>{span.name}</strong><time>{formatShortDateTime(span.createdAt)}</time></header>
          <span className={`trace-status ${statusGroupFromSpan(span)}`}>{span.status}</span>
          {span.outputSummary !== null ? <TracePayload title="Event" value={span.outputSummary} /> : null}
          {eventPayloads(span.attributes).map((event, index) => (
            <TracePayload key={`${span.id}-${index}`} title="Linked log" value={event} />
          ))}
        </article>
      ))}
      {rawEvents.map((event, index) => <TracePayload key={index} title="Linked log" value={event} />)}
    </div>
  );
}

function TracePayload({ title, value, tone }: { title: string; value: unknown; tone?: "error" }) {
  const decoded = decodeTracePayload(value);
  return (
    <section className={tone === "error" ? "trace-payload error" : "trace-payload"}>
      <span>{title}</span>
      {decoded.kind === "conversation" ? (
        <div className="trace-conversation">
          {decoded.messages.map((message, index) => (
            <article className={`trace-message ${message.role.toLowerCase()}`} key={`${message.role}-${index}`}>
              <header>
                <strong>{message.role}</strong>
                {message.name ? <span>{message.name}</span> : null}
                {message.toolCallId ? <code>{message.toolCallId}</code> : null}
              </header>
              {message.content ? <p>{message.content}</p> : null}
              {message.toolCalls.map((toolCall, toolIndex) => (
                <section className="trace-tool-call" key={`${toolCall.id ?? toolCall.name}-${toolIndex}`}>
                  <header><strong>{toolCall.name}</strong>{toolCall.id ? <code>{toolCall.id}</code> : null}</header>
                  <StructuredValue value={toolCall.arguments} />
                </section>
              ))}
            </article>
          ))}
        </div>
      ) : decoded.kind === "embedding" ? (
        <dl className="trace-embedding-summary">
          <div><dt>Vectors</dt><dd>{decoded.count}</dd></div>
          <div><dt>Dimensions</dt><dd>{decoded.dimensions ?? "-"}</dd></div>
          <div><dt>Model</dt><dd>{decoded.model ?? "-"}</dd></div>
          <div><dt>Usage</dt><dd><StructuredValue value={decoded.usage} /></dd></div>
        </dl>
      ) : <StructuredValue value={decoded.value} />}
    </section>
  );
}

function StructuredValue({ value, depth = 0 }: { value: unknown; depth?: number }) {
  if (value === null || value === undefined) return <span className="trace-scalar">-</span>;
  if (typeof value === "string") return <p className="trace-text-value">{value}</p>;
  if (typeof value === "number" || typeof value === "boolean") {
    return <code className="trace-scalar">{String(value)}</code>;
  }
  if (depth >= 8) return <span className="trace-scalar">Nested value</span>;
  if (Array.isArray(value)) {
    if (value.length > 32 && value.every((item) => typeof item === "number")) {
      return <span className="trace-scalar">{value.length} numeric values</span>;
    }
    return (
      <ol className="trace-structured-list">
        {value.map((item, index) => <li key={index}><StructuredValue value={item} depth={depth + 1} /></li>)}
      </ol>
    );
  }
  if (isRecord(value)) {
    const entries = Object.entries(value);
    if (entries.length === 0) return <span className="trace-scalar">Empty object</span>;
    return (
      <dl className="trace-structured-object">
        {entries.map(([key, item]) => (
          <div key={key}>
            <dt>{readableKey(key)}</dt>
            <dd><StructuredValue value={item} depth={depth + 1} /></dd>
          </div>
        ))}
      </dl>
    );
  }
  return <span className="trace-scalar">{String(value)}</span>;
}

function reconcileTraces(
  primary: EndpointTrace[],
  secondary: EndpointTrace[],
): EndpointTrace[] {
  const previousById = new Map(secondary.map((trace) => [trace.id, trace]));
  const seen = new Set<string>();
  const merged: EndpointTrace[] = [];
  for (const trace of [...primary, ...secondary]) {
    if (seen.has(trace.id)) continue;
    seen.add(trace.id);
    const previous = previousById.get(trace.id);
    merged.push(previous && sameTraceRevision(previous, trace) ? previous : trace);
  }
  const next = sortTraces(merged);
  if (next.length === secondary.length && next.every((trace, index) => trace === secondary[index])) return secondary;
  return next;
}

function sameTraceRevision(a: EndpointTrace, b: EndpointTrace): boolean {
  const aPresentation = traceListPresentation(a);
  const bPresentation = traceListPresentation(b);
  return a.id === b.id
    && a.status === b.status
    && a.latencyMs === b.latencyMs
    && a.completedAt === b.completedAt
    && a.error === b.error
    && a.profileName === b.profileName
    && a.providerKey === b.providerKey
    && a.gatewayId === b.gatewayId
    && a.gatewayName === b.gatewayName
    && formatJson(a.gatewayMetadata) === formatJson(b.gatewayMetadata)
    && formatJson(a.request) === formatJson(b.request)
    && formatJson(a.response) === formatJson(b.response)
    && aPresentation.model === bPresentation.model
    && aPresentation.ttftMs === bPresentation.ttftMs
    && aPresentation.tokensPerSecond === bPresentation.tokensPerSecond
    && aPresentation.appScore === bPresentation.appScore;
}

function reconcileSpans(next: TraceSpan[], current: TraceSpan[]): TraceSpan[] {
  const currentById = new Map(current.map((span) => [span.id, span]));
  const reconciled = [...next].sort(compareSpanTime).map((span) => {
    const previous = currentById.get(span.id);
    return previous && sameTraceSpanRevision(previous, span) ? previous : span;
  });
  if (reconciled.length === current.length && reconciled.every((span, index) => span === current[index])) return current;
  return reconciled;
}

function reconcileScores(next: TraceScore[], current: TraceScore[]): TraceScore[] {
  if (next.length === current.length && next.every((score, index) => sameScore(score, current[index]))) return current;
  return next;
}

function sameScore(a: TraceScore, b: TraceScore | undefined): boolean {
  return Boolean(b)
    && a.id === b?.id
    && a.spanId === b.spanId
    && a.name === b.name
    && a.dataType === b.dataType
    && formatJson(a.value) === formatJson(b.value);
}

function sortTraces(traces: EndpointTrace[]): EndpointTrace[] {
  return [...traces].sort((a, b) => {
    const byTime = Date.parse(b.createdAt) - Date.parse(a.createdAt);
    return byTime || b.id.localeCompare(a.id);
  });
}

function statusGroupFromSpan(span: TraceSpan): TraceStatusGroup {
  const normalized = span.status.toLowerCase();
  if (["created", "pending", "queued", "running", "started", "streaming"].includes(normalized)) return "running";
  if (["completed", "ok", "passed", "success", "succeeded"].includes(normalized)) return "passed";
  if (["cancelled", "error", "failed", "rejected", "timed_out", "timeout"].includes(normalized)) return "problems";
  return "unknown";
}

function appScoreClass(value: string | null): string {
  const normalized = value?.toLowerCase();
  if (["pass", "passed", "true", "accepted"].includes(normalized ?? "")) return "app-score passed";
  if (["fail", "failed", "false", "rejected"].includes(normalized ?? "")) return "app-score problems";
  return "app-score";
}

function traceMetadata(trace: EndpointTrace): Record<string, unknown> {
  return {
    traceId: trace.id,
    requestId: trace.requestId,
    endpointId: trace.endpointId,
    projectId: trace.projectId,
    gatewaySessionId: trace.gatewaySessionId,
    gatewayId: trace.gatewayId,
    gatewayName: trace.gatewayName,
    profileId: trace.profileId,
    profileVersionId: trace.profileVersionId,
    profileVersionNumber: trace.profileVersionNumber,
    operation: trace.operation,
    providerKey: trace.providerKey,
    capabilityKind: trace.capabilityKind,
    selectionKey: trace.selectionKey,
    createdAt: trace.createdAt,
    completedAt: trace.completedAt,
  };
}

function eventPayloads(attributes: Record<string, unknown>): unknown[] {
  return [attributes.events, attributes.logs, attributes.log].flatMap((value) => {
    if (value === undefined || value === null) return [];
    return Array.isArray(value) ? value : [value];
  });
}

function traceUsageFromResponse(value: unknown): TraceUsage | null {
  if (!isRecord(value)) return null;
  if (isRecord(value.usage)) return value.usage;
  if (isRecord(value.metadata) && isRecord(value.metadata.usage)) return value.metadata.usage;
  return null;
}

function formatUsage(usage: TraceUsage | null): string {
  if (!usage) return "-";
  const input = firstNumber(usage.inputTokens, usage.promptTokens, usage.input_tokens, usage.prompt_tokens);
  const output = firstNumber(usage.outputTokens, usage.completionTokens, usage.output_tokens, usage.completion_tokens);
  if (input === null && output === null) return "-";
  return `${input ?? "-"} in / ${output ?? "-"} out`;
}

function formatScoreValue(value: unknown): string {
  if (typeof value === "string") return value;
  if (typeof value === "boolean" || typeof value === "number") return String(value);
  return formatJson(value);
}

function timelineWindow(trace: EndpointTrace, spans: TraceSpan[]): { start: number; duration: number } {
  const ends = spans.map((span) => spanTimelineBounds(trace, span).endMs);
  return {
    start: 0,
    duration: Math.max(1, trace.latencyMs ?? 0, ...ends),
  };
}

function waterfallStyle(
  trace: EndpointTrace,
  span: TraceSpan,
  window: { start: number; duration: number },
): CSSProperties {
  const bounds = spanTimelineBounds(trace, span);
  const left = ((bounds.startMs - window.start) / window.duration) * 100;
  const width = ((bounds.endMs - bounds.startMs || 1) / window.duration) * 100;
  return {
    left: `${Math.max(0, Math.min(99, left))}%`,
    width: `${Math.max(1, Math.min(100 - left, width))}%`,
  };
}

function spanTimelineBounds(
  trace: EndpointTrace,
  span: TraceSpan,
): { startMs: number; endMs: number } {
  const offsets = observationTimelineOffsets(span);
  if (offsets) return offsets;

  const traceStart = Date.parse(trace.createdAt);
  const spanStart = Date.parse(span.createdAt);
  const startMs = Number.isFinite(traceStart) && Number.isFinite(spanStart)
    ? Math.max(0, spanStart - traceStart)
    : 0;
  const explicitEnd = span.completedAt ? Date.parse(span.completedAt) : Number.NaN;
  const endMs = Number.isFinite(traceStart) && Number.isFinite(explicitEnd)
    ? Math.max(startMs, explicitEnd - traceStart)
    : startMs + Math.max(0, span.durationMs ?? 0);
  return { startMs, endMs };
}

function compareSpanTime(a: TraceSpan, b: TraceSpan): number {
  const aOffsets = observationTimelineOffsets(a);
  const bOffsets = observationTimelineOffsets(b);
  if (aOffsets && bOffsets && aOffsets.startMs !== bOffsets.startMs) {
    return aOffsets.startMs - bOffsets.startMs;
  }
  if (aOffsets && !bOffsets) return -1;
  if (!aOffsets && bOffsets) return 1;
  return Date.parse(a.createdAt) - Date.parse(b.createdAt);
}

function detailTabLabel(tab: DetailTab): string {
  if (tab === "io") return "I/O";
  return tab.charAt(0).toUpperCase() + tab.slice(1);
}

function readTraceSelection(): {
  traceId: string | null;
  invocationId: string | null;
  observationId: string | null;
} {
  if (typeof window === "undefined") {
    return { traceId: null, invocationId: null, observationId: null };
  }
  return traceSelectionFromUrl(window.location.href);
}

function writeTraceSelection(traceId: string | null, observationId: string | null) {
  if (typeof window === "undefined") return;
  const next = traceSelectionUrl(window.location.href, traceId, observationId);
  const current = `${window.location.pathname}${window.location.search}${window.location.hash}`;
  if (next !== current) window.history.replaceState(window.history.state, "", next);
}

function uniqueSorted(values: string[]): string[] {
  return Array.from(new Set(values)).sort((a, b) => a.localeCompare(b));
}

function formatJson(value: unknown): string {
  if (value === null || value === undefined) return "-";
  if (typeof value === "string") return value;
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

function formatDate(value: string | null): string {
  if (!value) return "-";
  const date = new Date(value);
  if (Number.isNaN(date.valueOf())) return "-";
  return new Intl.DateTimeFormat("en", {
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
    month: "short",
    second: "2-digit",
    timeZoneName: "short",
    year: "numeric",
  }).format(date);
}

function formatShortDateTime(value: string | null): string {
  if (!value) return "-";
  const date = new Date(value);
  if (Number.isNaN(date.valueOf())) return "-";
  return new Intl.DateTimeFormat("en", {
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    month: "short",
    second: "2-digit",
  }).format(date);
}

function localDateWindow(from: string, to: string): { from: string | null; to: string | null } {
  return {
    from: localDateBoundary(from, false),
    to: localDateBoundary(to, true),
  };
}

function localDateBoundary(value: string, nextDay: boolean): string | null {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
  if (!match) return null;
  const date = new Date(Number(match[1]), Number(match[2]) - 1, Number(match[3]));
  if (nextDay) date.setDate(date.getDate() + 1);
  return Number.isNaN(date.valueOf()) ? null : date.toISOString();
}

function readableKey(value: string): string {
  const spaced = value
    .replaceAll("_", " ")
    .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
    .trim();
  return spaced ? spaced.charAt(0).toUpperCase() + spaced.slice(1) : value;
}

function shortId(value: string, length = 8): string {
  return value.length <= length ? value : `${value.slice(0, length)}...`;
}

function firstNumber(...values: unknown[]): number | null {
  for (const value of values) {
    if (typeof value === "number" && Number.isFinite(value)) return value;
  }
  return null;
}

function numberSetting(label: string, value: unknown): string | null {
  return typeof value === "number" && Number.isFinite(value) ? `${label} ${value}` : null;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
