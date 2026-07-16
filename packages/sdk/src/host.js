import { RUNTIME_GAME_ID_PARAM, VIFU_RUNTIME_METHODS } from "./constants.js";
import { currentLocation, currentWindow, defaultRuntimeParam, isObject } from "./helpers.js";

export function hostSourcePath() {
  const gameId = defaultRuntimeParam(RUNTIME_GAME_ID_PARAM);
  if (gameId) return `/games/${gameId.replace(/[^a-zA-Z0-9_.:-]/g, "_").slice(0, 96)}`;
  const path = String(currentLocation()?.pathname || "");
  const previewMatch = /^\/preview\/([^/]+)/.exec(path);
  if (previewMatch?.[1]) return `/games/${previewMatch[1]}`;
  const publishedMatch = /^\/v1\/assets\/runtime-static\/(?:public|member)\/([^/]+)/.exec(path);
  if (publishedMatch?.[1]) return `/games/${publishedMatch[1]}`;
  return "/games/runtime";
}

export function createHostEvent(type, data = {}, options = {}) {
  return {
    specversion: "1.0",
    id: typeof options.id === "string" && options.id.trim()
      ? options.id.trim()
      : `evt-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`,
    source: typeof options.source === "string" && options.source.trim() ? options.source.trim() : hostSourcePath(),
    type: String(type || "").trim() || "vifu.runtime.event",
    time: typeof options.time === "string" && options.time.trim() ? options.time : new Date().toISOString(),
    data: isObject(data) ? data : {},
  };
}

export function postRawHostMessage(message) {
  const win = currentWindow();
  if (!win?.parent || win.parent === win || typeof win.parent.postMessage !== "function") return false;
  try {
    win.parent.postMessage(message, "*");
    return true;
  } catch {
    return false;
  }
}

export function createHostFacade(notify) {
  function emitEvent(type, data = {}, options = {}) {
    const event = createHostEvent(type, data, options);
    notify(VIFU_RUNTIME_METHODS.hostEvent, event);
    postRawHostMessage(event);
    return event;
  }

  function openExternal(input = {}) {
    const payload = isObject(input) ? { ...input } : {};
    if (!payload.source) payload.source = hostSourcePath();
    notify(VIFU_RUNTIME_METHODS.hostOpenExternal, payload);
    postRawHostMessage({ type: "vifu.openExternal", ...payload });
    return payload;
  }

  return {
    emitEvent,
    openExternal,
  };
}
