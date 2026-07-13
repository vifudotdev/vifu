export class RequestBodyTooLargeError extends Error {}

export function isSameOriginRequest(request: Request): boolean {
  const origin = request.headers.get("origin");
  if (!origin) return true;
  try {
    const originUrl = new URL(origin);
    const requestUrl = new URL(request.url);
    if (originUrl.origin === requestUrl.origin) return true;
    const host = request.headers.get("host")?.trim();
    return !!host
      && !/[\u0000-\u001f\u007f\s/]/.test(host)
      && originUrl.host === host
      && originUrl.protocol === requestUrl.protocol;
  } catch {
    return false;
  }
}

export async function readLimitedBody(request: Request, maxBytes: number): Promise<string> {
  const declared = request.headers.get("content-length");
  if (declared && /^\d+$/.test(declared) && Number(declared) > maxBytes) {
    throw new RequestBodyTooLargeError();
  }
  if (!request.body) return "";
  const reader = request.body.getReader();
  const chunks: Uint8Array[] = [];
  let length = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      length += value.byteLength;
      if (length > maxBytes) {
        await reader.cancel().catch(() => undefined);
        throw new RequestBodyTooLargeError();
      }
      chunks.push(value);
    }
  } finally {
    reader.releaseLock();
  }
  const result = new Uint8Array(length);
  let offset = 0;
  for (const chunk of chunks) {
    result.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return new TextDecoder().decode(result);
}
