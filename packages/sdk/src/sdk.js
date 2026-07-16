import {
  VIFU_PROTOCOL_VERSION,
  VIFU_RUNTIME_CAPABILITY,
  VIFU_RUNTIME_METHODS,
  VIFU_SDK_VERSION,
} from "./constants.js";
import { currentWindow, isObject } from "./helpers.js";
import { createHostFacade } from "./host.js";
import { createLogger, logSdk } from "./logger.js";
import {
  createDefaultPlatformAdapter,
  normalizePlatformStatus,
  resolvePlatformConfig,
} from "./platform.js";
import { hostMessageFromEnvelope, resolveTransport } from "./transport.js";

const DEFAULT_HOST_REQUEST_TIMEOUT_MS = 30000;
const HOST_READY_WAIT_TIMEOUT_MS = 10000;

function createPendingRequestMap() {
  return new Map();
}

function isJsonRpcMessage(value) {
  return isObject(value)
    && value.jsonrpc === "2.0"
    && (typeof value.method === "string" || typeof value.id !== "undefined");
}

function normalizeInvokeArgs(args) {
  return isObject(args) ? args : {};
}

export function createVifuSDK(options = {}) {
  const readyCallbacks = [];
  const transport = resolveTransport(options);
  const logger = createLogger(options.logger);
  const platformConfig = resolvePlatformConfig(options.platform);
  const hasCustomPlatformAdapter = Boolean(platformConfig.adapter);
  const platformAdapter = platformConfig.adapter || createDefaultPlatformAdapter(platformConfig);
  const hostPendingRequests = createPendingRequestMap();
  let initialized = false;
  let runtimeReadyAnnounced = false;
  let disposeTransport = null;
  let nextHostRequestId = 1;
  let sdk;

  const documentTitle = options.documentTitle || currentWindow()?.document?.title || "vifu-game";

  function post(message) {
    transport.post(message);
  }

  function notify(method, params) {
    post({ jsonrpc: "2.0", method, params });
  }

  function getPlatformStatus() {
    try {
      return normalizePlatformStatus(
        typeof platformAdapter.status === "function" ? platformAdapter.status() : {},
        platformAdapter.name || "custom",
      );
    } catch (error) {
      logSdk(logger, "warn", "Runtime platform status failed", {
        adapter: platformAdapter.name || "custom",
        error: error instanceof Error ? error.message : String(error),
      });
      return normalizePlatformStatus({}, platformAdapter.name || "custom");
    }
  }

  function announceRuntimeReady(options = {}) {
    if ((runtimeReadyAnnounced && !options.force) || transport.kind === "none") return;
    runtimeReadyAnnounced = true;
    const platformStatus = getPlatformStatus();
    notify(VIFU_RUNTIME_METHODS.runtimeReady, {
      protocolVersion: VIFU_PROTOCOL_VERSION,
      sdkVersion: VIFU_SDK_VERSION,
      capability: VIFU_RUNTIME_CAPABILITY,
      transport: transport.kind || "custom",
      platformStatus,
      runtime: {
        name: documentTitle,
        version: VIFU_SDK_VERSION,
      },
    });
  }

  function getStatus() {
    return {
      sdkVersion: VIFU_SDK_VERSION,
      protocolVersion: VIFU_PROTOCOL_VERSION,
      capability: VIFU_RUNTIME_CAPABILITY,
      transport: transport.kind || "custom",
      hostConnected: initialized,
      platformStatus: getPlatformStatus(),
    };
  }

  function markHostReady() {
    if (initialized) return;
    initialized = true;
    const callbacks = readyCallbacks.splice(0);
    for (const callback of callbacks) callback(sdk);
  }

  function resolveHostPending(message) {
    if (!isObject(message) || typeof message.id === "undefined" || typeof message.method === "string") return false;
    const waiter = hostPendingRequests.get(message.id);
    if (!waiter) return false;
    hostPendingRequests.delete(message.id);
    clearTimeout(waiter.timeout);
    if (message.error) waiter.reject(new Error(message.error.message || "Host runtime request failed"));
    else waiter.resolve(message.result);
    return true;
  }

  function sendHostRequest(method, params, options = {}) {
    if (transport.kind === "none") {
      return Promise.reject(new Error(`Host transport is unavailable for ${method}`));
    }
    const id = `host-${nextHostRequestId++}`;
    const timeoutMs = Number.isFinite(options.timeoutMs) && options.timeoutMs > 0
      ? options.timeoutMs
      : DEFAULT_HOST_REQUEST_TIMEOUT_MS;
    return new Promise((resolve, reject) => {
      const timeout = setTimeout(() => {
        const waiter = hostPendingRequests.get(id);
        if (!waiter) return;
        hostPendingRequests.delete(id);
        waiter.reject(new Error(`${method} timed out`));
      }, timeoutMs);
      hostPendingRequests.set(id, { resolve, reject, timeout });
      post({ jsonrpc: "2.0", id, method, params });
    });
  }

  function waitForHostReady(timeoutMs = HOST_READY_WAIT_TIMEOUT_MS) {
    if (initialized) return Promise.resolve(sdk);
    if (transport.kind === "none") return Promise.reject(new Error("Host transport is unavailable"));
    announceRuntimeReady({ force: true });
    return new Promise((resolve, reject) => {
      let timeout;
      const onReady = () => {
        clearTimeout(timeout);
        resolve(sdk);
      };
      timeout = setTimeout(() => {
        const index = readyCallbacks.indexOf(onReady);
        if (index >= 0) readyCallbacks.splice(index, 1);
        reject(new Error("Host runtime bridge initialization timed out"));
      }, timeoutMs);
      readyCallbacks.push(onReady);
    });
  }

  async function invokeHostCapability(capabilityId, args = {}, options = {}) {
    const normalizedCapabilityId = String(capabilityId || "").trim();
    if (!normalizedCapabilityId) throw new Error("vifu.invoke capability id is required");
    const descriptor = {
      adapter: platformAdapter.name || "custom",
      capabilityId: normalizedCapabilityId,
    };
    logSdk(logger, "debug", "Runtime capability invoke start", descriptor);
    try {
      if (hasCustomPlatformAdapter && typeof platformAdapter.invoke === "function") {
        const result = await platformAdapter.invoke(normalizedCapabilityId, normalizeInvokeArgs(args));
        logSdk(logger, "debug", "Runtime capability invoke complete", descriptor);
        return result;
      }
      if (!initialized && transport.kind !== "none") {
        await waitForHostReady(options.readyTimeoutMs);
      }
      if (initialized) {
        const result = await sendHostRequest(VIFU_RUNTIME_METHODS.invoke, {
          capabilityId: normalizedCapabilityId,
          arguments: normalizeInvokeArgs(args),
        }, options);
        logSdk(logger, "debug", "Runtime capability invoke complete", {
          ...descriptor,
          source: "host",
        });
        return result;
      }
      if (typeof platformAdapter.invoke !== "function") {
        throw new Error(`Platform capability ${normalizedCapabilityId} is not available`);
      }
      const result = await platformAdapter.invoke(normalizedCapabilityId, normalizeInvokeArgs(args));
      logSdk(logger, "debug", "Runtime capability invoke complete", descriptor);
      return result;
    } catch (error) {
      logSdk(logger, "error", "Runtime capability invoke failed", {
        ...descriptor,
        error: error instanceof Error ? error.message : String(error),
      });
      throw error;
    }
  }

  function handleMessage(message) {
    if (!isJsonRpcMessage(message)) return false;
    if (resolveHostPending(message)) return true;
    if (message.method === VIFU_RUNTIME_METHODS.hostReady) {
      markHostReady();
      return true;
    }
    if (typeof message.id !== "undefined" && typeof message.method === "string") {
      post({
        jsonrpc: "2.0",
        id: message.id,
        error: {
          code: -32601,
          message: `Method not found: ${message.method}`,
        },
      });
      return true;
    }
    return false;
  }

  function handleEnvelope(envelopeOrMessage) {
    const message = hostMessageFromEnvelope(envelopeOrMessage, true);
    if (message) return handleMessage(message);
    return false;
  }

  function onReady(callback) {
    if (typeof callback !== "function") return;
    if (initialized) {
      callback(sdk);
      return;
    }
    readyCallbacks.push(callback);
  }

  function dispose() {
    if (typeof disposeTransport === "function") disposeTransport();
    disposeTransport = null;
    for (const waiter of hostPendingRequests.values()) {
      clearTimeout(waiter.timeout);
      waiter.reject(new Error("Runtime bridge disposed"));
    }
    hostPendingRequests.clear();
  }

  const host = createHostFacade(notify);

  sdk = {
    version: VIFU_SDK_VERSION,
    protocolVersion: VIFU_PROTOCOL_VERSION,
    ready: () => waitForHostReady(),
    onReady,
    status: getStatus,
    runtime: {
      ready: () => waitForHostReady(),
      status: getStatus,
      isConnected: () => initialized,
      emitEvent: host.emitEvent,
      openExternal: host.openExternal,
    },
    invoke: invokeHostCapability,
    _handleEnvelope: handleEnvelope,
    _notify: notify,
    _disposeTransport: dispose,
  };

  if (transport.kind !== "none" && typeof transport.start === "function") {
    disposeTransport = transport.start((message) => {
      handleEnvelope(message);
    });
    announceRuntimeReady();
  }

  return sdk;
}

export const createClient = createVifuSDK;
export const createGameRuntimeSDK = createVifuSDK;
