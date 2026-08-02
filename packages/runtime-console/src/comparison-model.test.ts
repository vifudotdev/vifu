import { describe, expect, it } from "vitest";
import {
  comparisonCorrelationPresentation,
  comparisonCoverageLabel,
  comparisonDeviceLabel,
  comparisonRoutePresentations,
  comparisonRunPresentation,
  formatComparisonCpu,
  formatComparisonDuration,
  formatComparisonRange,
  formatComparisonRss,
  sortRuntimeComparisons,
} from "./comparison-model";
import type { RuntimeComparison, RuntimeComparisonRun } from "./types";

describe("comparison history presentation", () => {
  it("sorts newest first without mutating the API response", () => {
    const older = comparison({ id: "older", startedAt: "2026-08-03T00:00:00.000Z" });
    const newer = comparison({ id: "newer", startedAt: "2026-08-03T00:01:00.000Z" });
    const input = [older, newer];

    expect(sortRuntimeComparisons(input).map(({ id }) => id)).toEqual(["newer", "older"]);
    expect(input.map(({ id }) => id)).toEqual(["older", "newer"]);
  });

  it("only calls an explicitly passed run runtime/contract verified", () => {
    expect(comparisonRunPresentation(run({ outcome: "passed" }))).toEqual({
      outcomeLabel: "runtime/contract verified",
      verified: true,
    });
    expect(comparisonRunPresentation(run({ outcome: "excluded" }))).toEqual({
      outcomeLabel: "excluded",
      verified: false,
    });
  });

  it("does not verify a passed label with incomplete warm telemetry", () => {
    expect(comparisonRunPresentation(run({ repeatTotal: null }))).toEqual({
      outcomeLabel: "passed · incomplete telemetry",
      verified: false,
    });
    expect(comparisonRunPresentation(run({
      repeatTotal: { median: 100, min: 90, max: 110, samples: 2 },
    }))).toEqual({
      outcomeLabel: "passed · incomplete telemetry",
      verified: false,
    });
  });

  it("shows display route labels while retaining canonical binding IDs", () => {
    const bindingId = "018f6a1c-73fe-7f8a-8c21-2ce847852c20";
    expect(comparisonRoutePresentations(run({
      routes: { [bindingId]: "qwen-2b" },
      routeLabels: { [bindingId]: "npc-planner · chat" },
    }))).toEqual([{
      bindingId,
      label: "npc-planner · chat",
      route: "qwen-2b",
    }]);
  });

  it("identifies an unlabeled canonical route as a binding, not an agent", () => {
    const bindingId = "018f6a1c-73fe-7f8a-8c21-2ce847852c20";
    expect(comparisonRoutePresentations(run({
      routes: { [bindingId]: "qwen-2b" },
      routeLabels: {},
    }))[0]?.label).toBe("Binding 018f6a1c…");
  });

  it("describes measured ranges and OS process memory without inventing values", () => {
    expect(formatComparisonRange(
      { median: 1_250, min: 1_100, max: 1_400, samples: 3 },
      formatComparisonDuration,
    )).toBe("1.25 s median · 1.1 s–1.4 s · n=3");
    expect(formatComparisonRss(512 * 1024 * 1024)).toBe("512 MiB");
    expect(formatComparisonRange(null, formatComparisonDuration)).toBe("-");
    expect(formatComparisonRss(null)).toBe("-");
  });

  it("preserves multicore OS process CPU values above 100 percent", () => {
    expect(formatComparisonCpu(246.25)).toBe("246.3%");
    expect(formatComparisonCpu(null)).toBe("-");
  });

  it("summarizes the tested scope and actual device fields", () => {
    const value = comparison({
      corpusAgents: 2,
      configuredModels: 5,
      testedModels: 4,
      passedModels: 3,
      device: { architecture: "arm64", backend: "Metal", os: "macOS" },
    });
    expect(comparisonCoverageLabel(value)).toBe("4/5 models tested · 3 passed · 2 corpus agents");
    expect(comparisonDeviceLabel(value)).toBe("macOS · arm64 · Metal");
  });

  it("keeps exact wall-clock and monotonic evidence for Arm capture correlation", () => {
    expect(comparisonCorrelationPresentation(comparison({ monotonicDurationMs: 9_750 }))).toEqual({
      comparisonId: "comparison-1",
      wallClockWindow: "2026-08-03T00:00:00.000Z → 2026-08-03T00:00:10.000Z",
      monotonicDuration: "9.75 s",
      measurementNote: "Correlation window only; not an Arm tool metric.",
    });
  });
});

function comparison(overrides: Partial<RuntimeComparison> = {}): RuntimeComparison {
  return {
    id: "comparison-1",
    projectId: "project-1",
    deploymentId: "deployment-1",
    gatewayId: "gateway-1",
    status: "completed",
    recommendation: "combination-1",
    notExhaustive: true,
    sequentialReplay: true,
    corpusAgents: 1,
    configuredModels: 2,
    testedModels: 2,
    passedModels: 2,
    device: { architecture: "arm64" },
    monotonicDurationMs: 10_000,
    startedAt: "2026-08-03T00:00:00.000Z",
    completedAt: "2026-08-03T00:00:10.000Z",
    runs: [],
    ...overrides,
  };
}

function run(overrides: Partial<RuntimeComparisonRun> = {}): RuntimeComparisonRun {
  return {
    id: "run-1",
    comparisonId: "comparison-1",
    combinationId: "combination-1",
    label: "Current routes",
    rule: "current",
    routes: { planner: "qwen-2b" },
    routeLabels: { planner: "Planner · chat" },
    outcome: "passed",
    firstTotalMs: 1_500,
    firstRunCold: true,
    repeatRunsResident: true,
    repeatTotal: { median: 1_000, min: 900, max: 1_100, samples: 3 },
    repeatTtft: null,
    tokensPerSecond: 12,
    firstProcessCpuPercent: 180,
    processCpuPercent: 140,
    peakRssBytes: 512 * 1024 * 1024,
    error: null,
    ...overrides,
  };
}
