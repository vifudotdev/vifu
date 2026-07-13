import type { AuthCapability, AuthMode, AuthProvider } from "./runtime-types";
import { configuredDashboardOrigin, normalizeHttpBase } from "./config";

export type OidcProviderConfig = {
  id: string;
  label: string;
  issuer: string;
  clientId: string;
  clientSecret: string;
  redirectUrl: string;
  scopes: string[];
  bootstrapEmail: string | null;
};

export function configuredAuthCapability(): AuthCapability {
  const mode = configuredAuthMode();
  const providers = configuredAuthProviders(mode);
  return {
    required: mode !== "none",
    mode,
    signupEnabled: signupEnabled(mode),
    providers,
  };
}

export function configuredAuthMode(): AuthMode {
  const raw = normalized(process.env.VIFU_AUTH_MODE ?? process.env.AUTH_MODE);
  if (raw === "none" || raw === "local-password" || raw === "oidc") return raw;

  const deploymentMode = normalized(process.env.VIFU_DEPLOYMENT_MODE);
  if (deploymentMode === "self-hosted") return "local-password";
  return "none";
}

export function passwordAuthEnabled(): boolean {
  if (process.env.AUTH_DISABLE_USERNAME_PASSWORD === "true") return false;
  if (process.env.VIFU_AUTH_PASSWORD_ENABLED === "false") return false;
  return true;
}

function configuredAuthProviders(mode: AuthMode): AuthProvider[] {
  if (mode === "none") return [];

  const providers: AuthProvider[] = [];
  if (passwordAuthEnabled()) {
    providers.push({ id: "password", kind: "password", label: "Email and password" });
  }
  const oidc = dashboardOidcEnabled() ? configuredOidcProvider() : null;
  if (oidc) providers.push(oidc);
  return providers;
}

function configuredOidcProvider(): AuthProvider | null {
  const config = configuredOidcProviderConfig();
  if (!config) return null;
  return {
    id: config.id,
    kind: "oidc",
    label: config.label,
  };
}

export function configuredOidcProviderConfig(providerId?: string, requestUrl?: string): OidcProviderConfig | null {
  const issuer = normalizeIssuer(process.env.AUTH_CUSTOM_ISSUER ?? process.env.VIFU_AUTH_OIDC_ISSUER);
  const clientId = normalized(process.env.AUTH_CUSTOM_CLIENT_ID ?? process.env.VIFU_AUTH_OIDC_CLIENT_ID);
  const clientSecret = normalized(process.env.AUTH_CUSTOM_CLIENT_SECRET ?? process.env.VIFU_AUTH_OIDC_CLIENT_SECRET);
  if (!issuer || !clientId || !clientSecret) return null;
  const id = normalized(process.env.AUTH_CUSTOM_ID ?? process.env.VIFU_AUTH_OIDC_ID) || "oidc";
  if (!/^[a-z0-9-]{1,48}$/.test(id)) return null;
  if (providerId && providerId !== id) return null;
  const redirectUrl = configuredOidcRedirectUrl(id, requestUrl);
  if (!redirectUrl) return null;
  const label = normalized(process.env.AUTH_CUSTOM_NAME ?? process.env.VIFU_AUTH_OIDC_NAME) || "Single sign-on";
  if (label.length > 80 || /[\u0000-\u001f\u007f]/.test(label)) return null;
  const scopes = configuredOidcScopes();
  if (!scopes.includes("openid")) return null;
  return {
    id,
    label,
    issuer,
    clientId,
    clientSecret,
    redirectUrl,
    scopes,
    bootstrapEmail: normalized(process.env.AUTH_CUSTOM_BOOTSTRAP_EMAIL ?? process.env.VIFU_AUTH_OIDC_BOOTSTRAP_EMAIL)?.toLowerCase() ?? null,
  };
}

function dashboardOidcEnabled(): boolean {
  return process.env.AUTH_ENABLE_OIDC === "true";
}

function signupEnabled(mode: AuthMode): boolean {
  if (mode === "none") return false;
  if (process.env.AUTH_DISABLE_SIGNUP === "true") return false;
  if (process.env.VIFU_SIGNUP_ENABLED === "false") return false;
  return true;
}

function normalized(value: string | undefined): string | null {
  const trimmed = value?.trim();
  return trimmed || null;
}

function configuredOidcRedirectUrl(providerId: string, requestUrl?: string): string | null {
  const explicit = normalizeHttpBase(process.env.AUTH_CUSTOM_REDIRECT_URL ?? process.env.VIFU_AUTH_OIDC_REDIRECT_URL);
  if (explicit) return explicit;
  if (!requestUrl) return null;
  const origin = configuredDashboardOrigin(requestUrl);
  return origin ? `${origin}/api/auth/oidc/${encodeURIComponent(providerId)}/callback` : null;
}

function configuredOidcScopes(): string[] {
  const raw = normalized(process.env.AUTH_CUSTOM_SCOPE ?? process.env.VIFU_AUTH_OIDC_SCOPES) ?? "openid profile email";
  return raw.split(/\s+/).filter(Boolean).slice(0, 16);
}

function normalizeIssuer(value: string | undefined): string | null {
  const base = normalizeHttpBase(value);
  if (!base) return null;
  const url = new URL(base);
  const isLoopback = url.protocol === "http:"
    && (url.hostname === "localhost" || url.hostname === "127.0.0.1" || url.hostname === "::1");
  if (url.protocol !== "https:" && !isLoopback) return null;
  return `${base}/`;
}
