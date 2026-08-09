"use client";

import { useEffect, useState } from "react";
import {
  comparisonCorrelationPresentation,
  comparisonCoverageLabel,
  comparisonDeviceLabel,
  comparisonRoutePresentations,
  comparisonRunPresentation,
  formatComparisonCpu,
  formatComparisonDuration,
  formatComparisonRange,
  formatComparisonRate,
  formatComparisonRss,
  sortRuntimeComparisons,
} from "../comparison-model";
import { useRuntimeConsoleHost } from "../host";
import type { RuntimeComparison, RuntimeComparisonRun } from "../types";

const COMPARISON_LIMIT = 20;
const COMPARISON_POLL_MS = 10_000;
const COMPARISON_REQUEST_TIMEOUT_MS = 8_000;

type RuntimeComparisonHistoryProps = {
  projectId: string;
  projectSlug: string;
};

type RuntimeComparisonsResponse = {
  comparisons?: RuntimeComparison[];
  error?: { message?: string };
};

export function RuntimeComparisonHistory({ projectId, projectSlug }: RuntimeComparisonHistoryProps) {
  const { request } = useRuntimeConsoleHost();
  const [comparisons, setComparisons] = useState<RuntimeComparison[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const controller = new AbortController();
    let timer: number | undefined;
    setComparisons([]);
    setLoading(true);
    setError(null);

    async function loadComparisons() {
      const requestController = new AbortController();
      const abortRequest = () => requestController.abort();
      let timedOut = false;
      const timeout = window.setTimeout(() => {
        timedOut = true;
        requestController.abort();
      }, COMPARISON_REQUEST_TIMEOUT_MS);
      controller.signal.addEventListener("abort", abortRequest, { once: true });
      try {
        const payload = await request<RuntimeComparisonsResponse>(
          `project/${encodeURIComponent(projectSlug)}/comparisons?limit=${COMPARISON_LIMIT}`,
          "GET",
          undefined,
          requestController.signal,
        );
        const records = Array.isArray(payload.comparisons) ? payload.comparisons : [];
        const next = sortRuntimeComparisons(
          records.filter((comparison) => comparison.projectId === projectId),
        ).slice(0, COMPARISON_LIMIT);
        setComparisons(next);
        setError(null);
      } catch (requestError) {
        if (!controller.signal.aborted) {
          setError(timedOut
            ? "Comparison history request timed out."
            : requestError instanceof Error ? requestError.message : "Failed to load comparison history.");
        }
      } finally {
        window.clearTimeout(timeout);
        controller.signal.removeEventListener("abort", abortRequest);
        if (!controller.signal.aborted) {
          setLoading(false);
          timer = window.setTimeout(loadComparisons, COMPARISON_POLL_MS);
        }
      }
    }

    void loadComparisons();
    return () => {
      controller.abort();
      if (timer !== undefined) window.clearTimeout(timer);
    };
  }, [projectId, projectSlug, request]);

  return (
    <section className="comparison-history" aria-labelledby="comparison-history-heading">
      <header className="comparison-history-header">
        <div>
          <strong id="comparison-history-heading">Comparison History</strong>
          <span>{comparisons.length} recent</span>
        </div>
        <small>Recorded device measurements</small>
      </header>

      {loading && comparisons.length === 0 ? (
        <p className="comparison-history-message">Loading comparison history...</p>
      ) : null}
      {error ? <p className="comparison-history-message error" role="alert">Comparison history: {error}</p> : null}
      {!loading && !error && comparisons.length === 0 ? (
        <p className="comparison-history-message">Run an optimization comparison in Vifu to see measured combinations here.</p>
      ) : null}

      {comparisons.length > 0 ? (
        <div className="comparison-history-list">
          {comparisons.map((comparison, index) => (
            <ComparisonRow comparison={comparison} defaultOpen={index === 0} key={comparison.id} />
          ))}
        </div>
      ) : null}
    </section>
  );
}

function ComparisonRow({ comparison, defaultOpen }: { comparison: RuntimeComparison; defaultOpen: boolean }) {
  const [open, setOpen] = useState(defaultOpen);
  const correlation = comparisonCorrelationPresentation(comparison);
  return (
    <details
      className="comparison-history-row"
      open={open}
      onToggle={(event) => setOpen(event.currentTarget.open)}
    >
      <summary>
        <span className={`comparison-status ${comparisonStatusTone(comparison.status)}`}>{formatOutcome(comparison.status)}</span>
        <time dateTime={comparison.startedAt} title={formatFullDate(comparison.startedAt)}>{formatCompactDate(comparison.startedAt)}</time>
        <strong>{comparison.recommendation ? `Recommended: ${comparison.recommendation}` : "No recommendation"}</strong>
        <span>{comparisonCoverageLabel(comparison)}</span>
        <span>{comparisonDeviceLabel(comparison)}</span>
        <span className="comparison-history-badges">
          {comparison.sequentialReplay ? <small>sequential replay</small> : null}
          {comparison.notExhaustive ? <small>not exhaustive</small> : null}
        </span>
      </summary>
      <div className="comparison-history-runs">
        <div className="comparison-run-details comparison-correlation" aria-label="Arm capture correlation">
          <dl>
            <div><dt>Arm capture correlation</dt><dd><code>{correlation.comparisonId}</code></dd></div>
            <div><dt>Wall-clock window</dt><dd>{correlation.wallClockWindow}</dd></div>
            <div><dt>Monotonic duration</dt><dd>{correlation.monotonicDuration}</dd></div>
          </dl>
          <p className="comparison-run-measurement-note">{correlation.measurementNote}</p>
        </div>
        {comparison.runs.length > 0 ? comparison.runs.map((run) => (
          <ComparisonRunRow
            key={run.id}
            run={run}
            recommended={comparison.recommendation === run.combinationId}
          />
        )) : <p className="comparison-history-message">No measured runs were recorded.</p>}
      </div>
    </details>
  );
}

function ComparisonRunRow({ run, recommended }: { run: RuntimeComparisonRun; recommended: boolean }) {
  const presentation = comparisonRunPresentation(run);
  const routes = comparisonRoutePresentations(run);
  return (
    <details className="comparison-run-row">
      <summary>
        <span className={presentation.verified ? "comparison-status passed" : "comparison-status neutral"}>
          {presentation.outcomeLabel}
        </span>
        <strong>{run.label}</strong>
        <small className="comparison-recommended">{recommended ? "recommended" : ""}</small>
        <span><small>First total</small>{formatComparisonDuration(run.firstTotalMs)}</span>
        <span><small>Repeat total</small>{run.repeatTotal ? formatComparisonDuration(run.repeatTotal.median) : "-"}</span>
        <span><small>TTFT</small>{run.repeatTtft ? formatComparisonDuration(run.repeatTtft.median) : "-"}</span>
        <span><small>Throughput</small>{formatComparisonRate(run.tokensPerSecond)}</span>
        <span title="Vifu OS process CPU; multicore usage may exceed 100%"><small>Vifu process CPU (warm median)</small>{formatComparisonCpu(run.processCpuPercent)}</span>
        <span><small>OS process peak RSS</small>{formatComparisonRss(run.peakRssBytes)}</span>
      </summary>
      <div className="comparison-run-details">
        <dl>
          <div><dt>Rule</dt><dd>{run.rule || "-"}</dd></div>
          <div><dt>First run state</dt><dd>{coldRunLabel(run.firstRunCold)}</dd></div>
          <div><dt>Repeat run state</dt><dd>{residentRunLabel(run.repeatRunsResident)}</dd></div>
          <div><dt>Repeat total</dt><dd>{formatComparisonRange(run.repeatTotal, formatComparisonDuration)}</dd></div>
          <div><dt>Repeat TTFT</dt><dd>{formatComparisonRange(run.repeatTtft, formatComparisonDuration)}</dd></div>
          <div><dt>Vifu process CPU (first)</dt><dd>{formatComparisonCpu(run.firstProcessCpuPercent)}</dd></div>
          <div><dt>Vifu process CPU (warm median)</dt><dd>{formatComparisonCpu(run.processCpuPercent)}</dd></div>
          <div><dt>OS process peak RSS</dt><dd>{formatComparisonRss(run.peakRssBytes)}</dd></div>
        </dl>
        <div className="comparison-run-routes">
          <span>Routes</span>
          {routes.length > 0 ? routes.map((route) => (
            <code key={route.bindingId} title={`Binding ${route.bindingId}`}>{route.label} → {route.route}</code>
          )) : <code>-</code>}
        </div>
        {run.error ? <p className="comparison-run-error">{run.error}</p> : null}
        <p className="comparison-run-measurement-note">CPU and RSS are Vifu OS process measurements. Multicore CPU can exceed 100%.</p>
      </div>
    </details>
  );
}

function comparisonStatusTone(status: string): string {
  const normalized = status.trim().toLowerCase();
  if (["completed", "passed", "success", "succeeded"].includes(normalized)) return "passed";
  if (["created", "pending", "queued", "running", "started"].includes(normalized)) return "running";
  if (["error", "failed", "cancelled", "timed_out", "timeout"].includes(normalized)) return "problems";
  return "neutral";
}

function coldRunLabel(value: boolean | null): string {
  if (value === true) return "cold start measured";
  if (value === false) return "not a cold start";
  return "cold state unknown";
}

function residentRunLabel(value: boolean | null): string {
  if (value === true) return "resident repeats measured";
  if (value === false) return "residency not confirmed";
  return "residency unknown";
}

function formatOutcome(value: string): string {
  const normalized = value.trim().toLowerCase();
  return normalized ? normalized.replaceAll("_", " ") : "unknown";
}

function formatCompactDate(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.valueOf())) return "Unknown time";
  return new Intl.DateTimeFormat("en", {
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    month: "short",
  }).format(date);
}

function formatFullDate(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.valueOf())) return "Unknown time";
  return new Intl.DateTimeFormat("en", {
    dateStyle: "medium",
    timeStyle: "long",
  }).format(date);
}
