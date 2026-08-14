import { describe, expect, it } from "vitest";
import { consoleRouteHref, readConsoleRoute } from "./route";

describe("embedded console route", () => {
  it("preserves trace deep-link query and fragment state", () => {
    const route = readConsoleRoute(
      "/apps/stardew-valley/logs",
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
      "/apps/stardew-valley/logs?invocationId=request-1&observationId=span-1#detail",
    );
  });

  it("allows ordinary project navigation to clear a prior trace selection", () => {
    expect(consoleRouteHref({ projectSlug: "stardew-valley", section: "agents" }))
      .toBe("/apps/stardew-valley/agents");
  });

  it("routes the ordinary pairing workflow through Devices", () => {
    const route = readConsoleRoute("/apps/arm-lab/devices");
    expect(route).toMatchObject({ projectSlug: "arm-lab", section: "devices" });
    expect(consoleRouteHref(route)).toBe("/apps/arm-lab/devices");
  });

  it("keeps legacy deployment links available for advanced settings", () => {
    expect(readConsoleRoute("/project/arm-lab/deployments").section).toBe("deployments");
    expect(consoleRouteHref(readConsoleRoute("/project/arm-lab/deployments")))
      .toBe("/apps/arm-lab/deployments");
  });
});
