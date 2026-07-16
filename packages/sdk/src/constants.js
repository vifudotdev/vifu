export const VIFU_RUNTIME_CAPABILITY = "game-runtime";
export const VIFU_PROTOCOL_VERSION = "vifu.runtime.2026-07-15";
export const VIFU_SDK_VERSION = "0.1.0-alpha.9";
export const RUNTIME_GAME_ID_PARAM = "runtime_game_id";

export const VIFU_RUNTIME_SOURCE = "vifu-game-runtime";
export const VIFU_HOST_SOURCE = "vifu-host";
export const VIFU_WEB_HOST_SOURCE = "vifu-web-host";
export const VIFU_IOS_HOST_SOURCE = "vifu-ios-host";
export const VIFU_RUNTIME_CONNECT_MESSAGE = "vifu.runtime.connect";

export const VIFU_RUNTIME_METHODS = Object.freeze({
  hostReady: "vifu.runtime/host_ready",
  runtimeReady: "vifu.runtime/ready",
  invoke: "vifu.runtime/invoke",
  hostEvent: "vifu.runtime/event",
  hostOpenExternal: "vifu.runtime/openExternal",
});

export const HOST_SOURCES = new Set([
  VIFU_HOST_SOURCE,
  VIFU_WEB_HOST_SOURCE,
  VIFU_IOS_HOST_SOURCE,
]);
