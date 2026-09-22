import { describe, expect, it } from "vitest";
import {
  isTraceRateLimited,
  shouldPollTraceObservations,
  TRACE_POLL_MS,
  tracePollDelay,
} from "./trace-polling";

describe("trace polling", () => {
  it("keeps three selected traces below one session's thirty-request budget", () => {
    const tabs = 3;
    const requestsPerTraceCycle = 3;
    expect(tabs * requestsPerTraceCycle * (60_000 / TRACE_POLL_MS)).toBeLessThan(30);
  });

  it("stops refreshing observations for completed or failed traces", () => {
    expect(shouldPollTraceObservations({ status: "completed", completedAt: "2026-01-01T00:00:00Z" })).toBe(false);
    expect(shouldPollTraceObservations({ status: "failed", completedAt: null })).toBe(false);
    expect(shouldPollTraceObservations({ status: "running", completedAt: null })).toBe(true);
    expect(shouldPollTraceObservations({ status: "future-status", completedAt: null })).toBe(true);
  });

  it("waits for the server on rate limits rather than repeating the request", () => {
    expect(isTraceRateLimited({ status: 429 })).toBe(true);
    expect(tracePollDelay({ status: 429 })).toBe(60_000);
    expect(tracePollDelay({ status: 429, retryAfterMilliseconds: 75_000 })).toBe(75_000);
    expect(tracePollDelay({ status: 429, retryAfterMilliseconds: 1_000 })).toBe(60_000);
    expect(tracePollDelay(new Error("temporary network failure"))).toBe(TRACE_POLL_MS);
  });
});
