import { expect, test } from "@playwright/test";

test("session remains valid across sidebar navigation on the bind address", async ({ context, page }) => {
  const email = process.env.VIFU_SELF_HOSTED_E2E_AUTH_EMAIL ?? "admin@self-hosted.example";
  const password = process.env.VIFU_SELF_HOSTED_E2E_AUTH_PASSWORD ?? "correct horse battery staple";

  await page.goto("/login");
  await page.getByLabel("Email").fill(email);
  await page.getByLabel("Password").fill(password);
  const loginResponsePromise = page.waitForResponse((response) =>
    response.request().method() === "POST" && response.url().endsWith("/api/auth/local/login")
  );
  await page.getByRole("button", { name: "Sign in" }).click();
  const loginResponse = await loginResponsePromise;
  expect(loginResponse.status()).toBe(303);

  const session = (await context.cookies()).find((cookie) => cookie.name === "vifu_session");
  expect(session).toBeDefined();
  expect(session?.httpOnly).toBe(true);
  expect(session?.path).toBe("/");
  expect(session?.domain).toBe("0.0.0.0");
  await expect(page).toHaveURL(/\/project\/[^/]+\/health$/);

  for (const [label, path] of [
    ["Gameplay", "gameplay"],
    ["API Keys", "api-keys"],
    ["Logs", "logs"],
    ["Settings", "settings"],
    ["Health", "health"],
  ] as const) {
    await page.getByRole("link", { name: label, exact: true }).click();
    await expect(page).toHaveURL(new RegExp(`/project/[^/]+/${path}$`));
    await expect(page.getByRole("heading", { level: 1, name: label })).toBeVisible();
  }
});
