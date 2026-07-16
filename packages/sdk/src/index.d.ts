export declare const VIFU_RUNTIME_CAPABILITY = "game-runtime";
export declare const VIFU_PROTOCOL_VERSION = "vifu.runtime.2026-07-15";
export declare const VIFU_SDK_VERSION = "0.1.0-alpha.9";

export declare const VIFU_RUNTIME_SOURCE = "vifu-game-runtime";
export declare const VIFU_HOST_SOURCE = "vifu-host";
export declare const VIFU_WEB_HOST_SOURCE = "vifu-web-host";
export declare const VIFU_IOS_HOST_SOURCE = "vifu-ios-host";
export declare const VIFU_RUNTIME_CONNECT_MESSAGE = "vifu.runtime.connect";

export declare const VIFU_RUNTIME_METHODS: Readonly<{
  hostReady: "vifu.runtime/host_ready";
  runtimeReady: "vifu.runtime/ready";
  invoke: "vifu.runtime/invoke";
  hostEvent: "vifu.runtime/event";
  hostOpenExternal: "vifu.runtime/openExternal";
}>;

export type VifuTransportKind = "auto" | "iframe" | "wkwebview" | "custom" | "none" | string;

export interface VifuJsonRpcMessage {
  jsonrpc: "2.0";
  id?: string | number | null;
  method?: string;
  params?: Record<string, unknown>;
  result?: unknown;
  error?: { code: number; message: string; data?: unknown };
}

export interface VifuRuntimeEnvelope {
  source: typeof VIFU_RUNTIME_SOURCE;
  message: VifuJsonRpcMessage | string;
}

export interface VifuHostEnvelope {
  source: typeof VIFU_HOST_SOURCE | typeof VIFU_WEB_HOST_SOURCE | typeof VIFU_IOS_HOST_SOURCE | string;
  message: VifuJsonRpcMessage | string;
}

export interface VifuTransport {
  kind?: VifuTransportKind;
  post(message: VifuJsonRpcMessage): void;
  start?(onMessage: (message: VifuJsonRpcMessage | unknown, rawMessage?: unknown) => void | Promise<void>): void | (() => void);
}

export type VifuTransportOption = "auto" | "iframe" | "wkwebview" | VifuTransport;

export interface VifuLogger {
  debug?(message: string, details?: Record<string, unknown>): void;
  info?(message: string, details?: Record<string, unknown>): void;
  warn?(message: string, details?: Record<string, unknown>): void;
  error?(message: string, details?: Record<string, unknown>): void;
}

export interface VifuPlatformStatus {
  available: boolean;
  adapter: string;
  gameId: string;
}

export interface VifuPlatformAdapter {
  name?: string;
  status?(): VifuPlatformStatus | Partial<VifuPlatformStatus>;
  invoke?(capabilityId: string, args?: Record<string, unknown>): unknown | Promise<unknown>;
}

export interface VifuPlatformConfig {
  adapter?: VifuPlatformAdapter | null;
  resolveRuntimeParam?: (name: string) => string;
}

export interface VifuRuntimeStatus {
  sdkVersion: typeof VIFU_SDK_VERSION;
  protocolVersion: typeof VIFU_PROTOCOL_VERSION;
  capability: typeof VIFU_RUNTIME_CAPABILITY;
  transport: string;
  hostConnected: boolean;
  platformStatus: VifuPlatformStatus;
}

export interface VifuRuntimeEventOptions {
  id?: string;
  source?: string;
  time?: string;
}

export interface VifuRuntimeEvent {
  specversion: "1.0";
  id: string;
  source: string;
  type: string;
  time: string;
  data: Record<string, unknown>;
}

export interface VifuOpenExternalInput {
  linkId?: string;
  href?: string;
  label?: string;
  source?: string;
  [key: string]: unknown;
}

export interface VifuRuntimeApi {
  ready(): Promise<VifuSDK>;
  status(): VifuRuntimeStatus;
  isConnected(): boolean;
  emitEvent(type: string, data?: Record<string, unknown>, options?: VifuRuntimeEventOptions): VifuRuntimeEvent;
  openExternal(input: VifuOpenExternalInput): VifuOpenExternalInput;
}

export interface CreateVifuSDKOptions {
  transport?: VifuTransportOption | "none";
  postMessage?: (message: VifuJsonRpcMessage) => void;
  documentTitle?: string;
  logger?: VifuLogger;
  platform?: VifuPlatformAdapter | VifuPlatformConfig;
}

export interface VifuInvokeOptions {
  timeoutMs?: number;
  readyTimeoutMs?: number;
}

export interface VifuSDK {
  version: typeof VIFU_SDK_VERSION;
  protocolVersion: typeof VIFU_PROTOCOL_VERSION;
  ready(): Promise<VifuSDK>;
  onReady(callback: (sdk: VifuSDK) => void): void;
  status(): VifuRuntimeStatus;
  runtime: VifuRuntimeApi;
  invoke<T = unknown>(capabilityId: string, args?: Record<string, unknown>, options?: VifuInvokeOptions): Promise<T>;
  _handleEnvelope(envelopeOrMessage: VifuHostEnvelope | VifuJsonRpcMessage | string | unknown): boolean;
  _notify(method: string, params?: Record<string, unknown>): void;
  _disposeTransport(): void;
}

export declare function createVifuSDK(options?: CreateVifuSDKOptions): VifuSDK;
export declare const createClient: typeof createVifuSDK;
export declare const createGameRuntimeSDK: typeof createVifuSDK;

declare global {
  interface Window {
    vifu?: VifuSDK;
    Vifu?: VifuSDK;
    __VIFU_RUNTIME_CONFIG__?: {
      gameId?: string;
      params?: Record<string, string | number | boolean>;
      [key: string]: unknown;
    };
  }
}
