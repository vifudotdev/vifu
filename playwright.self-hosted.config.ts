import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./tests-e2e-self-hosted",
  timeout: 45_000,
  reporter: "list",
  use: {
    baseURL: process.env.VIFU_SELF_HOSTED_E2E_DASHBOARD_URL,
    trace: "on-first-retry",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
});
