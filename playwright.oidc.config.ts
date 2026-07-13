import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./tests-e2e-oidc",
  timeout: 45_000,
  reporter: "list",
  use: {
    baseURL: process.env.VIFU_OIDC_E2E_DASHBOARD_URL,
    trace: "on-first-retry",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
});
