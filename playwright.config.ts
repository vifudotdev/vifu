import { defineConfig, devices } from "@playwright/test";

const dashboardUrl = "http://127.0.0.1:6792";
const mockApiUrl = "http://127.0.0.1:6793";

export default defineConfig({
  testDir: "./tests-e2e",
  timeout: 45_000,
  expect: { timeout: 10_000 },
  fullyParallel: false,
  reporter: "list",
  use: {
    baseURL: dashboardUrl,
    trace: "on-first-retry",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
  webServer: [
    {
      command: "node scripts/mock-account-api.mjs",
      url: `${mockApiUrl}/health`,
      reuseExistingServer: false,
      timeout: 30_000,
    },
    {
      command: "bun run --cwd npm-packages/dashboard dev:e2e",
      url: dashboardUrl,
      reuseExistingServer: false,
      timeout: 120_000,
      env: {
        VIFU_API_BASE_URL: mockApiUrl,
        VIFU_DASHBOARD_URL: dashboardUrl,
        VIFU_AUTH_URL: mockApiUrl,
      },
    },
  ],
});
