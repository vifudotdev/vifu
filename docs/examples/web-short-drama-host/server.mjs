import { createReadStream } from "node:fs";
import { readFile, stat } from "node:fs/promises";
import { createServer } from "node:http";
import { extname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("./public/", import.meta.url));
const gameUrl = (process.env.VIFU_GAME_URL || "http://127.0.0.1:6790/last-train-to-the-moon/v1/game").replace(/\/+$/, "");
const apiKey = await loadApiKey();
const port = numberFromEnvironment("PORT", 4180);
const hostCapabilities = [
  "vifu.world.object-action.v1",
  "vifu.presentation.image.v1",
  "vifu.presentation.audio.v1",
];

const server = createServer(async (request, response) => {
  try {
    const url = new URL(request.url || "/", `http://${request.headers.host || "127.0.0.1"}`);
    if (url.pathname.startsWith("/api/")) {
      await handleApi(request, response, url);
      return;
    }
    await serveStatic(response, url.pathname);
  } catch (error) {
    if (response.headersSent) {
      response.destroy(error instanceof Error ? error : undefined);
      return;
    }
    sendJson(response, 500, {
      error: { message: error instanceof Error ? error.message : "The web host failed." },
    });
  }
});

server.listen(port, "127.0.0.1", () => {
  console.log(`Vifu web short-drama host: http://127.0.0.1:${port}`);
  console.log(`Game endpoint: ${gameUrl}`);
});

async function handleApi(request, response, url) {
  if (request.method === "GET" && url.pathname === "/api/bootstrap") {
    const [game, manifest, presentation] = await Promise.all([
      requestJson(""),
      requestJson("/manifest"),
      requestJson("/presentation"),
    ]);
    sendJson(response, 200, { game, manifest, presentation });
    return;
  }

  if (request.method === "POST" && url.pathname === "/api/run") {
    const input = await readJsonBody(request);
    const result = await requestJson("/run", {
      method: "POST",
      body: {
        host: {
          engine: "web-short-drama",
          adapterVersion: "vifu-example-v1",
          capabilities: hostCapabilities,
          locale: typeof input.locale === "string" ? input.locale : "en",
        },
        input: input.input && typeof input.input === "object" ? input.input : {},
        idempotencyKey: `web-run:${crypto.randomUUID()}`,
      },
    });
    sendJson(response, 201, result);
    return;
  }

  const assetMatch = url.pathname.match(/^\/api\/assets\/([0-9a-f-]{36})$/i);
  if (request.method === "GET" && assetMatch) {
    await proxyAsset(request, response, assetMatch[1]);
    return;
  }

  const sessionMatch = url.pathname.match(/^\/api\/sessions\/([0-9a-f-]{36})$/i);
  if (request.method === "GET" && sessionMatch) {
    sendJson(response, 200, await requestJson(`/sessions/${sessionMatch[1]}`));
    return;
  }

  const commandMatch = url.pathname.match(/^\/api\/sessions\/([0-9a-f-]{36})\/commands$/i);
  if (request.method === "POST" && commandMatch) {
    const body = await readJsonBody(request);
    sendJson(response, 200, await requestJson(`/sessions/${commandMatch[1]}/commands`, {
      method: "POST",
      body,
    }));
    return;
  }

  const eventMatch = url.pathname.match(/^\/api\/sessions\/([0-9a-f-]{36})\/events$/i);
  if (request.method === "GET" && eventMatch) {
    await proxyEvents(request, response, eventMatch[1], url.searchParams.get("after"));
    return;
  }

  sendJson(response, 404, { error: { message: "Unknown host API route." } });
}

async function requestJson(path, options = {}) {
  const upstream = await fetch(`${gameUrl}${path}`, {
    method: options.method || "GET",
    headers: {
      Accept: "application/json",
      Authorization: `Bearer ${apiKey}`,
      ...(options.body ? { "Content-Type": "application/json" } : {}),
    },
    body: options.body ? JSON.stringify(options.body) : undefined,
  });
  const payload = await upstream.json().catch(() => null);
  if (!upstream.ok) {
    throw new Error(payload?.error?.message || `Vifu returned HTTP ${upstream.status}.`);
  }
  return payload;
}

async function proxyAsset(request, response, versionId) {
  const upstream = await fetch(`${gameUrl}/assets/${versionId}`, {
    headers: {
      Accept: request.headers.accept || "*/*",
      Authorization: `Bearer ${apiKey}`,
      ...(request.headers.range ? { Range: request.headers.range } : {}),
    },
  });
  if (!upstream.ok || !upstream.body) {
    const payload = await upstream.json().catch(() => null);
    sendJson(response, upstream.status, payload || { error: { message: "Asset unavailable." } });
    return;
  }
  response.writeHead(upstream.status, compactHeaders({
    "accept-ranges": upstream.headers.get("accept-ranges"),
    "cache-control": "private, max-age=31536000, immutable",
    "content-length": upstream.headers.get("content-length"),
    "content-range": upstream.headers.get("content-range"),
    "content-type": upstream.headers.get("content-type") || "application/octet-stream",
    etag: upstream.headers.get("etag"),
  }));
  await pipeWebStream(upstream.body, response);
}

async function proxyEvents(request, response, sessionId, after) {
  const controller = new AbortController();
  response.on("close", () => controller.abort());
  const upstream = await fetch(`${gameUrl}/sessions/${sessionId}/events`, {
    headers: {
      Accept: "text/event-stream",
      Authorization: `Bearer ${apiKey}`,
      "Last-Event-ID": String(request.headers["last-event-id"] || after || "0"),
    },
    signal: controller.signal,
  });
  if (!upstream.ok || !upstream.body) {
    const payload = await upstream.json().catch(() => null);
    sendJson(response, upstream.status, payload || { error: { message: "Event stream unavailable." } });
    return;
  }
  response.writeHead(200, {
    "Cache-Control": "no-cache, no-transform",
    Connection: "keep-alive",
    "Content-Type": "text/event-stream",
    "X-Accel-Buffering": "no",
  });
  try {
    await pipeWebStream(upstream.body, response);
  } catch (error) {
    if (!controller.signal.aborted) throw error;
  }
}

async function pipeWebStream(stream, response) {
  const reader = stream.getReader();
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    if (!response.write(Buffer.from(value))) {
      await new Promise((resolveDrain) => response.once("drain", resolveDrain));
    }
  }
  response.end();
}

async function serveStatic(response, pathname) {
  const requested = pathname === "/" ? "index.html" : decodeURIComponent(pathname.slice(1));
  const file = resolve(root, requested);
  if (!file.startsWith(root)) {
    sendJson(response, 404, { error: { message: "Not found." } });
    return;
  }
  const metadata = await stat(file).catch(() => null);
  if (!metadata?.isFile()) {
    sendJson(response, 404, { error: { message: "Not found." } });
    return;
  }
  response.writeHead(200, {
    "Cache-Control": "no-cache",
    "Content-Length": metadata.size,
    "Content-Security-Policy": "default-src 'self'; img-src 'self' data:; media-src 'self'; connect-src 'self'; style-src 'self'; script-src 'self'; base-uri 'none'; frame-ancestors 'none'",
    "Content-Type": mimeType(file),
    "Referrer-Policy": "no-referrer",
    "X-Content-Type-Options": "nosniff",
  });
  createReadStream(file).pipe(response);
}

async function readJsonBody(request) {
  let body = "";
  for await (const chunk of request) {
    body += chunk;
    if (body.length > 64 * 1024) throw new Error("Request body is too large.");
  }
  if (!body) return {};
  const value = JSON.parse(body);
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error("Request body must be a JSON object.");
  }
  return value;
}

async function loadApiKey() {
  if (process.env.VIFU_API_KEY?.trim()) return process.env.VIFU_API_KEY.trim();
  const path = process.env.VIFU_API_KEY_FILE?.trim();
  if (!path) throw new Error("Set VIFU_API_KEY or VIFU_API_KEY_FILE before starting the web host.");
  const value = (await readFile(path, "utf8")).trim();
  if (!value) throw new Error("VIFU_API_KEY_FILE is empty.");
  return value;
}

function sendJson(response, status, value) {
  const body = JSON.stringify(value);
  response.writeHead(status, {
    "Cache-Control": "no-store",
    "Content-Length": Buffer.byteLength(body),
    "Content-Type": "application/json; charset=utf-8",
    "X-Content-Type-Options": "nosniff",
  });
  response.end(body);
}

function compactHeaders(headers) {
  return Object.fromEntries(Object.entries(headers).filter(([, value]) => value));
}

function mimeType(file) {
  return ({
    ".css": "text/css; charset=utf-8",
    ".html": "text/html; charset=utf-8",
    ".js": "text/javascript; charset=utf-8",
    ".svg": "image/svg+xml",
  })[extname(file)] || "application/octet-stream";
}

function numberFromEnvironment(name, fallback) {
  const value = Number(process.env[name] || fallback);
  if (!Number.isInteger(value) || value < 1 || value > 65535) {
    throw new Error(`${name} must be a valid TCP port.`);
  }
  return value;
}
