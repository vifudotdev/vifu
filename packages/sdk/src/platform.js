import { RUNTIME_GAME_ID_PARAM } from "./constants.js";
import { defaultRuntimeParam, isObject } from "./helpers.js";
export function isPlatformAdapter(value) {
  return isObject(value) && (
    typeof value.status === "function"
    || typeof value.invoke === "function"
  );
}

export function resolvePlatformConfig(rawPlatform) {
  if (isPlatformAdapter(rawPlatform)) {
    return {
      adapter: rawPlatform,
      resolveRuntimeParam: undefined,
    };
  }
  const config = isObject(rawPlatform) ? rawPlatform : {};
  return {
    adapter: isPlatformAdapter(config.adapter) ? config.adapter : null,
    resolveRuntimeParam: typeof config.resolveRuntimeParam === "function" ? config.resolveRuntimeParam : undefined,
  };
}

export function normalizePlatformStatus(rawStatus, adapterName) {
  const status = isObject(rawStatus) ? rawStatus : {};
  return {
    available: status.available === true,
    adapter: typeof status.adapter === "string" && status.adapter.trim() ? status.adapter.trim() : adapterName,
    gameId: typeof status.gameId === "string" ? status.gameId : "",
  };
}

export function createDefaultPlatformAdapter(config = {}) {
  const resolveRuntimeParam = typeof config.resolveRuntimeParam === "function"
    ? config.resolveRuntimeParam
    : defaultRuntimeParam;

  return {
    name: "runtime-launch-params",
    status() {
      return {
        available: false,
        adapter: "runtime-launch-params",
        gameId: resolveRuntimeParam(RUNTIME_GAME_ID_PARAM) || "",
      };
    },
    async invoke(capabilityId) {
      throw new Error(`Platform capability ${String(capabilityId || "")} is not available`);
    },
  };
}
