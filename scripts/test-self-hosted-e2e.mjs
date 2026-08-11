import { readFile, writeFile } from "node:fs/promises";

const apiBaseUrl = (process.env.VIFU_E2E_API_URL || "http://127.0.0.1:6790").replace(/\/+$/, "");
const dashboardBaseUrl = (process.env.VIFU_E2E_DASHBOARD_URL || apiBaseUrl).replace(/\/+$/, "");
const adminKey = process.env.VIFU_E2E_ADMIN_KEY || process.env.VIFU_ADMIN_KEY || "";
const statePath = process.env.VIFU_E2E_STATE_PATH || "/tmp/vifu-self-hosted-e2e.json";
const openClawMockUrl = process.env.VIFU_E2E_OPENCLAW_MOCK_URL?.replace(/\/+$/, "") || null;
const openAiMockUrl = process.env.VIFU_E2E_OPENAI_MOCK_URL?.replace(/\/+$/, "") || null;
const openAiProviderBaseUrl = process.env.VIFU_E2E_OPENAI_PROVIDER_BASE_URL?.replace(/\/+$/, "") || null;
const openAiProviderToken = process.env.VIFU_E2E_OPENAI_PROVIDER_TOKEN || "";
const expectTimeout = process.env.VIFU_E2E_EXPECT_TIMEOUT === "1";
const command = process.argv[2] || "setup";
let runtimeCredential = adminKey;

if (command === "setup") await setup();
else if (command === "pairing") await pairing();
else if (command === "pairing-restart") await pairingRestart();
else if (command === "pairing-revoke") await pairingRevoke();
else if (command === "verify") await verify();
else if (command === "cleanup") await cleanup();
else throw new Error("Usage: node scripts/test-self-hosted-e2e.mjs [setup|pairing|pairing-restart|pairing-revoke|verify|cleanup]");

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
  assert(dashboardHtml.includes("Create your first app"), "Dashboard did not render after Admin Key authentication");
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
  const providerKey = agent.metadata?.providerKey;
  assert(providerKey, "The Agent Gateway did not identify the source provider");
  const secondaryAgent = agentGateway.agents?.find((item) =>
    item.id
      && item.id !== agent.id
      && item.metadata?.providerKey === providerKey
  ) ?? null;
  const projectAgentIds = secondaryAgent ? [agent.id, secondaryAgent.id] : [agent.id];
  const providerCatalog = await request("/v1/provider-catalog");
  assert(
    providerCatalog.custom?.some((provider) => provider.providerKey === providerKey),
    "The Agent Gateway provider is missing from the provider catalog",
  );
  if (openAiProviderBaseUrl) {
    assert(
      providerCatalog.custom?.some((provider) =>
        provider.providerKey === "openai-compatible-e2e"
          && provider.providerType === "vifu-runtime"
          && provider.config?.localProviderType === "openai-compatible"
          && provider.config?.capabilities?.includes("chat")
          && provider.config?.capabilities?.includes("embedding")
      ),
      "The providers.json OpenAI-compatible provider is missing from the provider catalog",
    );
  }

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
    body: projectKeyBody(project.id, "E2E project key", { transcriptions: "access" }),
  })).apiKey;
  const providerWorkflow = openAiProviderBaseUrl
    ? await exerciseOpenAiCompatibleProjectFlow({ project, projectKey, suffix })
    : null;

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
  const expectedModelIds = [
    profile.slug,
    secondaryProfile?.slug,
    ...(providerWorkflow?.modelSlugs ?? []),
  ].filter(Boolean);
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
    profileIds: [
      profile.id,
      secondaryProfile?.id,
      ...(providerWorkflow?.profileIds ?? []),
    ].filter(Boolean),
    gatewayId: agentGateway.gatewayId,
    sessionId: agentGateway.sessionId,
    requestIds: [
      ...calls.map(completionRequestId),
      ...(providerWorkflow?.requestIds ?? []),
    ],
    openAiProviderKey: providerWorkflow?.projectProviderKey ?? null,
    openAiAvailableProviderKey: providerWorkflow?.availableProviderKey ?? null,
    openAiProfileSlug: providerWorkflow?.directProfileSlug ?? null,
    openAiModelSlugs: providerWorkflow?.modelSlugs ?? [],
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
  const [apps, profiles, bindings, endpoints, agentGateways, traces] = await Promise.all([
    request("/v1/apps"),
    request("/v1/profiles"),
    request("/v1/bindings"),
    request("/v1/endpoints"),
    request("/v1/agent-gateways"),
    request("/v1/traces?limit=500"),
  ]);
  assert(apps.apps.some((item) => item.id === state.projectId), "App was not persisted");
  assert(apps.apps.some((item) => item.id === state.scopeTargetProjectId), "API key target App was not persisted");
  assert(state.profileIds.every((id) => profiles.profiles.some((item) => item.id === id)), "Profiles were not persisted");
  assert(state.bindingIds.every((id) => bindings.bindings.some((item) => item.id === id)), "Bindings were not persisted");
  assert(state.endpointIds.every((id) => endpoints.endpoints.some((item) => item.id === id)), "Endpoints were not persisted");
  assert(state.requestIds.every((id) => traces.traces.some((item) => item.requestId === id)), "Traces were not persisted");
  assert(traces.traces.some((item) => item.projectId === state.projectId), "Project endpoint traces were not persisted");
  if (state.openAiProviderKey) {
    const providers = (await request(`/v1/apps/${state.projectSlug}/providers`)).providers ?? [];
    assert(
      providers.some((item) => item.providerKey === state.openAiProviderKey && item.status === "online"),
      "Project OpenAI-compatible provider was not persisted as online",
    );
    assert(
      providers.some((item) => item.providerKey === state.openAiAvailableProviderKey && item.sourceKind === "custom"),
      "Attached providers.json provider was not persisted",
    );
    assert(
      state.openAiModelSlugs.every((slug) => profiles.profiles.some((item) => item.slug === slug)),
      "OpenAI-compatible provider profiles were not persisted",
    );
    assert(
      traces.traces.some((item) => item.projectId === state.projectId && item.providerKey === state.openAiProviderKey),
      "OpenAI-compatible provider traces were not persisted",
    );
  }
  const resumed = agentGateways.agentGateways.find((item) => item.gatewayId === state.gatewayId && item.status === "connected");
  assert(resumed, "Agent Gateway did not reconnect");
  assert(resumed.sessionId === state.sessionId, "Agent Gateway did not resume its session");
  if (state.pairingGatewayId) {
    const paired = await waitFor("the paired Agent Gateway to reconnect after deployment restart", async () =>
      findConnectedGateway({ gatewayId: state.pairingGatewayId })
    );
    assert(paired, "Paired Agent Gateway did not reconnect after deployment restart");
  }
  console.log(JSON.stringify({
    status: "ok",
    persistedEndpoints: state.endpointIds.length,
    persistedTraces: state.requestIds.length,
    projectPersisted: true,
    agentGatewayResumed: true,
    authSessionPersisted: true,
  }));
}

async function pairing() {
  const state = JSON.parse(await readFile(statePath, "utf8"));
  runtimeCredential = adminKey;
  const pending = await waitFor("a pending Agent Gateway pairing request", async () => {
    const pairings = (await request("/v1/agent-gateway-pairings")).pairings ?? [];
    return pairings.find((item) => item.status === "pending");
  });
  assert(pending.machineId, "Pairing request did not include a machine id");
  const pairPage = await fetch(`${dashboardBaseUrl}/pair?request=${encodeURIComponent(pending.id)}`, {
    headers: { cookie: `vifu_admin_session=${state.authCookieValue}` },
  });
  const pairHtml = await pairPage.text();
  assert(pairPage.ok && pairHtml.includes("Authorize this device"), "Dashboard pairing page did not render");

  const approved = (await request(`/v1/agent-gateway-pairings/${pending.id}/approve`, { method: "POST" })).pairing;
  assert(approved.status === "approved", "Pairing request was not approved");
  const consumed = await waitFor("the approved Agent Gateway pairing request to be consumed", async () => {
    const pairing = (await request(`/v1/agent-gateway-pairings/${pending.id}`)).pairing;
    return pairing.status === "consumed" ? pairing : null;
  });
  const pairedGateway = await waitFor("the paired Agent Gateway to connect", async () =>
    findConnectedGateway({ excludeGatewayId: state.gatewayId })
  );
  assert(
    pairedGateway.agents?.some((agent) => agent.id === "guide-agent"),
    "Paired Agent Gateway did not expose the OpenClaw guide agent",
  );

  await writeFile(statePath, `${JSON.stringify({
    ...state,
    pairingRequestId: consumed.id,
    pairingMachineId: pending.machineId,
    pairingGatewayId: pairedGateway.gatewayId,
    pairingSessionId: pairedGateway.sessionId,
  }, null, 2)}\n`, { mode: 0o600 });
  console.log(JSON.stringify({
    status: "ok",
    pairingRequestId: consumed.id,
    pairingGatewayId: pairedGateway.gatewayId,
    pairingConsumed: true,
  }));
}

async function pairingRestart() {
  const state = JSON.parse(await readFile(statePath, "utf8"));
  runtimeCredential = adminKey;
  assert(state.pairingGatewayId && state.pairingMachineId, "Pairing state is missing");
  const pairedGateway = await waitFor("the paired Agent Gateway to reconnect after restart", async () =>
    findConnectedGateway({ gatewayId: state.pairingGatewayId })
  );
  assert(
    pairedGateway.sessionId === state.pairingSessionId,
    "Paired Agent Gateway did not resume its stored session after restart",
  );
  const pairings = (await request("/v1/agent-gateway-pairings")).pairings ?? [];
  assert(
    !pairings.some((item) => item.machineId === state.pairingMachineId && item.status === "pending"),
    "Paired Agent Gateway requested a new pairing after restart",
  );
  console.log(JSON.stringify({
    status: "ok",
    pairingGatewayId: pairedGateway.gatewayId,
    pairingRestartResumed: true,
  }));
}

async function pairingRevoke() {
  const state = JSON.parse(await readFile(statePath, "utf8"));
  runtimeCredential = adminKey;
  assert(state.pairingGatewayId && state.pairingMachineId, "Pairing state is missing");
  await request(`/v1/agent-gateways/${state.pairingGatewayId}/revoke`, { method: "POST" });
  const pending = await waitFor("a new pending Agent Gateway pairing request after revocation", async () => {
    const pairings = (await request("/v1/agent-gateway-pairings")).pairings ?? [];
    return pairings.find((item) =>
      item.machineId === state.pairingMachineId
        && item.status === "pending"
        && item.id !== state.pairingRequestId
    );
  });
  assert(pending.id, "Revoked Agent Gateway did not create a new pairing request");
  await request(`/v1/agent-gateway-pairings/${pending.id}/approve`, { method: "POST" });
  await waitFor("the post-revocation pairing request to be consumed", async () => {
    const pairing = (await request(`/v1/agent-gateway-pairings/${pending.id}`)).pairing;
    return pairing.status === "consumed" ? pairing : null;
  });
  const reauthorized = await waitFor("the revoked Agent Gateway to reconnect after approval", async () =>
    findConnectedGateway({ gatewayId: state.pairingGatewayId })
  );
  await writeFile(statePath, `${JSON.stringify({
    ...state,
    pairingRevocationRequestId: pending.id,
    pairingSessionIdAfterRevoke: reauthorized.sessionId,
  }, null, 2)}\n`, { mode: 0o600 });
  console.log(JSON.stringify({
    status: "ok",
    pairingGatewayId: reauthorized.gatewayId,
    pairingRevokedAndReapproved: true,
  }));
}

async function cleanup() {
  const state = JSON.parse(await readFile(statePath, "utf8"));
  runtimeCredential = adminKey;
  await Promise.all(state.endpointIds.map((id) => request(`/v1/endpoints/${id}`, { method: "DELETE" })));
  await request(`/v1/apps/${state.projectId}`, { method: "DELETE" });
  await request(`/v1/apps/${state.scopeTargetProjectId}`, { method: "DELETE" });
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
  const created = (await request("/v1/apps", {
    method: "POST",
    body: { name, slug },
  })).app;
  await request(`/v1/apps/${created.id}`, {
    method: "PATCH",
    body: { gatewayId },
  });
  const provider = await request(`/v1/apps/${slug}/providers`, {
    method: "POST",
    body: {
      source: { kind: "custom", key: providerKey },
    },
  });
  assert(
    provider.addedAgents >= minimumAgents,
    `Provider setup added ${provider.addedAgents ?? 0} agents; expected at least ${minimumAgents}`,
  );
  return (await request(`/v1/apps/${created.id}`)).app;
}

async function exerciseOpenAiCompatibleProjectFlow({ project, projectKey, suffix }) {
  assert(openAiProviderBaseUrl, "VIFU_E2E_OPENAI_PROVIDER_BASE_URL is required for OpenAI-compatible E2E");
  const projectCatalog = await request(`/v1/apps/${project.slug}/provider-catalog`);
  const availableOpenAi = projectCatalog.custom?.find((provider) => provider.providerKey === "openai-compatible-e2e");
  const availableOpenAiAlt = projectCatalog.custom?.find((provider) => provider.providerKey === "openai-compatible-e2e-alt");
  assert(availableOpenAi, "Project provider catalog did not include the providers.json OpenAI-compatible provider");
  assert(availableOpenAiAlt, "Project provider catalog did not include the second providers.json OpenAI-compatible provider");
  assert(availableOpenAi.providerType === "vifu-runtime", "providers.json OpenAI-compatible provider must be exposed as vifu-runtime");
  assert(
    availableOpenAi.config?.localProviderType === "openai-compatible",
    "providers.json OpenAI-compatible provider lost its local provider type",
  );
  assert(
    availableOpenAi.config?.capabilities?.includes("chat")
      && availableOpenAi.config?.capabilities?.includes("embedding"),
    "providers.json OpenAI-compatible provider did not report chat and embedding capabilities",
  );

  const profilesBeforeAttach = new Set(((await request(`/v1/apps/${project.slug}/profiles`)).profiles ?? [])
    .map((profile) => profile.id));
  const attachedAvailable = await request(`/v1/apps/${project.slug}/providers`, {
    method: "POST",
    body: {
      source: { kind: "custom", key: "openai-compatible-e2e" },
    },
  });
  assert(attachedAvailable.provider?.sourceKind === "custom", "Available provider attach did not preserve sourceKind=custom");
  assert(attachedAvailable.provider?.sourceKey === "openai-compatible-e2e", "Available provider attach used the wrong source key");
  assert(attachedAvailable.provider?.status === "online", "Available providers.json provider was not online after attach");
  assert(attachedAvailable.addedAgents >= 1, "Available providers.json provider did not add its discovered agent");
  const profilesAfterAttach = ((await request(`/v1/apps/${project.slug}/profiles`)).profiles ?? [])
    .filter((profile) => !profilesBeforeAttach.has(profile.id));

  const offlineProvider = await request(`/v1/apps/${project.slug}/providers`, {
    method: "POST",
    body: {
      source: { kind: "registry", key: "openai-compatible" },
      name: "E2E Offline OpenAI-compatible Provider",
      baseUrl: "http://127.0.0.1:1/v1",
      config: {},
      secrets: {},
    },
  });
  assert(offlineProvider.provider?.status === "offline", "Unreachable OpenAI-compatible provider was not marked offline");
  assert(offlineProvider.message, "Unreachable OpenAI-compatible provider did not return a health message");
  await request(`/v1/apps/${project.slug}/providers/${offlineProvider.provider.providerKey}`, { method: "DELETE" });

  const projectProvider = await request(`/v1/apps/${project.slug}/providers`, {
    method: "POST",
    body: {
      source: { kind: "registry", key: "openai-compatible" },
      name: "E2E OpenAI Project Provider",
      baseUrl: openAiProviderBaseUrl,
      config: {},
      secrets: openAiProviderToken ? { token: openAiProviderToken } : {},
    },
  });
  assert(projectProvider.provider?.sourceKind === "registry", "Project provider did not preserve sourceKind=registry");
  assert(projectProvider.provider?.providerType === "openai-compatible", "Project provider used the wrong provider type");
  assert(projectProvider.provider?.status === "online", "Project OpenAI-compatible provider was not online");
  if (openAiProviderToken) {
    assert(
      projectProvider.provider.secretKeys?.includes("token"),
      "Project OpenAI-compatible provider did not persist the configured token key",
    );
  }
  const testedProvider = await request(
    `/v1/apps/${project.slug}/providers/${projectProvider.provider.providerKey}/test`,
    { method: "POST", body: {} },
  );
  assert(testedProvider.provider?.status === "online", "Project OpenAI-compatible provider test did not report online");

  const directProfile = (await request("/v1/profiles", {
    method: "POST",
    body: openAiProfileBody({
      projectId: project.id,
      slug: `openai-compatible-e2e-${suffix}`,
      providerKey: projectProvider.provider.providerKey,
    }),
  })).profile;

  const imageChat = await request(`/${project.slug}/v1/chat/completions`, {
    method: "POST",
    body: imageChatCompletionBody(directProfile, "describe the test image"),
  }, projectKey.key);
  assert(
    completionContent(imageChat).includes("image=true"),
    "OpenAI-compatible chat did not forward image content",
  );

  const embedding = await request(`/${project.slug}/v1/embeddings`, {
    method: "POST",
    body: {
      model: directProfile.slug,
      input: ["parsnip", "watering can"],
    },
  }, projectKey.key);
  assert(
    embedding.data?.length === 2
      && Array.isArray(embedding.data[0]?.embedding)
      && embedding.data[0].embedding.length === 4,
    "OpenAI-compatible embedding endpoint did not return embeddings",
  );

  const transcription = await transcriptionRequest(
    `/${project.slug}/v1/audio/transcriptions`,
    directProfile.slug,
    projectKey.key,
  );
  assert(
    transcription.text === "mock transcription from OpenAI-compatible provider",
    "OpenAI-compatible transcription endpoint did not return provider text",
  );

  if (openAiMockUrl) {
    const metrics = await fetch(`${openAiMockUrl}/metrics`).then((response) => response.json());
    assert(metrics.modelsRequests >= 2, "OpenAI-compatible provider health checks did not call /v1/models");
    assert(metrics.chatImageRequests >= 1, "OpenAI-compatible provider did not receive an image chat request");
    assert(metrics.embeddingRequests >= 1, "OpenAI-compatible provider did not receive an embedding request");
    assert(metrics.transcriptionRequests >= 1, "OpenAI-compatible provider did not receive a transcription request");
    assert(metrics.unauthorizedRequests === 0, "OpenAI-compatible mock saw unauthorized requests");
  }

  const traces = (await request("/v1/traces?limit=500")).traces ?? [];
  const imageChatRequestId = completionRequestId(imageChat);
  assert(
    traces.some((trace) =>
      trace.requestId === imageChatRequestId
        && trace.projectId === project.id
        && trace.providerKey === projectProvider.provider.providerKey
        && trace.capabilityKind === "chat"
        && trace.status === "completed"
    ),
    "OpenAI-compatible image chat trace was not persisted",
  );
  for (const capability of ["embedding", "transcription"]) {
    assert(
      traces.some((trace) =>
        trace.projectId === project.id
          && trace.providerKey === projectProvider.provider.providerKey
          && trace.capabilityKind === capability
          && trace.status === "completed"
      ),
      `OpenAI-compatible ${capability} trace was not persisted`,
    );
  }

  return {
    availableProviderKey: attachedAvailable.provider.providerKey,
    projectProviderKey: projectProvider.provider.providerKey,
    directProfileSlug: directProfile.slug,
    modelSlugs: [
      directProfile.slug,
      ...profilesAfterAttach.map((profile) => profile.slug),
    ],
    profileIds: [
      directProfile.id,
      ...profilesAfterAttach.map((profile) => profile.id),
    ],
    requestIds: [imageChatRequestId],
  };
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

async function findConnectedGateway({ gatewayId = null, excludeGatewayId = null }) {
  const agentGateways = (await request("/v1/agent-gateways")).agentGateways ?? [];
  return agentGateways.find((item) =>
    item.status === "connected"
      && (!gatewayId || item.gatewayId === gatewayId)
      && (!excludeGatewayId || item.gatewayId !== excludeGatewayId)
  ) ?? null;
}

async function waitFor(description, read, { attempts = 60, intervalMs = 1_000 } = {}) {
  let lastError = null;
  for (let attempt = 0; attempt < attempts; attempt += 1) {
    try {
      const value = await read();
      if (value) return value;
    } catch (error) {
      lastError = error;
    }
    await new Promise((resolve) => setTimeout(resolve, intervalMs));
  }
  const suffix = lastError instanceof Error ? `: ${lastError.message}` : "";
  throw new Error(`Timed out waiting for ${description}${suffix}`);
}

function rawRequest(path, init = {}, credential = runtimeCredential) {
  const headers = new Headers(init.headers);
  headers.set("accept", "application/json");
  if (credential) headers.set("authorization", `Bearer ${credential}`);
  const body = init.body === undefined ? undefined : JSON.stringify(init.body);
  if (body !== undefined) headers.set("content-type", "application/json");
  return fetch(`${apiBaseUrl}${path}`, { ...init, headers, body });
}

async function transcriptionRequest(path, model, credential = runtimeCredential) {
  const form = new FormData();
  form.set("model", model);
  form.set("file", new Blob([testWavBytes()], { type: "audio/wav" }), "vifu-e2e.wav");
  const headers = new Headers({ accept: "application/json" });
  if (credential) headers.set("authorization", `Bearer ${credential}`);
  const response = await fetch(`${apiBaseUrl}${path}`, {
    method: "POST",
    headers,
    body: form,
  });
  const payload = await response.json().catch(() => null);
  if (!response.ok) {
    throw new Error(`POST ${path}: ${payload?.error?.message || `HTTP ${response.status}`}`);
  }
  return payload ?? {};
}

function chatCompletionBody(endpoint, message) {
  return {
    model: endpoint.slug,
    messages: [{ role: "user", content: message }],
    stream: false,
  };
}

function imageChatCompletionBody(profile, message) {
  return {
    model: profile.slug,
    messages: [{
      role: "user",
      content: [
        { type: "text", text: message },
        {
          type: "image_url",
          image_url: {
            url: "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/p9sAAAAASUVORK5CYII=",
          },
        },
      ],
    }],
    stream: false,
  };
}

function openAiProfileBody({ projectId, slug, providerKey }) {
  return {
    projectId,
    name: "E2E OpenAI-compatible Agent",
    slug,
    description: "Exercises project-level OpenAI-compatible chat, image, embedding, and transcription.",
    persona: { files: {} },
    runtime: {},
    presentation: {},
    source: {
      type: "openai-compatible",
      providerKey,
      managed: false,
    },
    capabilities: [
      profileCapability("chat", providerKey, "vifu-e2e-chat"),
      profileCapability("embedding", providerKey, "vifu-e2e-embedding"),
      profileCapability("transcription", providerKey, "vifu-e2e-transcription"),
    ],
    changeSummary: "Created by self-hosted Docker E2E",
  };
}

function profileCapability(kind, providerKey, resourceId) {
  return {
    kind,
    providerType: "openai-compatible",
    providerKey,
    resourceId,
    config: {},
    inputSchema: {},
    outputSchema: {},
  };
}

function testWavBytes() {
  return new Uint8Array([
    0x52, 0x49, 0x46, 0x46, 0x24, 0x00, 0x00, 0x00,
    0x57, 0x41, 0x56, 0x45, 0x66, 0x6d, 0x74, 0x20,
    0x10, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00,
    0x40, 0x1f, 0x00, 0x00, 0x80, 0x3e, 0x00, 0x00,
    0x02, 0x00, 0x10, 0x00, 0x64, 0x61, 0x74, 0x61,
    0x00, 0x00, 0x00, 0x00,
  ]);
}

function projectKeyBody(projectId, name, permissionOverrides = {}) {
  return {
    projectId,
    name,
    agentScope: { mode: "all" },
    permissions: apiKeyPermissions(permissionOverrides),
  };
}

function apiKeyPermissions(overrides = {}) {
  return {
    chatCompletions: "access",
    embeddings: "access",
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
