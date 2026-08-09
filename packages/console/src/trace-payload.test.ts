import { describe, expect, it } from "vitest";
import { decodeTracePayload } from "./trace-payload";

describe("decodeTracePayload", () => {
  it("turns OpenAI chat requests into role-aware messages", () => {
    expect(decodeTracePayload({
      model: "agent-alias",
      messages: [{ role: "user", content: "Cut the next patch of grass." }],
    })).toEqual({
      kind: "conversation",
      messages: [{
        content: "Cut the next patch of grass.",
        name: null,
        role: "user",
        toolCalls: [],
        toolCallId: null,
      }],
    });
  });

  it("decodes assistant tool calls and their JSON arguments", () => {
    const decoded = decodeTracePayload({
      choices: [{
        message: {
          role: "assistant",
          content: null,
          tool_calls: [{
            id: "call-1",
            function: { name: "move", arguments: "{\"direction\":\"north\"}" },
          }],
        },
      }],
    });

    expect(decoded).toMatchObject({
      kind: "conversation",
      messages: [{
        role: "assistant",
        toolCalls: [{ name: "move", arguments: { direction: "north" } }],
      }],
    });
  });

  it("summarizes embeddings instead of rendering vector walls", () => {
    expect(decodeTracePayload({
      data: [
        { embedding: [0.1, 0.2, 0.3], index: 0 },
        { embedding: [0.4, 0.5, 0.6], index: 1 },
      ],
      model: "local-embed",
      usage: { prompt_tokens: 4 },
    })).toEqual({
      kind: "embedding",
      count: 2,
      dimensions: 3,
      model: "local-embed",
      usage: { prompt_tokens: 4 },
    });
  });
});
