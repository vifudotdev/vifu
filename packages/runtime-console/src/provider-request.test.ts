import { describe, expect, test } from "vitest";

import { providerSettingsRequestBody } from "./provider-request";

describe("provider settings requests", () => {
  test("parses portable runtime settings and resources as JSON objects", () => {
    const form = new FormData();
    form.set("name", "Local Qwen");
    form.set("settings", "{\"contextSize\":4096}");
    form.set("resources", "{\"model\":\"model:qwen-demo\"}");

    const body = providerSettingsRequestBody(undefined, {
      source: { kind: "registry", key: "vifu-runtime" },
      name: "Local Qwen",
      baseUrl: "",
      fields: [
        { key: "settings", label: "Runtime settings", kind: "json", required: true, secret: false },
        { key: "resources", label: "Runtime resources", kind: "json", required: true, secret: false },
      ],
    }, form);

    expect(body.config).toEqual({
      settings: { contextSize: 4096 },
      resources: { model: "model:qwen-demo" },
    });
  });

  test("rejects non-object runtime settings", () => {
    const form = new FormData();
    form.set("settings", "[]");

    expect(() => providerSettingsRequestBody(undefined, {
      source: { kind: "registry", key: "vifu-runtime" },
      name: "Local Qwen",
      baseUrl: "",
      fields: [
        { key: "settings", label: "Runtime settings", kind: "json", required: true, secret: false },
      ],
    }, form)).toThrow("Runtime settings must be a JSON object.");
  });
});
