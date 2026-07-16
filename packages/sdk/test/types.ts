import {
  VIFU_HOST_SOURCE,
  VIFU_RUNTIME_METHODS,
  VIFU_RUNTIME_SOURCE,
  createClient,
  createGameRuntimeSDK,
  createVifuSDK,
  type VifuHostEnvelope,
  type VifuJsonRpcMessage,
  type VifuPlatformAdapter,
  type VifuTransport,
} from "../dist/index.js";

const transport: VifuTransport = {
  kind: "custom",
  post(message: VifuJsonRpcMessage) {
    void message.jsonrpc;
  },
  start(onMessage) {
    void onMessage({ jsonrpc: "2.0", method: VIFU_RUNTIME_METHODS.hostReady });
  },
};

const adapter: VifuPlatformAdapter = {
  name: "types-adapter",
  status: () => ({ available: true, adapter: "types-adapter", gameId: "types-game" }),
  invoke: (capabilityId, args) => ({ capabilityId, args }),
};

const vifu = createVifuSDK({ transport, documentTitle: "types", platform: adapter });
const client = createClient({ transport: "none" });
const runtimeClient = createGameRuntimeSDK({ transport: "none" });
const browserVifu = window.vifu ?? window.Vifu;
browserVifu?.status();

const status = vifu.status();
const connected: boolean = vifu.runtime.isConnected();
const invokePromise: Promise<{ ok?: boolean }> = vifu.invoke("example.echo", { text: "hello" });
const runtimeEvent = vifu.runtime.emitEvent("example.reader.open", { chapter: 1 }, { source: "/games/reader" });
const openExternal = vifu.runtime.openExternal({ href: "https://example.com", linkId: "source" });

void client.version;
void runtimeClient.protocolVersion;
void status.protocolVersion;
void status.platformStatus.adapter;
void connected;
void invokePromise;
void runtimeEvent.type;
void openExternal.href;

const envelope: VifuHostEnvelope = {
  source: VIFU_HOST_SOURCE,
  message: { jsonrpc: "2.0", method: VIFU_RUNTIME_METHODS.hostReady },
};
void vifu._handleEnvelope(envelope);

const runtimeEnvelope = {
  source: VIFU_RUNTIME_SOURCE,
  message: { jsonrpc: "2.0" as const, method: VIFU_RUNTIME_METHODS.runtimeReady },
};
void runtimeEnvelope;
