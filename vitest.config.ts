import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    include: [
      "npm-packages/**/*.test.{ts,tsx,js,jsx}",
      "packages/**/*.test.{ts,tsx,js,jsx}",
    ],
  },
});
