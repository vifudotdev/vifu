import { describe, expect, test } from "vitest";

import { forwardRuntimeResponse } from "./runtime-proxy";

describe("forwardRuntimeResponse", () => {
  test("preserves a created response", async () => {
    const response = await forwardRuntimeResponse(new Response(
      JSON.stringify({ id: "deployment-1" }),
      {
        status: 201,
        headers: {
          "content-type": "application/json; charset=utf-8",
          "set-cookie": "internal=value",
        },
      },
    ));

    expect(response.status).toBe(201);
    expect(response.headers.get("content-type")).toBe("application/json; charset=utf-8");
    expect(response.headers.get("cache-control")).toBe("no-store");
    expect(response.headers.get("set-cookie")).toBeNull();
    expect(await response.json()).toEqual({ id: "deployment-1" });
  });

  test("preserves structured runtime errors", async () => {
    const response = await forwardRuntimeResponse(Response.json(
      { error: { code: "release_conflict", message: "Release already exists." } },
      { status: 409, statusText: "Conflict" },
    ));

    expect(response.status).toBe(409);
    expect(response.statusText).toBe("Conflict");
    expect(await response.json()).toEqual({
      error: { code: "release_conflict", message: "Release already exists." },
    });
  });

  test("does not attach a body to a no-content response", async () => {
    const response = await forwardRuntimeResponse(new Response(null, { status: 204 }));

    expect(response.status).toBe(204);
    expect(response.headers.get("content-type")).toBeNull();
    expect(await response.text()).toBe("");
  });
});
