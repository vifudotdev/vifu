import { describe, expect, it } from "vitest";
import { chatCompletionsUrl, inferenceApiBaseUrl } from "./inference-url";

describe("inference API URLs", () => {
  it("uses the fixed OpenAI-compatible path without a project slug", () => {
    expect(chatCompletionsUrl("https://api.vifu.dev")).toBe(
      "https://api.vifu.dev/v1/chat/completions",
    );
  });

  it("preserves an operator-owned base path", () => {
    expect(inferenceApiBaseUrl("https://runtime.example.test/vifu/")).toBe(
      "https://runtime.example.test/vifu/v1",
    );
  });

  it("removes query and fragment data from configured origins", () => {
    expect(chatCompletionsUrl("http://127.0.0.1:6790/?source=console#runtime")).toBe(
      "http://127.0.0.1:6790/v1/chat/completions",
    );
  });
});
