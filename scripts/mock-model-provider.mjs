import { createServer } from "node:http";

const host = process.env.MODEL_FIXTURE_HOST?.trim() || "127.0.0.1";
const port = Number(process.env.MODEL_FIXTURE_PORT || 18791);
const model = "vifu-test";

createServer(async (request, response) => {
  if (request.method === "GET" && request.url === "/v1/models") {
    return json(response, 200, {
      object: "list",
      data: [{ id: model, object: "model", owned_by: "vifu-test" }],
    });
  }
  if (request.method !== "POST" || request.url !== "/v1/chat/completions") {
    return json(response, 404, { error: { message: "Not found" } });
  }

  const body = await readJson(request);
  const userMessage = Array.isArray(body.messages)
    ? [...body.messages].reverse().find((message) => message?.role === "user")
    : null;
  const marker = collectStrings(body).find((value) => value.includes("-vifu-e2e"));
  const prompt = marker || contentText(userMessage?.content) || contentText(body.input);
  const reply = `Real OpenClaw relayed: ${prompt}`;
  if (body.stream === true) return stream(response, reply);
  return json(response, 200, completion(reply));
}).listen(port, host, () => {
  console.log(`Model fixture listening on http://${host}:${port}`);
});

function completion(content) {
  return {
    id: "chatcmpl-vifu-model-fixture",
    object: "chat.completion",
    created: Math.floor(Date.now() / 1000),
    model,
    choices: [{ index: 0, message: { role: "assistant", content }, finish_reason: "stop" }],
    usage: { prompt_tokens: 1, completion_tokens: 1, total_tokens: 2 },
  };
}

function contentText(value) {
  if (typeof value === "string") return value;
  if (!Array.isArray(value)) return "";
  return value
    .map((part) => {
      if (typeof part === "string") return part;
      if (!part || typeof part !== "object") return "";
      return typeof part.text === "string"
        ? part.text
        : typeof part.content === "string"
          ? part.content
          : "";
    })
    .filter(Boolean)
    .join("\n");
}

function collectStrings(value, result = []) {
  if (typeof value === "string") {
    result.push(value);
  } else if (Array.isArray(value)) {
    for (const item of value) collectStrings(item, result);
  } else if (value && typeof value === "object") {
    for (const item of Object.values(value)) collectStrings(item, result);
  }
  return result;
}

function stream(response, content) {
  response.writeHead(200, {
    "content-type": "text/event-stream; charset=utf-8",
    "cache-control": "no-store",
    connection: "keep-alive",
  });
  const base = {
    id: "chatcmpl-vifu-model-fixture",
    object: "chat.completion.chunk",
    created: Math.floor(Date.now() / 1000),
    model,
  };
  response.write(`data: ${JSON.stringify({ ...base, choices: [{ index: 0, delta: { role: "assistant", content }, finish_reason: null }] })}\n\n`);
  response.write(`data: ${JSON.stringify({ ...base, choices: [{ index: 0, delta: {}, finish_reason: "stop" }] })}\n\n`);
  response.end("data: [DONE]\n\n");
}

async function readJson(request) {
  const chunks = [];
  for await (const chunk of request) chunks.push(chunk);
  return JSON.parse(Buffer.concat(chunks).toString("utf8") || "{}");
}

function json(response, status, body) {
  response.writeHead(status, {
    "content-type": "application/json; charset=utf-8",
    "cache-control": "no-store",
  });
  response.end(JSON.stringify(body));
}
