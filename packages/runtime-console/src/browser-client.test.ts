import { afterEach, describe, expect, it, vi } from "vitest";

import {
  runtimeBrowserRequest,
  runtimeBrowserUpload,
  runtimeRequestUsesServerDeadline,
} from "./browser-client";

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

  it("leaves inference request lifetime to the runtime", async () => {
    vi.useFakeTimers();
    const fetcher = abortablePendingFetch();
    vi.stubGlobal("fetch", fetcher);
    const controller = new AbortController();

    const request = runtimeBrowserRequest(
      "chat/completions",
      "POST",
      { messages: [] },
      controller.signal,
    ).catch((error: unknown) => error);
    await vi.advanceTimersByTimeAsync(8_000);
    expect((fetcher.mock.calls[0]?.[1]?.signal as AbortSignal).aborted).toBe(false);
    controller.abort();
    await expect(request).resolves.toMatchObject({ message: "request aborted" });
  });

  it("keeps ordinary mutation requests bounded", async () => {
    vi.useFakeTimers();
    vi.stubGlobal("fetch", abortablePendingFetch());

    const request = runtimeBrowserRequest("projects", "POST", { name: "demo" });
    const rejection = expect(request).rejects.toMatchObject({
      status: 408,
      message: "Runtime request timed out.",
    });
    await vi.advanceTimersByTimeAsync(8_000);
    await rejection;
  });

  it("bounds uploads independently from inference", async () => {
    vi.useFakeTimers();
    vi.stubGlobal("fetch", abortablePendingFetch());

    const request = runtimeBrowserUpload(
      "project/demo/extensions/runtime",
      new FormData(),
    );
    const rejection = expect(request).rejects.toMatchObject({
      status: 408,
      message: "Runtime request timed out.",
    });
    await vi.advanceTimersByTimeAsync(60_000);
    await rejection;
  });

  it("delegates only invocation paths to server deadlines", () => {
    expect(runtimeRequestUsesServerDeadline("chat/completions", "POST")).toBe(true);
    expect(runtimeRequestUsesServerDeadline("demo/v1/embeddings", "POST")).toBe(true);
    expect(runtimeRequestUsesServerDeadline("projects", "POST")).toBe(false);
    expect(runtimeRequestUsesServerDeadline("chat/completions", "PATCH")).toBe(false);
  });
});

function abortablePendingFetch() {
  return vi.fn((_input: RequestInfo | URL, init?: RequestInit) => new Promise<Response>((_resolve, reject) => {
    const abort = () => reject(new Error("request aborted"));
    if (init?.signal?.aborted) abort();
    else init?.signal?.addEventListener("abort", abort, { once: true });
  }));
}
