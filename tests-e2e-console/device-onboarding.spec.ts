import { expect, test, type Route } from "@playwright/test";

const projectId = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const deploymentId = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
const gatewayId = "android-arm-optimized";
const now = "2026-08-14T00:00:00Z";

test("pairs the default device from Overview and keeps environments in advanced settings", async ({ page }) => {
  let paired = false;
  await page.route("**/api/runtime/**", async (route) => {
    const request = route.request();
    const path = new URL(request.url()).pathname.replace(/^\/api\/runtime\/?/, "");
    if (request.method() === "POST" && path === "apps/arm-lab/deployments/development/agent-gateway-enrollments") {
      paired = true;
      return json(route, 201, {
        enrollmentId: "enrollment-1",
        deployment: "development",
        enrollmentToken: "synthetic-pairing-token",
        expiresAt: "2099-08-14T00:05:00Z",
        pairing: {
          serverUrl: "https://vifu.local:6790",
          pairingUri: "https://vifu.local/pair",
          pairingDeepLink: "vifu://gateway/enroll?server=synthetic",
          pairingQrSvg: "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 1 1\"><path d=\"M0 0h1v1H0z\"/></svg>",
        },
      });
    }
    if (request.method() !== "GET") return json(route, 405, { error: "unexpected method" });
    if (path === "status") {
      return json(route, 200, {
        service: "vifu-server",
        status: "ready",
        version: "0.1.12",
        mode: "local",
        capabilities: {
          apps: true,
          profiles: true,
          endpoints: true,
          bindings: true,
          apiKeys: true,
          agentGateways: true,
          providerConnections: true,
          traces: true,
        },
        agentGateways: paired ? 1 : 0,
      });
    }
    if (path === "apps") return json(route, 200, { apps: [project()] });
    if (path === "provider-adapters") return json(route, 200, { providerAdapters: [] });
    if (path === "apps/arm-lab/profiles") return json(route, 200, { profiles: [] });
    if (path === "apps/arm-lab/bindings") return json(route, 200, { bindings: [] });
    if (path === "apps/arm-lab/endpoints") return json(route, 200, { endpoints: [] });
    if (path === "apps/arm-lab/api-keys") return json(route, 200, { apiKeys: [] });
    if (path === "apps/arm-lab/agent-gateways") return json(route, 200, { agentGateways: paired ? [gateway()] : [] });
    if (path === "apps/arm-lab/agents") return json(route, 200, { agents: [] });
    if (path.startsWith("apps/arm-lab/traces")) return json(route, 200, { traces: [] });
    if (path === "apps/arm-lab/deployments") return json(route, 200, { deployments: [deployment(paired)] });
    if (path === "apps/arm-lab/runtime-releases") return json(route, 200, { releases: [] });
    if (path === "apps/arm-lab/providers") return json(route, 200, { providers: [] });
    if (path === "apps/arm-lab/provider-catalog") return json(route, 200, { registry: [], custom: [] });
    if (path === "apps/arm-lab/agent-candidates") return json(route, 200, { candidates: [] });
    return json(route, 404, { error: `unexpected path: ${path}` });
  });

  await page.goto("/index.html");
  await page.getByRole("link", { name: "ARM lab" }).click();

  await expect(page.getByRole("heading", { name: "Overview" })).toBeVisible();
  await expect(page.getByRole("link", { name: "Devices", exact: true })).toBeVisible();
  await expect(page.getByRole("link", { name: "Deployments", exact: true })).toHaveCount(0);
  await expect(page.getByText("Pair your first device", { exact: true })).toBeVisible();
  await expect(page.getByLabel("Environment")).toHaveCount(0);

  await page.getByRole("button", { name: "Pair device" }).click();
  await expect(page.getByRole("status").filter({ hasText: "Pair a device" })).toBeVisible();
  await expect(page.getByText("1 device online", { exact: true }).first()).toBeVisible();

  await page.getByRole("link", { name: "Devices", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Devices", exact: true })).toBeVisible();
  await expect(page.getByText("Vifu Starter Optimized · Pixel 9", { exact: true })).toBeVisible();
  await expect(page.getByText("development", { exact: true })).toHaveCount(0);

  await page.getByRole("link", { name: "Settings", exact: true }).click();
  const advanced = page.getByText("Advanced runtime configuration", { exact: true });
  await expect(advanced).toBeVisible();
  await expect(page.getByText("New environment", { exact: true })).not.toBeVisible();
  await advanced.click();
  await expect(page.getByText("New environment", { exact: true })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Configuration releases" })).toBeVisible();

  await page.setViewportSize({ width: 390, height: 844 });
  await expect(page.getByRole("link", { name: "Settings", exact: true })).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth + 1)).toBe(true);
});

function project() {
  return {
    id: projectId,
    appId: `vifu_app_${"a".repeat(64)}`,
    slug: "arm-lab",
    name: "ARM lab",
    description: "Compare Android inference runtimes",
    gatewayId: "project-arm-lab",
    enabled: true,
    bindingIds: [],
    createdAt: now,
    updatedAt: now,
  };
}

function deployment(paired: boolean) {
  return {
    id: deploymentId,
    projectId,
    name: "development",
    isPrimary: true,
    configSyncEnabled: true,
    traceMode: "summary",
    remoteInvocationEnabled: false,
    activeReleaseVersion: null,
    gatewayIds: paired ? [gatewayId] : [],
    applyStates: [],
    createdAt: now,
    updatedAt: now,
  };
}

function gateway() {
  return {
    id: "gateway-session-1",
    gatewayId,
    sessionId: "gateway-session-1",
    status: "connected",
    agents: [{ id: "android-local-chat", name: "Android llama" }],
    metadata: {
      name: "Vifu Starter Optimized · Pixel 9",
      kind: "mobile",
      platform: "android",
      device: { manufacturer: "Google", model: "Pixel 9" },
      application: { name: "Vifu Starter Optimized", version: "0.1.1" },
    },
    connectedAt: now,
    lastSeenAt: now,
    disconnectedAt: null,
  };
}

function json(route: Route, status: number, body: unknown) {
  return route.fulfill({ status, contentType: "application/json", body: JSON.stringify(body) });
}
