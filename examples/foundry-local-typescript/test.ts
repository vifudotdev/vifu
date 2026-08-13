import { strict as assert } from "node:assert";

import { VifuRuntime } from "../../npm-packages/sdk/dist/index.js";
import { registerFoundryAgent } from "./provider.js";

const client = {
  async *completeStreamingChat() {
    yield { choices: [{ delta: { content: "local " } }] };
    yield { choices: [{ delta: { content: "answer" } }] };
  },
};

const runtime = new VifuRuntime("foundry-local-typescript-test");
registerFoundryAgent(runtime, client, { model: "test-model" });
const result = await runtime.invoke({
  endpoint: "foundry-chat",
  input: { prompt: "hello" },
});

assert.deepEqual(result.output, { text: "local answer" });
assert.deepEqual(result.trace.map((stage) => stage.name), [
  "first_token",
  "decode",
  "provider.invoke",
]);
assert.equal((result.metadata as { model: string }).model, "test-model");
runtime.close();

console.log("Foundry Local TypeScript adapter passed");
