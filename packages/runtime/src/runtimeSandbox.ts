import { RUNTIME_IFRAME_SANDBOX, RUNTIME_IFRAME_SCROLLING } from "./runtimeUrl.js";

export interface RuntimeIframePolicyOptions {
  sameOriginTemplateTypes?: Iterable<string>;
  sameOriginTemplateIds?: Iterable<string>;
  scrollingTemplateIds?: Iterable<string>;
}

function normalizedSet(values: Iterable<string> | undefined): Set<string> {
  return new Set(Array.from(values ?? [], (value) => value.trim().toLowerCase()).filter(Boolean));
}

function normalize(value?: string | null): string {
  return value?.trim().toLowerCase() || "";
}

export function runtimeIframeUsesSameOrigin(
  templateType?: string | null,
  templateId?: string | null,
  options: RuntimeIframePolicyOptions = {},
): boolean {
  const sameOriginTemplateTypes = normalizedSet(options.sameOriginTemplateTypes);
  const sameOriginTemplateIds = normalizedSet(options.sameOriginTemplateIds);
  return sameOriginTemplateTypes.has(normalize(templateType))
    || sameOriginTemplateIds.has(normalize(templateId));
}

export function runtimeIframeSandboxForTemplate(
  templateType?: string | null,
  templateId?: string | null,
  options: RuntimeIframePolicyOptions = {},
): string {
  if (!runtimeIframeUsesSameOrigin(templateType, templateId, options)) return RUNTIME_IFRAME_SANDBOX;
  return `${RUNTIME_IFRAME_SANDBOX} allow-same-origin`;
}

export function runtimeIframeScrollingForTemplate(
  templateId?: string | null,
  options: RuntimeIframePolicyOptions = {},
): string {
  const scrollingTemplateIds = normalizedSet(options.scrollingTemplateIds);
  if (scrollingTemplateIds.has(normalize(templateId))) return "auto";
  return RUNTIME_IFRAME_SCROLLING;
}
