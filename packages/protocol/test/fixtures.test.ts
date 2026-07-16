import { readdirSync, readFileSync } from "node:fs";

import { describe, expect, it } from "vitest";

import {
  NODE_INVOKE_REQUEST_EVENT,
  NODE_INVOKE_RESULT_METHOD,
  NodeInvokeRequestPayloadSchema,
  NodeInvokeResultErrorSchema,
  NodeInvokeResultParamsSchema,
  isGatewayFrame,
  isRequestFrame,
  isEventFrame,
} from "../src";

const fixtureUrl = new URL("../fixtures/gateway-frame/", import.meta.url);

function fixture(name: string): unknown {
  return JSON.parse(readFileSync(new URL(name, fixtureUrl), "utf8"));
}

function fixtureNames(): string[] {
  return readdirSync(fixtureUrl)
    .filter((name) => name.endsWith(".json"))
    .sort();
}

describe("@vifu/protocol shared gateway frame fixtures", () => {
  it("accepts every shared gateway frame fixture", () => {
    const names = fixtureNames();
    expect(names.length).toBeGreaterThan(0);
    for (const name of names) {
      expect(isGatewayFrame(fixture(name)), name).toBe(true);
    }
  });

  it("keeps node invoke fixture names aligned", () => {
    const request = fixture("node-invoke-request.json");
    expect(isEventFrame(request)).toBe(true);
    if (isEventFrame(request)) {
      expect(request.event).toBe(NODE_INVOKE_REQUEST_EVENT);
    }

    const result = fixture("node-invoke-result.json");
    expect(isRequestFrame(result)).toBe(true);
    if (isRequestFrame(result)) {
      expect(result.method).toBe(NODE_INVOKE_RESULT_METHOD);
    }
  });

  it("keeps node invoke schemas omit-only for absent JSON payload fields", () => {
    expect(NodeInvokeRequestPayloadSchema.properties.paramsJSON).toEqual({ type: "string" });
    expect(NodeInvokeResultParamsSchema.properties.payloadJSON).toEqual({ type: "string" });
    expect(NodeInvokeResultParamsSchema.properties.error).toBe(NodeInvokeResultErrorSchema);
  });
});
