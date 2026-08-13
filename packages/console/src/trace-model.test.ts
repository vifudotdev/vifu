import { describe, expect, it } from "vitest";
import {
  buildTraceObservationForest,
  exactTraceLookupPath,
  flattenTraceObservationForest,
  observationMetadata,
  observationModel,
  observationTimelineOffsets,
  observationTtftMs,
  observationType,
  retainPinnedTrace,
  sameTraceSpanRevision,
  traceIdForInvocation,
  traceEventSpansForSelection,
  traceIoValues,
  traceListPresentation,
  traceListPath,
  traceScoresForSelection,
  traceSelectionFromUrl,
  traceSelectionUrl,
  traceStatusGroup,
} from "./trace-model";
import type { EndpointTrace, TraceScore, TraceSpan } from "./types";

describe("traceListPresentation", () => {
  it("keeps running, successful, and failed traces in distinct status groups", () => {
    expect(traceStatusGroup("running")).toBe("running");
    expect(traceStatusGroup("completed")).toBe("passed");
    expect(traceStatusGroup("timeout")).toBe("problems");
  });

  it("reads optional model performance and app score fields without requiring them", () => {
    const trace = endpointTrace({
      latencyMs: 2_000,
      request: { model: "qwen2.5:2b" },
      response: {
        metadata: { decodeMs: 1_000, ttftMs: 240 },
        scores: [
          score("app.output_accepted", "pass"),
          score("app.action_applied", "pass"),
          score("app.frame_presented", "pass"),
        ],
        usage: { completion_tokens: 20 },
      },
    });

    expect(traceListPresentation(trace)).toMatchObject({
      agent: "Planner",
      appScore: "pass",
      model: "qwen2.5:2b",
      status: "passed",
      tokensPerSecond: 20,
      ttftMs: 240,
    });
  });

  it("uses the reported Gateway name for trace attribution", () => {
    const trace = endpointTrace({
      gatewayId: "gateway-kitchen-light",
      gatewayName: "Kitchen light",
      gatewayMetadata: { kind: "light", room: "kitchen" },
    });

    expect(traceListPresentation(trace)).toMatchObject({
      gateway: "Kitchen light",
      gatewayId: "gateway-kitchen-light",
    });
  });

  it("leaves unavailable metrics unknown", () => {
    expect(traceListPresentation(endpointTrace()).model).toBeNull();
    expect(traceListPresentation(endpointTrace()).ttftMs).toBeNull();
    expect(traceListPresentation(endpointTrace()).appScore).toBe("unknown");
  });

  it("does not let an early application pass hide a later failure", () => {
    const scores = [
      score("app.output_accepted", "pass"),
      score("app.action_applied", "fail"),
      score("app.frame_presented", "unknown"),
    ];
    const trace = endpointTrace({ response: { scores } });

    expect(traceListPresentation(trace).appScore).toBe("fail");
    expect(traceStatusGroup("unknown")).toBe("unknown");
  });

  it("uses persisted runtime aggregates and promotes application failure to Problems", () => {
    const trace = endpointTrace({
      status: "completed",
      model: "qwen2.5-2b",
      completionStartMs: 125,
      usage: { inputTokens: 12, outputTokens: 8 },
      decodeMs: 200,
      appOutcome: "fail",
      response: {},
    });

    expect(traceListPresentation(trace)).toMatchObject({
      appScore: "fail",
      model: "qwen2.5-2b",
      status: "problems",
      statusLabel: "application failed",
      tokensPerSecond: 40,
      ttftMs: 125,
    });
    expect(trace.status).toBe("completed");
  });

  it("keeps not-applicable application feedback distinct from pass", () => {
    expect(traceListPresentation(endpointTrace({ appOutcome: "notApplicable" })).appScore)
      .toBe("not applicable");
    expect(traceListPresentation(endpointTrace({ appOutcome: "unknown" })).appScore)
      .toBe("unknown");
  });
});

describe("trace list paths", () => {
  it("encodes a stable cursor and UTC date window", () => {
    const path = traceListPath("stardew valley", {
      from: "2026-08-03T00:00:00.000Z",
      to: "2026-08-04T00:00:00.000Z",
      before: {
        createdAt: "2026-08-03T12:34:56.789Z",
        traceId: "trace/older",
      },
      limit: 100,
    });
    const url = new URL(path, "http://vifu.invalid/");

    expect(url.pathname).toBe("/apps/stardew%20valley/traces");
    expect(Object.fromEntries(url.searchParams)).toEqual({
      beforeCreatedAt: "2026-08-03T12:34:56.789Z",
      beforeTraceId: "trace/older",
      from: "2026-08-03T00:00:00.000Z",
      limit: "100",
      to: "2026-08-04T00:00:00.000Z",
    });
  });
});

describe("trace observation tree", () => {
  it("orders parents before children and preserves orphan observations", () => {
    const spans = [
      traceSpan({ id: "child", parentSpanId: "root", createdAt: "2026-08-03T00:00:01.000Z" }),
      traceSpan({ id: "orphan", parentSpanId: "missing", createdAt: "2026-08-03T00:00:02.000Z" }),
      traceSpan({ id: "root", parentSpanId: null, createdAt: "2026-08-03T00:00:00.000Z" }),
    ];

    const flattened = flattenTraceObservationForest(buildTraceObservationForest(spans));
    expect(flattened.map(({ depth, span }) => [span.id, depth])).toEqual([
      ["root", 0],
      ["child", 1],
      ["orphan", 0],
    ]);
  });

  it("turns cyclic parent references into safe roots", () => {
    const spans = [
      traceSpan({ id: "a", parentSpanId: "b" }),
      traceSpan({ id: "b", parentSpanId: "a" }),
    ];

    const flattened = flattenTraceObservationForest(buildTraceObservationForest(spans));
    expect(flattened.map(({ span }) => span.id).sort()).toEqual(["a", "b"]);
  });

  it("orders observations by request offsets before persistence timestamps", () => {
    const spans = [
      traceSpan({
        id: "later",
        createdAt: "2026-08-03T00:00:01.000Z",
        attributes: { startOffsetMs: 80, endOffsetMs: 90 },
      }),
      traceSpan({
        id: "earlier",
        createdAt: "2026-08-03T00:00:02.000Z",
        attributes: { startOffsetMs: 10, endOffsetMs: 20 },
      }),
    ];

    expect(flattenTraceObservationForest(buildTraceObservationForest(spans))
      .map(({ span }) => span.id)).toEqual(["earlier", "later"]);
  });

  it("reads Langfuse-style generation fields while retaining legacy kind metadata", () => {
    const span = traceSpan({
      observationType: "generation",
      kind: "provider.invoke",
      model: "llama-3.2-3b",
      completionStartMs: 180,
    });
    expect(observationType(span)).toBe("generation");
    expect(observationModel(span)).toBe("llama-3.2-3b");
    expect(observationTtftMs(span)).toBe(180);
  });

  it("projects canonical observation metadata with correlation IDs and model identity", () => {
    const span = traceSpan({
      id: "observation-1",
      traceId: "trace-1",
      parentSpanId: "root-observation",
      name: "Decode",
      observationType: "generation",
      providerKey: "local-llama",
      capabilityKind: "chat",
      model: "qwen2.5:2b",
      modelParameters: { temperature: 0.2 },
      attributes: {
        startOffsetMs: 12,
        events: [{ level: "debug", message: "attached event" }],
        _vifuTraceIo: { outputCanonical: true },
      },
    });

    expect(observationMetadata(span)).toEqual({
      traceId: "trace-1",
      observationId: "observation-1",
      parentObservationId: "root-observation",
      observationType: "generation",
      name: "Decode",
      providerKey: "local-llama",
      model: "qwen2.5:2b",
      capabilityKind: "chat",
      modelParameters: { temperature: 0.2 },
      attributes: { startOffsetMs: 12 },
    });
  });

  it("uses canonical root Generation I/O with legacy trace fallback", () => {
    const trace = endpointTrace({ request: { legacy: "input" }, response: { legacy: "output" } });
    const canonicalRoot = traceSpan({
      id: "root",
      parentSpanId: null,
      observationType: "generation",
      inputSummary: { messages: 1 },
      outputSummary: null,
      attributes: {
        _vifuTraceIo: { inputCanonical: true, outputCanonical: true },
      },
    });

    expect(traceIoValues(trace, [canonicalRoot], null)).toEqual({
      input: { messages: 1 },
      output: null,
    });
    expect(traceIoValues(trace, [], null)).toEqual({
      input: { legacy: "input" },
      output: { legacy: "output" },
    });
    const legacyRoot = traceSpan({
      parentSpanId: null,
      observationType: "generation",
      inputSummary: { messageCount: 1 },
      outputSummary: { choiceCount: 1 },
    });
    expect(traceIoValues(trace, [legacyRoot], legacyRoot)).toEqual({
      input: { legacy: "input" },
      output: { legacy: "output" },
    });
  });

  it("uses persisted request-relative offsets even when storage timestamps are late", () => {
    const span = traceSpan({
      createdAt: "2026-08-03T00:00:10.000Z",
      durationMs: 80,
      attributes: { startOffsetMs: 25, endOffsetMs: 105 },
    });

    expect(observationTimelineOffsets(span)).toEqual({ startMs: 25, endMs: 105 });
    expect(observationTimelineOffsets(traceSpan({ attributes: { startOffsetMs: 40 }, durationMs: 10 })))
      .toEqual({ startMs: 40, endMs: 50 });
    expect(observationTimelineOffsets(traceSpan({ attributes: { startOffsetMs: -1 } }))).toBeNull();
  });

  it("reconciles canonical I/O and timing-marker-only updates", () => {
    const before = traceSpan({
      inputSummary: { legacy: true },
      attributes: { startOffsetMs: 10 },
    });
    const canonical = traceSpan({
      inputSummary: { messages: 1 },
      attributes: {
        startOffsetMs: 10,
        endOffsetMs: 90,
        _vifuTraceIo: { inputCanonical: true },
      },
    });

    expect(sameTraceSpanRevision(before, canonical)).toBe(false);
    expect(sameTraceSpanRevision(canonical, { ...canonical })).toBe(true);
    expect(sameTraceSpanRevision(canonical, {
      ...canonical,
      modelParameters: { temperature: 0.2 },
    })).toBe(false);
  });
});

describe("invocation deep links", () => {
  it("resolves the canonical invocation id to its stored trace id", () => {
    const trace = endpointTrace({ id: "trace-1", requestId: "invocation-1" });
    expect(traceIdForInvocation([trace], "invocation-1")).toBe("trace-1");
    expect(traceIdForInvocation([trace], "missing")).toBeNull();
  });

  it("round-trips trace and observation selection without retaining invocation lookup state", () => {
    expect(traceSelectionFromUrl(
      "https://vifu.test/project/demo/logs?traceId=trace-1&observationId=span-1",
    )).toEqual({
      traceId: "trace-1",
      invocationId: null,
      observationId: "span-1",
    });
    expect(traceSelectionUrl(
      "https://vifu.test/project/demo/logs?invocationId=request-1#detail",
      "trace-1",
      "span-1",
    )).toBe("/project/demo/logs?traceId=trace-1&observationId=span-1#detail");
  });

  it("builds bounded exact lookup paths for invocation and stored trace ids", () => {
    expect(exactTraceLookupPath("demo/project", "requestId", "request/id"))
      .toBe("apps/demo%2Fproject/traces?requestId=request%2Fid&limit=1");
    expect(exactTraceLookupPath("demo/project", "traceId", "trace/id"))
      .toBe("apps/demo%2Fproject/traces?traceId=trace%2Fid&limit=1");
  });
});

describe("exact trace retention", () => {
  it("pins an exact deep-link result outside the newest trace window", () => {
    const traces = Array.from({ length: 101 }, (_, index) => ({ id: `trace-${index}` }));

    const retained = retainPinnedTrace(traces, 100, "trace-100");

    expect(retained).toHaveLength(100);
    expect(retained.at(-1)?.id).toBe("trace-100");
    expect(retained.some((trace) => trace.id === "trace-99")).toBe(false);
  });
});

describe("score selection", () => {
  it("shows all trace scores at the root and only exact observation scores in detail", () => {
    const rootScore = score("trace.quality", "pass");
    const selectedScore = { ...score("app.action_applied", "fail"), spanId: "span-selected" };
    const otherScore = { ...score("app.frame_presented", "pass"), spanId: "span-other" };
    const scores = [rootScore, selectedScore, otherScore];

    expect(traceScoresForSelection(scores, null)).toEqual(scores);
    expect(traceScoresForSelection(scores, "span-selected")).toEqual([selectedScore]);
  });
});

describe("event selection", () => {
  it("keeps events inside the selected observation subtree", () => {
    const spans = [
      traceSpan({ id: "root", parentSpanId: null, observationType: "generation" }),
      traceSpan({ id: "selected", parentSpanId: "root", observationType: "span" }),
      traceSpan({ id: "nested", parentSpanId: "selected", observationType: "span" }),
      traceSpan({ id: "inside", parentSpanId: "nested", observationType: "event" }),
      traceSpan({ id: "outside", parentSpanId: "root", observationType: "event" }),
    ];

    expect(traceEventSpansForSelection(spans, "selected").map((span) => span.id)).toEqual(["inside"]);
    expect(traceEventSpansForSelection(spans, null).map((span) => span.id)).toEqual(["inside", "outside"]);
  });

  it("stops safely when parent references cycle", () => {
    const spans = [
      traceSpan({ id: "a", parentSpanId: "b", observationType: "span" }),
      traceSpan({ id: "b", parentSpanId: "a", observationType: "span" }),
      traceSpan({ id: "event", parentSpanId: "a", observationType: "event" }),
    ];

    expect(traceEventSpansForSelection(spans, "missing")).toEqual([]);
  });
});

function endpointTrace(overrides: Partial<EndpointTrace> = {}): EndpointTrace {
  return {
    id: "trace-1",
    requestId: "request-1",
    endpointId: null,
    projectId: "project-1",
    gatewaySessionId: null,
    profileId: "profile-1",
    profileVersionId: "version-1",
    profileSlug: "planner",
    profileName: "Planner",
    profileVersionNumber: 1,
    operation: "chat.completions",
    providerKey: "local-llama",
    capabilityKind: "chat",
    selectionKey: null,
    status: "completed",
    latencyMs: 100,
    request: {},
    response: {},
    error: null,
    createdAt: "2026-08-03T00:00:00.000Z",
    completedAt: "2026-08-03T00:00:00.100Z",
    ...overrides,
  };
}

function traceSpan(overrides: Partial<TraceSpan> = {}): TraceSpan {
  return {
    id: "span-1",
    traceId: "trace-1",
    parentSpanId: null,
    name: "provider.invoke",
    kind: "span",
    status: "completed",
    providerKey: "local-llama",
    capabilityKind: "chat",
    durationMs: 100,
    inputSummary: null,
    outputSummary: null,
    attributes: {},
    error: null,
    createdAt: "2026-08-03T00:00:00.000Z",
    completedAt: "2026-08-03T00:00:00.100Z",
    ...overrides,
  };
}

function score(name: string, value: string): TraceScore {
  return {
    id: `score-${name}`,
    traceId: "trace-1",
    spanId: null,
    name,
    dataType: "categorical",
    value,
    source: "app",
    createdAt: "2026-08-03T00:00:00.000Z",
  };
}
