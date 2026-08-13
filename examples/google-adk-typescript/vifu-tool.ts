import type { VifuRuntime } from "../../npm-packages/sdk/dist/index.js";

export function createVifuToolHandler(runtime: VifuRuntime, endpoint: string) {
  return async ({ prompt }: { prompt: string }) => {
    const invocation = await runtime.invoke({
      endpoint,
      sessionId: "google-adk",
      input: { prompt },
    });
    return {
      output: invocation.output,
      vifuInvocationId: invocation.invocationId,
    };
  };
}
