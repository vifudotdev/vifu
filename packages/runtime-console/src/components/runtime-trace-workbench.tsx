"use client";

import { Search, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { CSSProperties } from "react";
import { Group as PanelGroup, Panel, Separator as PanelResizeHandle } from "react-resizable-panels";
import { List } from "react-window";
import { runtimeBrowserRequest } from "../browser-client";
import type { EndpointTrace, TraceSpan } from "../types";

const MAX_LOGS = 100;
const ROW_HEIGHT = 32;

type RuntimeTraceWorkbenchProps = {
  projectId: string;
  projectSlug: string;
  traces: EndpointTrace[];
};

type RuntimeTraceResponse = {
  traces?: EndpointTrace[];
  error?: { message?: string };
};

type RuntimeTraceSpansResponse = {
  spans?: TraceSpan[];
  error?: { message?: string };
};

export function RuntimeTraceWorkbench({ projectId, projectSlug, traces: initialTraces }: RuntimeTraceWorkbenchProps) {
  const [traces, setTraces] = useState(() => sortTraces(initialTraces).slice(0, MAX_LOGS));
  const [pausedTraces, setPausedTraces] = useState<EndpointTrace[]>([]);
  const [paused, setPaused] = useState(false);
  const [query, setQuery] = useState("");
  const [agentFilter, setAgentFilter] = useState("all");
  const [statusFilter, setStatusFilter] = useState("all");
  const [hiddenTraceIds, setHiddenTraceIds] = useState<Set<string>>(() => new Set());
  const [selectedTraceId, setSelectedTraceId] = useState<string | null>(null);
  const [selectedSpans, setSelectedSpans] = useState<TraceSpan[]>([]);
  const [spansLoading, setSpansLoading] = useState(false);
  const [spansError, setSpansError] = useState<string | null>(null);
  const [streamError, setStreamError] = useState<string | null>(null);
  const tracesRef = useRef(traces);
  const pausedTracesRef = useRef(pausedTraces);

  useEffect(() => {
    setTraces((current) => mergeTraces(sortTraces(initialTraces), current).slice(0, MAX_LOGS));
  }, [initialTraces]);

  useEffect(() => {
    tracesRef.current = traces;
  }, [traces]);

  useEffect(() => {
    pausedTracesRef.current = pausedTraces;
  }, [pausedTraces]);

  useEffect(() => {
    const controller = new AbortController();

    async function poll() {
      try {
        const payload = await runtimeBrowserRequest<RuntimeTraceResponse>(
          `project/${encodeURIComponent(projectSlug)}/traces?limit=100`,
          "GET",
          undefined,
          controller.signal,
        );
        const fetched = sortTraces((payload.traces ?? []).filter((trace) => trace.projectId === projectId));
        setStreamError(null);
        if (paused) {
          const knownTraceIds = new Set([...tracesRef.current, ...pausedTracesRef.current].map((trace) => trace.id));
          const newPausedTraces = fetched.filter((trace) => !knownTraceIds.has(trace.id));
          if (newPausedTraces.length > 0) {
            setPausedTraces((current) => mergeTraces(newPausedTraces, current).slice(0, MAX_LOGS));
          }
        } else {
          setTraces((current) => mergeTraces(fetched, current).slice(0, MAX_LOGS));
          setPausedTraces([]);
        }
      } catch (error) {
        if (!controller.signal.aborted) {
          setStreamError(error instanceof Error ? error.message : "Failed to load logs.");
        }
      }
    }

    void poll();
    const timer = window.setInterval(poll, paused ? 3500 : 2000);
    return () => {
      controller.abort();
      window.clearInterval(timer);
    };
  }, [paused, projectId, projectSlug]);

  const agentOptions = useMemo(() => {
    return Array.from(new Set([...traces, ...pausedTraces].map(traceAgentLabel))).sort((a, b) => a.localeCompare(b));
  }, [pausedTraces, traces]);

  const visibleTraces = useMemo(() => {
    const normalizedQuery = query.trim().toLowerCase();
    return traces.filter((trace) => {
      if (hiddenTraceIds.has(trace.id)) return false;
      if (agentFilter !== "all" && traceAgentLabel(trace) !== agentFilter) return false;
      if (statusFilter !== "all" && !traceMatchesLogType(trace, statusFilter)) return false;
      if (!normalizedQuery) return true;
      return traceSearchText(trace).includes(normalizedQuery);
    });
  }, [agentFilter, hiddenTraceIds, query, statusFilter, traces]);

  const selectedTrace = selectedTraceId
    ? visibleTraces.find((trace) => trace.id === selectedTraceId) ?? traces.find((trace) => trace.id === selectedTraceId) ?? null
    : null;

  useEffect(() => {
    const controller = new AbortController();
    if (!selectedTraceId) {
      setSelectedSpans([]);
      setSpansError(null);
      setSpansLoading(false);
      return () => controller.abort();
    }
    setSelectedSpans([]);
    setSpansError(null);
    setSpansLoading(true);
    void (async () => {
      try {
        const payload = await runtimeBrowserRequest<RuntimeTraceSpansResponse>(
          `project/${encodeURIComponent(projectSlug)}/traces/${encodeURIComponent(selectedTraceId)}/spans`,
          "GET",
          undefined,
          controller.signal,
        );
        setSelectedSpans(payload.spans ?? []);
      } catch (error: unknown) {
        if (!controller.signal.aborted) setSpansError(error instanceof Error ? error.message : "Failed to load trace spans.");
      } finally {
        if (!controller.signal.aborted) setSpansLoading(false);
      }
    })();
    return () => controller.abort();
  }, [projectSlug, selectedTraceId]);

  const selectTrace = useCallback((trace: EndpointTrace) => {
    setSelectedTraceId(trace.id);
  }, []);

  const rowProps = useMemo<TraceRowProps>(() => ({
    onSelectTrace: selectTrace,
    selectedTraceId,
    traces: visibleTraces,
  }), [selectTrace, selectedTraceId, visibleTraces]);

  const goLive = () => {
    setTraces((current) => mergeTraces(pausedTracesRef.current, current).slice(0, MAX_LOGS));
    setPausedTraces([]);
    setPaused(false);
  };

  const clearVisibleLogs = () => {
    setHiddenTraceIds((current) => {
      const next = new Set(current);
      for (const trace of visibleTraces) next.add(trace.id);
      return next;
    });
    setSelectedTraceId(null);
  };

  return (
    <div className="convex-log-workbench">
      <div className="convex-log-toolbar">
        <div className="convex-log-toolbar-spacer" />
        <label>
          <span className="sr-only">Filter agents</span>
          <select value={agentFilter} onChange={(event) => setAgentFilter(event.target.value)}>
            <option value="all">All agents</option>
            {agentOptions.map((agent) => <option key={agent} value={agent}>{agent}</option>)}
          </select>
        </label>
        <label>
          <span className="sr-only">Filter status</span>
          <select value={statusFilter} onChange={(event) => setStatusFilter(event.target.value)}>
            <option value="all">All log types</option>
            <option value="success">success</option>
            <option value="failure">failure</option>
            <option value="debug">debug</option>
            <option value="info">info</option>
            <option value="warn">warn</option>
            <option value="error">error</option>
          </select>
        </label>
      </div>
      <div className="convex-log-search-row">
        <label className="convex-log-search">
          <Search aria-hidden="true" />
          <span className="sr-only">Filter logs</span>
          <input
            placeholder="Filter logs..."
            type="search"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
          />
        </label>
        <button className="secondary-button" type="button" disabled={visibleTraces.length === 0} onClick={clearVisibleLogs}>Clear Logs</button>
      </div>
      {streamError ? <div className="inline-notice error">Log stream error: {streamError}</div> : null}
      <div className="convex-log-sheet">
        <PanelGroup orientation="horizontal" id="vifu-logs-content" className="convex-log-panels">
          <Panel id="log-list-panel" minSize={selectedTrace ? 38 : 100} defaultSize={selectedTrace ? 62 : 100}>
            <div className="convex-log-list">
              <div className="convex-log-list-header">
                <div>Timestamp</div>
                <div>ID</div>
                <div>Status</div>
                <div>Agent</div>
                <button className="secondary-button small" type="button" onClick={() => paused ? goLive() : setPaused(true)}>
                  {paused ? `Go Live${pausedTraces.length > 0 ? ` (${pausedTraces.length})` : ""}` : "Pause"}
                </button>
              </div>
              {visibleTraces.length === 0 ? (
                <div className="convex-log-empty">
                  {query || agentFilter !== "all" || statusFilter !== "all" ? "No logs match your filters." : "Waiting for new logs..."}
                </div>
              ) : (
                <List
                  className="convex-log-virtual-list"
                  defaultHeight={560}
                  overscanCount={20}
                  rowComponent={TraceRow}
                  rowCount={visibleTraces.length}
                  rowHeight={ROW_HEIGHT}
                  rowProps={rowProps}
                  style={{ height: "100%", width: "100%" }}
                />
              )}
            </div>
          </Panel>
          {selectedTrace ? (
            <>
              <PanelResizeHandle className="convex-log-resize-handle" />
              <Panel id="log-drilldown-panel" minSize={28} defaultSize={38}>
                <TraceDrilldown
                  trace={selectedTrace}
                  spans={selectedSpans}
                  spansLoading={spansLoading}
                  spansError={spansError}
                  onClose={() => setSelectedTraceId(null)}
                  onFilterByRequestId={(requestId) => setQuery(requestId)}
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
  selectedTraceId: string | null;
  traces: EndpointTrace[];
};

function TraceRow({
  ariaAttributes,
  index,
  onSelectTrace,
  selectedTraceId,
  style,
  traces,
}: TraceRowProps & {
  ariaAttributes?: React.AriaAttributes & { role: "listitem" };
  index?: number;
  style?: CSSProperties;
}) {
  const trace = traces[index ?? 0];
  if (!trace) return null;
  const selected = trace.id === selectedTraceId;
  return (
    <div {...ariaAttributes} className="convex-log-row-wrap" style={style}>
      <button
        className={selected ? "convex-log-row selected" : "convex-log-row"}
        data-log-key={trace.id}
        onClick={() => onSelectTrace(trace)}
        type="button"
      >
        <time>{formatDate(trace.createdAt)}</time>
        <code>{shortId(trace.requestId, 4)}</code>
        <span className={statusClass(trace)}>{traceStatusLabel(trace)}</span>
        <strong>{traceAgentLabel(trace)}</strong>
        <p>{tracePreview(trace)}</p>
      </button>
    </div>
  );
}

function TraceDrilldown({
  onClose,
  onFilterByRequestId,
  trace,
  spans,
  spansLoading,
  spansError,
}: {
  onClose: () => void;
  onFilterByRequestId: (requestId: string) => void;
  trace: EndpointTrace;
  spans: TraceSpan[];
  spansLoading: boolean;
  spansError: string | null;
}) {
  return (
    <aside className="convex-log-drilldown">
      <header>
        <div>
          <time>{formatDate(trace.createdAt)}</time>
        <span className={statusClass(trace)}>{traceStatusLabel(trace)}</span>
        </div>
        <div>
          <button className="secondary-button small" type="button" onClick={() => onFilterByRequestId(trace.requestId)}>Filter request</button>
          <button className="icon-button" type="button" aria-label="Close log details" onClick={onClose}><X aria-hidden="true" /></button>
        </div>
      </header>
      <dl className="convex-log-metadata">
        <div><dt>Request ID</dt><dd><code>{trace.requestId}</code></dd></div>
        <div><dt>Agent</dt><dd>{traceAgentLabel(trace)}</dd></div>
        <div><dt>Operation</dt><dd><code>{trace.operation}</code></dd></div>
        <div><dt>Profile</dt><dd><code title={trace.profileId ?? undefined}>{trace.profileSlug ?? (trace.profileId ? shortId(trace.profileId, 12) : "-")}</code></dd></div>
        <div><dt>Version</dt><dd><code title={trace.profileVersionId ?? undefined}>{trace.profileVersionNumber === null ? (trace.profileVersionId ? shortId(trace.profileVersionId, 12) : "-") : `v${trace.profileVersionNumber}`}</code></dd></div>
        <div><dt>Capability</dt><dd>{trace.capabilityKind ?? "-"}</dd></div>
        <div><dt>Provider</dt><dd><code>{trace.providerKey ?? "-"}</code></dd></div>
        <div><dt>Latency</dt><dd>{trace.latencyMs === null ? "-" : `${trace.latencyMs} ms`}</dd></div>
        <div><dt>Gateway</dt><dd>{trace.gatewaySessionId ? shortId(trace.gatewaySessionId, 12) : "-"}</dd></div>
      </dl>
      <TraceSpanTimeline spans={spans} loading={spansLoading} error={spansError} />
      <div className="convex-log-detail-tabs">
        <TracePayload title="Request" value={trace.request} />
        <TracePayload title="Response" value={trace.response} />
        {trace.error ? <TracePayload title="Error" value={trace.error} tone="error" /> : null}
      </div>
    </aside>
  );
}

function TraceSpanTimeline({ spans, loading, error }: { spans: TraceSpan[]; loading: boolean; error: string | null }) {
  return (
    <section className="trace-span-timeline">
      <header><strong>Execution</strong><span>{spans.length} spans</span></header>
      {loading ? <p>Loading execution spans...</p> : null}
      {error ? <p className="inline-error" role="alert">{error}</p> : null}
      {!loading && !error && spans.length === 0 ? <p>No execution spans recorded.</p> : null}
      {spans.map((span) => (
        <details key={span.id}>
          <summary>
            <span className={`trace-span-status ${span.status}`} />
            <strong>{span.name}</strong>
            <small>{span.providerKey ?? span.kind}</small>
            <time>{span.durationMs === null ? "-" : `${span.durationMs} ms`}</time>
          </summary>
          <dl>
            <div><dt>Kind</dt><dd>{span.kind}</dd></div>
            <div><dt>Capability</dt><dd>{span.capabilityKind ?? "-"}</dd></div>
            <div><dt>Status</dt><dd>{span.status}</dd></div>
          </dl>
          {span.inputSummary !== null ? <TracePayload title="Input" value={span.inputSummary} /> : null}
          {span.outputSummary !== null ? <TracePayload title="Output" value={span.outputSummary} /> : null}
          {span.error ? <TracePayload title="Error" value={span.error} tone="error" /> : null}
        </details>
      ))}
    </section>
  );
}

function TracePayload({ title, value, tone }: { title: string; value: unknown; tone?: "error" }) {
  return (
    <section className={tone === "error" ? "trace-payload error" : "trace-payload"}>
      <span>{title}</span>
      <pre>{formatJson(value)}</pre>
    </section>
  );
}

function mergeTraces(primary: EndpointTrace[], secondary: EndpointTrace[]): EndpointTrace[] {
  const seen = new Set<string>();
  return sortTraces([...primary, ...secondary]).filter((trace) => {
    if (seen.has(trace.id)) return false;
    seen.add(trace.id);
    return true;
  });
}

function sortTraces(traces: EndpointTrace[]): EndpointTrace[] {
  return [...traces].sort((a, b) => Date.parse(b.createdAt) - Date.parse(a.createdAt));
}

function statusClass(trace: EndpointTrace): string {
  const level = traceLogLevel(trace);
  if (level === "info") return "status-label ready";
  if (level === "debug") return "status-label pending";
  if (level === "warn") return "status-label warning";
  return "status-label off";
}

function traceMatchesLogType(trace: EndpointTrace, type: string): boolean {
  if (type === "success") return trace.status === "completed";
  if (type === "failure") return trace.status !== "completed" && trace.status !== "pending";
  return traceLogLevel(trace) === type;
}

function traceStatusLabel(trace: EndpointTrace): string {
  return traceLogLevel(trace);
}

function traceLogLevel(trace: EndpointTrace): "debug" | "info" | "warn" | "error" {
  const requestLevel = readLogLevel(trace.request);
  if (requestLevel) return requestLevel;
  const responseLevel = readLogLevel(trace.response);
  if (responseLevel) return responseLevel;
  if (trace.status === "pending") return "debug";
  if (trace.status === "completed") return "info";
  return "error";
}

function readLogLevel(value: unknown): "debug" | "info" | "warn" | "error" | null {
  if (!isRecord(value)) return null;
  const rawLevel = value.level ?? value.logLevel;
  if (typeof rawLevel === "string") {
    const level = rawLevel.toLowerCase();
    if (level === "debug" || level === "info" || level === "warn" || level === "error") return level;
  }
  if (isRecord(value.metadata)) return readLogLevel(value.metadata);
  return null;
}

function traceAgentLabel(trace: EndpointTrace): string {
  if (trace.profileName?.trim()) return trace.profileName;
  if (trace.profileSlug?.trim()) return trace.profileSlug;
  if (isRecord(trace.request) && typeof trace.request.model === "string") return trace.request.model;
  if (isRecord(trace.response) && typeof trace.response.agentId === "string") return trace.response.agentId;
  if (isRecord(trace.request) && isRecord(trace.request.metadata) && typeof trace.request.metadata.agent === "string") {
    return trace.request.metadata.agent;
  }
  if (trace.profileId) return `Profile ${shortId(trace.profileId, 8)}`;
  return trace.gatewaySessionId ? `Gateway ${shortId(trace.gatewaySessionId, 8)}` : "Agent";
}

function tracePreview(trace: EndpointTrace): string {
  if (trace.error) return trace.error;
  if (isRecord(trace.request) && Array.isArray(trace.request.messages)) {
    const last = [...trace.request.messages].reverse().find(isRecord);
    if (last && typeof last.content === "string" && last.content.trim()) return last.content;
  }
  if (isRecord(trace.request) && typeof trace.request.message === "string" && trace.request.message.trim()) {
    return trace.request.message;
  }
  if (isRecord(trace.response) && typeof trace.response.reply === "string" && trace.response.reply.trim()) {
    return trace.response.reply;
  }
  return "Agent invocation";
}

function traceSearchText(trace: EndpointTrace): string {
  return [
    trace.requestId,
    trace.status,
    trace.operation,
    trace.providerKey ?? "",
    trace.capabilityKind ?? "",
    trace.profileId ?? "",
    trace.profileVersionId ?? "",
    trace.profileSlug ?? "",
    trace.profileName ?? "",
    trace.profileVersionNumber?.toString() ?? "",
    traceAgentLabel(trace),
    tracePreview(trace),
    trace.error ?? "",
    formatJson(trace.request),
    formatJson(trace.response),
  ].join(" ").toLowerCase();
}

function formatJson(value: unknown): string {
  if (value === null || value === undefined) return "-";
  if (typeof value === "string") return value;
  return JSON.stringify(value, null, 2);
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
    timeZone: "UTC",
    timeZoneName: "short",
    year: "numeric",
  }).format(date);
}

function shortId(value: string, length = 8): string {
  return value.length <= length ? value : `${value.slice(0, length)}...`;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
