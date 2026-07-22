import { describe, expect, test } from "vitest";
import { isAllowedProjectGamePath } from "./runtime-proxy-policy";

describe("runtime Game proxy policy", () => {
  test("allows the management session collection and detail routes", () => {
    expect(isAllowedProjectGamePath(["project", "demo", "game", "sessions"])).toBe(true);
    expect(isAllowedProjectGamePath([
      "project",
      "demo",
      "game",
      "sessions",
      "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
    ])).toBe(true);
    expect(isAllowedProjectGamePath([
      "project",
      "demo",
      "game",
      "localization",
      "translate",
    ])).toBe(true);
    expect(isAllowedProjectGamePath([
      "project",
      "demo",
      "game",
      "assets",
      "asset-id",
      "versions",
      "version-id",
      "content",
    ])).toBe(true);
    expect(isAllowedProjectGamePath([
      "project",
      "demo",
      "game",
      "releases",
      "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
      "export",
    ])).toBe(true);
  });

  test("rejects unregistered nested Game routes", () => {
    expect(isAllowedProjectGamePath([
      "project",
      "demo",
      "game",
      "sessions",
      "session-id",
      "delete",
    ])).toBe(false);
    expect(isAllowedProjectGamePath([
      "project",
      "demo",
      "game",
      "unknown",
    ])).toBe(false);
  });
});
