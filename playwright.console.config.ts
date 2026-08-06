import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./tests-e2e-console",
  timeout: 30_000,
  reporter: "list",
  use: {
    baseURL: "http://127.0.0.1:4174",
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
  },
  webServer: {
    command: "python3 -m http.server 4174 --bind 127.0.0.1 --directory target/vifu-console-assets",
    url: "http://127.0.0.1:4174/index.html",
    reuseExistingServer: false,
  },
  projects: [{
    name: "chromium",
    use: { ...devices["Desktop Chrome"] },
  }],
});
