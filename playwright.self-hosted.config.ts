import { existsSync } from "node:fs";
import { defineConfig, devices } from "@playwright/test";

const browserChannel = process.env.VIFU_PLAYWRIGHT_CHANNEL
  ?? (process.platform === "darwin"
    && existsSync("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")
    ? "chrome"
    : undefined);

export default defineConfig({
  testDir: "./tests-e2e-self-hosted",
  timeout: 45_000,
  reporter: "list",
  use: {
    baseURL: process.env.VIFU_SELF_HOSTED_E2E_DASHBOARD_URL,
    trace: "on-first-retry",
  },
  projects: [{
    name: "chromium",
    use: {
      ...devices["Desktop Chrome"],
      ...(browserChannel ? { channel: browserChannel } : {}),
    },
  }],
});
