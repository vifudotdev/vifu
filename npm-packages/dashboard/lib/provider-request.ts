import type { ProjectProvider, ProviderAdapterField } from "./runtime-types";

export type ProviderDialogChoice = {
  source: { kind: "registry" | "custom"; key: string };
  name: string;
  baseUrl: string;
  fields: ProviderAdapterField[];
};

export function providerSettingsRequestBody(
  provider: ProjectProvider | undefined,
  choice: ProviderDialogChoice,
  form: FormData,
) {
  const attachAvailableProvider = !provider && choice.source.kind === "custom";
  const config: Record<string, unknown> = {};
  const secrets: Record<string, string> = {};
  if (!attachAvailableProvider) {
    for (const field of choice.fields) {
      if (field.key === "baseUrl") continue;
      const value = String(form.get(field.key) ?? "").trim();
      if (!value) continue;
      if (field.secret) secrets[field.key] = value;
      else config[field.key] = value;
    }
  }
  const name = String(form.get("name") ?? choice.name).trim();
  return attachAvailableProvider
    ? { source: choice.source, name }
    : {
        ...(provider ? {} : { source: choice.source }),
        name,
        baseUrl: String(form.get("baseUrl") ?? choice.baseUrl).trim(),
        config,
        secrets,
      };
}
