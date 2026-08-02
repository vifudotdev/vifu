import { afterEach, describe, expect, it, vi } from "vitest";

import { runtimeBrowserRequest } from "./browser-client";

describe("runtimeBrowserRequest", () => {
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("bounds a browser request that never responds", async () => {
    vi.useFakeTimers();
    vi.stubGlobal("fetch", abortablePendingFetch());

    const request = runtimeBrowserRequest("status");
    const rejection = expect(request).rejects.toMatchObject({
      status: 408,
      message: "Runtime request timed out.",
    });
    await vi.advanceTimersByTimeAsync(8_000);
    await rejection;
  });

  it("preserves caller cancellation instead of reporting a timeout", async () => {
    vi.useFakeTimers();
    vi.stubGlobal("fetch", abortablePendingFetch());
    const controller = new AbortController();

    const request = runtimeBrowserRequest("status", "GET", undefined, controller.signal);
    const rejection = expect(request).rejects.toThrow("request aborted");
    controller.abort();
    await rejection;
  });
});

function abortablePendingFetch() {
  return vi.fn((_input: RequestInfo | URL, init?: RequestInit) => new Promise<Response>((_resolve, reject) => {
    const abort = () => reject(new Error("request aborted"));
    if (init?.signal?.aborted) abort();
    else init?.signal?.addEventListener("abort", abort, { once: true });
  }));
}
