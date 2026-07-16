import { readFileSync } from "node:fs";
import vm from "node:vm";
import { describe, expect, it } from "vitest";
import {
  VIFU_HOST_SOURCE,
  VIFU_PROTOCOL_VERSION,
  VIFU_RUNTIME_CAPABILITY,
  VIFU_RUNTIME_CONNECT_MESSAGE,
  VIFU_RUNTIME_METHODS,
  VIFU_RUNTIME_SOURCE,
  VIFU_SDK_VERSION,
  VIFU_WEB_HOST_SOURCE,
  createClient,
  createGameRuntimeSDK,
  createVifuSDK,
} from "../dist/index.js";

function createHarness() {
  const messages = [];
  const vifu = createVifuSDK({
    documentTitle: "sdk-test",
    postMessage: (message) => messages.push(message),
  });
  return { vifu, messages };
}

describe("@vifu/sdk", () => {
  it("exposes a small runtime-only public facade", () => {
    const vifu = createClient({ transport: "none" });

    expect(vifu.version).toBe(VIFU_SDK_VERSION);
    expect(vifu.protocolVersion).toBe(VIFU_PROTOCOL_VERSION);
    expect(vifu.status()).toMatchObject({
      sdkVersion: VIFU_SDK_VERSION,
      protocolVersion: VIFU_PROTOCOL_VERSION,
      capability: VIFU_RUNTIME_CAPABILITY,
      hostConnected: false,
    });

    for (const key of [
      "companion",
      "agent",
      "ai",
      "plugins",
    ]) {
      expect(vifu).not.toHaveProperty(key);
    }
  });

  it("calls a custom platform adapter with generic capability IDs", async () => {
    const calls = [];
    const vifu = createVifuSDK({
      platform: {
        name: "local-adapter",
        status: () => ({ available: true, adapter: "local-adapter", gameId: "demo" }),
        invoke: (capabilityId, args) => {
          calls.push({ capabilityId, args });
          return { ok: true, capabilityId, args };
        },
      },
    });

    await expect(vifu.invoke("example.echo", { text: "hello" })).resolves.toEqual({
      ok: true,
      capabilityId: "example.echo",
      args: { text: "hello" },
    });
    expect(calls).toEqual([{ capabilityId: "example.echo", args: { text: "hello" } }]);
    expect(vifu.status().platformStatus).toEqual({
      available: true,
      adapter: "local-adapter",
      gameId: "demo",
    });
  });

  it("routes capability calls through a ready host bridge", async () => {
    const { vifu, messages } = createHarness();

    expect(vifu._handleEnvelope({
      source: VIFU_HOST_SOURCE,
      message: { jsonrpc: "2.0", method: VIFU_RUNTIME_METHODS.hostReady },
    })).toBe(true);

    const result = vifu.invoke("example.save", { slot: "autosave" });
    const request = messages.find((message) => message.method === VIFU_RUNTIME_METHODS.invoke);
    expect(request.params).toEqual({
      capabilityId: "example.save",
      arguments: { slot: "autosave" },
    });

    expect(vifu._handleEnvelope({
      jsonrpc: "2.0",
      id: request.id,
      result: { ok: true, slot: "autosave" },
    })).toBe(true);
    await expect(result).resolves.toEqual({ ok: true, slot: "autosave" });
  });

  it("sends runtime host events and external-link requests", () => {
    const { vifu, messages } = createHarness();
    const event = vifu.runtime.emitEvent("game.item.collected", { itemId: "key" }, { source: "/games/demo" });
    const link = vifu.runtime.openExternal({ href: "https://example.com", label: "Help" });

    expect(event).toMatchObject({
      specversion: "1.0",
      source: "/games/demo",
      type: "game.item.collected",
      data: { itemId: "key" },
    });
    expect(link).toMatchObject({ href: "https://example.com", label: "Help" });
    expect(messages.map((message) => message.method)).toEqual([
      VIFU_RUNTIME_METHODS.hostEvent,
      VIFU_RUNTIME_METHODS.hostOpenExternal,
    ]);
  });

  it("announces runtime readiness over iframe message channels", async () => {
    const posted = [];
    const listeners = new Set();
    const fakeWindow = {
      parent: { postMessage: (message) => posted.push(message) },
      addEventListener: (_type, listener) => listeners.add(listener),
      removeEventListener: (_type, listener) => listeners.delete(listener),
      document: { title: "Runtime Test" },
    };
    const previousWindow = globalThis.window;
    globalThis.window = fakeWindow;
    try {
      const vifu = createVifuSDK({ transport: "iframe" });
      expect(posted[0]).toMatchObject({
        source: VIFU_RUNTIME_SOURCE,
        message: {
          method: VIFU_RUNTIME_METHODS.runtimeReady,
          params: {
            protocolVersion: VIFU_PROTOCOL_VERSION,
            sdkVersion: VIFU_SDK_VERSION,
            capability: VIFU_RUNTIME_CAPABILITY,
          },
        },
      });

      const portMessages = [];
      const fakePort = {
        postMessage: (message) => portMessages.push(message),
        start() {},
        close() {},
      };
      for (const listener of listeners) {
        listener({
          data: {
            source: VIFU_WEB_HOST_SOURCE,
            type: VIFU_RUNTIME_CONNECT_MESSAGE,
          },
          ports: [fakePort],
        });
      }
      vifu.runtime.emitEvent("game.started", { ok: true });
      expect(portMessages.some((message) => message.method === VIFU_RUNTIME_METHODS.hostEvent)).toBe(true);
      vifu._disposeTransport();
    } finally {
      globalThis.window = previousWindow;
    }
  });

  it("handles host envelopes and rejects unknown host requests", () => {
    const { vifu, messages } = createHarness();
    expect(vifu._handleEnvelope({
      source: VIFU_HOST_SOURCE,
      message: JSON.stringify({ jsonrpc: "2.0", method: VIFU_RUNTIME_METHODS.hostReady }),
    })).toBe(true);
    expect(vifu.runtime.isConnected()).toBe(true);

    expect(vifu._handleEnvelope({
      source: VIFU_HOST_SOURCE,
      message: { jsonrpc: "2.0", id: 10, method: "unknown.method" },
    })).toBe(true);
    expect(messages.at(-1)).toEqual({
      jsonrpc: "2.0",
      id: 10,
      error: {
        code: -32601,
        message: "Method not found: unknown.method",
      },
    });
  });

  it("creates the browser global without companion APIs", async () => {
    const source = readFileSync(new URL("../dist/browser/vifu-sdk.js", import.meta.url), "utf8");
    const posted = [];
    const context = {
      console,
      setTimeout,
      clearTimeout,
      window: {
        parent: { postMessage: (message) => posted.push(message) },
        addEventListener() {},
        removeEventListener() {},
        document: { title: "Browser Runtime" },
      },
    };
    context.globalThis = context.window;
    vm.createContext(context);
    vm.runInContext(source, context);

    expect(context.window.vifu.version).toBe(VIFU_SDK_VERSION);
    expect(context.window.vifu.protocolVersion).toBe(VIFU_PROTOCOL_VERSION);
    expect(context.window.Vifu).toBe(context.window.vifu);
    expect(context.window.vifu).not.toHaveProperty("companion");
    expect(posted[0]).toMatchObject({
      source: VIFU_RUNTIME_SOURCE,
      message: { method: VIFU_RUNTIME_METHODS.runtimeReady },
    });
  });

  it("exports createGameRuntimeSDK as a runtime-focused alias", () => {
    const vifu = createGameRuntimeSDK({ transport: "none" });
    expect(vifu.status().capability).toBe(VIFU_RUNTIME_CAPABILITY);
  });
});
