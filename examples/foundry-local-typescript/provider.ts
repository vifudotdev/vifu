import type {
  VifuAgentRequest,
  VifuAgentTrace,
  VifuRuntime,
} from "../../npm-packages/sdk/dist/index.js";

interface FoundryChunk {
  choices?: Array<{ delta?: { content?: string | null } }>;
}

export interface FoundryChatClient {
  completeStreamingChat(messages: Array<{ role: string; content: string }>): AsyncIterable<FoundryChunk>;
}

export function registerFoundryAgent(
  runtime: VifuRuntime,
  client: FoundryChatClient,
  options: { model: string; endpoint?: string },
): VifuRuntime {
  const endpoint = options.endpoint ?? "foundry-chat";
  return runtime.agent({
    id: "foundry-local",
    endpoint,
    metadata: { model: options.model, framework: "foundry-local" },
    handler: (request, trace) => invokeFoundry(request, trace, client, options.model),
  });
}

async function invokeFoundry(
  request: VifuAgentRequest,
  trace: VifuAgentTrace,
  client: FoundryChatClient,
  model: string,
) {
  const input = request.input as { prompt: string };
  const stream = client.completeStreamingChat([{ role: "user", content: input.prompt }]);
  const iterator = stream[Symbol.asyncIterator]();
  const parts: string[] = [];

  await trace.stage("first_token", async () => {
    while (true) {
      const next = await iterator.next();
      if (next.done) return;
      const content = next.value.choices?.[0]?.delta?.content;
      if (content) {
        parts.push(content);
        trace.outputDelta({ text: content });
        return;
      }
    }
  }, { model });

  await trace.stage("decode", async () => {
    while (true) {
      const next = await iterator.next();
      if (next.done) return;
      const content = next.value.choices?.[0]?.delta?.content;
      if (content) {
        parts.push(content);
        trace.outputDelta({ text: content });
      }
    }
  }, { model });

  return {
    output: { text: parts.join("") },
    metadata: { model, provider: "foundry-local" },
  };
}
