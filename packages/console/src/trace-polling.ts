import { traceStatusGroup } from "./trace-model";
import type { EndpointTrace } from "./types";

// A selected active trace reads the list, spans, and scores on each cycle.
// Keep live refreshes gentle enough for rate-limited authenticated gateways.
export const TRACE_POLL_MS = 20_000;
const TRACE_RATE_LIMIT_RETRY_MS = 60_000;

export function isTraceRateLimited(error: unknown): error is {
  status: 429;
  retryAfterMilliseconds?: number;
} {
  return error !== null
    && typeof error === "object"
    && "status" in error
    && error.status === 429;
}

export function tracePollDelay(error: unknown): number {
  if (!isTraceRateLimited(error)) return TRACE_POLL_MS;
  const retryAfter = error.retryAfterMilliseconds;
  return Math.max(
    TRACE_RATE_LIMIT_RETRY_MS,
    typeof retryAfter === "number" && Number.isFinite(retryAfter) ? retryAfter : 0,
  );
}

export function shouldPollTraceObservations(
  trace: Pick<EndpointTrace, "status" | "completedAt"> | null,
): boolean {
  if (!trace) return true;
  if (trace.completedAt) return false;
  const status = traceStatusGroup(trace.status);
  return status === "running" || status === "unknown";
}
