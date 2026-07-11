import { expect, test } from "@playwright/test";

test("signed-out dashboard redirects to marketing sign in", async ({ page }) => {
  await page.goto("/dashboard");
  await expect(page).toHaveURL("http://127.0.0.1:6793/login?returnTo=%2Fdashboard");
  await expect(page.getByRole("heading", { name: "Marketing sign in" })).toBeVisible();
});

test("an existing user magic link opens the dashboard", async ({ page }) => {
  await page.goto("/verify-email?code=existing-user&email=dev%40example.com&returnTo=%2Fdashboard");
  await expect(page).toHaveURL("http://127.0.0.1:6792/dashboard");
  await expect(page.getByRole("heading", { name: "Overview" })).toBeVisible();
  await expect(page.getByText("Existing creator").first()).toBeVisible();
  await expect(page.getByRole("navigation", { name: "Cloud navigation" })).toBeVisible();
});

test("a magic link cannot redirect to a backslash-encoded external host", async ({ page }) => {
  await page.goto("/verify-email?code=existing-user&email=dev%40example.com&returnTo=%2F%5Cexample.com");
  await expect(page).toHaveURL("http://127.0.0.1:6792/dashboard");
});

test("a new user magic link hands onboarding to the marketing app", async ({ page }) => {
  await page.goto("/verify-email?code=new-user&email=new%40example.com&returnTo=%2Fonboarding");
  await expect(page).toHaveURL("http://127.0.0.1:6793/onboarding");
  await expect(page.getByRole("heading", { name: "Marketing onboarding" })).toBeVisible();
});

test("sign out clears the dashboard session and returns to marketing", async ({ page }) => {
  await page.goto("/verify-email?code=existing-user&email=dev%40example.com&returnTo=%2Fdashboard");
  await expect(page).toHaveURL("http://127.0.0.1:6792/dashboard");
  await page.goto("/auth/logout");
  await expect(page).toHaveURL("http://127.0.0.1:6793/login");

  await page.goto("http://127.0.0.1:6792/dashboard");
  await expect(page).toHaveURL("http://127.0.0.1:6793/login?returnTo=%2Fdashboard");
});
