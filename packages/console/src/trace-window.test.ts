import { describe, expect, it } from "vitest";
import { traceDateWindowChanged } from "./trace-window";

describe("trace date window state", () => {
  it("preserves server-rendered traces when the initial window is unchanged", () => {
    const initial = { from: null, to: null };

    expect(traceDateWindowChanged(initial, { from: null, to: null })).toBe(false);
  });

  it("resets traces after the user changes either date boundary", () => {
    const initial = { from: null, to: null };

    expect(traceDateWindowChanged(initial, {
      from: "2026-08-10T00:00:00.000Z",
      to: null,
    })).toBe(true);
    expect(traceDateWindowChanged(initial, {
      from: null,
      to: "2026-08-11T00:00:00.000Z",
    })).toBe(true);
  });
});
