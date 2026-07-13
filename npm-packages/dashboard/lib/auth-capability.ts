import { configuredAuthCapability } from "./dashboard-auth-config";
import { signupEnabled } from "./dashboard-auth-store";
import type { AuthCapability } from "./runtime-types";

export async function loadAuthCapability(): Promise<AuthCapability> {
  const auth = configuredAuthCapability();
  if (!auth.providers?.some((provider) => provider.kind === "password")) return auth;
  try {
    return { ...auth, signupEnabled: auth.signupEnabled && await signupEnabled() };
  } catch {
    return auth;
  }
}
