export type JsonPrimitive = string | number | boolean | null;
export type JsonValue = JsonPrimitive | JsonValue[] | { [key: string]: JsonValue };

export interface VifuAgentRequest {
  appId: string;
  endpoint: string;
  sessionId: string;
  agent: {
    id: string;
    name: string;
    provider: string;
    capabilities: string[];
    metadata: JsonValue;
  };
  capability: string;
  input: JsonValue;
  metadata: JsonValue;
  state: JsonValue;
  stateRevision: number;
}

export interface VifuAgentResponse {
  output: JsonValue;
  metadata?: JsonValue;
  state?: JsonValue;
}

export type VifuProviderStage =
  | "queue"
  | "load"
  | "tokenize"
  | "prefill"
  | "first_token"
  | "decode"
  | "validate";

type NativeTraceEmitter = (event: JsonValue) => void;

export class VifuAgentTrace {
  constructor(private readonly emit: NativeTraceEmitter) {}

  activity(): void {
    this.emit({ type: "activity" });
  }

  outputDelta(value: JsonValue): void {
    this.emit({ type: "outputDelta", value });
  }

  async stage<T>(
    stage: VifuProviderStage,
    operation: () => T | Promise<T>,
    metadata: JsonValue = {},
  ): Promise<T> {
    const started = performance.now();
    this.emit({ type: "stageStarted", stage, metadata });
    try {
      const result = await operation();
      this.emit({
        type: "stageCompleted",
        stage,
        elapsedMs: Math.max(0, Math.round(performance.now() - started)),
        metadata,
      });
      return result;
    } catch (error) {
      this.emit({
        type: "stageFailed",
        stage,
        elapsedMs: Math.max(0, Math.round(performance.now() - started)),
        error: error instanceof Error ? error.message : "provider stage failed",
        metadata,
      });
      throw error;
    }
  }
}

export type VifuAgentHandler = (
  request: VifuAgentRequest,
  trace: VifuAgentTrace,
) => JsonValue | VifuAgentResponse | Promise<JsonValue | VifuAgentResponse>;

export interface VifuAgentOptions {
  id: string;
  name?: string;
  endpoint?: string;
  providerId?: string;
  capability?: string;
  timeoutMs?: number;
  metadata?: JsonValue;
  handler: VifuAgentHandler;
}

export interface VifuGatewayAgentDefinition {
  id: string;
  name: string;
  endpoint: string;
  providerId: string;
  capability: string;
  timeoutMs: number;
  metadata: JsonValue;
}

export interface VifuInvocationOptions {
  endpoint: string;
  input: JsonValue;
  sessionId?: string;
  metadata?: JsonValue;
}

export interface VifuInvocation {
  invocationId: string;
  appId: string;
  endpoint: string;
  sessionId: string;
  agentId: string;
  providerId: string;
  capability: string;
  output: JsonValue;
  metadata: JsonValue;
  state: JsonValue;
  stateRevision: number;
  trace: VifuTraceStage[];
}

export interface VifuTraceStage {
  name: string;
  status: string;
  durationMs: number;
  attributes: JsonValue;
}

export interface VifuPendingTrace {
  id: string;
  appId: string;
  invocationId: string;
  endpoint: string;
  agent: string;
  provider: string;
  capability: string;
  status: string;
  durationMs: number;
  createdAtMs: number;
}

interface NativeInvocationData {
  format: "json" | "binary";
  value: JsonValue;
}

interface NativeInvocationOutput {
  invocationId: string;
  projectId: string;
  endpoint: string;
  sessionId: string;
  agent: string;
  provider: string;
  capability: string;
  data: NativeInvocationData;
  metadata: JsonValue;
  snapshot: { revision: number; state: JsonValue };
  trace: VifuTraceStage[];
}

interface NativePendingTrace {
  id: string;
  projectId: string;
  invocationId: string;
  endpoint: string;
  agent: string;
  provider: string;
  capability: string;
  status: string;
  durationMs: number;
  createdAtMs: number;
}

interface NativeRuntime {
  readonly appId: string;
  registerProvider(
    providerId: string,
    handler: (request: VifuAgentRequest, emit: NativeTraceEmitter) => ReturnType<VifuAgentHandler>,
  ): void;
  registerAgent(
    agentId: string,
    name: string,
    providerId: string,
    capabilitiesJson: string,
    metadataJson: string,
  ): void;
  registerEndpoint(name: string, agentId: string, capability: string, timeoutMs: bigint): void;
  invoke(
    endpoint: string,
    sessionId: string,
    inputJson: string,
    metadataJson: string,
  ): Promise<string>;
  exportSnapshot(): Uint8Array;
  restoreSnapshot(bytes: Uint8Array): void;
  pendingTraces(limit: number): string;
  acknowledgeTraces(traceIdsJson: string): void;
  free(): void;
}

interface NativeRuntimeConstructor {
  new (appId: string): NativeRuntime;
}

interface NativeModule {
  VifuRuntime: NativeRuntimeConstructor;
}

declare const require: (path: string) => unknown;

function loadNativeModule(): NativeModule {
  return require("./wasm/vifu_wasm.js") as NativeModule;
}

export class VifuRuntime {
  readonly #native: NativeRuntime;
  readonly #agents = new Map<string, VifuGatewayAgentDefinition>();
  readonly #gatewayHandlers = new Map<string, VifuAgentHandler>();

  constructor(appId: string) {
    this.#native = new (loadNativeModule().VifuRuntime)(appId);
  }

  get appId(): string {
    return this.#native.appId;
  }

  agent(options: VifuAgentOptions): this {
    const providerId = options.providerId ?? `${options.id}-provider`;
    const capability = options.capability ?? "chat";
    const endpoint = options.endpoint ?? options.id;
    const timeoutMs = options.timeoutMs ?? 30_000;
    const handler = async (request: VifuAgentRequest, emit: NativeTraceEmitter) => {
      const response = await options.handler(request, new VifuAgentTrace(emit));
      return isAgentResponse(response) ? response : { output: response };
    };

    this.#native.registerProvider(providerId, handler);
    this.#native.registerAgent(
      options.id,
      options.name ?? options.id,
      providerId,
      JSON.stringify([capability]),
      JSON.stringify(options.metadata ?? {}),
    );
    this.#native.registerEndpoint(endpoint, options.id, capability, BigInt(timeoutMs));
    this.#agents.set(options.id, {
      id: options.id,
      name: options.name ?? options.id,
      endpoint,
      providerId,
      capability,
      timeoutMs,
      metadata: options.metadata ?? {},
    });
    this.#gatewayHandlers.set(endpoint, options.handler);
    return this;
  }

  async invoke(options: VifuInvocationOptions): Promise<VifuInvocation> {
    const encoded = await this.#native.invoke(
      options.endpoint,
      options.sessionId ?? "default",
      JSON.stringify(options.input),
      JSON.stringify(options.metadata ?? {}),
    );
    return toInvocation(JSON.parse(encoded) as NativeInvocationOutput);
  }

  exportSnapshot(): Uint8Array {
    return this.#native.exportSnapshot();
  }

  restoreSnapshot(bytes: Uint8Array): void {
    this.#native.restoreSnapshot(bytes);
  }

  pendingTraces(limit = 100): VifuPendingTrace[] {
    const traces = JSON.parse(this.#native.pendingTraces(limit)) as NativePendingTrace[];
    return traces.map((trace) => ({
      id: trace.id,
      appId: trace.projectId,
      invocationId: trace.invocationId,
      endpoint: trace.endpoint,
      agent: trace.agent,
      provider: trace.provider,
      capability: trace.capability,
      status: trace.status,
      durationMs: trace.durationMs,
      createdAtMs: trace.createdAtMs,
    }));
  }

  acknowledgeTraces(traceIds: string[]): void {
    this.#native.acknowledgeTraces(JSON.stringify(traceIds));
  }

  close(): void {
    this.#native.free();
  }

  gatewayAgentDefinitions(): VifuGatewayAgentDefinition[] {
    return [...this.#agents.values()];
  }

  async invokeFromGateway(
    request: VifuAgentRequest,
    emit: NativeTraceEmitter,
  ): Promise<VifuAgentResponse> {
    const handler = this.#gatewayHandlers.get(request.endpoint);
    if (!handler) throw new Error(`Gateway requested an unknown endpoint: ${request.endpoint}`);
    const response = await handler(request, new VifuAgentTrace(emit));
    return isAgentResponse(response) ? response : { output: response };
  }

  async connect(
    pairingCode?: string,
    options: import("./gateway.js").VifuGatewayOptions = {},
  ): Promise<import("./gateway.js").VifuGateway> {
    const { VifuGateway } = await import("./gateway.js");
    return VifuGateway.connect(this, pairingCode, options);
  }
}

function isAgentResponse(value: JsonValue | VifuAgentResponse): value is VifuAgentResponse {
  return typeof value === "object" && value !== null && !Array.isArray(value) && "output" in value;
}

function toInvocation(output: NativeInvocationOutput): VifuInvocation {
  return {
    invocationId: output.invocationId,
    appId: output.projectId,
    endpoint: output.endpoint,
    sessionId: output.sessionId,
    agentId: output.agent,
    providerId: output.provider,
    capability: output.capability,
    output: output.data.value,
    metadata: output.metadata,
    state: output.snapshot.state,
    stateRevision: output.snapshot.revision,
    trace: output.trace,
  };
}

export { GatewayPairing, VifuGateway } from "./gateway.js";
export type {
  VifuGatewayOptions,
  VifuGatewayState,
  VifuGatewayStatus,
} from "./gateway.js";
export { VifuServer } from "./server.js";
export type { VifuServerOptions } from "./server.js";
