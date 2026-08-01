import { readFile } from "node:fs/promises";
import { describe, expect, it } from "vitest";

import {
  VIFU_RUNTIME_BRIDGE_EVENTS,
  VIFU_RUNTIME_BRIDGE_METHODS,
  VIFU_RUNTIME_BRIDGE_PROTOCOL_VERSION,
  decodeGatewayFrame,
  isEventFrame,
  isRequestFrame,
  isResponseFrame,
} from "../src";

async function fixture(name: string): Promise<string> {
  return readFile(new URL(`../fixtures/runtime-bridge/${name}`, import.meta.url), "utf8");
}

describe("runtime bridge protocol", () => {
  it("decodes the shared hello and invoke request fixtures", async () => {
    const hello = decodeGatewayFrame(await fixture("hello.json"));
    expect(isRequestFrame(hello)).toBe(true);
    if (!isRequestFrame(hello)) return;
    expect(hello.method).toBe(VIFU_RUNTIME_BRIDGE_METHODS.HELLO);
    expect(hello.params).toEqual({
      protocol: VIFU_RUNTIME_BRIDGE_PROTOCOL_VERSION,
    });

    const invoke = decodeGatewayFrame(await fixture("invoke.json"));
    expect(isRequestFrame(invoke)).toBe(true);
    if (!isRequestFrame(invoke)) return;
    expect(invoke.method).toBe(VIFU_RUNTIME_BRIDGE_METHODS.INVOKE);
  });

  it("decodes response and streaming event fixtures", async () => {
    const response = decodeGatewayFrame(await fixture("invoke-accepted.json"));
    expect(isResponseFrame(response)).toBe(true);

    const delta = decodeGatewayFrame(await fixture("output-delta.json"));
    expect(isEventFrame(delta)).toBe(true);
    if (!isEventFrame(delta)) return;
    expect(delta.event).toBe(VIFU_RUNTIME_BRIDGE_EVENTS.OUTPUT_DELTA);
  });
});
