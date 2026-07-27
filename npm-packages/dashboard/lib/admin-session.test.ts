import { describe, expect, it } from "vitest";
import {
  adminKeysMatch,
  createAdminSession,
  verifyAdminSession,
} from "./admin-session";

describe("admin session", () => {
  it("signs a session without embedding the admin key", () => {
    const adminKey = "admin-key-with-sufficient-entropy";
    const session = createAdminSession(adminKey, 1_000);

    expect(session).not.toContain(adminKey);
    expect(verifyAdminSession(session, adminKey, 2_000)).toBe(true);
  });

  it("rejects tampered, expired, and differently signed sessions", () => {
    const session = createAdminSession("correct-admin-key", 1_000);

    expect(verifyAdminSession(`${session}x`, "correct-admin-key", 2_000)).toBe(false);
    expect(verifyAdminSession(session, "other-admin-key", 2_000)).toBe(false);
    expect(verifyAdminSession(session, "correct-admin-key", 12 * 60 * 60 * 1000 + 1_001)).toBe(false);
  });

  it("compares submitted admin keys exactly", () => {
    expect(adminKeysMatch("same-key", "same-key")).toBe(true);
    expect(adminKeysMatch("same-key ", "same-key")).toBe(false);
    expect(adminKeysMatch("wrong-key", "same-key")).toBe(false);
  });
});
