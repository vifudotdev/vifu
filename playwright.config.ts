import { defineConfig, devices } from "@playwright/test";

const dashboardUrl = process.env.VIFU_SELF_HOSTED_E2E_DASHBOARD_URL ?? "http://127.0.0.1:6790";

export default defineConfig({
  testDir: "./tests-e2e-self-hosted",
  timeout: 45_000,
  expect: { timeout: 10_000 },
  fullyParallel: false,
  reporter: "list",
  use: {
    baseURL: dashboardUrl,
    trace: "on-first-retry",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
});
