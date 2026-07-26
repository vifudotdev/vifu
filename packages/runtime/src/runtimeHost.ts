export interface JsonRpcRequest {
  jsonrpc: "2.0";
  id?: string | number;
  method: string;
  params?: unknown;
}

export type JsonRpcId = string | number;

export interface JsonRpcResponse {
  jsonrpc: "2.0";
  id?: JsonRpcId;
  result?: unknown;
  error?: { code?: number; message: string; data?: unknown };
}

export type JsonRpcMessage = JsonRpcRequest | JsonRpcResponse;

export interface RuntimeEnvelope<T = unknown> {
  source: "vifu-host" | "vifu-runtime" | string;
  message: T;
}

export interface CloudEvent<T = unknown> {
  specversion: "1.0";
  id: string;
  source: string;
  type: string;
  time: string;
  datacontenttype: "application/json";
  data: T;
}

export interface CloudEventOptions {
  id?: string;
  source?: string;
  time?: string | Date;
}

export interface RuntimeHostOptions {
  iframe?: HTMLIFrameElement;
  frame?: RuntimeFramePort;
  targetOrigin?: string;
  hostSource?: string;
  runtimeSource?: string;
  onMessage?: (message: JsonRpcRequest) => void;
  onDebugEvent?: (event: CloudEvent) => void;
}

export interface RuntimeFramePort {
  kind: "iframe" | "electron-webview" | string;
  postEnvelope(envelope: RuntimeEnvelope, targetOrigin: string): void;
}

export interface RuntimeElectronWebviewElement {
  executeJavaScript?: (code: string, userGesture?: boolean) => Promise<unknown>;
}

export interface RuntimeHost {
  post(method: string, params?: unknown, id?: string | number): void;
  postMessage(message: JsonRpcMessage): void;
  postRequest(request: JsonRpcRequest): void;
  postResponse(response: JsonRpcResponse): void;
  emitDebug(type: string, data: unknown): CloudEvent;
  flushDebugEvents(): CloudEvent[];
  dispose(): void;
}

export function createRuntimeHost(options: RuntimeHostOptions): RuntimeHost {
  const hostSource = options.hostSource || "vifu-host";
  const runtimeSource = options.runtimeSource || "vifu-runtime";
  const targetOrigin = options.targetOrigin || "*";
  const frame = options.frame || (options.iframe ? createIframeRuntimeFrame(options.iframe) : null);
  const debugEvents: CloudEvent[] = [];

  if (!frame) {
    throw new Error("Runtime host requires an iframe or frame adapter");
  }
  const runtimeFrame = frame;

  const listener = (event: MessageEvent) => {
    const envelope = parseEnvelope(event.data);
    if (!envelope || envelope.source !== runtimeSource || !isJsonRpc(envelope.message)) return;
    options.onMessage?.(envelope.message);
  };
  const eventTarget = typeof window === "undefined" ? null : window;
  eventTarget?.addEventListener("message", listener);

  function postRequest(message: JsonRpcRequest) {
    postMessage(message);
  }

  function postResponse(message: JsonRpcResponse) {
    postMessage(message);
  }

  function postMessage(message: JsonRpcMessage) {
    runtimeFrame.postEnvelope({ source: hostSource, message }, targetOrigin);
  }

  function post(method: string, params?: unknown, id?: string | number) {
    postRequest(createJsonRpcRequest(method, params, id));
  }

  function emitDebug(type: string, data: unknown): CloudEvent {
    const event = createCloudEvent(type, data, { source: hostSource });
    debugEvents.push(event);
    options.onDebugEvent?.(event);
    return event;
  }

  return {
    post,
    postMessage,
    postRequest,
    postResponse,
    emitDebug,
    flushDebugEvents() {
      return debugEvents.splice(0, debugEvents.length);
    },
    dispose() {
      eventTarget?.removeEventListener("message", listener);
    },
  };
}

export function createIframeRuntimeFrame(iframe: HTMLIFrameElement): RuntimeFramePort {
  return {
    kind: "iframe",
    postEnvelope(envelope, targetOrigin) {
      iframe.contentWindow?.postMessage(envelope, targetOrigin);
    },
  };
}

export function createElectronWebviewRuntimeFrame(webview: RuntimeElectronWebviewElement): RuntimeFramePort {
  return {
    kind: "electron-webview",
    postEnvelope(envelope, targetOrigin) {
      const script = `window.postMessage(${JSON.stringify(envelope)}, ${JSON.stringify(targetOrigin || "*")});`;
      void webview.executeJavaScript?.(script, false).catch(() => undefined);
    },
  };
}

export function createJsonRpcRequest(method: string, params?: unknown, id?: JsonRpcId): JsonRpcRequest {
  const request: JsonRpcRequest = {
    jsonrpc: "2.0",
    method,
  };
  if (typeof id !== "undefined") request.id = id;
  if (typeof params !== "undefined") request.params = params;
  return request;
}

export function createCloudEvent<T = unknown>(
  type: string,
  data: T,
  options: CloudEventOptions = {},
): CloudEvent<T> {
  return {
    specversion: "1.0",
    id: options.id || `evt-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`,
    source: options.source || "vifu-host",
    type,
    time: options.time instanceof Date ? options.time.toISOString() : options.time || new Date().toISOString(),
    datacontenttype: "application/json",
    data,
  };
}

export function parseRuntimeEnvelope(value: unknown): RuntimeEnvelope | null {
  return parseEnvelope(value);
}

export function isJsonRpcRequest(value: unknown): value is JsonRpcRequest {
  return isJsonRpc(value);
}

export function isJsonRpcMessage(value: unknown): value is JsonRpcMessage {
  return isJsonRpc(value) || isJsonRpcResponse(value);
}

function parseEnvelope(value: unknown): RuntimeEnvelope | null {
  const data = typeof value === "string" ? tryJson(value) : value;
  if (!data || typeof data !== "object" || Array.isArray(data)) return null;
  const envelope = data as Partial<RuntimeEnvelope>;
  if (typeof envelope.source !== "string") return null;
  return {
    source: envelope.source,
    message: typeof envelope.message === "string" ? tryJson(envelope.message) : envelope.message,
  };
}

function isJsonRpc(value: unknown): value is JsonRpcRequest {
  return Boolean(
    value
    && typeof value === "object"
    && !Array.isArray(value)
    && (value as JsonRpcRequest).jsonrpc === "2.0"
    && typeof (value as JsonRpcRequest).method === "string",
  );
}

function isJsonRpcResponse(value: unknown): value is JsonRpcResponse {
  return Boolean(
    value
    && typeof value === "object"
    && !Array.isArray(value)
    && (value as JsonRpcResponse).jsonrpc === "2.0"
    && typeof (value as JsonRpcRequest).method === "undefined"
    && (
      typeof (value as JsonRpcResponse).id !== "undefined"
      || Object.prototype.hasOwnProperty.call(value, "result")
      || Object.prototype.hasOwnProperty.call(value, "error")
    ),
  );
}

function tryJson(value: string): unknown {
  try {
    return JSON.parse(value);
  } catch {
    return null;
  }
}
