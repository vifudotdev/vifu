import { strict as assert } from "node:assert";

import { VifuRuntime } from "../../npm-packages/sdk/dist/index.js";
import { createVifuToolHandler } from "./vifu-tool.js";

const runtime = new VifuRuntime("google-adk-typescript-test");
runtime.agent({
  id: "local",
  handler: (request) => ({ text: (request.input as { prompt: string }).prompt }),
});

const result = await createVifuToolHandler(runtime, "local")({ prompt: "hello" });
assert.deepEqual(result.output, { text: "hello" });
assert.ok(result.vifuInvocationId);
assert.equal(runtime.pendingTraces().length, 1);
runtime.close();

console.log("Google ADK TypeScript adapter passed");
