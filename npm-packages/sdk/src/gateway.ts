import type {
  JsonValue,
  VifuGatewayAgentDefinition,
  VifuRuntime,
} from "./index.js";

declare const require: (module: string) => unknown;
declare const __dirname: string;

interface TextReadable {
  setEncoding(encoding: string): void;
  on(event: "data", listener: (chunk: string) => void): void;
}

interface TextWritable {
  readonly destroyed: boolean;
  write(chunk: string): void;
  end(): void;
}

interface GatewayChildProcess {
  readonly stdin: TextWritable;
  readonly stdout: TextReadable;
  readonly stderr: TextReadable;
  readonly exitCode: number | null;
  on(event: "error", listener: (error: Error) => void): void;
  on(
    event: "exit",
    listener: (code: number | null, signal: string | null) => void,
  ): void;
  once(event: "exit", listener: () => void): void;
  kill(signal: string): void;
}

const { spawn } = require("node:child_process") as {
  spawn(command: string, args: string[], options: { stdio: string[] }): GatewayChildProcess;
};
const nodeOs = require("node:os") as {
  homedir(): string;
  platform(): string;
};
const nodePath = require("node:path") as {
  join(...parts: string[]): string;
};

export type VifuGatewayState =
  | "stopped"
  | "connecting"
  | "connected"
  | "reconnecting"
  | "authorizationRequired"
  | "degraded"
  | "failed";

export interface VifuGatewayStatus {
  state: VifuGatewayState;
  lastError?: string;
  gatewayId?: string;
  pairingUrl?: string;
}

export interface VifuGatewayOptions {
  name?: string;
  dataDir?: string;
  metadata?: JsonValue;
  captureTraceContent?: boolean;
  gatewayExecutable?: string;
  gatewayArguments?: string[];
}

export class GatewayPairing {
  private constructor(
    readonly serverUrl: string,
    readonly enrollmentToken: string,
    readonly serverCertificateDer?: number[],
  ) {}

  static parse(code: string): GatewayPairing {
    let parsed: URL;
    try {
      parsed = new URL(code.trim());
    } catch {
      throw new TypeError("pairing code must be a Vifu Gateway pairing link");
    }
    let values: URLSearchParams;
    if (parsed.protocol === "vifu:" && parsed.hostname === "gateway" && parsed.pathname === "/enroll") {
      values = parsed.searchParams;
    } else if (
      parsed.protocol === "https:" &&
      (parsed.hostname === "vifu.ai" || parsed.hostname === "www.vifu.ai") &&
      parsed.pathname === "/pair"
    ) {
      values = new URLSearchParams(parsed.hash.slice(1));
    } else {
      throw new TypeError("pairing code must use vifu://gateway/enroll or https://vifu.ai/pair");
    }
    const serverUrl = requiredPairingValue(values, "server").replace(/\/$/, "");
    const enrollmentToken = requiredPairingValue(values, "token");
    if (!enrollmentToken.startsWith("vifu_ge_")) {
      throw new TypeError("pairing code has an invalid enrollment token");
    }
    const certificate = values.get("certificate");
    return new GatewayPairing(
      serverUrl,
      enrollmentToken,
      certificate ? decodeBase64(certificate) : undefined,
    );
  }
}

export class VifuGateway {
  #status: VifuGatewayStatus = { state: "connecting" };
  #stdout = "";
  #stderr = "";
  #closed = false;
  #started = false;

  private constructor(
    private readonly runtime: VifuRuntime,
    private readonly process: GatewayChildProcess,
  ) {
    process.stdout.setEncoding("utf8");
    process.stdout.on("data", (chunk: string) => this.consumeOutput(chunk));
    process.stderr.setEncoding("utf8");
    process.stderr.on("data", (chunk: string) => {
      this.#stderr = `${this.#stderr}${chunk}`.slice(-8_192);
    });
    process.on("error", (error) => {
      this.#status = { state: "failed", lastError: error.message };
    });
    process.on("exit", (code: number | null, signal: string | null) => {
      if (!this.#closed && this.#status.state !== "failed") {
        const detail = this.#stderr.trim();
        this.#status = {
          state: "failed",
          lastError: detail || `Gateway host stopped (${signal ?? code ?? "unknown"})`,
        };
      } else if (this.#closed) {
        this.#status = { state: "stopped", gatewayId: this.#status.gatewayId };
      }
    });
  }

  static async connect(
    runtime: VifuRuntime,
    pairingCode?: string,
    options: VifuGatewayOptions = {},
  ): Promise<VifuGateway> {
    const agents = runtime.gatewayAgentDefinitions();
    if (agents.length === 0) {
      throw new Error("register at least one agent before connecting the Gateway");
    }
    const pairing = pairingCode === undefined ? undefined : GatewayPairing.parse(pairingCode);
    const executable = options.gatewayExecutable ?? defaultGatewayExecutable();
    const child = spawn(executable, options.gatewayArguments ?? [], {
      stdio: ["pipe", "pipe", "pipe"],
    });
    const gateway = new VifuGateway(runtime, child);
    gateway.send({
      type: "start",
      appId: runtime.appId,
      dataDir:
        options.dataDir ??
        nodePath.join(nodeOs.homedir(), ".vifu", "sdk", "typescript", runtime.appId),
      serverUrl: pairing?.serverUrl,
      enrollmentToken: pairing?.enrollmentToken,
      serverCertificateDer: pairing?.serverCertificateDer,
      name: options.name,
      metadata: options.metadata ?? {
        platform: nodeOs.platform(),
      },
      captureTraceContent: options.captureTraceContent ?? false,
      agents,
    });
    await gateway.waitUntilStarted();
    return gateway;
  }

  get status(): VifuGatewayStatus {
    return { ...this.#status };
  }

  async waitUntilConnected(timeoutMs = 20_000): Promise<VifuGatewayStatus> {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      if (this.#status.state === "connected") return this.status;
      if (this.#status.state === "authorizationRequired" || this.#status.state === "failed") {
        throw new Error(this.#status.lastError ?? `Gateway is ${this.#status.state}`);
      }
      await delay(50);
    }
    throw new Error(`Gateway did not connect within ${timeoutMs} ms`);
  }

  async close(timeoutMs = 5_000): Promise<void> {
    if (this.#closed) return;
    this.#closed = true;
    this.send({ type: "stop" });
    this.process.stdin.end();
    const exited = await Promise.race([
      new Promise<boolean>((resolve) => this.process.once("exit", () => resolve(true))),
      delay(timeoutMs).then(() => false),
    ]);
    if (!exited) this.process.kill("SIGTERM");
  }

  private async waitUntilStarted(timeoutMs = 5_000): Promise<void> {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      if (this.#status.state === "failed") {
        throw new Error(this.#status.lastError ?? "Gateway host failed to start");
      }
      if (this.process.exitCode !== null) {
        throw new Error(this.#stderr.trim() || "Gateway host stopped during startup");
      }
      if (this.#started || this.#status.state !== "connecting") return;
      await delay(20);
    }
    throw new Error("Gateway host did not start within 5000 ms");
  }

  private consumeOutput(chunk: string): void {
    this.#stdout += chunk;
    while (true) {
      const newline = this.#stdout.indexOf("\n");
      if (newline < 0) return;
      const line = this.#stdout.slice(0, newline);
      this.#stdout = this.#stdout.slice(newline + 1);
      if (line.trim().length === 0) continue;
      let message: GatewayHostMessage;
      try {
        message = JSON.parse(line) as GatewayHostMessage;
      } catch {
        this.#status = { state: "failed", lastError: "Gateway host returned an invalid message" };
        continue;
      }
      if (message.type === "status") {
        this.#status = {
          state: message.state,
          lastError: message.lastError,
          gatewayId: message.gatewayId,
          pairingUrl: message.pairingUrl,
        };
      } else if (message.type === "invoke") {
        void this.handleInvocation(message);
      } else if (message.type === "started") {
        this.#started = true;
      }
    }
  }

  private async handleInvocation(message: GatewayHostInvoke): Promise<void> {
    try {
      const response = await this.runtime.invokeFromGateway(
        message.request,
        (event) => this.send({ type: "trace", id: message.id, event }),
      );
      this.send({
        type: "result",
        id: message.id,
        ok: true,
        output: response.output,
        metadata: response.metadata ?? {},
        state: response.state,
      });
    } catch (error) {
      this.send({
        type: "result",
        id: message.id,
        ok: false,
        error: error instanceof Error ? error.message : "TypeScript provider failed",
      });
    }
  }

  private send(message: JsonValue | Record<string, unknown>): void {
    if (!this.process.stdin.destroyed) {
      this.process.stdin.write(`${JSON.stringify(message)}\n`);
    }
  }
}

interface GatewayHostStatus {
  type: "status";
  state: VifuGatewayState;
  lastError?: string;
  gatewayId?: string;
  pairingUrl?: string;
}

interface GatewayHostInvoke {
  type: "invoke";
  id: string;
  request: import("./index.js").VifuAgentRequest;
}

type GatewayHostMessage = GatewayHostStatus | GatewayHostInvoke | { type: "started" };

function requiredPairingValue(values: URLSearchParams, key: string): string {
  const value = values.get(key)?.trim();
  if (!value) throw new TypeError(`pairing code is missing ${key}`);
  return value;
}

function defaultGatewayExecutable(): string {
  const filename = nodeOs.platform() === "win32" ? "vifu-sdk-gateway.exe" : "vifu-sdk-gateway";
  return nodePath.join(__dirname, "native", filename);
}

function delay(milliseconds: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

function decodeBase64(value: string): number[] {
  const normalized = value.replace(/-/g, "+").replace(/_/g, "/");
  const padding = "=".repeat((4 - (normalized.length % 4)) % 4);
  const decoded = atob(`${normalized}${padding}`);
  return Array.from(decoded, (character) => character.charCodeAt(0));
}
