import {
  HOST_SOURCES,
  VIFU_PROTOCOL_VERSION,
  VIFU_RUNTIME_CONNECT_MESSAGE,
  VIFU_RUNTIME_METHODS,
  VIFU_RUNTIME_SOURCE,
} from "./constants.js";
import { currentWindow, isObject, maybeParseJson } from "./helpers.js";

function isJsonRpcMessage(value) {
  return isObject(value)
    && value.jsonrpc === "2.0"
    && (typeof value.method === "string" || typeof value.id !== "undefined");
}

export function runtimeEnvelope(message) {
  return { source: VIFU_RUNTIME_SOURCE, message };
}

export function hostMessageFromEnvelope(value, allowDirect = false) {
  const data = maybeParseJson(value);
  if (allowDirect && isJsonRpcMessage(data)) return data;
  if (!isObject(data)) return null;
  const source = typeof data.source === "string" ? data.source : "";
  if (!HOST_SOURCES.has(source)) return null;
  const message = maybeParseJson(data.message);
  return isJsonRpcMessage(message) ? message : null;
}

function createNoopTransport() {
  return {
    kind: "none",
    post() {},
  };
}

function createCompatTransport(postMessage) {
  return {
    kind: "custom",
    post: postMessage,
  };
}

function messagePortFromConnectEvent(event) {
  const data = maybeParseJson(event?.data);
  if (!isObject(data)) return null;
  const source = typeof data.source === "string" ? data.source : "";
  if (!HOST_SOURCES.has(source)) return null;
  if (data.type !== VIFU_RUNTIME_CONNECT_MESSAGE) return null;
  const port = Array.isArray(event.ports) ? event.ports[0] : null;
  return port && typeof port.postMessage === "function" ? port : null;
}

function createIframeTransport(win = currentWindow()) {
  let activePort = null;
  return {
    kind: "iframe",
    post(message) {
      if (activePort) {
        activePort.postMessage(message);
        return;
      }
      win?.parent?.postMessage(runtimeEnvelope(message), "*");
    },
    start(onMessage) {
      if (!win || typeof win.addEventListener !== "function") return undefined;
      let portCleanup = null;
      const connectPort = (port) => {
        if (activePort === port) return;
        if (typeof portCleanup === "function") portCleanup();
        activePort = port;
        const portListener = (event) => {
          const message = hostMessageFromEnvelope(event.data, true);
          if (message) void onMessage(message, event.data);
        };
        if (typeof port.addEventListener === "function") {
          port.addEventListener("message", portListener);
          if (typeof port.start === "function") port.start();
          portCleanup = () => {
            port.removeEventListener("message", portListener);
            if (typeof port.close === "function") port.close();
            if (activePort === port) activePort = null;
          };
        } else {
          port.onmessage = portListener;
          if (typeof port.start === "function") port.start();
          portCleanup = () => {
            port.onmessage = null;
            if (typeof port.close === "function") port.close();
            if (activePort === port) activePort = null;
          };
        }
        void onMessage({
          jsonrpc: "2.0",
          method: VIFU_RUNTIME_METHODS.hostReady,
          params: {
            protocolVersion: VIFU_PROTOCOL_VERSION,
            transport: "messagechannel",
          },
        });
      };
      const listener = (event) => {
        const port = messagePortFromConnectEvent(event);
        if (port) {
          connectPort(port);
          return;
        }
        const message = hostMessageFromEnvelope(event.data, false);
        if (message) void onMessage(message, event.data);
      };
      win.addEventListener("message", listener);
      return () => {
        win.removeEventListener("message", listener);
        if (typeof portCleanup === "function") portCleanup();
      };
    },
  };
}

function webkitMessageHandler(win = currentWindow()) {
  const webkit = win?.webkit;
  const handlers = webkit && isObject(webkit.messageHandlers) ? webkit.messageHandlers : undefined;
  const handler = handlers && isObject(handlers.vifuHost) ? handlers.vifuHost : undefined;
  return handler && typeof handler.postMessage === "function" ? handler : null;
}

function createWKWebViewTransport(win = currentWindow()) {
  return {
    kind: "wkwebview",
    post(message) {
      webkitMessageHandler(win)?.postMessage(runtimeEnvelope(message));
    },
    start(onMessage) {
      if (!win || typeof win.addEventListener !== "function") return undefined;
      const listener = (event) => {
        const message = hostMessageFromEnvelope(event.data, false);
        if (message) void onMessage(message, event.data);
      };
      win.addEventListener("message", listener);
      return () => win.removeEventListener("message", listener);
    },
  };
}

function createAutoTransport() {
  const win = currentWindow();
  if (!win) return createNoopTransport();
  if (webkitMessageHandler(win)) return createWKWebViewTransport(win);
  if (win.parent && win.parent !== win) return createIframeTransport(win);
  return createNoopTransport();
}

function isTransport(value) {
  return isObject(value) && typeof value.post === "function";
}

export function resolveTransport(options) {
  if (typeof options.postMessage === "function") return createCompatTransport(options.postMessage);
  const transport = options.transport ?? "auto";
  if (isTransport(transport)) return transport;
  if (transport === "iframe") return createIframeTransport();
  if (transport === "wkwebview") return createWKWebViewTransport();
  if (transport === "auto") return createAutoTransport();
  return createNoopTransport();
}
