import { FoundryLocalManager } from "foundry-local-sdk";

import { VifuRuntime } from "../../npm-packages/sdk/dist/index.js";
import { registerFoundryAgent } from "./provider.js";

const modelAlias = "qwen2.5-0.5b";
const manager = FoundryLocalManager.create({ appName: "vifu-foundry-local" });
await manager.downloadAndRegisterEps(() => {});
const model = await manager.catalog.getModel(modelAlias);
await model.download(() => {});
await model.load();

const runtime = new VifuRuntime("foundry-local-typescript");
registerFoundryAgent(runtime, model.createChatClient(), { model: modelAlias });

const result = await runtime.invoke({
  endpoint: "foundry-chat",
  input: { prompt: "Explain local inference in one sentence." },
});
console.log(result.output);
console.log({ invocationId: result.invocationId, stages: result.trace });

runtime.close();
await model.unload();
