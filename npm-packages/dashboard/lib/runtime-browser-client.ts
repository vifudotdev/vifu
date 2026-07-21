export class RuntimeBrowserError extends Error {
  readonly status: number;
  readonly payload: unknown;

  constructor(status: number, message: string, payload: unknown) {
    super(message);
    this.name = "RuntimeBrowserError";
    this.status = status;
    this.payload = payload;
  }
}

export async function runtimeBrowserRequest<T = unknown>(
  path: string,
  method: "GET" | "POST" | "PUT" | "PATCH" | "DELETE" = "GET",
  body?: unknown,
): Promise<T> {
  const response = await fetch(`/api/runtime/${path}`, {
    method,
    headers: body === undefined ? undefined : { "content-type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (response.status === 204) return undefined as T;
  const payload = await response.json().catch(() => null) as T | { error?: unknown } | null;
  if (!response.ok) {
    throw new RuntimeBrowserError(response.status, runtimeBrowserErrorMessage(payload), payload);
  }
  return (payload ?? {}) as T;
}

export async function runtimeBrowserUpload<T = unknown>(
  path: string,
  formData: FormData,
): Promise<T> {
  const response = await fetch(`/api/runtime/${path}`, {
    method: "POST",
    body: formData,
  });
  const payload = await response.json().catch(() => null) as T | { error?: unknown } | null;
  if (!response.ok) {
    throw new RuntimeBrowserError(response.status, runtimeBrowserErrorMessage(payload), payload);
  }
  return (payload ?? {}) as T;
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
