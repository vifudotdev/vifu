import { readFile, writeFile } from "node:fs/promises";

const apiBaseUrl = (process.env.VIFU_E2E_API_URL || "http://127.0.0.1:6790").replace(/\/+$/, "");
const dashboardBaseUrl = (process.env.VIFU_E2E_DASHBOARD_URL || "http://127.0.0.1:6791").replace(/\/+$/, "");
const adminKey = process.env.VIFU_E2E_ADMIN_KEY || process.env.VIFU_ADMIN_KEY || "";
const statePath = process.env.VIFU_E2E_STATE_PATH || "/tmp/vifu-self-hosted-e2e.json";
const openClawMockUrl = process.env.VIFU_E2E_OPENCLAW_MOCK_URL?.replace(/\/+$/, "") || null;
const expectTimeout = process.env.VIFU_E2E_EXPECT_TIMEOUT === "1";
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
  assert(signupProbe.status === 404, "Dashboard still exposes the removed signup route");
  const authPage = await fetch(`${dashboardBaseUrl}/login`);
  const authHtml = await authPage.text();
  assert(authPage.ok && authHtml.includes("Connect to Vifu"), "Dashboard Admin Key page is unavailable");
  assert(
    !/email|sign up|create your account|postgresql database/i.test(authHtml),
    "Dashboard exposes removed account authentication",
  );
  const invalidLogin = await dashboardForm("/api/auth/admin-key", {
    adminKey: "incorrect-admin-key",
    returnTo: "/project",
  });
  assert(
    invalidLogin.status === 303 && invalidLogin.headers.get("location")?.includes("auth_error"),
    "An invalid Admin Key was accepted",
  );
  const initialAuthResponse = await dashboardForm("/api/auth/admin-key", {
    adminKey,
    returnTo: "/project",
  });
  assert(initialAuthResponse.status === 303, `Dashboard Admin Key authentication returned HTTP ${initialAuthResponse.status}`);
  assert(!initialAuthResponse.headers.get("location")?.includes("auth_error"), "Dashboard rejected the configured Admin Key");
  const initialCookie = sessionCookie(initialAuthResponse);
  assert(initialCookie.httpOnly, "Dashboard Admin Key session cookie is not HttpOnly");
  assert(initialCookie.sameSite === "lax", "Dashboard Admin Key session cookie must use SameSite=Lax");
  assert(!initialCookie.domain, "Dashboard Admin Key session cookie must be host-only");
  assert(!initialCookie.cookieValue.includes(adminKey), "Dashboard session cookie exposed the Admin Key");
  const dashboardAfterSignup = await fetch(`${dashboardBaseUrl}/project`, {
    headers: { cookie: `vifu_admin_session=${initialCookie.cookieValue}` },
  });
  assert(dashboardAfterSignup.ok, "Dashboard did not accept the Admin Key session cookie");
  const dashboardHtml = await dashboardAfterSignup.text();
  assert(dashboardHtml.includes("Create your first project"), "Dashboard did not render after Admin Key authentication");
  assert(!dashboardHtml.includes(adminKey), "Dashboard HTML exposed the bootstrap admin key");
  const getLogoutResponse = await fetch(`${dashboardBaseUrl}/auth/logout`, {
    headers: { cookie: `vifu_admin_session=${initialCookie.cookieValue}` },
    redirect: "manual",
  });
  assert(getLogoutResponse.status === 405, "Dashboard logout accepted a GET request");
  const dashboardAfterGetLogout = await fetch(`${dashboardBaseUrl}/project`, {
    headers: { cookie: `vifu_admin_session=${initialCookie.cookieValue}` },
  });
  assert(dashboardAfterGetLogout.ok, "Dashboard GET logout invalidated the Admin Key session");
  const logoutResponse = await fetch(`${dashboardBaseUrl}/auth/logout`, {
    method: "POST",
    headers: {
      cookie: `vifu_admin_session=${initialCookie.cookieValue}`,
      origin: dashboardBaseUrl,
    },
    redirect: "manual",
  });
  assert(logoutResponse.status >= 300 && logoutResponse.status < 400, "Dashboard logout did not redirect");
  const loginResponse = await dashboardForm("/api/auth/admin-key", {
    adminKey,
    returnTo: "/project",
  });
  assert(loginResponse.status === 303, `Dashboard Admin Key login returned HTTP ${loginResponse.status}`);
  const loginCookie = sessionCookie(loginResponse);
  assert(loginCookie.cookieValue, "Dashboard login did not set a signed session cookie");
  runtimeCredential = adminKey;

  const agentGateways = (await request("/v1/agent-gateways")).agentGateways ?? [];
  const expectedMockAgent = openClawMockUrl ? "guide-agent" : null;
  const agentGateway = expectedMockAgent
    ? agentGateways.find((item) =>
      item.status === "connected"
        && item.agents?.some((agent) => agent.id === expectedMockAgent)
    )
    : agentGateways.find((item) => item.status === "connected");
  assert(agentGateway, expectedMockAgent
    ? `A connected Vifu Agent Gateway exposing ${expectedMockAgent} is required`
    : "A connected Vifu Agent Gateway is required");
  const agent = expectedMockAgent
    ? agentGateway.agents?.find((item) => item.id === expectedMockAgent)
    : agentGateway.agents?.[0];
  assert(agent?.id, "The Agent Gateway did not report an agent");
  const secondaryAgent = agentGateway.agents?.find((item) => item.id && item.id !== agent.id) ?? null;
  const projectAgentIds = secondaryAgent ? [agent.id, secondaryAgent.id] : [agent.id];
  const providerKey = agent.metadata?.providerKey;
  assert(providerKey, "The Agent Gateway did not identify the source provider");
  const providerCatalog = await request("/v1/provider-catalog");
  assert(
    providerCatalog.custom?.some((provider) => provider.providerKey === providerKey),
    "The Agent Gateway provider is missing from the provider catalog",
  );

  const project = await createProjectWithProvider({
    name: `E2E Project ${suffix}`,
    slug: `e2e-project-${suffix}`,
    gatewayId: agentGateway.gatewayId,
    providerKey,
    minimumAgents: projectAgentIds.length,
  });
  const scopeTargetProject = await createProjectWithProvider({
    name: `E2E Scope Target ${suffix}`,
    slug: `e2e-scope-target-${suffix}`,
    gatewayId: agentGateway.gatewayId,
    providerKey,
    minimumAgents: 1,
  });
  const bindings = (await request("/v1/bindings")).bindings ?? [];
  const binding = bindings.find((item) => project.bindingIds.includes(item.id) && item.agentId === agent.id);
  assert(binding, "Project creation did not create a binding for the selected agent");
  const secondaryBinding = secondaryAgent
    ? bindings.find((item) => project.bindingIds.includes(item.id) && item.agentId === secondaryAgent.id)
    : null;
  if (secondaryAgent) assert(secondaryBinding, "Project creation did not bind the second agent");
  const profiles = (await request("/v1/profiles")).profiles ?? [];
  const profile = profiles.find((item) => item.id === binding.profileId);
  assert(profile, "Project creation did not create a profile for the selected agent");
  const secondaryProfile = secondaryBinding
    ? profiles.find((item) => item.id === secondaryBinding.profileId)
    : null;
  if (secondaryBinding) assert(secondaryProfile, "Project creation did not create the second agent profile");

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
  const secondaryEndpoint = secondaryBinding && secondaryProfile
    ? (await request("/v1/endpoints", {
      method: "POST",
      body: {
        name: "E2E secondary endpoint",
        slug: `e2e-${suffix}-secondary`,
        profileId: secondaryProfile.id,
        bindingId: secondaryBinding.id,
        requestTimeoutMs: 10000,
      },
    })).endpoint
    : null;
  const allEndpoints = secondaryEndpoint ? [...endpoints, secondaryEndpoint] : endpoints;

  const projectKey = (await request("/v1/api-keys", {
    method: "POST",
    body: projectKeyBody(project.id, "E2E project key"),
  })).apiKey;

  const editableKey = (await request("/v1/api-keys", {
    method: "POST",
    body: projectKeyBody(project.id, "E2E editable key"),
  })).apiKey;
  const selectedScopeUpdate = (await request(`/v1/api-keys/${editableKey.id}`, {
    method: "PATCH",
    body: {
      name: "E2E edited key",
      agentScope: { mode: "selected", profileIds: [profile.id] },
      permissions: apiKeyPermissions({
        chatCompletions: "access",
        agents: "read",
        project: "write",
      }),
    },
  })).apiKey;
  assert(selectedScopeUpdate.name === "E2E edited key", "API key name update was not persisted");
  assert(
    selectedScopeUpdate.agentScope?.mode === "selected"
      && selectedScopeUpdate.agentScope.profileIds?.[0] === profile.id,
    "Selected agent scope was not persisted",
  );
  assert(
    selectedScopeUpdate.permissions?.chatCompletions === "access"
      && selectedScopeUpdate.permissions.agents === "read"
      && selectedScopeUpdate.permissions.project === "write",
    "API key permissions were not persisted",
  );
  const selectedModels = await request(`/${project.slug}/v1/models`, {}, editableKey.key);
  assert(
    selectedModels.data?.length === 1 && selectedModels.data[0]?.id === profile.slug,
    "Selected agent scope returned a model from another profile",
  );
  if (secondaryEndpoint) {
    const selectedDenied = await rawRequest(
      `/${project.slug}/v1/chat/completions`,
      { method: "POST", body: chatCompletionBody(secondaryEndpoint, "selected scope probe") },
      editableKey.key,
    );
    const selectedDeniedPayload = await selectedDenied.json();
    assert(
      selectedDenied.status === 403 && selectedDeniedPayload?.error?.code === "agent_access_denied",
      "Selected agent scope allowed another project binding",
    );
  }

  const scopeUpdate = (await request(`/v1/api-keys/${editableKey.id}`, {
    method: "PATCH",
    body: {
      projectId: scopeTargetProject.id,
      agentScope: { mode: "all" },
    },
  })).apiKey;
  assert(scopeUpdate.projectId === scopeTargetProject.id, "API key project scope update was not persisted");
  const formerScopeAccess = await rawRequest(`/${project.slug}/v1/models`, {}, editableKey.key);
  assert(formerScopeAccess.status === 403, "API key retained access to its former project scope");
  await formerScopeAccess.arrayBuffer();
  const targetScopeModels = await request(`/${scopeTargetProject.slug}/v1/models`, {}, editableKey.key);
  assert(Array.isArray(targetScopeModels.data), "API key could not access its updated project scope");

  const endpointDeniedKey = (await request("/v1/api-keys", {
    method: "POST",
    body: {
      ...projectKeyBody(project.id, "E2E endpoint permission probe"),
      permissions: apiKeyPermissions({
        chatCompletions: "none",
        agents: "read",
        project: "read",
      }),
    },
  })).apiKey;
  assert(
    endpointDeniedKey.permissions?.chatCompletions === "none"
      && endpointDeniedKey.permissions.agents === "read"
      && endpointDeniedKey.permissions.project === "read",
    "API key creation did not persist endpoint permissions",
  );
  const deniedModels = await rawRequest(`/${project.slug}/v1/models`, {}, endpointDeniedKey.key);
  const deniedModelsPayload = await deniedModels.json();
  assert(
    deniedModels.status === 403 && deniedModelsPayload?.error?.code === "endpoint_access_denied",
    "A key without Chat Completions access listed endpoint models",
  );
  const deniedCompletion = await rawRequest(
    `/${project.slug}/v1/chat/completions`,
    { method: "POST", body: chatCompletionBody(endpoints[0], "endpoint permission probe") },
    endpointDeniedKey.key,
  );
  const deniedCompletionPayload = await deniedCompletion.json();
  assert(
    deniedCompletion.status === 403 && deniedCompletionPayload?.error?.code === "endpoint_access_denied",
    "A key without Chat Completions access invoked the endpoint",
  );
  const endpointPermissionUpdate = (await request(`/v1/api-keys/${endpointDeniedKey.id}`, {
    method: "PATCH",
    body: {
      permissions: apiKeyPermissions({
        chatCompletions: "access",
        agents: "none",
        project: "none",
      }),
    },
  })).apiKey;
  assert(
    endpointPermissionUpdate.permissions?.chatCompletions === "access",
    "API key endpoint permission update was not persisted",
  );
  const allowedModels = await request(`/${project.slug}/v1/models`, {}, endpointDeniedKey.key);
  assert(Array.isArray(allowedModels.data), "Updated endpoint permission did not authorize model discovery");

  const disposableKey = (await request("/v1/api-keys", {
    method: "POST",
    body: projectKeyBody(project.id, "E2E revocation probe"),
  })).apiKey;
  const activeDelete = await rawRequest(`/v1/api-keys/${disposableKey.id}`, { method: "DELETE" });
  assert(activeDelete.status === 409, "An active API key was permanently deleted without revocation");
  await activeDelete.arrayBuffer();
  await request(`/v1/api-keys/${disposableKey.id}/revoke`, { method: "POST" });
  const revokedUpdate = await rawRequest(`/v1/api-keys/${disposableKey.id}`, {
    method: "PATCH",
    body: { agentScope: { mode: "all" } },
  });
  assert(revokedUpdate.status === 409, "A revoked API key could still be edited");
  await revokedUpdate.arrayBuffer();
  const revokedAccess = await rawRequest(`/${project.slug}/v1/models`, {}, disposableKey.key);
  assert(revokedAccess.status === 403, "A revoked API key still authorized requests");
  await revokedAccess.arrayBuffer();
  await request(`/v1/api-keys/${disposableKey.id}`, { method: "DELETE" });
  const keysAfterDelete = (await request("/v1/api-keys")).apiKeys ?? [];
  assert(!keysAfterDelete.some((key) => key.id === disposableKey.id), "A deleted API key record remained visible");

  const visibleModels = await request(`/${project.slug}/v1/models`, {}, projectKey.key);
  const visibleModelIds = new Set(visibleModels.data?.map((model) => model.id) ?? []);
  const expectedModelIds = [profile.slug, secondaryProfile?.slug].filter(Boolean);
  assert(
    visibleModelIds.size === expectedModelIds.length
      && expectedModelIds.every((modelId) => visibleModelIds.has(modelId)),
    "Project key did not list all project models",
  );

  const missingModel = await rawRequest(
    `/${project.slug}/v1/chat/completions`,
    { method: "POST", body: { messages: [{ role: "user", content: "Hello" }] } },
    projectKey.key,
  );
  const missingModelPayload = await missingModel.json();
  assert(
    missingModel.status === 400 && missingModelPayload?.error?.code === "model_required",
    "Project key without model did not return model_required",
  );

  const outsideProject = await rawRequest(
    `/${project.slug}/v1/chat/completions`,
    { method: "POST", body: { model: "outside-project-agent", messages: [{ role: "user", content: "Hello" }] } },
    projectKey.key,
  );
  const outsideProjectPayload = await outsideProject.json();
  assert(
    outsideProject.status === 403 && outsideProjectPayload?.error?.code === "agent_access_denied",
    "Project key targeting an outside model did not return agent_access_denied",
  );

  const calls = await Promise.all(Array.from({ length: 10 }, async (_, index) => {
    const message = `parallel call ${index + 1}`;
    const result = await request(
      `/${project.slug}/v1/chat/completions`,
      { method: "POST", body: chatCompletionBody(profile, message) },
      projectKey.key,
    );
    assert(result.model === profile.slug, `Concurrent call ${index + 1} returned the wrong model`);
    assert(
      completionContent(result).includes(message),
      `Concurrent call ${index + 1} returned the wrong reply`,
    );
    return result;
  }));

  let canceledRequest = false;
  if (openClawMockUrl && expectTimeout) {
    const timeoutResponse = await rawRequest(
      `/${project.slug}/v1/chat/completions`,
      { method: "POST", body: chatCompletionBody(profile, "delay:2000") },
      projectKey.key,
    );
    assert(timeoutResponse.status === 504, `Expected timeout status 504, got ${timeoutResponse.status}`);
    await timeoutResponse.arrayBuffer();
    await new Promise((resolve) => setTimeout(resolve, 200));
    const metrics = await fetch(`${openClawMockUrl}/metrics`).then((response) => response.json());
    assert(metrics.canceledRequests >= 1, "Agent Gateway cancellation did not close the OpenClaw request");
    canceledRequest = true;
  }

  const traces = (await request("/v1/traces?limit=500")).traces ?? [];
  const requestIds = new Set(calls.map(completionRequestId));
  const completed = traces.filter((trace) => requestIds.has(trace.requestId) && trace.status === "completed");
  assert(completed.length === 10, `Expected 10 completed traces, found ${completed.length}`);
  assert(
    completed.every((trace) => trace.projectId === project.id && trace.profileId === profile.id),
    "Project invocation traces lost their project or profile attribution",
  );
  if (openClawMockUrl && expectTimeout) {
    assert(traces.some((trace) => trace.status === "timed_out"), "Timed-out trace was not persisted");
  }

  const state = {
    projectId: project.id,
    projectSlug: project.slug,
    scopeTargetProjectId: scopeTargetProject.id,
    endpointIds: allEndpoints.map((endpoint) => endpoint.id),
    bindingIds: [binding.id, secondaryBinding?.id].filter(Boolean),
    profileIds: [profile.id, secondaryProfile?.id].filter(Boolean),
    gatewayId: agentGateway.gatewayId,
    sessionId: agentGateway.sessionId,
    requestIds: calls.map(completionRequestId),
    authCookieValue: loginCookie.cookieValue,
  };
  await writeFile(statePath, `${JSON.stringify(state, null, 2)}\n`, { mode: 0o600 });
  console.log(JSON.stringify({
    status: "ok",
    gatewayId: state.gatewayId,
    sessionId: state.sessionId,
    endpoints: allEndpoints.length,
    concurrentCalls: calls.length,
    completedTraces: completed.length,
    canceledRequest,
    adminKeyAccess: true,
  }));
}

async function verify() {
  const state = JSON.parse(await readFile(statePath, "utf8"));
  runtimeCredential = adminKey;
  const dashboardSession = await fetch(`${dashboardBaseUrl}/project`, {
    headers: { cookie: `vifu_admin_session=${state.authCookieValue}` },
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
  assert(projects.projects.some((item) => item.id === state.scopeTargetProjectId), "API key target project was not persisted");
  assert(state.profileIds.every((id) => profiles.profiles.some((item) => item.id === id)), "Profiles were not persisted");
  assert(state.bindingIds.every((id) => bindings.bindings.some((item) => item.id === id)), "Bindings were not persisted");
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
  await Promise.all(state.endpointIds.map((id) => request(`/v1/endpoints/${id}`, { method: "DELETE" })));
  await request(`/v1/projects/${state.projectId}`, { method: "DELETE" });
  await request(`/v1/projects/${state.scopeTargetProjectId}`, { method: "DELETE" });
  await fetch(`${dashboardBaseUrl}/auth/logout`, {
    method: "POST",
    headers: {
      cookie: `vifu_admin_session=${state.authCookieValue}`,
      origin: dashboardBaseUrl,
    },
    redirect: "manual",
  });
  console.log(JSON.stringify({ status: "ok", cleanedEndpoints: state.endpointIds.length }));
}

async function createProjectWithProvider({ name, slug, gatewayId, providerKey, minimumAgents }) {
  const created = (await request("/v1/projects", {
    method: "POST",
    body: { name, slug },
  })).project;
  await request(`/v1/projects/${created.id}`, {
    method: "PATCH",
    body: { gatewayId },
  });
  const provider = await request(`/v1/project/${slug}/providers`, {
    method: "POST",
    body: {
      source: { kind: "custom", key: providerKey },
    },
  });
  assert(
    provider.addedAgents >= minimumAgents,
    `Provider setup added ${provider.addedAgents ?? 0} agents; expected at least ${minimumAgents}`,
  );
  return (await request(`/v1/projects/${created.id}`)).project;
}

async function request(path, init = {}, credential = runtimeCredential) {
  const response = await rawRequest(path, init, credential);
  const responseBody = await response.text();
  let payload = null;
  try {
    payload = responseBody ? JSON.parse(responseBody) : null;
  } catch {
    // Keep the raw body below so non-JSON proxy and framework errors remain actionable.
  }
  if (!response.ok) {
    const message = payload?.error?.message || responseBody || `HTTP ${response.status}`;
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

function chatCompletionBody(endpoint, message) {
  return {
    model: endpoint.slug,
    messages: [{ role: "user", content: message }],
    stream: false,
  };
}

function projectKeyBody(projectId, name) {
  return {
    projectId,
    name,
    agentScope: { mode: "all" },
    permissions: apiKeyPermissions(),
  };
}

function apiKeyPermissions(overrides = {}) {
  return {
    chatCompletions: "access",
    speech: "none",
    transcriptions: "none",
    realtime: "none",
    runtime: "none",
    agents: "none",
    project: "none",
    ...overrides,
  };
}

function completionContent(result) {
  const content = result?.choices?.[0]?.message?.content;
  return typeof content === "string" ? content : "";
}

function completionRequestId(result) {
  const id = typeof result?.id === "string" ? result.id : "";
  assert(id.startsWith("chatcmpl-"), "Chat completion id did not include the Vifu request id");
  return id.slice("chatcmpl-".length);
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
  const cookieValue = decodeURIComponent(/(?:^|,\s*)vifu_admin_session=([^;]+)/.exec(header)?.[1] || "");
  return {
    cookieValue,
    httpOnly: /;\s*httponly(?:;|$)/i.test(header),
    sameSite: /;\s*samesite=([^;]+)/i.exec(header)?.[1]?.toLowerCase() || "",
    domain: /;\s*domain=([^;]+)/i.exec(header)?.[1] || "",
  };
}

function assert(condition, message) {
  if (!condition) throw new Error(message);
}
