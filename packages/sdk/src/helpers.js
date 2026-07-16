export function isObject(value) {
  return Boolean(value && typeof value === "object" && !Array.isArray(value));
}

export function asArray(value) {
  if (Array.isArray(value)) return value;
  if (typeof value === "undefined" || value === null) return [];
  return [value];
}

export function maybeParseJson(value) {
  if (typeof value !== "string") return value;
  try {
    return JSON.parse(value);
  } catch {
    return null;
  }
}

export function currentWindow() {
  return typeof window !== "undefined" ? window : undefined;
}

export function currentLocation() {
  if (typeof location !== "undefined") return location;
  return currentWindow()?.location;
}

export function runtimeSearchParam(name, rawLocation = currentLocation()) {
  if (!rawLocation?.search) return "";
  try {
    return new URLSearchParams(rawLocation.search).get(name) || "";
  } catch {
    return "";
  }
}

export function runtimeHashParam(name, rawLocation = currentLocation()) {
  const rawHash = rawLocation?.hash;
  if (!rawHash) return "";
  const normalized = rawHash.startsWith("#") ? rawHash.slice(1) : rawHash;
  if (!normalized) return "";
  try {
    return new URLSearchParams(normalized).get(name) || "";
  } catch {
    return "";
  }
}

function coerceRuntimeParamValue(value) {
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  return "";
}

export function runtimeBootstrapConfig(rawWindow = currentWindow()) {
  const config = rawWindow?.__VIFU_RUNTIME_CONFIG__;
  return isObject(config) ? config : null;
}

export function runtimeBootstrapParam(name, rawWindow = currentWindow()) {
  const config = runtimeBootstrapConfig(rawWindow);
  if (!config) return "";

  const params = isObject(config.params) ? config.params : {};
  const explicit = coerceRuntimeParamValue(params[name]);
  if (explicit) return explicit;

  const aliases = {
    runtime_game_id: ["gameId"],
  }[name] || [];
  for (const alias of aliases) {
    const value = coerceRuntimeParamValue(config[alias]);
    if (value) return value;
  }
  return "";
}

export function defaultRuntimeParam(name) {
  return runtimeHashParam(name) || runtimeSearchParam(name) || runtimeBootstrapParam(name);
}
