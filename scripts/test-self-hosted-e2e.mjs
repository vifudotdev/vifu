import { readFile, writeFile } from "node:fs/promises";

const apiBaseUrl = (process.env.VIFU_E2E_API_URL || "http://127.0.0.1:6790").replace(/\/+$/, "");
const dashboardBaseUrl = (process.env.VIFU_E2E_DASHBOARD_URL || "http://127.0.0.1:6791").replace(/\/+$/, "");
const adminKey = process.env.VIFU_E2E_ADMIN_KEY || process.env.VIFU_ADMIN_KEY || "";
const authEmail = process.env.VIFU_E2E_AUTH_EMAIL || "admin@self-hosted.example";
const authPassword = process.env.VIFU_E2E_AUTH_PASSWORD || "correct horse battery staple";
const statePath = process.env.VIFU_E2E_STATE_PATH || "/tmp/vifu-self-hosted-e2e.json";
const openClawMockUrl = process.env.VIFU_E2E_OPENCLAW_MOCK_URL?.replace(/\/+$/, "") || null;
const command = process.argv[2] || "setup";
let runtimeCredential = adminKey;

if (command === "setup") await setup();
else if (command === "verify") await verify();
else if (command === "cleanup") await cleanup();
else throw new Error("Usage: node scripts/test-self-hosted-e2e.mjs [setup|verify|cleanup]");

async function setup() {
  assert(adminKey, "VIFU_E2E_ADMIN_KEY or VIFU_ADMIN_KEY is required");
  const suffix = Date.now().toString(36);
  const status = await request("/v1/status");
  assert(status.capabilities?.agentGateways === true, "Agent Gateway capability is required");
  assert(status.capabilities?.endpoints === true, "Endpoint capability is required");
  const signupProbe = await fetch(`${dashboardBaseUrl}/signup`, { redirect: "manual" });
  const signupHtml = signupProbe.status === 200 ? await signupProbe.text() : "";
  const signupOpen = signupProbe.status === 200 && signupHtml.includes("Create your account");
  const authPagePath = signupOpen ? "/signup" : "/login";
  const authPage = await fetch(`${dashboardBaseUrl}${authPagePath}`);
  const authHtml = await authPage.text();
  assert(authPage.ok && authHtml.includes(signupOpen ? "Create your account" : "Sign in"), "Dashboard auth page is unavailable");
  assert(!/self-hosted deployment|deployment administrator|postgresql database/i.test(authHtml), "Auth UI exposes deployment internals");

  let initialAuthResponse = signupOpen
    ? await dashboardForm("/api/auth/local/signup", {
      email: authEmail,
      password: authPassword,
      displayName: "Self-hosted Admin",
      returnTo: "/project",
    })
    : await dashboardForm("/api/auth/local/login", {
      email: authEmail,
      password: authPassword,
      returnTo: "/project",
    });
  if (initialAuthResponse.status === 303 && initialAuthResponse.headers.get("location")?.includes("auth_error")) {
    initialAuthResponse = await dashboardForm("/api/auth/local/login", {
      email: authEmail,
      password: authPassword,
      returnTo: "/project",
    });
  }
  assert(initialAuthResponse.status === 303, `Dashboard authentication returned HTTP ${initialAuthResponse.status}`);
  const signupCookie = sessionCookie(initialAuthResponse);
  assert(signupCookie.httpOnly, "Dashboard local session cookie is not HttpOnly");
  assert(signupCookie.sameSite === "lax", "Dashboard local session cookie must use SameSite=Lax");
  assert(!signupCookie.domain, "Dashboard local session cookie must be host-only");
  const dashboardAfterSignup = await fetch(`${dashboardBaseUrl}/project`, {
    headers: { cookie: `vifu_session=${signupCookie.cookieValue}` },
  });
  assert(dashboardAfterSignup.ok, "Dashboard did not accept the local session cookie");
  const dashboardHtml = await dashboardAfterSignup.text();
  assert(dashboardHtml.includes("Create your first project"), "Dashboard did not render after local signup");
  assert(!dashboardHtml.includes(adminKey), "Dashboard HTML exposed the bootstrap admin key");
  const getLogoutResponse = await fetch(`${dashboardBaseUrl}/auth/logout`, {
    headers: { cookie: `vifu_session=${signupCookie.cookieValue}` },
    redirect: "manual",
  });
  assert(getLogoutResponse.status === 405, "Dashboard logout accepted a GET request");
  const dashboardAfterGetLogout = await fetch(`${dashboardBaseUrl}/project`, {
    headers: { cookie: `vifu_session=${signupCookie.cookieValue}` },
  });
  assert(dashboardAfterGetLogout.ok, "Dashboard GET logout invalidated the web session");
  const logoutResponse = await fetch(`${dashboardBaseUrl}/auth/logout`, {
    method: "POST",
    headers: {
      cookie: `vifu_session=${signupCookie.cookieValue}`,
      origin: dashboardBaseUrl,
    },
    redirect: "manual",
  });
  assert(logoutResponse.status >= 300 && logoutResponse.status < 400, "Dashboard logout did not redirect");
  const invalidLogin = await dashboardForm("/api/auth/local/login", {
    email: authEmail,
    password: "incorrect password",
    returnTo: "/project",
  });
  assert(invalidLogin.status === 303 && invalidLogin.headers.get("location")?.includes("auth_error"), "An invalid password was accepted");
  const loginResponse = await dashboardForm("/api/auth/local/login", {
    email: authEmail,
    password: authPassword,
    returnTo: "/project",
  });
  assert(loginResponse.status === 303, `Dashboard login returned HTTP ${loginResponse.status}`);
  const loginCookie = sessionCookie(loginResponse);
  assert(loginCookie.backendToken, "Dashboard login did not set an opaque session cookie");
  runtimeCredential = adminKey;
  const authAfterSignup = await fetch(`${dashboardBaseUrl}/signup`, { redirect: "manual" });
  assert(authAfterSignup.status === 200, "Signup closed after first-account initialization");
  const secondSignup = await dashboardForm("/api/auth/local/signup", {
    email: `second-${suffix}@self-hosted.example`,
    password: authPassword,
    displayName: "Second Operator",
    returnTo: "/project",
  });
  assert(secondSignup.status === 303 && !secondSignup.headers.get("location")?.includes("auth_error"), "Open self-hosted signup rejected another account");

  const agentGateways = (await request("/v1/agent-gateways")).agentGateways ?? [];
  const agentGateway = agentGateways.find((item) => item.status === "connected");
  assert(agentGateway, "A connected Vifu Agent Gateway is required");
  const agent = agentGateway.agents?.find((item) => item.id === "guide-agent") ?? agentGateway.agents?.[0];
  assert(agent?.id, "The Agent Gateway did not report an agent");

  const project = (await request("/v1/projects", {
    method: "POST",
    body: {
      name: `E2E Project ${suffix}`,
      slug: `e2e-project-${suffix}`,
      gatewayId: agentGateway.gatewayId,
      agentIds: [agent.id],
    },
  })).project;
  const bindings = (await request("/v1/bindings")).bindings ?? [];
  const binding = bindings.find((item) => project.bindingIds.includes(item.id));
  assert(binding, "Project creation did not create a binding for the selected agent");
  const profiles = (await request("/v1/profiles")).profiles ?? [];
  const profile = profiles.find((item) => item.id === binding.profileId);
  assert(profile, "Project creation did not create a profile for the selected agent");

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
    assert(metrics.canceledRequests >= 1, "Agent Gateway cancellation did not close the OpenClaw request");
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
    projectId: project.id,
    projectSlug: project.slug,
    endpointIds: endpoints.map((endpoint) => endpoint.id),
    gatewayId: agentGateway.gatewayId,
    sessionId: agentGateway.sessionId,
    requestIds: calls.map((call) => call.requestId),
    authCookieValue: loginCookie.cookieValue,
  };
  await writeFile(statePath, `${JSON.stringify(state, null, 2)}\n`, { mode: 0o600 });
  console.log(JSON.stringify({
    status: "ok",
    gatewayId: state.gatewayId,
    sessionId: state.sessionId,
    endpoints: state.endpointIds.length,
    concurrentCalls: calls.length,
    completedTraces: completed.length,
    canceledRequest,
    localAuth: true,
  }));
}

async function verify() {
  const state = JSON.parse(await readFile(statePath, "utf8"));
  runtimeCredential = adminKey;
  const dashboardSession = await fetch(`${dashboardBaseUrl}/project`, {
    headers: { cookie: `vifu_session=${state.authCookieValue}` },
  });
  assert(dashboardSession.ok, "Dashboard web session did not survive restart");
  const [projects, profiles, bindings, endpoints, agentGateways, traces] = await Promise.all([
    request("/v1/projects"),
    request("/v1/profiles"),
    request("/v1/bindings"),
    request("/v1/endpoints"),
    request("/v1/agent-gateways"),
    request("/v1/traces?limit=500"),
  ]);
  assert(projects.projects.some((item) => item.id === state.projectId), "Project was not persisted");
  assert(profiles.profiles.some((item) => item.id === state.profileId), "Profile was not persisted");
  assert(bindings.bindings.some((item) => item.id === state.bindingId), "Binding was not persisted");
  assert(state.endpointIds.every((id) => endpoints.endpoints.some((item) => item.id === id)), "Endpoints were not persisted");
  assert(state.requestIds.every((id) => traces.traces.some((item) => item.requestId === id)), "Traces were not persisted");
  assert(traces.traces.some((item) => item.projectId === state.projectId), "Project endpoint traces were not persisted");
  const resumed = agentGateways.agentGateways.find((item) => item.gatewayId === state.gatewayId && item.status === "connected");
  assert(resumed, "Agent Gateway did not reconnect");
  assert(resumed.sessionId === state.sessionId, "Agent Gateway did not resume its session");
  console.log(JSON.stringify({
    status: "ok",
    persistedEndpoints: state.endpointIds.length,
    persistedTraces: state.requestIds.length,
    projectPersisted: true,
    agentGatewayResumed: true,
    authSessionPersisted: true,
  }));
}

async function cleanup() {
  const state = JSON.parse(await readFile(statePath, "utf8"));
  runtimeCredential = adminKey;
  await request(`/v1/projects/${state.projectId}`, { method: "DELETE" });
  await Promise.all(state.endpointIds.map((id) => request(`/v1/endpoints/${id}`, { method: "DELETE" })));
  await request(`/v1/bindings/${state.bindingId}`, { method: "DELETE" });
  await request(`/v1/profiles/${state.profileId}`, { method: "DELETE" });
  await fetch(`${dashboardBaseUrl}/auth/logout`, {
    method: "POST",
    headers: {
      cookie: `vifu_session=${state.authCookieValue}`,
      origin: dashboardBaseUrl,
    },
    redirect: "manual",
  });
  console.log(JSON.stringify({ status: "ok", cleanedEndpoints: state.endpointIds.length }));
}

async function request(path, init = {}, credential = runtimeCredential) {
  const response = await rawRequest(path, init, credential);
  const payload = await response.json().catch(() => null);
  if (!response.ok) {
    const message = payload?.error?.message || `HTTP ${response.status}`;
    throw new Error(`${init.method || "GET"} ${path}: ${message}`);
  }
  return payload ?? {};
}

function rawRequest(path, init = {}, credential = runtimeCredential) {
  const headers = new Headers(init.headers);
  headers.set("accept", "application/json");
  if (credential) headers.set("authorization", `Bearer ${credential}`);
  const body = init.body === undefined ? undefined : JSON.stringify(init.body);
  if (body !== undefined) headers.set("content-type", "application/json");
  return fetch(`${apiBaseUrl}${path}`, { ...init, headers, body });
}

function dashboardForm(path, values) {
  return fetch(`${dashboardBaseUrl}${path}`, {
    method: "POST",
    headers: {
      "content-type": "application/x-www-form-urlencoded",
      origin: dashboardBaseUrl,
    },
    body: new URLSearchParams(values),
    redirect: "manual",
  });
}

function sessionCookie(response) {
  const header = response.headers.get("set-cookie") || "";
  const cookieValue = decodeURIComponent(/(?:^|,\s*)vifu_session=([^;]+)/.exec(header)?.[1] || "");
  let backendToken = "";
  try {
    const stored = JSON.parse(Buffer.from(cookieValue, "base64url").toString("utf8"));
    if ((stored.adapter === "dashboard" || stored.adapter === "runtime") && typeof stored.token === "string") {
      backendToken = stored.token;
    }
  } catch {
    backendToken = "";
  }
  return {
    cookieValue,
    backendToken,
    httpOnly: /;\s*httponly(?:;|$)/i.test(header),
    sameSite: /;\s*samesite=([^;]+)/i.exec(header)?.[1]?.toLowerCase() || "",
    domain: /;\s*domain=([^;]+)/i.exec(header)?.[1] || "",
  };
}

function assert(condition, message) {
  if (!condition) throw new Error(message);
}
