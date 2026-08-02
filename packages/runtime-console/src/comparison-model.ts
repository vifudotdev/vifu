import type {
  RuntimeComparison,
  RuntimeComparisonMetricRange,
  RuntimeComparisonRun,
} from "./types";

export type ComparisonRunPresentation = {
  outcomeLabel: string;
  verified: boolean;
};

export type ComparisonRoutePresentation = {
  bindingId: string;
  label: string;
  route: string;
};

export type ComparisonCorrelationPresentation = {
  comparisonId: string;
  wallClockWindow: string;
  monotonicDuration: string;
  measurementNote: string;
};

export function sortRuntimeComparisons(comparisons: RuntimeComparison[]): RuntimeComparison[] {
  return [...comparisons].sort((a, b) => {
    const byStartedAt = sortableTime(b.startedAt) - sortableTime(a.startedAt);
    return byStartedAt || b.id.localeCompare(a.id);
  });
}

export function comparisonRunPresentation(run: RuntimeComparisonRun): ComparisonRunPresentation {
  const normalized = run.outcome.trim().toLowerCase();
  if (normalized === "passed") {
    if (hasCompleteRuntimeEvidence(run)) {
      return { outcomeLabel: "runtime/contract verified", verified: true };
    }
    return { outcomeLabel: "passed · incomplete telemetry", verified: false };
  }
  return {
    outcomeLabel: normalized ? normalized.replaceAll("_", " ") : "unknown",
    verified: false,
  };
}

function hasCompleteRuntimeEvidence(run: RuntimeComparisonRun): boolean {
  const range = run.repeatTotal;
  return isFiniteNonNegative(run.firstTotalMs)
    && range !== null
    && isFiniteNonNegative(range.median)
    && isFiniteNonNegative(range.min)
    && isFiniteNonNegative(range.max)
    && range.min <= range.median
    && range.median <= range.max
    && range.samples === 3;
}

export function comparisonRoutePresentations(run: RuntimeComparisonRun): ComparisonRoutePresentation[] {
  return Object.entries(run.routes)
    .map(([bindingId, route]) => ({
      bindingId,
      label: run.routeLabels?.[bindingId]?.trim() || `Binding ${shortId(bindingId, 8)}`,
      route,
    }))
    .sort((a, b) => a.label.localeCompare(b.label) || a.bindingId.localeCompare(b.bindingId));
}

export function comparisonDeviceLabel(comparison: RuntimeComparison): string {
  return uniqueNonEmpty([
    comparison.device.os,
    comparison.device.architecture,
    comparison.device.backend,
  ]).join(" · ") || "Device details unavailable";
}

export function comparisonCoverageLabel(comparison: RuntimeComparison): string {
  const agentLabel = comparison.corpusAgents === 1 ? "agent" : "agents";
  return `${comparison.testedModels}/${comparison.configuredModels} models tested · ${comparison.passedModels} passed · ${comparison.corpusAgents} corpus ${agentLabel}`;
}

export function comparisonCorrelationPresentation(
  comparison: RuntimeComparison,
): ComparisonCorrelationPresentation {
  return {
    comparisonId: comparison.id,
    wallClockWindow: `${comparison.startedAt} → ${comparison.completedAt ?? "running"}`,
    monotonicDuration: formatComparisonDuration(comparison.monotonicDurationMs),
    measurementNote: "Correlation window only; not an Arm tool metric.",
  };
}

export function formatComparisonDuration(value: number | null): string {
  if (!isFiniteNonNegative(value)) return "-";
  if (value < 1_000) return `${formatNumber(value, value < 10 ? 1 : 0)} ms`;
  return `${formatNumber(value / 1_000, value < 10_000 ? 2 : 1)} s`;
}

export function formatComparisonRate(value: number | null): string {
  if (!isFiniteNonNegative(value)) return "-";
  return `${formatNumber(value, 1)} tok/s`;
}

export function formatComparisonCpu(value: number | null): string {
  if (!isFiniteNonNegative(value)) return "-";
  return `${formatNumber(value, 1)}%`;
}

export function formatComparisonRss(value: number | null): string {
  if (!isFiniteNonNegative(value)) return "-";
  const mebibytes = value / (1024 * 1024);
  if (mebibytes < 1024) return `${formatNumber(mebibytes, 1)} MiB`;
  return `${formatNumber(mebibytes / 1024, 2)} GiB`;
}

export function formatComparisonRange(
  range: RuntimeComparisonMetricRange | null,
  formatter: (value: number | null) => string,
): string {
  if (!range || !isFiniteNonNegative(range.median) || !isFiniteNonNegative(range.min) || !isFiniteNonNegative(range.max)) {
    return "-";
  }
  const samples = Number.isInteger(range.samples) && range.samples >= 0 ? range.samples : 0;
  return `${formatter(range.median)} median · ${formatter(range.min)}–${formatter(range.max)} · n=${samples}`;
}

function sortableTime(value: string): number {
  const parsed = Date.parse(value);
  return Number.isFinite(parsed) ? parsed : 0;
}

function uniqueNonEmpty(values: Array<string | undefined>): string[] {
  const seen = new Set<string>();
  const result: string[] = [];
  for (const value of values) {
    const normalized = value?.trim();
    if (!normalized || seen.has(normalized)) continue;
    seen.add(normalized);
    result.push(normalized);
  }
  return result;
}

function isFiniteNonNegative(value: number | null): value is number {
  return value !== null && Number.isFinite(value) && value >= 0;
}

function formatNumber(value: number, digits: number): string {
  return value.toLocaleString("en", {
    maximumFractionDigits: digits,
    minimumFractionDigits: 0,
    useGrouping: false,
  });
}

function shortId(value: string, length: number): string {
  return value.length <= length ? value : `${value.slice(0, length)}…`;
}
