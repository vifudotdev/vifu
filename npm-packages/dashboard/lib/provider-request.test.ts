import { describe, expect, it } from "vitest";

import { providerSettingsRequestBody, type ProviderDialogChoice } from "./provider-request";

describe("providerSettingsRequestBody", () => {
  it("attaches an available runtime provider without copying settings", () => {
    const choice: ProviderDialogChoice = {
      source: { kind: "custom", key: "local-openai" },
      name: "Local OpenAI",
      baseUrl: "http://runtime-owned.example/v1",
      fields: [
        { key: "baseUrl", label: "Base URL", kind: "url", required: true, secret: false },
        { key: "token", label: "API key", kind: "password", required: false, secret: true },
      ],
    };
    const form = new FormData();
    form.set("name", "Project Local OpenAI");
    form.set("baseUrl", "http://should-not-copy.example/v1");
    form.set("token", "should-not-copy");

    expect(providerSettingsRequestBody(undefined, choice, form)).toEqual({
      source: { kind: "custom", key: "local-openai" },
      name: "Project Local OpenAI",
    });
  });

  it("creates a project-local provider from a template with settings", () => {
    const choice: ProviderDialogChoice = {
      source: { kind: "registry", key: "openai-compatible" },
      name: "OpenAI-compatible",
      baseUrl: "",
      fields: [
        { key: "baseUrl", label: "Base URL", kind: "url", required: true, secret: false },
        { key: "token", label: "API key", kind: "password", required: false, secret: true },
        { key: "organization", label: "Organization", kind: "text", required: false, secret: false },
      ],
    };
    const form = new FormData();
    form.set("name", "Project OpenAI");
    form.set("baseUrl", "http://127.0.0.1:8080/v1");
    form.set("token", "local-test-token");
    form.set("organization", "test-org");

    expect(providerSettingsRequestBody(undefined, choice, form)).toEqual({
      source: { kind: "registry", key: "openai-compatible" },
      name: "Project OpenAI",
      baseUrl: "http://127.0.0.1:8080/v1",
      config: { organization: "test-org" },
      secrets: { token: "local-test-token" },
    });
  });
});
