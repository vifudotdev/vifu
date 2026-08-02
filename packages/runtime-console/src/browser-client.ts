export type RuntimeRequestMethod = "GET" | "POST" | "PUT" | "PATCH" | "DELETE";

export class RuntimeBrowserError extends Error {
  readonly status: number;
  readonly payload: unknown;
  readonly retryAfterMilliseconds?: number;

  constructor(
    status: number,
    message: string,
    payload: unknown,
    retryAfterMilliseconds?: number,
  ) {
    super(message);
    this.name = "RuntimeBrowserError";
    this.status = status;
    this.payload = payload;
    this.retryAfterMilliseconds = retryAfterMilliseconds;
  }
}

export type RuntimeBrowserRequest = <T = unknown>(
  path: string,
  method?: RuntimeRequestMethod,
  body?: unknown,
  signal?: AbortSignal,
) => Promise<T>;

export type RuntimeBrowserUpload = <T = unknown>(
  path: string,
  formData: FormData,
  signal?: AbortSignal,
) => Promise<T>;

const RUNTIME_BROWSER_REQUEST_TIMEOUT_MS = 8_000;
const RUNTIME_BROWSER_UPLOAD_TIMEOUT_MS = 60_000;

declare global {
  interface Window {
    __VIFU_RUNTIME_CONSOLE_API_BASE__?: string;
  }
}

export function runtimeConsoleApiBase(): string {
  if (typeof window !== "undefined") {
    const configured = window.__VIFU_RUNTIME_CONSOLE_API_BASE__?.trim();
    if (configured) return trimTrailingSlash(configured);
  }
  return "/api/runtime";
}

export function runtimeConsoleApiUrl(path: string): string {
  const base = runtimeConsoleApiBase();
  const normalized = path.replace(/^\/+/, "");
  return normalized ? `${base}/${normalized}` : base;
}

export const runtimeBrowserRequest: RuntimeBrowserRequest = async <T = unknown>(
  path: string,
  method: RuntimeRequestMethod = "GET",
  body?: unknown,
  signal?: AbortSignal,
): Promise<T> => {
  const response = await fetchWithTimeout(runtimeConsoleApiUrl(path), {
    method,
    headers: body === undefined ? undefined : { "content-type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal,
  }, RUNTIME_BROWSER_REQUEST_TIMEOUT_MS);
  if (response.status === 204) return undefined as T;
  const payload = await response.json().catch(() => null) as T | { error?: unknown } | null;
  if (!response.ok) {
    throw new RuntimeBrowserError(
      response.status,
      runtimeBrowserErrorMessage(payload),
      payload,
      retryAfterMilliseconds(response.headers.get("retry-after")),
    );
  }
  return (payload ?? {}) as T;
};

export const runtimeBrowserUpload: RuntimeBrowserUpload = async <T = unknown>(
  path: string,
  formData: FormData,
  signal?: AbortSignal,
): Promise<T> => {
  const response = await fetchWithTimeout(runtimeConsoleApiUrl(path), {
    method: "POST",
    body: formData,
    signal,
  }, RUNTIME_BROWSER_UPLOAD_TIMEOUT_MS);
  const payload = await response.json().catch(() => null) as T | { error?: unknown } | null;
  if (!response.ok) {
    throw new RuntimeBrowserError(
      response.status,
      runtimeBrowserErrorMessage(payload),
      payload,
      retryAfterMilliseconds(response.headers.get("retry-after")),
    );
  }
  return (payload ?? {}) as T;
};

async function fetchWithTimeout(
  input: RequestInfo | URL,
  init: RequestInit,
  timeoutMs: number,
): Promise<Response> {
  const controller = new AbortController();
  const parentSignal = init.signal;
  let timedOut = false;
  const abortFromParent = () => controller.abort();
  if (parentSignal?.aborted) controller.abort();
  else parentSignal?.addEventListener("abort", abortFromParent, { once: true });
  const timeout = setTimeout(() => {
    timedOut = true;
    controller.abort();
  }, timeoutMs);
  try {
    return await fetch(input, { ...init, signal: controller.signal });
  } catch (error) {
    if (timedOut) {
      throw new RuntimeBrowserError(408, "Runtime request timed out.", null);
    }
    throw error;
  } finally {
    clearTimeout(timeout);
    parentSignal?.removeEventListener("abort", abortFromParent);
  }
}

function runtimeBrowserErrorMessage(payload: unknown): string {
  if (!payload || typeof payload !== "object") return "Runtime request failed.";
  const error = (payload as { error?: unknown }).error;
  if (typeof error === "string" && error.trim()) return error.trim();
  if (error && typeof error === "object") {
    const message = (error as { message?: unknown }).message;
    if (typeof message === "string" && message.trim()) return message.trim();
  }
  return "Runtime request failed.";
}

function retryAfterMilliseconds(value: string | null): number | undefined {
  if (!value) return undefined;
  const seconds = Number(value);
  if (Number.isFinite(seconds) && seconds >= 0) return seconds * 1_000;
  const at = Date.parse(value);
  return Number.isFinite(at) ? Math.max(0, at - Date.now()) : undefined;
}

function trimTrailingSlash(value: string): string {
  return value.length > 1 ? value.replace(/\/+$/, "") : value;
}
