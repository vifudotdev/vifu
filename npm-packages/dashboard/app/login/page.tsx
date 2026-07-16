import { redirect } from "next/navigation";
import { AuthScreen } from "../../components/auth-screen";
import { loadAuthCapability } from "../../lib/auth-capability";
import { authProviders, authRequired } from "../../lib/auth-providers";
import { sanitizeReturnTo } from "../../lib/config";

export const dynamic = "force-dynamic";

export default async function LoginPage({ searchParams }: {
  searchParams: Promise<Record<string, string | string[] | undefined>>;
}) {
  const [params, auth] = await Promise.all([searchParams, loadAuthCapability()]);
  const returnTo = sanitizeReturnTo(readParam(params.returnTo) ?? "/project");
  if (!authRequired(auth)) redirect(returnTo);
  return (
    <AuthScreen
      providers={authProviders(auth)}
      intent="login"
      signupEnabled={auth.signupEnabled}
      returnTo={returnTo}
      email={readParam(params.email) ?? undefined}
      error={readParam(params.auth_error) ?? undefined}
    />
  );
}

function readParam(value: string | string[] | undefined): string | null {
  return Array.isArray(value) ? value[0] ?? null : value ?? null;
}
