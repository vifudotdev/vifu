import { VifuRuntime } from "../../npm-packages/sdk/dist/index.js";

const runtime = new VifuRuntime("typescript-starter");

runtime.agent({
  id: "guide",
  name: "Local Guide",
  metadata: { model: "typescript-echo" },
  async handler(request, trace) {
    const input = request.input as { prompt: string };
    return trace.stage("decode", async () => ({
        output: { text: `Local answer: ${input.prompt}` },
        metadata: { model: "typescript-echo" },
      }),
      { model: "typescript-echo" },
    );
  },
});

const invocation = await runtime.invoke({
  endpoint: "guide",
  sessionId: "first-session",
  input: { prompt: "Where did this agent run?" },
});

console.log(invocation.output);
console.log({
  invocationId: invocation.invocationId,
  durationMs: invocation.trace[0]?.durationMs,
  pendingTraces: runtime.pendingTraces().length,
});

runtime.close();
