import { NextResponse } from "next/server";
import { AuthorityError, resolveAuthority } from "../../../../lib/authority";
import { VifuHttpError } from "../../../../lib/deployment-client";
import { isSameOriginRequest } from "../../../../lib/request-security";
import { forwardRuntimeResponse } from "../../../../lib/runtime-proxy";

const ALLOWED_ROOTS = new Set([
  "chat",
  "projects",
  "profiles",
  "bindings",
  "endpoints",
  "models",
  "api-keys",
  "agent-gateways",
  "agent-gateway-pairings",
  "project",
  "provider-adapters",
  "provider-catalog",
  "runtime-extensions",
  "traces",
  "status",
  "guest",
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

export async function PUT(request: Request, context: RuntimeRouteContext): Promise<Response> {
  return proxyRuntimeRequest(request, context, "PUT");
}

export async function DELETE(request: Request, context: RuntimeRouteContext): Promise<Response> {
  return proxyRuntimeRequest(request, context, "DELETE");
}

async function proxyRuntimeRequest(
  request: Request,
  context: RuntimeRouteContext,
  method: "GET" | "POST" | "PUT" | "PATCH" | "DELETE",
): Promise<Response> {
  try {
    if (method !== "GET" && !isSameOriginRequest(request)) {
      return errorResponse(403, "INVALID_ORIGIN", "Invalid runtime request origin.");
    }
    const { path } = await context.params;
    if (!isAllowedPath(path)) return errorResponse(404, "NOT_FOUND", "Resource not found.");
    const body = method === "GET" ? undefined : await readBody(request);
    const authority = await resolveAuthority({ redirectToLogin: false });
    const query = new URL(request.url).search;
    const runtimePath = runtimeApiPath(path);
    const headers = new Headers();
    const contentType = request.headers.get("content-type");
    if (contentType) headers.set("content-type", contentType);
    const response = await authority.deployment.rawRequest(
      `${runtimePath}${query}`,
      { method, body, headers },
      false,
      runtimeRequestUsesServerDeadline(path, method) ? null : undefined,
    );
    return forwardRuntimeResponse(response);
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

export function runtimeRequestUsesServerDeadline(
  path: string[],
  method: "GET" | "POST" | "PUT" | "PATCH" | "DELETE",
): boolean {
  if (method !== "POST") return false;
  if (path.length === 2) return path[0] === "chat" && path[1] === "completions";
  if (path[1] !== "v1") return false;
  if (path.length === 3) {
    return ["embeddings", "rpc", "agents"].includes(path[2] ?? "");
  }
  if (path.length !== 4) return false;
  return (path[2] === "chat" && path[3] === "completions")
    || (path[2] === "audio" && ["speech", "transcriptions"].includes(path[3] ?? ""))
    || (path[2] === "realtime" && path[3] === "sessions");
}

async function readBody(request: Request): Promise<ArrayBuffer | undefined> {
  const declaredLength = Number(request.headers.get("content-length") ?? 0);
  if (Number.isFinite(declaredLength) && declaredLength > MAX_BODY_BYTES) {
    throw new BodyError(413, "Request body is too large.");
  }
  const body = await request.arrayBuffer();
  if (body.byteLength > MAX_BODY_BYTES) throw new BodyError(413, "Request body is too large.");
  return body.byteLength > 0 ? body : undefined;
}

function isAllowedPath(path: string[]): boolean {
  if (path.length < 1 || path.length > 8) return false;
  if (!path.every((segment) => /^[A-Za-z0-9._:-]{1,128}$/.test(segment))) return false;
  if (isProjectScopedEndpointPath(path)) return true;
  if (!ALLOWED_ROOTS.has(path[0] ?? "")) return false;
  if (path[0] === "chat") return path.length === 2 && path[1] === "completions";
  if (path[0] === "models") return path.length === 1;
  if (path[0] === "provider-adapters") return path.length === 1;
  if (path[0] === "provider-catalog") return path.length === 1;
  if (path[0] === "guest") return path.length === 2 && path[1] === "claim";
  if (path[0] === "agent-gateway-pairings") {
    return path.length === 1
      || path.length === 2
      || (path.length === 3 && (path[2] === "approve" || path[2] === "reject"));
  }
  if (path[0] === "project") {
    if (path.length < 3) return false;
    if (path[2] === "providers") {
      return path.length === 3
        || path.length === 4
        || (path.length === 5 && path[4] === "test");
    }
    if (path[2] === "agent-candidates") return path.length === 3;
    if (path[2] === "deployments") {
      if (path.length === 3 || path.length === 4) return true;
      if (path.length === 5) {
        return path[4] === "promote" || path[4] === "agent-gateway-enrollments";
      }
      if (path.length === 6 && path[4] === "agent-gateways") return true;
      return path.length === 7
        && path[4] === "runtime-releases"
        && path[6] === "activate";
    }
    if (path[2] === "runtime-releases") return path.length === 3 || path.length === 4;
    if (["bindings", "endpoints"].includes(path[2] ?? "")) {
      return path.length === 3 || path.length === 4;
    }
    if (path[2] === "api-keys") {
      return path.length === 3
        || path.length === 4
        || (path.length === 5 && path[4] === "revoke");
    }
    if (["agent-gateways", "provider-adapters", "provider-catalog"].includes(path[2] ?? "")) {
      return path.length === 3;
    }
    if (path[2] === "traces") {
      return path.length === 3
        || (path.length === 5 && (path[4] === "spans" || path[4] === "scores"));
    }
    if (path[2] === "comparisons") return path.length === 3;
    if (path[2] === "agents") {
      return path.length === 3
        || (path.length === 4 && path[3] === "import")
        || (path.length === 5 && path[4] === "restore");
    }
    if (path[2] === "profiles") {
      if (path.length === 3 || path.length === 4) return true;
      if (path.length === 5) {
        return path[4] === "versions" || path[4] === "rollout" || path[4] === "test";
      }
      if (path.length === 6) return path[4] === "source" && path[5] === "sync";
      return path.length === 7
        && path[4] === "versions"
        && (path[6] === "activate" || path[6] === "archive");
    }
    if (path[2] === "extensions") {
      return path.length === 4 && path[3] === "runtime";
    }
    return false;
  }
  if (path.length === 2 && path[0] === "traces") return false;
  return true;
}

function runtimeApiPath(path: string[]): string {
  if (isProjectScopedEndpointPath(path)) {
    return `/${path.map(encodeURIComponent).join("/")}`;
  }
  return `/v1/${path.map(encodeURIComponent).join("/")}`;
}

function isProjectScopedEndpointPath(path: string[]): boolean {
  if (path[1] !== "v1") return false;
  if (path.length === 3) return path[2] === "models" || path[2] === "embeddings";
  if (path.length === 4 && path[2] === "chat") return path[3] === "completions";
  if (path.length === 4 && path[2] === "audio") {
    return path[3] === "speech" || path[3] === "transcriptions";
  }
  if (path.length === 4 && path[2] === "realtime") {
    return path[3] === "sessions";
  }
  if (path.length === 3 && path[2] === "rpc") return true;
  return path.length === 3 && path[2] === "agents";
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
