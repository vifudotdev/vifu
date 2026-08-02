import { describe, expect, it } from "vitest";

import { POST } from "../app/api/runtime/[...path]/route";

describe("runtime API proxy", () => {
  it("rejects cross-origin mutations before resolving dashboard authority", async () => {
    const response = await POST(
      new Request("https://dashboard.vifu.test/api/runtime/projects", {
        method: "POST",
        headers: { origin: "https://untrusted.example" },
      }),
      { params: Promise.resolve({ path: ["projects"] }) },
    );

    expect(response.status).toBe(403);
    await expect(response.json()).resolves.toMatchObject({
      error: { code: "INVALID_ORIGIN" },
    });
  });
});
