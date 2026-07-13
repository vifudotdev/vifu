import Image from "next/image";
import Link from "next/link";
import type { AuthProvider } from "../lib/runtime-types";

export function AuthScreen({
  providers,
  intent,
  signupEnabled,
  returnTo,
  email,
  error,
  unavailable = false,
}: {
  providers: AuthProvider[];
  intent: "login" | "signup";
  signupEnabled: boolean;
  returnTo: string;
  email?: string;
  error?: string;
  unavailable?: boolean;
}) {
  const isSignup = intent === "signup";
  const password = providers.find((provider) => provider.kind === "password");
  const oidcProviders = providers.filter((provider) => provider.kind === "oidc");

  return (
    <main className="auth-shell">
      <section className="auth-panel" aria-label={isSignup ? "Create your account" : "Sign in to Vifu"}>
        <Link className="auth-brand" href="/" aria-label="Vifu Dashboard">
          <Image src="/brand/vifu-icon-512.png" width={40} height={40} alt="" priority />
          <span>Vifu</span>
        </Link>

        {unavailable ? (
          <div className="auth-form auth-message">
            <p className="auth-eyebrow">Service unavailable</p>
            <h1>Cannot reach Vifu server</h1>
            <p>Vifu could not load the sign-in settings.</p>
            <div className="auth-notice">Check the server address and try again.</div>
          </div>
        ) : (
          <div className="auth-form">
            <h1>{isSignup ? "Create your account" : "Sign in"}</h1>
            <p className="auth-copy">{isSignup ? "Create an account to manage your agents and endpoints." : "Continue to your dashboard."}</p>

            {oidcProviders.map((provider) => (
              <a
                className="auth-provider-button"
                href={`/api/auth/oidc/${encodeURIComponent(provider.id)}/start?returnTo=${encodeURIComponent(returnTo)}`}
                key={provider.id}
              >
                {provider.label}
              </a>
            ))}

            {oidcProviders.length > 0 && password ? <div className="auth-divider"><span>or</span></div> : null}

            {password ? (
              <form action={`/api/auth/local/${intent}`} method="post">
                <input type="hidden" name="returnTo" value={returnTo} />
                {isSignup ? (
                  <label>
                    Display name
                    <input name="displayName" autoComplete="name" maxLength={96} required />
                  </label>
                ) : null}
                <label>
                  Email
                  <input name="email" type="email" autoComplete="email" defaultValue={email} maxLength={320} required />
                </label>
                <label>
                  Password
                  <input
                    name="password"
                    type="password"
                    autoComplete={isSignup ? "new-password" : "current-password"}
                    minLength={12}
                    maxLength={128}
                    required
                  />
                </label>
                <button type="submit">{isSignup ? "Create account" : "Sign in"}</button>
              </form>
            ) : null}

            {error ? <div className="auth-error" role="alert">{error}</div> : null}
          </div>
        )}

        {!unavailable ? <footer className="auth-footer">
          {isSignup ? (
            <span>Already registered? <Link href="/login">Sign in</Link></span>
          ) : signupEnabled ? (
            <span>Need an account? <Link href="/signup">Sign up</Link></span>
          ) : (
            <span>Account creation is disabled for this deployment.</span>
          )}
        </footer> : null}
      </section>
    </main>
  );
}
