declare const require: (module: string) => unknown;

interface TextReadable {
  setEncoding(encoding: string): void;
  on(event: "data", listener: (chunk: string) => void): void;
}

interface ServerChildProcess {
  readonly stdout: TextReadable;
  readonly stderr: TextReadable;
  readonly exitCode: number | null;
  readonly killed: boolean;
  once(event: "error" | "exit", listener: () => void): void;
  kill(signal: string): void;
}

const { spawn } = require("node:child_process") as {
  spawn(command: string, args: string[], options: { stdio: string[] }): ServerChildProcess;
};

export interface VifuServerOptions {
  executable?: string;
  profile?: string;
  arguments?: string[];
  waitMs?: number;
}

export class VifuServer {
  #output = "";

  private constructor(private readonly process: ServerChildProcess) {
    process.stdout.setEncoding("utf8");
    process.stderr.setEncoding("utf8");
    process.stdout.on("data", (chunk: string) => this.remember(chunk));
    process.stderr.on("data", (chunk: string) => this.remember(chunk));
  }

  static async start(options: VifuServerOptions = {}): Promise<VifuServer> {
    const args = options.arguments ? [...options.arguments] : ["--no-browser"];
    if (!options.arguments && options.profile) args.push("--profile", options.profile);
    const process = spawn(options.executable ?? "vifu", args, {
      stdio: ["ignore", "pipe", "pipe"],
    });
    const server = new VifuServer(process);
    const waitMs = options.waitMs ?? 1_000;
    const result = await Promise.race([
      new Promise<"error" | "exit">((resolve) => {
        process.once("error", () => resolve("error"));
        process.once("exit", () => resolve("exit"));
      }),
      delay(waitMs).then(() => "ready" as const),
    ]);
    if (result !== "ready") {
      throw new Error(server.recentOutput || `Vifu Server could not start (${result})`);
    }
    return server;
  }

  get running(): boolean {
    return this.process.exitCode === null && !this.process.killed;
  }

  get recentOutput(): string {
    return this.#output;
  }

  async close(timeoutMs = 5_000): Promise<void> {
    if (!this.running) return;
    this.process.kill("SIGTERM");
    const exited = await Promise.race([
      new Promise<boolean>((resolve) => this.process.once("exit", () => resolve(true))),
      delay(timeoutMs).then(() => false),
    ]);
    if (!exited) this.process.kill("SIGKILL");
  }

  private remember(chunk: string): void {
    this.#output = `${this.#output}${chunk}`.slice(-16_384);
  }
}

function delay(milliseconds: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}
