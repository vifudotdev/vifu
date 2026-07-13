const DEFAULT_LOCAL_API_BASE_URL = "http://127.0.0.1:6790";
const DEFAULT_BROWSER_API_BASE_URL = "http://localhost:6790";

export function configuredApiBaseUrl(): string {
  return normalizeHttpBase(process.env.VIFU_API_BASE_URL) ?? DEFAULT_LOCAL_API_BASE_URL;
}

export function configuredBrowserApiBaseUrl(): string {
  return normalizeHttpBase(process.env.NEXT_PUBLIC_VIFU_API_BASE_URL)
    ?? normalizeHttpBase(process.env.NEXT_PUBLIC_DEPLOYMENT_URL)
    ?? DEFAULT_BROWSER_API_BASE_URL;
}

export function configuredProjectDomain(): string {
  return process.env.NEXT_PUBLIC_VIFU_PROJECT_DOMAIN?.trim().replace(/^\.+|\.+$/g, "") || "localhost";
}

export function configuredAdminKey(): string | null {
  const value = process.env.VIFU_ADMIN_KEY?.trim();
  return value || null;
}

export function configuredDashboardOrigin(requestUrl: string): string | null {
  const configured = process.env.VIFU_DASHBOARD_URL?.trim();
  if (configured) return normalizeOrigin(configured);
  return new URL(requestUrl).origin;
}

export function dashboardLoginPath(returnTo?: string): string {
  const url = new URL("/login", "https://dashboard.invalid");
  if (returnTo) url.searchParams.set("returnTo", sanitizeReturnTo(returnTo));
  return `${url.pathname}${url.search}`;
}

export function dashboardSignupPath(returnTo?: string): string {
  const url = new URL("/signup", "https://dashboard.invalid");
  if (returnTo) url.searchParams.set("returnTo", sanitizeReturnTo(returnTo));
  return `${url.pathname}${url.search}`;
}

export function sanitizeReturnTo(value: string | null | undefined): string {
  if (!value || !value.startsWith("/") || value.startsWith("//")) return "/dashboard";
  if (/[\u0000-\u001f\u007f]/.test(value)) return "/dashboard";
  try {
    const base = new URL("https://dashboard.invalid");
    const target = new URL(value, base);
    if (target.origin !== base.origin) return "/dashboard";
    return `${target.pathname}${target.search}${target.hash}`;
  } catch {
    return "/dashboard";
  }
}

export function appendApiPath(apiBaseUrl: string, path: string): string {
  const base = normalizeHttpBase(apiBaseUrl);
  if (!base) throw new Error("Vifu API base URL is not configured.");
  return `${base}${path.startsWith("/") ? path : `/${path}`}`;
}

export function normalizeHttpBase(value: string | null | undefined): string | null {
  const raw = value?.trim();
  if (!raw) return null;
  let url: URL;
  try {
    url = new URL(raw);
  } catch {
    return null;
  }
  if (url.protocol !== "https:" && url.protocol !== "http:") return null;
  if (url.username || url.password || url.search || url.hash) return null;
  return url.toString().replace(/\/+$/, "");
}

function normalizeOrigin(value: string): string | null {
  const normalized = normalizeHttpBase(value);
  if (!normalized) return null;
  const url = new URL(normalized);
  return url.pathname === "/" ? url.origin : null;
}
