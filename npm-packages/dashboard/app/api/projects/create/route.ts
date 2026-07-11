import { NextResponse } from "next/server";
import { createCloudClient } from "../../../../lib/client";
import { readDashboardSession } from "../../../../lib/session";

export const dynamic = "force-dynamic";

export async function POST(request: Request): Promise<Response> {
  const session = await readDashboardSession();
  if (!session?.token) return NextResponse.json({ error: { message: "Sign in before creating a project." } }, { status: 401 });

  const body = await request.json().catch(() => ({})) as Record<string, unknown>;
  const projectName = readString(body.projectName);
  if (!projectName) return NextResponse.json({ error: { message: "Project name is required." } }, { status: 400 });

  const client = createCloudClient(session.token);
  if (!client) return NextResponse.json({ error: { message: "Vifu API base URL is not configured." } }, { status: 503 });

  try {
    const payload = await client.createProject({
      projectName,
      useCase: readString(body.useCase) || undefined,
    });
    return NextResponse.json(payload, { status: 201 });
  } catch (error) {
    return NextResponse.json({
      error: { message: error instanceof Error ? error.message : "Unable to create project." },
    }, { status: 400 });
  }
}

function readString(value: unknown): string {
  return typeof value === "string" && value.trim() ? value.trim().slice(0, 96) : "";
}
