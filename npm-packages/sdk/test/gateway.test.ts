import path from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, test } from "vitest";

import { GatewayPairing, VifuRuntime, VifuServer } from "../dist/index.js";

const packageRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

describe("TypeScript Gateway host", () => {
  test("manages a native Vifu Server process", async () => {
    const server = await VifuServer.start({
      executable: process.execPath,
      arguments: [path.join(packageRoot, "test", "server-host-fixture.cjs")],
      waitMs: 20,
    });
    expect(server.running).toBe(true);
    await server.close();
    expect(server.running).toBe(false);
  });

  test("returns completed provider stages from the WebAssembly Runtime", async () => {
    const runtime = new VifuRuntime("typescript-stage-test");
    runtime.agent({
      id: "guide",
      handler(_request, trace) {
        return trace.stage("decode", () => ({ text: "done" }), {
          model: "fixture-model",
        });
      },
    });

    const result = await runtime.invoke({ endpoint: "guide", input: {} });

    expect(result.trace.map((stage) => stage.name)).toEqual(["decode", "provider.invoke"]);
    runtime.close();
  });

  test("parses direct and web pairing links", () => {
    const direct = GatewayPairing.parse(
      "vifu://gateway/enroll?server=http%3A%2F%2F127.0.0.1%3A6790&token=vifu_ge_fixture",
    );
    const web = GatewayPairing.parse(
      "https://vifu.ai/pair#server=http%3A%2F%2F127.0.0.1%3A6790&token=vifu_ge_fixture",
    );
    expect(direct.serverUrl).toBe("http://127.0.0.1:6790");
    expect(web.enrollmentToken).toBe("vifu_ge_fixture");
  });

  test("routes a host invocation through the registered handler with trace stages", async () => {
    const runtime = new VifuRuntime("typescript-gateway-test");
    runtime.agent({
      id: "guide",
      handler(request, trace) {
        const input = request.input as { prompt: string };
        return trace.stage(
          "decode",
          () => ({ text: `Local answer: ${input.prompt}` }),
          { model: "fixture-model" },
        );
      },
    });
    const gateway = await runtime.connect(
      "vifu://gateway/enroll?server=http%3A%2F%2F127.0.0.1%3A6790&token=vifu_ge_fixture",
      {
        gatewayExecutable: process.execPath,
        gatewayArguments: [path.join(packageRoot, "test", "gateway-host-fixture.cjs")],
      },
    );
    const status = await gateway.waitUntilConnected();
    expect(status.gatewayId).toBe("gateway-fixture");
    expect(runtime.pendingTraces()).toHaveLength(0);
    await gateway.close();
    runtime.close();
  });
});
