import type { AuthCapability, AuthMode, AuthProvider, AuthProviderKind } from "./runtime-types";

export function authProviders(auth: AuthCapability): AuthProvider[] {
  if (Array.isArray(auth.providers)) return auth.providers.filter(isAuthProvider);
  const fallback = legacyProvider(auth.mode);
  return fallback ? [fallback] : [];
}

export function hasAuthProvider(auth: AuthCapability, kind: AuthProviderKind): boolean {
  return authProviders(auth).some((provider) => provider.kind === kind);
}

export function authRequired(auth: AuthCapability): boolean {
  return auth.required ?? authProviders(auth).length > 0;
}

function legacyProvider(mode: AuthMode): AuthProvider | null {
  if (mode === "local-password") return { id: "password", kind: "password", label: "Email and password" };
  if (mode === "oidc") return { id: "oidc", kind: "oidc", label: "Single sign-on" };
  return null;
}

function isAuthProvider(value: unknown): value is AuthProvider {
  if (!value || typeof value !== "object") return false;
  const provider = value as Partial<AuthProvider>;
  return typeof provider.id === "string"
    && typeof provider.label === "string"
    && (provider.kind === "password" || provider.kind === "oidc");
}
