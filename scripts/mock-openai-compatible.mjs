import { createServer } from "node:http";

const host = process.env.OPENAI_COMPATIBLE_MOCK_HOST?.trim() || "127.0.0.1";
const port = Number(process.env.OPENAI_COMPATIBLE_MOCK_PORT || 18901);
const token = process.env.OPENAI_COMPATIBLE_MOCK_TOKEN?.trim() || "";
const maxBodyBytes = 2 * 1024 * 1024;

const metrics = {
  chatRequests: 0,
  chatImageRequests: 0,
  embeddingRequests: 0,
  transcriptionRequests: 0,
  unauthorizedRequests: 0,
  modelsRequests: 0,
};

const server = createServer(async (request, response) => {
  try {
    if (request.method === "GET" && request.url === "/health") {
      return json(response, 200, { status: "ok", service: "openai-compatible-mock" });
    }
    if (request.method === "GET" && request.url === "/metrics") {
      return json(response, 200, metrics);
    }
    if (!authorized(request)) {
      metrics.unauthorizedRequests += 1;
      return json(response, 401, { error: { message: "Unauthorized" } });
    }
    if (request.method === "GET" && request.url === "/v1/models") {
      metrics.modelsRequests += 1;
      return json(response, 200, {
        object: "list",
        data: [
          { id: "vifu-e2e-chat", object: "model", owned_by: "vifu-e2e" },
          { id: "vifu-e2e-chat-alt", object: "model", owned_by: "vifu-e2e" },
          { id: "vifu-e2e-embedding", object: "model", owned_by: "vifu-e2e" },
          { id: "vifu-e2e-transcription", object: "model", owned_by: "vifu-e2e" },
        ],
      });
    }
    if (request.method === "POST" && request.url === "/v1/chat/completions") {
      const input = JSON.parse(await readBody(request));
      const userMessage = latestUserMessage(input.messages);
      const hasImage = messageHasImage(input.messages);
      metrics.chatRequests += 1;
      if (hasImage) metrics.chatImageRequests += 1;
      return json(response, 200, {
        id: "chatcmpl-openai-compatible-e2e",
        object: "chat.completion",
        created: Math.floor(Date.now() / 1000),
        model: input.model || "vifu-e2e-chat",
        choices: [{
          index: 0,
          finish_reason: "stop",
          message: {
            role: "assistant",
            content: `OpenAI-compatible mock received: ${userMessage}; image=${hasImage}`,
          },
        }],
        usage: { prompt_tokens: 8, completion_tokens: 7, total_tokens: 15 },
      });
    }
    if (request.method === "POST" && request.url === "/v1/embeddings") {
      const input = JSON.parse(await readBody(request));
      const items = Array.isArray(input.input) ? input.input : [input.input];
      metrics.embeddingRequests += 1;
      return json(response, 200, {
        object: "list",
        model: input.model || "vifu-e2e-embedding",
        data: items.map((_, index) => ({
          object: "embedding",
          index,
          embedding: [0.125, 0.25, 0.5, 1],
        })),
        usage: { prompt_tokens: items.length, total_tokens: items.length },
      });
    }
    if (request.method === "POST" && request.url === "/v1/audio/transcriptions") {
      const body = await readBody(request);
      metrics.transcriptionRequests += 1;
      if (!body.includes('name="model"') || !body.includes("vifu-e2e-transcription")) {
        return json(response, 400, { error: { message: "transcription model was not forwarded" } });
      }
      if (!body.includes('name="file"')) {
        return json(response, 400, { error: { message: "audio file was not forwarded" } });
      }
      return json(response, 200, {
        text: "mock transcription from OpenAI-compatible provider",
      });
    }
    return json(response, 404, { error: { message: "Not found" } });
  } catch (error) {
    return json(response, error?.code === "BODY_TOO_LARGE" ? 413 : 400, {
      error: { message: error instanceof Error ? error.message : "Invalid request" },
    });
  }
});

server.listen(port, host, () => {
  console.log(`OpenAI-compatible mock listening on http://${host}:${port}`);
});

function latestUserMessage(messages) {
  if (!Array.isArray(messages)) return "";
  const item = [...messages].reverse().find((message) => message?.role === "user");
  const content = item?.content;
  if (typeof content === "string") return content;
  if (!Array.isArray(content)) return "";
  return content
    .filter((part) => part?.type === "text" && typeof part.text === "string")
    .map((part) => part.text)
    .join(" ");
}

function messageHasImage(messages) {
  return Array.isArray(messages)
    && messages.some((message) => Array.isArray(message?.content)
      && message.content.some((part) => part?.type === "image_url" && part.image_url?.url));
}

function readBody(request) {
  return new Promise((resolve, reject) => {
    const chunks = [];
    let size = 0;
    request.on("data", (chunk) => {
      size += chunk.length;
      if (size > maxBodyBytes) {
        const error = new Error("Request body is too large");
        error.code = "BODY_TOO_LARGE";
        reject(error);
        request.destroy();
        return;
      }
      chunks.push(chunk);
    });
    request.on("end", () => resolve(Buffer.concat(chunks).toString("utf8")));
    request.on("error", reject);
  });
}

function authorized(request) {
  if (!token) return true;
  return request.headers.authorization === `Bearer ${token}`;
}

function json(response, status, body) {
  response.writeHead(status, {
    "content-type": "application/json; charset=utf-8",
    "cache-control": "no-store",
  });
  response.end(JSON.stringify(body));
}
