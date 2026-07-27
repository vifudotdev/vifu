import Image from "next/image";
import Link from "next/link";

export function AuthScreen({
  returnTo,
  error,
  unavailable = false,
}: {
  returnTo: string;
  error?: string;
  unavailable?: boolean;
}) {
  return (
    <main className="auth-shell">
      <section className="auth-panel" aria-label="Connect to Vifu">
        <Link className="auth-brand" href="/" aria-label="Vifu Console">
          <Image src="/brand/vifu-lockup.png" width={100} height={40} alt="Vifu" priority />
        </Link>

        {unavailable ? (
          <div className="auth-form auth-message">
            <p className="auth-eyebrow">Setup required</p>
            <h1>Admin access is not configured</h1>
            <p>Configure an admin key for this deployment, then reload the page.</p>
          </div>
        ) : (
          <div className="auth-form">
            <h1>Connect to Vifu</h1>
            <p className="auth-copy">Enter the admin key for this Vifu deployment.</p>
            <form action="/api/auth/admin-key" method="post">
              <input type="hidden" name="returnTo" value={returnTo} />
              <label>
                Admin key
                <input
                  name="adminKey"
                  type="password"
                  autoComplete="current-password"
                  maxLength={4096}
                  required
                  autoFocus
                />
              </label>
              <button type="submit">Connect</button>
            </form>

            {error ? <div className="auth-error" role="alert">{error}</div> : null}
          </div>
        )}

        {!unavailable ? (
          <footer className="auth-footer">
            <span>Admin keys grant full access to this deployment. Keep yours private.</span>
          </footer>
        ) : null}
      </section>
    </main>
  );
}
