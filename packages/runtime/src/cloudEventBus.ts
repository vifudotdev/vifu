const FLUSH_INTERVAL_MS = 5_000;
const MAX_BUFFER = 50;
const MAX_BATCH_BYTES = 200 * 1024;

export interface CloudEventLike {
  specversion?: string;
  id?: string;
  source?: string;
  type?: string;
  time?: string;
  data?: unknown;
}

interface RegisteredFrame {
  source: string;
  contentWindow: Window;
}

export type CloudEventSender = (events: CloudEventLike[], context: { reason: FlushReason }) => void | Promise<void>;

export interface BusOptions {
  sendEvents: CloudEventSender;
  warn?: (message: string, error?: unknown) => void;
}

export type EventListener = (event: CloudEventLike) => void;
type FlushReason = "timer" | "overflow" | "stop";

export class CloudEventBus {
  private frames = new Set<RegisteredFrame>();
  private buffer: CloudEventLike[] = [];
  private listeners = new Set<EventListener>();
  private timer: ReturnType<typeof setInterval> | null = null;
  private listening = false;
  private flushing = false;
  private opts: BusOptions;
  private readonly listener = (event: MessageEvent) => this.handleMessage(event);

  constructor(opts: BusOptions) {
    this.opts = opts;
  }

  configure(opts: BusOptions): void {
    this.opts = opts;
  }

  start(): void {
    if (this.listening || typeof window === "undefined") return;
    this.listening = true;
    window.addEventListener("message", this.listener);
    this.timer = setInterval(() => void this.flush("timer"), FLUSH_INTERVAL_MS);
  }

  stop(): void {
    if (!this.listening) return;
    this.listening = false;
    window.removeEventListener("message", this.listener);
    if (this.timer) {
      clearInterval(this.timer);
      this.timer = null;
    }
    void this.flush("stop");
  }

  registerGameIframe(iframe: HTMLIFrameElement, source: string): () => void {
    const contentWindow = iframe.contentWindow;
    if (!contentWindow) return () => undefined;
    const entry: RegisteredFrame = { contentWindow, source };
    this.frames.add(entry);
    return () => this.frames.delete(entry);
  }

  enqueueTrusted(event: CloudEventLike): void {
    if (!isCloudEventShape(event)) return;
    this.buffer.push(event);
    this.notify(event);
    if (this.buffer.length >= MAX_BUFFER) void this.flush("overflow");
  }

  subscribe(listener: EventListener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  private notify(event: CloudEventLike): void {
    for (const fn of this.listeners) {
      try {
        fn(event);
      } catch (error) {
        this.warn("listener threw", error);
      }
    }
  }

  private handleMessage(event: MessageEvent): void {
    const frame = [...this.frames].find((candidate) => candidate.contentWindow === event.source);
    if (!frame) return;
    const data = event.data;
    if (!isCloudEventShape(data)) return;
    const trusted: CloudEventLike = { ...data, source: frame.source };
    this.buffer.push(trusted);
    this.notify(trusted);
    if (this.buffer.length >= MAX_BUFFER) void this.flush("overflow");
  }

  private async flush(reason: FlushReason): Promise<void> {
    if (this.flushing || this.buffer.length === 0) return;
    this.flushing = true;
    try {
      while (this.buffer.length > 0) {
        const chunk = this.takeChunk();
        await this.send(chunk, reason);
      }
    } finally {
      this.flushing = false;
    }
  }

  private takeChunk(): CloudEventLike[] {
    let bytes = 2;
    const chunk: CloudEventLike[] = [];
    while (this.buffer.length > 0) {
      const candidate = this.buffer[0];
      const candidateBytes = JSON.stringify(candidate).length + 1;
      if (chunk.length > 0 && bytes + candidateBytes > MAX_BATCH_BYTES) break;
      chunk.push(this.buffer.shift()!);
      bytes += candidateBytes;
      if (chunk.length >= 100) break;
    }
    return chunk;
  }

  private async send(events: CloudEventLike[], reason: FlushReason): Promise<void> {
    try {
      await this.opts.sendEvents(events, { reason });
    } catch (error) {
      this.warn(`sendEvents failed (reason=${reason})`, error);
    }
  }

  private warn(message: string, error?: unknown): void {
    const warn = this.opts.warn ?? ((text, thrown) => console.warn(`[cloudEventBus] ${text}`, thrown));
    warn(message, error);
  }
}

function isCloudEventShape(value: unknown): value is CloudEventLike {
  if (!value || typeof value !== "object") return false;
  const candidate = value as CloudEventLike;
  return (
    candidate.specversion === "1.0"
    && typeof candidate.id === "string"
    && typeof candidate.type === "string"
  );
}

let singleton: CloudEventBus | null = null;

export function getCloudEventBus(opts: BusOptions): CloudEventBus {
  if (!singleton) {
    singleton = new CloudEventBus(opts);
    singleton.start();
  } else {
    singleton.configure(opts);
  }
  return singleton;
}
