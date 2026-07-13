import { handleLocalAuth } from "../../../../../lib/local-auth-handler";

export const dynamic = "force-dynamic";

export function POST(request: Request): Promise<Response> {
  return handleLocalAuth(request, "login");
}
