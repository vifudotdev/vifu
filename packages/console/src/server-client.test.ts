import { afterEach, describe, expect, it, vi } from "vitest";

import { DeploymentClient } from "./server-client";

describe("DeploymentClient request timeout", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("bounds a deployment request that never responds", async () => {
    vi.useFakeTimers();
    const client = new DeploymentClient({
      apiBaseUrl: "http://runtime.example",
      fetcher: abortablePendingFetch(),
    });

    const request = client.status();
    const rejection = expect(request).rejects.toMatchObject({
      status: 504,
      message: "Vifu API request timed out.",
    });
    await vi.advanceTimersByTimeAsync(8_000);
    await rejection;
  });

  it("can delegate a long request deadline to the runtime", async () => {
    vi.useFakeTimers();
    const fetcher = abortablePendingFetch();
    const client = new DeploymentClient({
      apiBaseUrl: "http://runtime.example",
      fetcher,
    });
    const controller = new AbortController();

    const request = client
      .rawRequest(
        "/v1/chat/completions",
        { method: "POST", signal: controller.signal },
        false,
        null,
      )
      .catch((error: unknown) => error);
    await vi.advanceTimersByTimeAsync(8_000);
    expect((fetcher.mock.calls[0]?.[1]?.signal as AbortSignal).aborted).toBe(false);
    controller.abort();
    await expect(request).resolves.toMatchObject({ message: "request aborted" });
  });
});

function abortablePendingFetch() {
  return vi.fn((_input: RequestInfo | URL, init?: RequestInit) => new Promise<Response>((_resolve, reject) => {
    const abort = () => reject(new Error("request aborted"));
    if (init?.signal?.aborted) abort();
    else init?.signal?.addEventListener("abort", abort, { once: true });
  }));
}
