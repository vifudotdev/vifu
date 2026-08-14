import type { EndpointTrace, TraceScore, TraceSpan, TraceUsage } from "./types";

export function traceIdForInvocation(
  traces: EndpointTrace[],
  invocationId: string | null,
): string | null {
  if (!invocationId) return null;
  return traces.find((trace) => trace.requestId === invocationId)?.id ?? null;
}

export type TraceStatusGroup = "running" | "passed" | "problems" | "unknown";

export type TraceListPresentation = {
  agent: string;
  appScore: string | null;
  gateway: string;
  gatewayId: string | null;
  latencyMs: number | null;
  model: string | null;
  status: TraceStatusGroup;
  statusLabel: string;
  tokensPerSecond: number | null;
  ttftMs: number | null;
};

export type TraceObservationNode = {
  children: TraceObservationNode[];
  span: TraceSpan;
};

export type FlatTraceObservation = {
  depth: number;
  span: TraceSpan;
};

export type TraceUrlSelection = {
  traceId: string | null;
  invocationId: string | null;
  observationId: string | null;
};

export type TraceListCursor = {
  createdAt: string;
  traceId: string;
};

export type TraceListQuery = {
  before?: TraceListCursor | null;
  from?: string | null;
  limit?: number;
  to?: string | null;
};

const RUNNING_STATUSES = new Set(["created", "pending", "queued", "running", "started", "streaming"]);
const PASSED_STATUSES = new Set(["completed", "ok", "passed", "success", "succeeded"]);
const PROBLEM_STATUSES = new Set(["cancelled", "error", "failed", "rejected", "timed_out", "timeout"]);

export function traceListPresentation(trace: EndpointTrace): TraceListPresentation {
  const runtimeStatus = traceStatusGroup(trace.status);
  const model = firstString(
    optionalTraceValue(trace, "model"),
    nestedValue(trace.request, "model"),
    nestedValue(trace.response, "model"),
    nestedValue(trace.response, "metadata", "model"),
  );
  const usage = traceUsageFromUnknown(
    optionalTraceValue(trace, "usage")
      ?? nestedValue(trace.response, "usage")
      ?? nestedValue(trace.response, "metadata", "usage"),
  );
  const ttftMs = firstFiniteNumber(
    optionalTraceValue(trace, "ttftMs"),
    optionalTraceValue(trace, "completionStartMs"),
    nestedValue(trace.response, "ttftMs"),
    nestedValue(trace.response, "timeToFirstTokenMs"),
    nestedValue(trace.response, "metadata", "ttftMs"),
  );
  const explicitTokensPerSecond = firstFiniteNumber(
    optionalTraceValue(trace, "tokensPerSecond"),
    nestedValue(trace.response, "tokensPerSecond"),
    nestedValue(trace.response, "metadata", "tokensPerSecond"),
  );
  const outputTokens = usageOutputTokens(usage);
  const decodeMs = firstFiniteNumber(
    trace.decodeMs,
    nestedValue(trace.response, "decodeMs"),
    nestedValue(trace.response, "metadata", "decodeMs"),
  );
  const derivedTokensPerSecond = outputTokens !== null && decodeMs !== null && decodeMs > 0
    ? outputTokens / (decodeMs / 1_000)
    : null;
  const scores = traceScoresFromUnknown(
    optionalTraceValue(trace, "scores") ?? nestedValue(trace.response, "scores"),
  );
  const appScore = normalizedScoreValue(trace.appOutcome) ?? appScoreLabel(scores);
  const status = appScore === "fail" ? "problems" : runtimeStatus;
  const gatewayId = firstString(
    trace.gatewayId,
    nestedValue(trace.request, "gatewayId"),
  );
  const gateway = firstString(
    trace.gatewayName,
    trace.gatewayMetadata?.name,
    nestedValue(trace.request, "gatewayName"),
  ) ?? (gatewayId ? gatewayId.replace(/^gateway-/, "Gateway ") : "Unknown Gateway");

  return {
    agent: traceAgentLabel(trace),
    appScore,
    gateway,
    gatewayId,
    latencyMs: trace.latencyMs,
    model,
    status,
    statusLabel: appScore === "fail"
      ? "application failed"
      : normalizedStatusLabel(trace.status, runtimeStatus),
    tokensPerSecond: explicitTokensPerSecond ?? derivedTokensPerSecond,
    ttftMs,
  };
}

export function traceStatusGroup(status: string): TraceStatusGroup {
  const normalized = status.trim().toLowerCase();
  if (RUNNING_STATUSES.has(normalized)) return "running";
  if (PASSED_STATUSES.has(normalized)) return "passed";
  if (PROBLEM_STATUSES.has(normalized)) return "problems";
  return "unknown";
}

export function traceAgentLabel(trace: EndpointTrace): string {
  if (trace.profileName?.trim()) return trace.profileName.trim();
  if (trace.profileSlug?.trim()) return trace.profileSlug.trim();
  const requestAgent = firstString(
    nestedValue(trace.request, "agent"),
    nestedValue(trace.request, "agentId"),
    nestedValue(trace.request, "metadata", "agent"),
    nestedValue(trace.response, "agent"),
    nestedValue(trace.response, "agentId"),
  );
  if (requestAgent) return requestAgent;
  if (trace.profileId) return `Profile ${shortId(trace.profileId, 8)}`;
  return trace.gatewaySessionId ? `Gateway ${shortId(trace.gatewaySessionId, 8)}` : "Agent";
}

export function traceProviderLabel(
  trace: EndpointTrace,
  providers: ReadonlyArray<{
    config?: Record<string, unknown>;
    name: string;
    providerKey: string;
  }>,
): string {
  const declaredName = firstString(
    trace.providerName,
    nestedValue(trace.request, "providerName"),
  );
  if (declaredName) return declaredName;
  if (!trace.providerKey) return "Unknown Provider";
  const exact = providers.find((provider) => provider.providerKey === trace.providerKey);
  if (exact) return exact.name;

  const runtimeProviderKey = firstString(
    nestedValue(trace.request, "runtimeProviderKey"),
    trace.providerKey.split("--", 1)[0],
  );
  const gatewayId = firstString(trace.gatewayId, nestedValue(trace.request, "gatewayId"));
  const compatible = providers.find((provider) => (
    firstString(provider.config?.runtimeProviderKey) === runtimeProviderKey
    && (!gatewayId || firstString(provider.config?.gatewayId) === gatewayId)
  ));
  if (compatible) return compatible.name;

  return "Unknown Provider";
}

export function traceSearchText(trace: EndpointTrace): string {
  const presentation = traceListPresentation(trace);
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
    presentation.agent,
    presentation.gateway,
    presentation.gatewayId ?? "",
    presentation.model ?? "",
    trace.error ?? "",
  ].join(" ").toLowerCase();
}

export function buildTraceObservationForest(spans: TraceSpan[]): TraceObservationNode[] {
  const ordered = [...spans].sort(compareSpans);
  const nodes = new Map<string, TraceObservationNode>();
  for (const span of ordered) nodes.set(span.id, { children: [], span });

  const roots: TraceObservationNode[] = [];
  for (const span of ordered) {
    const node = nodes.get(span.id);
    if (!node) continue;
    const parent = span.parentSpanId ? nodes.get(span.parentSpanId) : undefined;
    if (!parent || parent === node || wouldCreateCycle(nodes, span.id, span.parentSpanId)) {
      roots.push(node);
    } else {
      parent.children.push(node);
    }
  }
  return roots;
}

export function flattenTraceObservationForest(forest: TraceObservationNode[]): FlatTraceObservation[] {
  const flattened: FlatTraceObservation[] = [];
  const visited = new Set<string>();
  const visit = (node: TraceObservationNode, depth: number) => {
    if (visited.has(node.span.id)) return;
    visited.add(node.span.id);
    flattened.push({ depth, span: node.span });
    for (const child of node.children) visit(child, depth + 1);
  };
  for (const root of forest) visit(root, 0);
  return flattened;
}

export function observationType(span: TraceSpan): string {
  const value = span.observationType?.trim() || span.kind.trim();
  return value ? value.toLowerCase() : "span";
}

export function observationModel(span: TraceSpan): string | null {
  return firstString(span.model, span.attributes.model, span.attributes.modelName);
}

export function observationTtftMs(span: TraceSpan): number | null {
  return firstFiniteNumber(
    span.completionStartMs,
    span.attributes.completionStartMs,
    span.attributes.ttftMs,
    span.attributes.timeToFirstTokenMs,
  );
}

export function observationUsage(span: TraceSpan): TraceUsage | null {
  return traceUsageFromUnknown(span.usage ?? span.attributes.usage);
}

export function observationMetadata(span: TraceSpan): Record<string, unknown> {
  return {
    traceId: span.traceId,
    observationId: span.id,
    parentObservationId: span.parentSpanId,
    observationType: observationType(span),
    name: span.name,
    providerKey: span.providerKey,
    model: observationModel(span),
    capabilityKind: span.capabilityKind,
    modelParameters: span.modelParameters ?? null,
    attributes: Object.fromEntries(
      Object.entries(span.attributes)
        .filter(([key]) => !key.startsWith("_vifu"))
        .filter(([key]) => !["events", "logs", "log"].includes(key.toLowerCase())),
    ),
  };
}

export function traceIoValues(
  trace: EndpointTrace,
  spans: TraceSpan[],
  selectedSpan: TraceSpan | null,
): { input: unknown; output: unknown } {
  const rootGeneration = spans.find((span) =>
    span.parentSpanId === null && observationType(span) === "generation"
  ) ?? null;
  if (selectedSpan && selectedSpan.id !== rootGeneration?.id) {
    return { input: selectedSpan.inputSummary, output: selectedSpan.outputSummary };
  }
  if (!rootGeneration) return { input: trace.request, output: trace.response };
  const marker = isRecord(rootGeneration.attributes._vifuTraceIo)
    ? rootGeneration.attributes._vifuTraceIo
    : {};
  return {
    input: marker.inputCanonical === true
      ? rootGeneration.inputSummary
      : trace.request,
    output: marker.outputCanonical === true
      ? rootGeneration.outputSummary
      : trace.response,
  };
}

export function observationTimelineOffsets(
  span: TraceSpan,
): { startMs: number; endMs: number } | null {
  const startMs = finiteNonNegativeNumber(span.attributes.startOffsetMs);
  if (startMs === null) return null;
  const explicitEndMs = finiteNonNegativeNumber(span.attributes.endOffsetMs);
  const endMs = explicitEndMs
    ?? (span.durationMs !== null && Number.isFinite(span.durationMs) && span.durationMs >= 0
      ? startMs + span.durationMs
      : startMs);
  if (endMs < startMs) return null;
  return { startMs, endMs };
}

export function traceScoresFromUnknown(value: unknown): TraceScore[] {
  if (!Array.isArray(value)) return [];
  return value.filter((candidate): candidate is TraceScore => {
    if (!isRecord(candidate)) return false;
    return typeof candidate.id === "string"
      && typeof candidate.traceId === "string"
      && typeof candidate.name === "string"
      && typeof candidate.dataType === "string";
  });
}

export function traceScoresForSelection(
  scores: TraceScore[],
  selectedObservationId: string | null,
): TraceScore[] {
  return selectedObservationId === null
    ? scores
    : scores.filter((score) => score.spanId === selectedObservationId);
}

export function traceEventSpansForSelection(
  spans: TraceSpan[],
  selectedObservationId: string | null,
): TraceSpan[] {
  const events = spans.filter((span) => observationType(span) === "event");
  if (selectedObservationId === null) return events;
  const spansById = new Map(spans.map((span) => [span.id, span]));
  return events.filter((event) => {
    if (event.id === selectedObservationId) return true;
    const visited = new Set<string>();
    let parentId = event.parentSpanId;
    while (parentId !== null && !visited.has(parentId)) {
      if (parentId === selectedObservationId) return true;
      visited.add(parentId);
      parentId = spansById.get(parentId)?.parentSpanId ?? null;
    }
    return false;
  });
}

export function sameTraceSpanRevision(a: TraceSpan, b: TraceSpan): boolean {
  return a.id === b.id
    && a.traceId === b.traceId
    && a.parentSpanId === b.parentSpanId
    && a.name === b.name
    && a.kind === b.kind
    && a.observationType === b.observationType
    && a.status === b.status
    && a.providerKey === b.providerKey
    && a.capabilityKind === b.capabilityKind
    && a.model === b.model
    && a.durationMs === b.durationMs
    && a.completionStartMs === b.completionStartMs
    && a.error === b.error
    && a.createdAt === b.createdAt
    && a.completedAt === b.completedAt
    && sameJson(a.modelParameters, b.modelParameters)
    && sameJson(a.usage, b.usage)
    && sameJson(a.inputSummary, b.inputSummary)
    && sameJson(a.outputSummary, b.outputSummary)
    && sameJson(a.attributes, b.attributes);
}

export function traceSelectionFromUrl(href: string): TraceUrlSelection {
  const params = new URL(href, "http://vifu.invalid").searchParams;
  return {
    traceId: params.get("traceId"),
    invocationId: params.get("invocationId"),
    observationId: params.get("observationId"),
  };
}

export function traceSelectionUrl(
  href: string,
  traceId: string | null,
  observationId: string | null,
): string {
  const url = new URL(href, "http://vifu.invalid");
  if (traceId) url.searchParams.set("traceId", traceId);
  else url.searchParams.delete("traceId");
  url.searchParams.delete("invocationId");
  if (traceId && observationId) url.searchParams.set("observationId", observationId);
  else url.searchParams.delete("observationId");
  return `${url.pathname}${url.search}${url.hash}`;
}

export function exactTraceLookupPath(
  projectSlug: string,
  field: "requestId" | "traceId",
  value: string,
): string {
  return `apps/${encodeURIComponent(projectSlug)}/traces?${field}=${encodeURIComponent(value)}&limit=1`;
}

export function traceListPath(
  projectSlug: string,
  query: TraceListQuery = {},
): string {
  const params = new URLSearchParams();
  if (query.from) params.set("from", query.from);
  if (query.to) params.set("to", query.to);
  if (query.before) {
    params.set("beforeCreatedAt", query.before.createdAt);
    params.set("beforeTraceId", query.before.traceId);
  }
  if (query.limit !== undefined) params.set("limit", String(query.limit));
  const suffix = params.toString();
  return `apps/${encodeURIComponent(projectSlug)}/traces${suffix ? `?${suffix}` : ""}`;
}

export function retainPinnedTrace<T extends { id: string }>(
  sortedTraces: T[],
  limit: number,
  pinnedTraceId: string | null,
): T[] {
  if (limit <= 0) return [];
  const limited = sortedTraces.slice(0, limit);
  if (!pinnedTraceId || limited.some((trace) => trace.id === pinnedTraceId)) return limited;
  const pinned = sortedTraces.find((trace) => trace.id === pinnedTraceId);
  if (!pinned) return limited;
  return [...limited.slice(0, limit - 1), pinned];
}

export function formatMetric(value: number | null, unit: string, digits = 0): string {
  if (value === null || !Number.isFinite(value)) return "-";
  return `${value.toFixed(digits)}${unit}`;
}

function normalizedStatusLabel(status: string, group: TraceStatusGroup): string {
  const normalized = status.trim().toLowerCase();
  if (normalized) return normalized.replaceAll("_", " ");
  if (group === "passed") return "completed";
  if (group === "running") return "running";
  if (group === "problems") return "failed";
  return "unknown";
}

function finiteNonNegativeNumber(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) && value >= 0 ? value : null;
}

function appScoreLabel(scores: TraceScore[]): string | null {
  const boundaries = new Map<string, string>();
  for (const score of scores) {
    const name = score.name.toLowerCase();
    const boundary = ["output_accepted", "action_applied", "frame_presented"]
      .find((candidate) => name.includes(candidate));
    if (!boundary) continue;
    const value = normalizedScoreValue(score.value);
    if (value) boundaries.set(boundary, value);
  }
  if (boundaries.size === 0) return "unknown";
  const outcomes = [...boundaries.values()];
  if (outcomes.includes("fail")) return "fail";
  if (outcomes.includes("unknown")) return "unknown";
  if (boundaries.size !== 3) return "unknown";
  return outcomes.includes("not applicable") ? "not applicable" : "pass";
}

function normalizedScoreValue(value: unknown): string | null {
  if (typeof value === "boolean") return value ? "pass" : "fail";
  if (typeof value !== "string") return null;
  const normalized = value.trim().toLowerCase();
  if (["pass", "passed", "true", "accepted"].includes(normalized)) return "pass";
  if (["notapplicable", "not_applicable", "not applicable", "n/a", "skipped"].includes(normalized)) return "not applicable";
  if (["fail", "failed", "false", "rejected"].includes(normalized)) return "fail";
  if (normalized === "unknown") return "unknown";
  return null;
}

function traceUsageFromUnknown(value: unknown): TraceUsage | null {
  if (!isRecord(value)) return null;
  return value;
}

function usageOutputTokens(usage: TraceUsage | null): number | null {
  if (!usage) return null;
  return firstFiniteNumber(
    usage.outputTokens,
    usage.completionTokens,
    usage.output_tokens,
    usage.completion_tokens,
  );
}

function compareSpans(a: TraceSpan, b: TraceSpan): number {
  const aOffsets = observationTimelineOffsets(a);
  const bOffsets = observationTimelineOffsets(b);
  if (aOffsets && bOffsets && aOffsets.startMs !== bOffsets.startMs) {
    return aOffsets.startMs - bOffsets.startMs;
  }
  if (aOffsets && !bOffsets) return -1;
  if (!aOffsets && bOffsets) return 1;
  const aTime = Date.parse(a.createdAt);
  const bTime = Date.parse(b.createdAt);
  if (Number.isFinite(aTime) && Number.isFinite(bTime) && aTime !== bTime) return aTime - bTime;
  return a.id.localeCompare(b.id);
}

function wouldCreateCycle(
  nodes: Map<string, TraceObservationNode>,
  childId: string,
  parentId: string | null,
): boolean {
  const seen = new Set<string>([childId]);
  let cursor = parentId;
  while (cursor) {
    if (seen.has(cursor)) return true;
    seen.add(cursor);
    cursor = nodes.get(cursor)?.span.parentSpanId ?? null;
  }
  return false;
}

function optionalTraceValue(trace: EndpointTrace, key: string): unknown {
  return (trace as unknown as Record<string, unknown>)[key];
}

function nestedValue(value: unknown, ...path: string[]): unknown {
  let cursor = value;
  for (const key of path) {
    if (!isRecord(cursor)) return undefined;
    cursor = cursor[key];
  }
  return cursor;
}

function firstString(...values: unknown[]): string | null {
  for (const value of values) {
    if (typeof value === "string" && value.trim()) return value.trim();
  }
  return null;
}

function firstFiniteNumber(...values: unknown[]): number | null {
  for (const value of values) {
    if (typeof value === "number" && Number.isFinite(value)) return value;
  }
  return null;
}

function shortId(value: string, length = 8): string {
  return value.length <= length ? value : `${value.slice(0, length)}...`;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function sameJson(left: unknown, right: unknown): boolean {
  return JSON.stringify(left) === JSON.stringify(right);
}
