import { readFile, writeFile } from "node:fs/promises";

const apiBaseUrl = (process.env.VIFU_E2E_API_URL || "http://127.0.0.1:6790").replace(/\/+$/, "");
const adminKey = process.env.VIFU_E2E_ADMIN_KEY || process.env.VIFU_ADMIN_KEY || "";
const statePath = process.env.VIFU_E2E_STATE_PATH || "/tmp/vifu-self-hosted-e2e.json";
const openClawMockUrl = process.env.VIFU_E2E_OPENCLAW_MOCK_URL?.replace(/\/+$/, "") || null;
const command = process.argv[2] || "setup";

if (command === "setup") await setup();
else if (command === "verify") await verify();
else if (command === "cleanup") await cleanup();
else throw new Error("Usage: node scripts/test-self-hosted-e2e.mjs [setup|verify|cleanup]");

async function setup() {
  assert(adminKey, "VIFU_E2E_ADMIN_KEY or VIFU_ADMIN_KEY is required");
  const suffix = Date.now().toString(36);
  const status = await request("/v1/status");
  assert(status.capabilities?.websocketRelay === true, "WebSocket relay capability is required");
  const connections = (await request("/v1/connections")).connections ?? [];
  const connection = connections.find((item) => item.status === "connected");
  assert(connection, "A connected Vifu connector is required");
  const agent = connection.agents?.find((item) => item.id === "guide-agent") ?? connection.agents?.[0];
  assert(agent?.id, "The connector did not report an agent");

  const profile = (await request("/v1/profiles", {
    method: "POST",
    body: {
      name: `E2E Guide ${suffix}`,
      slug: `e2e-guide-${suffix}`,
      instructions: "Reply through the selected local agent.",
    },
  })).profile;
  const binding = (await request("/v1/bindings", {
    method: "POST",
    body: {
      profileId: profile.id,
      provider: "openclaw",
      connectorId: connection.connectorId,
      agentId: agent.id,
      config: {},
    },
  })).binding;

  const endpoints = await Promise.all(Array.from({ length: 10 }, async (_, index) => {
    return (await request("/v1/endpoints", {
      method: "POST",
      body: {
        name: `E2E endpoint ${index + 1}`,
        slug: `e2e-${suffix}-${index + 1}`,
        profileId: profile.id,
        bindingId: binding.id,
        requestTimeoutMs: 10000,
      },
    })).endpoint;
  }));

  const createdKeys = await Promise.all(endpoints.map(async (endpoint, index) => {
    return (await request("/v1/api-keys", {
      method: "POST",
      body: { endpointId: endpoint.id, name: `E2E key ${index + 1}` },
    })).apiKey;
  }));

  const crossEndpoint = await rawRequest(
    `/v1/endpoints/${endpoints[1].id}/invoke`,
    { method: "POST", body: { message: "must be denied" } },
    createdKeys[0].key,
  );
  assert(crossEndpoint.status === 403, "An endpoint key authorized a different endpoint");

  const calls = await Promise.all(endpoints.map(async (endpoint, index) => {
    const message = `parallel call ${index + 1}`;
    const result = await request(
      `/v1/endpoints/${endpoint.id}/invoke`,
      { method: "POST", body: { message } },
      createdKeys[index].key,
    );
    assert(result.output?.agentId === agent.id, `Endpoint ${index + 1} used the wrong agent`);
    assert(
      typeof result.output?.reply === "string" && result.output.reply.includes(message),
      `Endpoint ${index + 1} returned the wrong reply`,
    );
    return result;
  }));

  let canceledRequest = false;
  if (openClawMockUrl) {
    await request(`/v1/endpoints/${endpoints[0].id}`, {
      method: "PATCH",
      body: { requestTimeoutMs: 500 },
    });
    const timeoutResponse = await rawRequest(
      `/v1/endpoints/${endpoints[0].id}/invoke`,
      { method: "POST", body: { message: "delay:2000" } },
      createdKeys[0].key,
    );
    assert(timeoutResponse.status === 504, `Expected timeout status 504, got ${timeoutResponse.status}`);
    await timeoutResponse.arrayBuffer();
    await new Promise((resolve) => setTimeout(resolve, 200));
    const metrics = await fetch(`${openClawMockUrl}/metrics`).then((response) => response.json());
    assert(metrics.canceledRequests >= 1, "Connector cancellation did not close the OpenClaw request");
    canceledRequest = true;
  }

  const traces = (await request("/v1/traces?limit=500")).traces ?? [];
  const requestIds = new Set(calls.map((call) => call.requestId));
  const completed = traces.filter((trace) => requestIds.has(trace.requestId) && trace.status === "completed");
  assert(completed.length === 10, `Expected 10 completed traces, found ${completed.length}`);
  if (openClawMockUrl) {
    assert(traces.some((trace) => trace.status === "timed_out"), "Timed-out trace was not persisted");
  }

  const state = {
    profileId: profile.id,
    bindingId: binding.id,
    endpointIds: endpoints.map((endpoint) => endpoint.id),
    connectorId: connection.connectorId,
    sessionId: connection.sessionId,
    requestIds: calls.map((call) => call.requestId),
  };
  await writeFile(statePath, `${JSON.stringify(state, null, 2)}\n`, { mode: 0o600 });
  console.log(JSON.stringify({
    status: "ok",
    connectorId: state.connectorId,
    sessionId: state.sessionId,
    endpoints: state.endpointIds.length,
    concurrentCalls: calls.length,
    completedTraces: completed.length,
    canceledRequest,
  }));
}

async function verify() {
  const state = JSON.parse(await readFile(statePath, "utf8"));
  const [profiles, bindings, endpoints, connections, traces] = await Promise.all([
    request("/v1/profiles"),
    request("/v1/bindings"),
    request("/v1/endpoints"),
    request("/v1/connections"),
    request("/v1/traces?limit=500"),
  ]);
  assert(profiles.profiles.some((item) => item.id === state.profileId), "Profile was not persisted");
  assert(bindings.bindings.some((item) => item.id === state.bindingId), "Binding was not persisted");
  assert(state.endpointIds.every((id) => endpoints.endpoints.some((item) => item.id === id)), "Endpoints were not persisted");
  assert(state.requestIds.every((id) => traces.traces.some((item) => item.requestId === id)), "Traces were not persisted");
  const resumed = connections.connections.find((item) => item.connectorId === state.connectorId && item.status === "connected");
  assert(resumed, "Connector did not reconnect");
  assert(resumed.sessionId === state.sessionId, "Connector did not resume its session");
  console.log(JSON.stringify({
    status: "ok",
    persistedEndpoints: state.endpointIds.length,
    persistedTraces: state.requestIds.length,
    connectorResumed: true,
  }));
}

async function cleanup() {
  const state = JSON.parse(await readFile(statePath, "utf8"));
  await Promise.all(state.endpointIds.map((id) => request(`/v1/endpoints/${id}`, { method: "DELETE" })));
  await request(`/v1/bindings/${state.bindingId}`, { method: "DELETE" });
  await request(`/v1/profiles/${state.profileId}`, { method: "DELETE" });
  console.log(JSON.stringify({ status: "ok", cleanedEndpoints: state.endpointIds.length }));
}

async function request(path, init = {}, credential = adminKey) {
  const response = await rawRequest(path, init, credential);
  const payload = await response.json().catch(() => null);
  if (!response.ok) {
    const message = payload?.error?.message || `HTTP ${response.status}`;
    throw new Error(`${init.method || "GET"} ${path}: ${message}`);
  }
  return payload ?? {};
}

function rawRequest(path, init = {}, credential = adminKey) {
  const headers = new Headers(init.headers);
  headers.set("accept", "application/json");
  headers.set("authorization", `Bearer ${credential}`);
  const body = init.body === undefined ? undefined : JSON.stringify(init.body);
  if (body !== undefined) headers.set("content-type", "application/json");
  return fetch(`${apiBaseUrl}${path}`, { ...init, headers, body });
}

function assert(condition, message) {
  if (!condition) throw new Error(message);
}
