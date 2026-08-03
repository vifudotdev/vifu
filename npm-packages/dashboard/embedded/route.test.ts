import { describe, expect, it } from "vitest";
import { consoleRouteHref, readConsoleRoute } from "./route";

describe("embedded console route", () => {
  it("preserves trace deep-link query and fragment state", () => {
    const route = readConsoleRoute(
      "/project/stardew-valley/logs",
      "?invocationId=request-1&observationId=span-1",
      "#detail",
    );

    expect(route).toMatchObject({
      projectSlug: "stardew-valley",
      section: "logs",
      search: "?invocationId=request-1&observationId=span-1",
      hash: "#detail",
    });
    expect(consoleRouteHref(route)).toBe(
      "/project/stardew-valley/logs?invocationId=request-1&observationId=span-1#detail",
    );
  });

  it("allows ordinary project navigation to clear a prior trace selection", () => {
    expect(consoleRouteHref({ projectSlug: "stardew-valley", section: "agents" }))
      .toBe("/project/stardew-valley/agents");
  });
});
