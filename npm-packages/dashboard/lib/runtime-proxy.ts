const BODYLESS_RESPONSE_STATUSES = new Set([204, 205, 304]);

export async function forwardRuntimeResponse(upstream: Response): Promise<Response> {
  const headers = new Headers({ "cache-control": "no-store" });
  const contentType = upstream.headers.get("content-type");
  if (contentType && !BODYLESS_RESPONSE_STATUSES.has(upstream.status)) {
    headers.set("content-type", contentType);
  }

  const body = BODYLESS_RESPONSE_STATUSES.has(upstream.status)
    ? null
    : await upstream.arrayBuffer();
  return new Response(body, {
    status: upstream.status,
    statusText: upstream.statusText,
    headers,
  });
}
