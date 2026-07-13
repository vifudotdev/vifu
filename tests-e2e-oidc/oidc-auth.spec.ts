import { expect, test } from "@playwright/test";

test("OIDC signs in through PKCE and creates one Vifu session", async ({ context, page }) => {
  await page.goto("/login");
  await expect(page.getByRole("link", { name: "Continue with Test Identity" })).toBeVisible();
  await expect(page.getByLabel("Email")).toBeVisible();

  await page.getByRole("link", { name: "Continue with Test Identity" }).click();
  await expect(page).toHaveURL(/\/dashboard$/);
  await expect(page.getByRole("heading", { name: "Overview" })).toBeVisible();
  await expect(page.getByText("oidc-admin@example.com").first()).toBeVisible();

  const cookies = await context.cookies();
  const session = cookies.find((cookie) => cookie.name === "vifu_session");
  expect(session?.httpOnly).toBe(true);
  expect(session?.sameSite).toBe("Lax");
  expect(cookies.filter((cookie) => cookie.name.startsWith("vifu_")).map((cookie) => cookie.name)).toEqual(["vifu_session"]);

  await page.getByRole("button", { name: "Sign out" }).click();
  await expect(page).toHaveURL(/\/login$/);
  expect((await context.cookies()).some((cookie) => cookie.name === "vifu_session")).toBe(false);
});
