export type DecodedToolCall = {
  arguments: unknown;
  id: string | null;
  name: string;
};

export type DecodedMessage = {
  content: string;
  name: string | null;
  role: string;
  toolCalls: DecodedToolCall[];
  toolCallId: string | null;
};

export type DecodedTracePayload =
  | { kind: "conversation"; messages: DecodedMessage[] }
  | { kind: "embedding"; count: number; dimensions: number | null; model: string | null; usage: unknown }
  | { kind: "structured"; value: unknown };

export function decodeTracePayload(value: unknown): DecodedTracePayload {
  const conversation = conversationMessages(value);
  if (conversation.length > 0) return { kind: "conversation", messages: conversation };

  const embedding = embeddingSummary(value);
  if (embedding) return embedding;
  return { kind: "structured", value };
}

function conversationMessages(value: unknown): DecodedMessage[] {
  if (Array.isArray(value)) return value.flatMap(messageFromUnknown);
  if (!isRecord(value)) return [];
  if (Array.isArray(value.messages)) return value.messages.flatMap(messageFromUnknown);
  if (Array.isArray(value.choices)) {
    return value.choices.flatMap((choice) => {
      if (!isRecord(choice)) return [];
      return messageFromUnknown(choice.message ?? choice.delta ?? choice.text);
    });
  }
  if (value.message !== undefined) return messageFromUnknown(value.message);
  if (typeof value.output_text === "string") {
    return [decodedMessage("assistant", value.output_text)];
  }
  if (typeof value.text === "string") {
    return [decodedMessage("assistant", value.text)];
  }
  return [];
}

function messageFromUnknown(value: unknown): DecodedMessage[] {
  if (typeof value === "string") return [decodedMessage("assistant", value)];
  if (!isRecord(value)) return [];
  const role = firstString(value.role) ?? "assistant";
  const content = contentText(value.content ?? value.text);
  const toolCalls = Array.isArray(value.tool_calls)
    ? value.tool_calls.flatMap(toolCallFromUnknown)
    : [];
  if (!content && toolCalls.length === 0 && typeof value.name !== "string") return [];
  return [{
    content,
    name: firstString(value.name),
    role,
    toolCalls,
    toolCallId: firstString(value.tool_call_id),
  }];
}

function decodedMessage(role: string, content: string): DecodedMessage {
  return { content, name: null, role, toolCalls: [], toolCallId: null };
}

function toolCallFromUnknown(value: unknown): DecodedToolCall[] {
  if (!isRecord(value)) return [];
  const fn = isRecord(value.function) ? value.function : value;
  const name = firstString(fn.name, value.name);
  if (!name) return [];
  return [{
    arguments: parseJsonString(fn.arguments),
    id: firstString(value.id),
    name,
  }];
}

function contentText(value: unknown): string {
  if (typeof value === "string") return value;
  if (!Array.isArray(value)) return value === null || value === undefined ? "" : scalarText(value);
  return value.map((part) => {
    if (typeof part === "string") return part;
    if (!isRecord(part)) return scalarText(part);
    const text = firstString(part.text, part.input_text, part.output_text);
    if (text) return text;
    const type = firstString(part.type) ?? "content";
    if (type.includes("image")) {
      const trace = isRecord(part.image_url) && isRecord(part.image_url.trace)
        ? part.image_url.trace
        : null;
      const bytes = trace && typeof trace.decodedBytes === "number"
        ? ` · ${trace.decodedBytes} bytes`
        : "";
      return `[image${bytes}]`;
    }
    return `[${type}]`;
  }).filter(Boolean).join("\n");
}

function embeddingSummary(value: unknown): DecodedTracePayload | null {
  if (!isRecord(value) || !Array.isArray(value.data)) return null;
  const embeddings = value.data.filter((entry) => isRecord(entry) && Array.isArray(entry.embedding));
  if (embeddings.length === 0) return null;
  const first = embeddings[0];
  return {
    kind: "embedding",
    count: embeddings.length,
    dimensions: isRecord(first) && Array.isArray(first.embedding) ? first.embedding.length : null,
    model: firstString(value.model),
    usage: value.usage ?? null,
  };
}

function parseJsonString(value: unknown): unknown {
  if (typeof value !== "string") return value ?? null;
  try {
    return JSON.parse(value) as unknown;
  } catch {
    return value;
  }
}

function scalarText(value: unknown): string {
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  return "";
}

function firstString(...values: unknown[]): string | null {
  for (const value of values) {
    if (typeof value === "string" && value.trim()) return value.trim();
  }
  return null;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
