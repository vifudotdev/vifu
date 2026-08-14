import { redirect } from "next/navigation";
import { AuthScreen } from "../../components/auth-screen";
import { hasValidAdminSession } from "../../lib/admin-session";
import { configuredAdminKey, sanitizeReturnTo } from "../../lib/config";

export const dynamic = "force-dynamic";

export default async function LoginPage({ searchParams }: {
  searchParams: Promise<Record<string, string | string[] | undefined>>;
}) {
  const params = await searchParams;
  const returnTo = sanitizeReturnTo(readParam(params.returnTo) ?? "/apps");
  const adminKey = configuredAdminKey();
  if (adminKey && await hasValidAdminSession(adminKey)) redirect(returnTo);
  return (
    <AuthScreen
      returnTo={returnTo}
      error={readParam(params.auth_error) ?? undefined}
      unavailable={!adminKey}
    />
  );
}

function readParam(value: string | string[] | undefined): string | null {
  return Array.isArray(value) ? value[0] ?? null : value ?? null;
}
