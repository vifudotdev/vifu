import { describe, expect, test } from "vitest";

import { PROJECT_NAVIGATION } from "./runtime-console";

describe("app navigation", () => {
  test("makes Devices the ordinary connection path and hides Deployments", () => {
    expect(PROJECT_NAVIGATION.map((item) => item.label)).toEqual([
      "Overview",
      "Devices",
      "Agents",
      "Providers",
      "API",
      "Traces",
      "Settings",
    ]);
  });
});
