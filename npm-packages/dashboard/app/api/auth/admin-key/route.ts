import { handleAdminKeyLogin } from "../../../../lib/admin-auth-handler";

export const dynamic = "force-dynamic";

export async function POST(request: Request): Promise<Response> {
  return handleAdminKeyLogin(request);
}
