import { NextResponse } from "next/server";
import { AuthorityError, resolveAuthority } from "../../../../lib/authority";
import { VifuHttpError } from "../../../../lib/deployment-client";

const ALLOWED_ROOTS = new Set([
  "projects",
  "profiles",
  "bindings",
  "endpoints",
  "api-keys",
  "agent-gateways",
  "traces",
]);
const MAX_BODY_BYTES = 1024 * 1024;

type RuntimeRouteContext = {
  params: Promise<{ path: string[] }>;
};

export const dynamic = "force-dynamic";

export async function GET(request: Request, context: RuntimeRouteContext): Promise<Response> {
  return proxyRuntimeRequest(request, context, "GET");
}

export async function POST(request: Request, context: RuntimeRouteContext): Promise<Response> {
  return proxyRuntimeRequest(request, context, "POST");
}

export async function PATCH(request: Request, context: RuntimeRouteContext): Promise<Response> {
  return proxyRuntimeRequest(request, context, "PATCH");
}

export async function DELETE(request: Request, context: RuntimeRouteContext): Promise<Response> {
  return proxyRuntimeRequest(request, context, "DELETE");
}

async function proxyRuntimeRequest(
  request: Request,
  context: RuntimeRouteContext,
  method: "GET" | "POST" | "PATCH" | "DELETE",
): Promise<Response> {
  try {
    const { path } = await context.params;
    if (!isAllowedPath(path)) return errorResponse(404, "NOT_FOUND", "Resource not found.");
    const body = method === "GET" || method === "DELETE" ? undefined : await readBody(request);
    const authority = await resolveAuthority({ redirectToLogin: false });
    const query = new URL(request.url).search;
    const response = await authority.deployment.request<unknown>(
      `/v1/${path.map(encodeURIComponent).join("/")}${query}`,
      { method, body },
    );
    return response === undefined
      ? new Response(null, { status: 204 })
      : NextResponse.json(response, {
        status: method === "POST" && path[2] !== "invoke" ? 201 : 200,
      });
  } catch (error) {
    if (error instanceof AuthorityError || error instanceof VifuHttpError) {
      return errorResponse(error.status, "RUNTIME_REQUEST_FAILED", error.message);
    }
    return errorResponse(
      error instanceof BodyError ? error.status : 500,
      "RUNTIME_REQUEST_FAILED",
      error instanceof Error ? error.message : "Runtime request failed.",
    );
  }
}

async function readBody(request: Request): Promise<ArrayBuffer> {
  const declaredLength = Number(request.headers.get("content-length") ?? 0);
  if (Number.isFinite(declaredLength) && declaredLength > MAX_BODY_BYTES) {
    throw new BodyError(413, "Request body is too large.");
  }
  const body = await request.arrayBuffer();
  if (body.byteLength > MAX_BODY_BYTES) throw new BodyError(413, "Request body is too large.");
  return body;
}

function isAllowedPath(path: string[]): boolean {
  if (path.length < 1 || path.length > 3 || !ALLOWED_ROOTS.has(path[0] ?? "")) return false;
  if (!path.every((segment) => /^[A-Za-z0-9._:-]{1,128}$/.test(segment))) return false;
  if (path.length === 3) return path[0] === "endpoints" && path[2] === "invoke";
  if (path.length === 2 && path[0] === "traces") return false;
  return true;
}

function errorResponse(status: number, code: string, message: string): Response {
  return NextResponse.json({ error: { code, message } }, { status });
}

class BodyError extends Error {
  readonly status: number;

  constructor(status: number, message: string) {
    super(message);
    this.status = status;
  }
}
