import { createServer } from "node:http";

const port = Number(process.env.VIFU_TEST_API_PORT ?? 6793);

createServer(async (request, response) => {
  const url = new URL(request.url ?? "/", `http://127.0.0.1:${port}`);
  response.setHeader("cache-control", "no-store");

  if (url.pathname === "/health") return json(response, 200, { status: "ok" });
  if (url.pathname === "/v1/status") {
    return json(response, 200, {
      service: "vifu-server",
      status: "ok",
      version: "0.1.0",
      mode: "cloud",
      connections: 0,
      capabilities: {
        profiles: true,
        endpoints: true,
        bindings: true,
        apiKeys: true,
        connections: true,
        traces: true,
        websocketRelay: true,
        account: true,
        teams: true,
        billing: true,
        managedDomains: true,
      },
    });
  }
  if (url.pathname === "/login" || url.pathname === "/onboarding") {
    response.setHeader("content-type", "text/html; charset=utf-8");
    response.end(`<h1>${url.pathname === "/login" ? "Marketing sign in" : "Marketing onboarding"}</h1>`);
    return;
  }

  const body = await readJson(request);
  if (url.pathname === "/v1/auth/magic-link/consume") {
    const isNewUser = body.code === "new-user";
    return json(response, 200, {
      token: "test-access-token",
      serverSessionId: isNewUser ? "session-new" : "session-existing",
      serverSessionExpiresIn: 3600,
      displayName: isNewUser ? "New creator" : "Existing creator",
      isNewUser,
      onboardingRequired: isNewUser,
      returnTo: body.returnTo,
    });
  }
  if (url.pathname === "/v1/auth/magic-link/session") {
    return json(response, 200, {
      token: "test-access-token",
      serverSessionId: body.serverSessionId,
      serverSessionExpiresIn: 3600,
      displayName: "Existing creator",
    });
  }
  if (url.pathname === "/v1/auth/magic-link/signout") return json(response, 200, { ok: true });
  if (url.pathname === "/v1/agent-console/dashboard") {
    return json(response, 200, {
      owner: { id: "user-test", displayName: "Existing creator" },
      credits: { available: 5000 },
      projects: [],
    });
  }
  if (url.pathname === "/v1/billing/account") return json(response, 200, { credits: { available: 5000 } });
  if (url.pathname === "/v1/profiles") return json(response, 200, { profiles: [] });
  if (url.pathname === "/v1/bindings") return json(response, 200, { bindings: [] });
  if (url.pathname === "/v1/endpoints") return json(response, 200, { endpoints: [] });
  if (url.pathname === "/v1/api-keys") return json(response, 200, { apiKeys: [] });
  if (url.pathname === "/v1/connections") return json(response, 200, { connections: [] });
  if (url.pathname === "/v1/traces") return json(response, 200, { traces: [] });

  return json(response, 404, { error: { code: "NOT_FOUND", message: `No mock for ${url.pathname}` } });
}).listen(port, "127.0.0.1");

async function readJson(request) {
  const chunks = [];
  for await (const chunk of request) chunks.push(chunk);
  const raw = Buffer.concat(chunks).toString("utf8");
  if (!raw) return {};
  try {
    return JSON.parse(raw);
  } catch {
    return {};
  }
}

function json(response, status, body) {
  response.statusCode = status;
  response.setHeader("content-type", "application/json; charset=utf-8");
  response.end(JSON.stringify(body));
}
