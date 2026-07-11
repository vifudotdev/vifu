import { createServer } from "node:http";

const host = process.env.OPENCLAW_MOCK_HOST?.trim() || "127.0.0.1";
const port = Number(process.env.OPENCLAW_MOCK_PORT || 18789);
const maxBodyBytes = 512 * 1024;
let canceledRequests = 0;

const server = createServer(async (request, response) => {
  if (request.method === "GET" && request.url === "/health") {
    return json(response, 200, { status: "ok", service: "openclaw-mock" });
  }
  if (request.method === "GET" && request.url === "/metrics") {
    return json(response, 200, { canceledRequests });
  }
  if (request.method === "GET" && request.url === "/v1/models") {
    return json(response, 200, {
      object: "list",
      data: [
        { id: "openclaw", object: "model", name: "OpenClaw" },
        { id: "openclaw/default", object: "model", name: "Default agent" },
        { id: "openclaw/guide-agent", object: "model", name: "Guide agent" },
        { id: "openclaw/writer-agent", object: "model", name: "Writer agent" },
      ],
    });
  }
  if (request.method === "POST" && request.url === "/v1/chat/completions") {
    try {
      const input = JSON.parse(await readBody(request));
      const message = Array.isArray(input.messages)
        ? [...input.messages].reverse().find((item) => item?.role === "user")?.content
        : "";
      const agentId = typeof input.model === "string" ? input.model.replace(/^openclaw[/:]/, "") : "default";
      const delay = typeof message === "string" ? /^delay:(\d+)$/.exec(message)?.[1] : null;
      if (delay) {
        response.once("close", () => {
          if (!response.writableEnded) canceledRequests += 1;
        });
        await new Promise((resolve) => setTimeout(resolve, Number(delay)));
        if (response.destroyed) return;
      }
      return json(response, 200, {
        id: "chatcmpl-vifu-test",
        object: "chat.completion",
        choices: [{
          index: 0,
          finish_reason: "stop",
          message: { role: "assistant", content: `OpenClaw received: ${message}` },
        }],
        model: `openclaw/${agentId}`,
        usage: { prompt_tokens: 1, completion_tokens: 1, total_tokens: 2 },
      });
    } catch (error) {
      return json(response, error?.code === "BODY_TOO_LARGE" ? 413 : 400, {
        error: { message: error instanceof Error ? error.message : "Invalid request" },
      });
    }
  }
  return json(response, 404, { error: { message: "Not found" } });
});

server.listen(port, host, () => {
  console.log(`OpenClaw mock listening on http://${host}:${port}`);
});

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

function json(response, status, body) {
  response.writeHead(status, {
    "content-type": "application/json; charset=utf-8",
    "cache-control": "no-store",
  });
  response.end(JSON.stringify(body));
}
