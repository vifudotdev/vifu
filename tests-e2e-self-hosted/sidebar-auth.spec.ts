import { expect, test } from "@playwright/test";
import { readFile } from "node:fs/promises";

test("session remains valid across sidebar navigation on the bind address", async ({ context, page }) => {
  const adminKey = process.env.VIFU_SELF_HOSTED_E2E_ADMIN_KEY;
  expect(adminKey, "VIFU_SELF_HOSTED_E2E_ADMIN_KEY is required").toBeTruthy();
  const keyName = `Playwright App key ${Date.now()}`;

  await page.goto("/login");
  await page.getByLabel("Admin key").fill(adminKey!);
  const loginResponsePromise = page.waitForResponse((response) =>
    response.request().method() === "POST" && response.url().endsWith("/api/auth/admin-key")
  );
  await page.getByRole("button", { name: "Connect" }).click();
  const loginResponse = await loginResponsePromise;
  expect(loginResponse.status()).toBe(303);
  expect(loginResponse.headers()["location"] ?? "").not.toContain("auth_error");

  const session = (await context.cookies()).find((cookie) => cookie.name === "vifu_admin_session");
  expect(session).toBeDefined();
  expect(session?.httpOnly).toBe(true);
  expect(session?.path).toBe("/");
  expect(session?.domain).toBe(new URL(page.url()).hostname);
  expect(session?.value).not.toContain(adminKey!);
  await expect(page).toHaveURL(/\/project$/);
  await expect(page.getByRole("heading", { level: 1, name: "Apps" })).toBeVisible();
  const projectCards = page.locator(".project-home-card");
  await expect(projectCards.first()).toBeVisible();
  expect(await projectCards.count()).toBeGreaterThanOrEqual(2);
  await projectCards.first().click();
  await expect(page).toHaveURL(/\/project\/[^/]+(?:\/overview)?$/);

  await page.getByRole("link", { name: "Agents", exact: true }).click();
  await expect(page).toHaveURL(/\/project\/[^/]+\/agents$/);
  await expect(page.getByRole("heading", { level: 1, name: "Agents" })).toBeVisible();
  const projectSwitcher = page.locator(".project-switcher");
  const projectSearch = projectSwitcher.getByLabel("Search apps");
  if (!await projectSearch.isVisible()) {
    await projectSwitcher.locator("summary").click();
  }
  await expect(projectSearch).toBeVisible();
  await projectSearch.fill("no-project-has-this-name");
  await expect(projectSwitcher.getByText("No matching apps", { exact: true })).toBeVisible();
  await projectSearch.fill("");
  const projectLinks = projectSwitcher.locator(".project-menu-list a");
  await expect(projectLinks.first()).toBeVisible();
  expect(await projectLinks.count()).toBeGreaterThanOrEqual(2);
  const targetAgentsHref = await projectLinks.nth(1).getAttribute("href");
  expect(targetAgentsHref).toMatch(/^\/project\/[^/]+\/agents$/);
  const targetProjectRoot = targetAgentsHref!.replace(/\/agents$/, "");
  await projectLinks.nth(1).click();
  await expect(page).toHaveURL(new URL(targetAgentsHref!, page.url()).toString());
  await expect(page.getByRole("link", { name: "Overview", exact: true }))
    .toHaveAttribute("href", `${targetProjectRoot}/overview`);

  for (const [label, path, heading] of [
    ["Overview", "overview", "Overview"],
    ["Agents", "agents", "Agents"],
    ["Providers", "providers", "Providers"],
    ["API", "api", "API Integrations"],
    ["Traces", "logs", "Traces"],
    ["Settings", "settings", "Settings"],
  ] as const) {
    const link = page.getByRole("link", { name: label, exact: true });
    const href = `${targetProjectRoot}/${path}`;
    await expect(link).toHaveAttribute("href", href);
    await Promise.all([
      page.waitForURL(new URL(href, page.url()).toString()),
      link.click(),
    ]);
    await expect(page.getByRole("heading", { level: 1, name: heading })).toBeVisible();
  }

  const statePath = process.env.VIFU_SELF_HOSTED_E2E_STATE_PATH;
  if (statePath) {
    const state = JSON.parse(await readFile(statePath, "utf8"));
    if (state.openAiProviderKey) {
      await page.goto(`/project/${state.projectSlug}/providers`);
      await expect(page.getByRole("heading", { level: 1, name: "Providers" })).toBeVisible();
      await expect(page.getByText("OpenAI Compatible E2E", { exact: true })).toBeVisible();
      await expect(page.getByText("E2E OpenAI Project Provider", { exact: true })).toBeVisible();
      await page.getByRole("button", { name: "Add provider" }).click();
      const providerDialog = page.getByRole("dialog");
      await expect(providerDialog.getByRole("heading", { name: "Choose a provider" })).toBeVisible();
      await expect(providerDialog.getByRole("button", { name: "OpenAI Compatible E2E", exact: true })).toHaveCount(0);
      await expect(providerDialog.getByRole("button", { name: "OpenAI Compatible E2E Alt", exact: true })).toHaveCount(0);
      await providerDialog.getByRole("button", { name: "Close" }).click();
    }
  }

  await page.getByRole("link", { name: "API", exact: true }).click();
  await page.getByRole("button", { name: "Create key" }).click();
  const keyDialog = page.getByRole("dialog");
  await expect(keyDialog.getByRole("heading", { name: "Create API key" })).toBeVisible();
  await expect(keyDialog.getByLabel("Name")).toHaveValue(/^App key - /);
  await expect(keyDialog.getByRole("combobox")).toHaveCount(0);
  await expect(keyDialog.getByRole("button", { name: "All agents", exact: true })).toHaveAttribute("aria-pressed", "true");
  await expect(
    keyDialog.getByRole("group", { name: "Chat Completions permission" }).getByRole("button", { name: "Access", exact: true }),
  ).toHaveAttribute("aria-pressed", "true");
  await expect(
    keyDialog.getByRole("group", { name: "Agents permission" }).getByRole("button", { name: "No access", exact: true }),
  ).toHaveAttribute("aria-pressed", "true");
  await keyDialog.getByRole("group", { name: "Agents permission" }).getByRole("button", { name: "Read", exact: true }).click();
  await keyDialog.getByRole("button", { name: "Selected agents", exact: true }).click();
  await expect(keyDialog.getByPlaceholder("Search agents")).toBeVisible();
  const agentOptions = keyDialog.locator(".api-key-agent-options input[type=checkbox]");
  const hasAgentOptions = await agentOptions.count() > 0;
  let selectedProfileId: string | null = null;
  if (hasAgentOptions) {
    await expect(agentOptions.first()).toBeVisible();
    selectedProfileId = await agentOptions.first().getAttribute("value");
    expect(selectedProfileId).toBeTruthy();
    await agentOptions.first().check();
    await expect(keyDialog.getByText("1 selected", { exact: true })).toBeVisible();
  } else {
    await keyDialog.getByRole("button", { name: "All agents", exact: true }).click();
  }

  await keyDialog.getByLabel("Name").fill(keyName);
  const createKeyResponsePromise = page.waitForResponse((response) => (
    response.request().method() === "POST"
      && /^\/api\/runtime\/apps\/[^/]+\/api-keys$/.test(new URL(response.url()).pathname)
  ));
  await keyDialog.getByRole("button", { name: "Create key" }).click();
  const createKeyResponse = await createKeyResponsePromise;
  expect(createKeyResponse.status()).toBe(201);
  const createKeyRequest = createKeyResponse.request().postDataJSON() as {
    agentScope?: { mode?: unknown; profileIds?: unknown };
  };
  expect(createKeyRequest.agentScope?.mode).toBe(hasAgentOptions ? "selected" : "all");
  if (hasAgentOptions) {
    expect(createKeyRequest.agentScope?.profileIds).toEqual([selectedProfileId]);
  }
  const createKeyPayload = await createKeyResponse.json() as {
    apiKey?: {
      key?: unknown;
      agentScope?: { mode?: unknown; profileIds?: unknown };
    };
  };
  expect(typeof createKeyPayload.apiKey?.key).toBe("string");
  expect(createKeyPayload.apiKey?.agentScope?.mode).toBe(hasAgentOptions ? "selected" : "all");
  if (hasAgentOptions) {
    expect(createKeyPayload.apiKey?.agentScope?.profileIds).toEqual([selectedProfileId]);
  }
  await expect(keyDialog.getByRole("heading", { name: "Save your API key" })).toBeVisible();
  await expect(keyDialog.getByText("Shown once")).toBeVisible();
  await keyDialog.getByRole("button", { name: "Done" }).click();

  await expect(page.getByRole("cell", { name: keyName, exact: true })).toBeVisible();
  await page.getByRole("button", { name: `Edit ${keyName}` }).click();
  const editKeyDialog = page.getByRole("dialog");
  await expect(editKeyDialog.getByRole("heading", { name: "Edit API key" })).toBeVisible();
  await expect(editKeyDialog.getByRole("button", { name: hasAgentOptions ? "Selected agents" : "All agents", exact: true })).toHaveAttribute("aria-pressed", "true");
  await expect(
    editKeyDialog.getByRole("group", { name: "Agents permission" }).getByRole("button", { name: "Read", exact: true }),
  ).toHaveAttribute("aria-pressed", "true");
  await editKeyDialog.getByRole("button", { name: "All agents", exact: true }).click();
  await editKeyDialog.getByRole("group", { name: "App permission" }).getByRole("button", { name: "Write", exact: true }).click();
  await editKeyDialog.getByRole("button", { name: "Save changes" }).click();
  const keyRow = page.getByRole("row").filter({ has: page.getByRole("cell", { name: keyName, exact: true }) });
  await expect(keyRow).toContainText("All agents");
  await expect(keyRow).toContainText("Chat completions, Embeddings, Agents read, App write");

  page.once("dialog", (dialog) => dialog.accept());
  const revokeKeyResponsePromise = page.waitForResponse((response) => (
    response.request().method() === "POST"
      && /^\/api\/runtime\/apps\/[^/]+\/api-keys\/[^/]+\/revoke$/.test(new URL(response.url()).pathname)
  ));
  await page.getByRole("button", { name: `Revoke ${keyName}` }).click();
  const revokeKeyResponse = await revokeKeyResponsePromise;
  expect(revokeKeyResponse.status()).toBe(200);
  await expect(page.getByRole("cell", { name: keyName, exact: true })).toHaveCount(0);
  const revokedTab = page.getByRole("tab", { name: /^Revoked / });
  await expect(revokedTab).toBeVisible();
  await revokedTab.click();
  await expect(page.getByRole("cell", { name: keyName, exact: true })).toBeVisible();
  page.once("dialog", (dialog) => dialog.accept());
  await page.getByRole("button", { name: `Delete ${keyName} record` }).click();
  await expect(page.getByRole("cell", { name: keyName, exact: true })).toHaveCount(0);
});
