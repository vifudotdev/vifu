import { FunctionTool, LlmAgent } from "@google/adk";
import { z } from "zod";

import { VifuRuntime } from "../../npm-packages/sdk/dist/index.js";
import { createVifuToolHandler } from "./vifu-tool.js";

const runtime = new VifuRuntime("google-adk-typescript");

runtime.agent({
  id: "local-reasoner",
  endpoint: "on-device-task",
  metadata: { model: "example-local-provider" },
  async handler(request, trace) {
    const input = request.input as { prompt: string };
    return trace.stage(
      "decode",
      async () => ({
        output: { text: `On-device result: ${input.prompt}` },
        metadata: { model: "example-local-provider" },
      }),
      { model: "example-local-provider" },
    );
  },
});

const askOnDevice = new FunctionTool({
  name: "ask_on_device",
  description: "Runs a task through the on-device Vifu agent.",
  parameters: z.object({
    prompt: z.string().describe("The task for the on-device agent."),
  }),
  execute: createVifuToolHandler(runtime, "on-device-task"),
});

export const rootAgent = new LlmAgent({
  name: "vifu_device_router",
  model: "gemini-flash-latest",
  instruction:
    "Use ask_on_device when the user asks you to run a task on the local device.",
  tools: [askOnDevice],
});
