import { NextResponse } from "next/server";
import { createCloudClient } from "../../../../lib/client";
import { readDashboardSession, resolveDashboardSessionId } from "../../../../lib/session";

export async function POST(request: Request): Promise<Response> {
  const session = await readRequestSession(request);
  if (!session?.token) {
    return NextResponse.json({ error: { code: "SIGNED_OUT", message: "Sign in before completing onboarding." } }, { status: 401 });
  }

  const input = await readPayload(request);
  const displayName = readString(input.displayName);
  const username = readString(input.username) || slugFromName(displayName) || "creator";
  const projectName = readString(input.projectName);
  const useCase = readString(input.useCase);
  const source = readString(input.source);
  const client = createCloudClient(session.token);

  if (!client) return NextResponse.json({ error: { code: "MISSING_API", message: "Vifu API base URL is not configured." } }, { status: 503 });
  if (!displayName || !projectName) {
    return NextResponse.json({ error: { code: "INVALID_ONBOARDING", message: "Username, name, and project are required." } }, { status: 400 });
  }

  try {
    const payload = await client.completeOnboarding({
      displayName,
      username,
      projectName,
      source: source || undefined,
      useCase: useCase || undefined,
      dataCollectionEnabled: input.dataCollectionEnabled !== false,
    });
    return NextResponse.json(payload);
  } catch (error) {
    return NextResponse.json({
      error: {
        code: "ONBOARDING_FAILED",
        message: error instanceof Error ? error.message : "Unable to complete onboarding.",
      },
    }, { status: 400 });
  }
}

async function readRequestSession(request: Request) {
  const dashboardSession = await readDashboardSession();
  if (dashboardSession?.token) return dashboardSession;
  const sharedSessionId = readOpaqueSessionId(request.headers.get("x-vifu-auth-session"));
  return sharedSessionId ? resolveDashboardSessionId(sharedSessionId) : null;
}

function readOpaqueSessionId(value: string | null): string {
  const normalized = value?.trim() ?? "";
  return normalized && !/[\u0000-\u001f\u007f]/.test(normalized) ? normalized : "";
}

async function readPayload(request: Request): Promise<Record<string, unknown>> {
  const contentType = request.headers.get("content-type") ?? "";
  if (contentType.includes("application/json")) {
    return await request.json().catch(() => ({})) as Record<string, unknown>;
  }
  const form = await request.formData();
  return Object.fromEntries(form.entries());
}

function readString(value: unknown): string {
  return typeof value === "string" && value.trim() ? value.trim().slice(0, 96) : "";
}

function slugFromName(value: string): string {
  return value
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 64);
}
