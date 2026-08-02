import { afterEach, describe, expect, it, vi } from "vitest";

import { DeploymentClient } from "./deployment-client";

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
});

function abortablePendingFetch() {
  return vi.fn((_input: RequestInfo | URL, init?: RequestInit) => new Promise<Response>((_resolve, reject) => {
    const abort = () => reject(new Error("request aborted"));
    if (init?.signal?.aborted) abort();
    else init?.signal?.addEventListener("abort", abort, { once: true });
  }));
}
