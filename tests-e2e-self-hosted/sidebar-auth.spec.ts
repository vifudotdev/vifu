import { expect, test } from "@playwright/test";

test("session remains valid across sidebar navigation on the bind address", async ({ context, page }) => {
  const email = process.env.VIFU_SELF_HOSTED_E2E_AUTH_EMAIL ?? "admin@self-hosted.example";
  const password = process.env.VIFU_SELF_HOSTED_E2E_AUTH_PASSWORD ?? "correct horse battery staple";

  await page.goto("/signup");
  if (await page.getByRole("heading", { level: 1, name: "Create your account" }).isVisible().catch(() => false)) {
    await page.getByLabel("Display name").fill("Self-hosted Admin");
    await page.getByLabel("Email").fill(email);
    await page.getByLabel("Password").fill(password);
    const signupResponsePromise = page.waitForResponse((response) =>
      response.request().method() === "POST" && response.url().endsWith("/api/auth/local/signup")
    );
    await page.getByRole("button", { name: "Create account" }).click();
    const signupResponse = await signupResponsePromise;
    expect(signupResponse.status()).toBe(303);
    const signupLocation = signupResponse.headers()["location"] ?? "";
    if (!signupLocation.includes("auth_error")) {
      await expect(page).toHaveURL(/\/project(?:\/[^/]+\/health)?$/);
      await page.request.post("/auth/logout", {
        headers: { origin: new URL(page.url()).origin },
        maxRedirects: 0,
      });
    }
  }

  await page.goto("/login");
  await page.getByLabel("Email").fill(email);
  await page.getByLabel("Password").fill(password);
  const loginResponsePromise = page.waitForResponse((response) =>
    response.request().method() === "POST" && response.url().endsWith("/api/auth/local/login")
  );
  await page.getByRole("button", { name: "Sign in" }).click();
  const loginResponse = await loginResponsePromise;
  expect(loginResponse.status()).toBe(303);
  expect(loginResponse.headers()["location"] ?? "").not.toContain("auth_error");

  const session = (await context.cookies()).find((cookie) => cookie.name === "vifu_session");
  expect(session).toBeDefined();
  expect(session?.httpOnly).toBe(true);
  expect(session?.path).toBe("/");
  expect(session?.domain).toBe(new URL(page.url()).hostname);
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
