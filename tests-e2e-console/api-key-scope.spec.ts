import { expect, test, type Route } from "@playwright/test";

const projectId = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const profileId = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
const keyId = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";
const now = "2026-08-06T00:00:00Z";

const project = {
  id: projectId,
  slug: "demo",
  name: "Demo project",
  description: null,
  gatewayId: "project-demo",
  enabled: true,
  bindingIds: [],
  createdAt: now,
  updatedAt: now,
};
const profile = {
  id: profileId,
  projectId,
  slug: "demo-agent",
  name: "Demo agent",
  description: null,
  activeVersionId: null,
  archivedAt: null,
  createdAt: now,
  updatedAt: now,
};
const permissions = {
  chatCompletions: "access",
  embeddings: "access",
  speech: "none",
  transcriptions: "none",
  realtime: "none",
  runtime: "none",
  agents: "read",
  project: "none",
};

test("created selected-agent key keeps its scope in the row and edit dialog", async ({ page }) => {
  let createdRecord: Record<string, unknown> | null = null;
  let createBody: Record<string, unknown> | null = null;
  let apiKeyListReads = 0;
  let delayNextStatusRead = false;
  await page.route("**/api/runtime/**", async (route) => {
    const request = route.request();
    const path = new URL(request.url()).pathname.replace(/^\/api\/runtime\/?/, "");
    if (request.method() === "POST" && path === "project/demo/api-keys") {
      createBody = request.postDataJSON() as Record<string, unknown>;
      createdRecord = {
        id: keyId,
        projectId,
        name: createBody.name,
        agentScope: createBody.agentScope,
        permissions: createBody.permissions,
        keyPrefix: "vifu_pk_contract",
        key: "vifu_pk_contract_secret",
        createdAt: now,
        revokedAt: null,
      };
      return json(route, 201, { apiKey: createdRecord });
    }
    if (request.method() !== "GET") return json(route, 405, { error: "unexpected method" });
    if (path === "status") {
      if (delayNextStatusRead) {
        delayNextStatusRead = false;
        await new Promise((resolve) => setTimeout(resolve, 750));
      }
      return json(route, 200, {
        service: "vifu-server",
        status: "ready",
        version: "0.1.10",
        mode: "local",
        capabilities: {
          projects: true,
          profiles: true,
          endpoints: false,
          bindings: false,
          apiKeys: true,
          agentGateways: false,
          providerConnections: false,
          traces: false,
        },
        agentGateways: 0,
      });
    }
    if (path === "projects") return json(route, 200, { projects: [project] });
    if (path === "project/demo/profiles") return json(route, 200, { profiles: [profile] });
    if (path === `project/demo/profiles/${profileId}`) {
      return json(route, 200, { profile, versions: [], rollout: [] });
    }
    if (path === "project/demo/api-keys") {
      apiKeyListReads += 1;
      if (createdRecord) await new Promise((resolve) => setTimeout(resolve, 250));
      const record = createdRecord ? { ...createdRecord, key: undefined } : null;
      return json(route, 200, { apiKeys: record ? [record] : [] });
    }
    if (path === "project/demo/provider-catalog") return json(route, 200, { registry: [], custom: [] });
    if (path === "project/demo/providers") return json(route, 200, { providers: [] });
    if (path === "project/demo/agent-candidates") return json(route, 200, { candidates: [] });
    if (path === "project/demo/deployments") return json(route, 200, { deployments: [] });
    if (path === "project/demo/runtime-releases") return json(route, 200, { releases: [] });
    return json(route, 404, { error: `unexpected path: ${path}` });
  });

  await page.goto("/index.html");
  await page.getByRole("link", { name: "Demo project" }).click();
  await expect(page.getByRole("heading", { name: "Overview" })).toBeVisible();
  delayNextStatusRead = true;
  await page.getByRole("link", { name: "API", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Loading console" })).toHaveCount(0);
  await expect(page.getByRole("heading", { name: "Overview" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "API Integrations" })).toBeVisible();
  await page.getByRole("button", { name: "Create key" }).click();
  const createDialog = page.getByRole("dialog");
  await createDialog.getByRole("group", { name: "Agents permission" })
    .getByRole("button", { name: "Read", exact: true }).click();
  await createDialog.getByRole("button", { name: "Selected agents", exact: true }).click();
  await createDialog.locator(`input[value="${profileId}"]`).check();
  await createDialog.getByLabel("Name").fill("Selected contract key");
  await createDialog.getByRole("button", { name: "Create key" }).click();

  await expect(createDialog.getByRole("heading", { name: "Save your API key" })).toBeVisible();
  expect(createBody?.agentScope).toEqual({ mode: "selected", profileIds: [profileId] });
  const readsBeforeDone = apiKeyListReads;
  await createDialog.getByRole("button", { name: "Done" }).click();
  await expect(page.getByRole("cell", { name: "Selected contract key", exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Edit Selected contract key" }).click();
  const editDialog = page.getByRole("dialog");
  await expect(editDialog.getByRole("button", { name: "Selected agents", exact: true }))
    .toHaveAttribute("aria-pressed", "true");
  await expect(editDialog.locator(`input[value="${profileId}"]`)).toBeChecked();
  await expect(editDialog.getByRole("group", { name: "Agents permission" })
    .getByRole("button", { name: "Read", exact: true })).toHaveAttribute("aria-pressed", "true");
  await page.waitForTimeout(300);
  await expect(editDialog.getByRole("heading", { name: "Edit API key" })).toBeVisible();
  expect(apiKeyListReads).toBe(readsBeforeDone);
});

function json(route: Route, status: number, body: unknown) {
  return route.fulfill({ status, contentType: "application/json", body: JSON.stringify(body) });
}
