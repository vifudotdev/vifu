# Self-hosting Vifu

The included Docker Compose stack runs one Vifu deployment. It requires no Vifu
account and communicates through ordinary HTTP and WebSocket contracts.

## Services

| Service | Role | Default address |
| --- | --- | --- |
| `postgres` | Durable runtime state | `127.0.0.1:5432` |
| `backend` | `vifu-server` HTTP and WebSocket runtime | `127.0.0.1:6790` |
| `dashboard` | Next.js standalone management console | `127.0.0.1:6791` |
| `agent-gateway` | Vifu adapter for external agent providers | internal WebSocket to `backend` |

## Start

From the repository root:

```bash
cd self-hosted/docker
cp .env.example .env
docker compose up -d
```

After that, normal restarts from `self-hosted/docker` are:

```bash
docker compose up -d
```

If you use the Bun workspace from the repository root, the equivalent starter is:

```bash
bun run self-host
```

Open `http://localhost:6791`, create the first local administrator, and verify
the runtime:

```bash
curl --fail --silent http://127.0.0.1:6790/health
curl --fail --silent http://127.0.0.1:6790/v1/status
```

PostgreSQL data is stored in the `postgres_data` Docker volume. Restarting or
recreating containers does not delete users, password credentials, web
sessions, profiles, bindings, endpoints, keys, Agent Gateway sessions, or traces.

## Configuration

The Docker directory follows the usual Compose convention: copy
`.env.example` to `.env` and change values there when needed. Blank authority
values are generated on first startup and stored in the `vifu_secrets` Docker
volume. Existing explicit values in `.env` are imported into that volume so
restores and rotations remain predictable. Compose accepts these values:

| Variable | Purpose |
| --- | --- |
| `VIFU_AUTH_MODE` | Dashboard auth mode label; the included deployment uses `local-password` |
| `AUTH_DISABLE_USERNAME_PASSWORD` | Disables the Dashboard email/password provider when set to `true` |
| `AUTH_DISABLE_SIGNUP` | Disables Dashboard account creation when set to `true` |
| `VIFU_AUTH_PASSWORD_ENABLED` | Enables the built-in Dashboard email/password provider unless set to `false` |
| `VIFU_SIGNUP_ENABLED` | Enables Dashboard signup unless set to `false` |
| `VIFU_ADMIN_KEY` | Optional explicit server-side Dashboard runtime credential, recovery, and automation |
| `VIFU_AGENT_GATEWAY_TOKEN` | Optional explicit Agent Gateway WebSocket authentication value |
| `VIFU_API_KEY_PEPPER` | Optional explicit one-way endpoint key hashing pepper |
| `VIFU_PROVIDER_SECRET_KEY` | Optional explicit server-side provider credential encryption key |
| `DATABASE_URL` | Optional explicit PostgreSQL connection string for Dashboard auth state and `vifu-server` runtime state |
| `VIFU_BIND_HOST` | Host interface for published ports |
| `VIFU_SERVER_PORT` | Published runtime port |
| `VIFU_DASHBOARD_PORT` | Published Dashboard port |
| `VIFU_CONFIG_DIR` | Host-side Vifu config directory; defaults to `~/.vifu` |
| `POSTGRES_DB`, `POSTGRES_USER`, `POSTGRES_PASSWORD` | Included PostgreSQL container |

### OIDC

OIDC is an explicit opt-in Dashboard provider. Configure all required values in
`.env` and recreate the Dashboard container:

```dotenv
AUTH_ENABLE_OIDC=true
VIFU_AUTH_OIDC_ISSUER=https://identity.example.com
VIFU_AUTH_OIDC_CLIENT_ID=replace-with-client-id
VIFU_AUTH_OIDC_CLIENT_SECRET=replace-with-client-secret
VIFU_AUTH_OIDC_REDIRECT_URL=https://dashboard.example.com/api/auth/oidc/oidc/callback
VIFU_AUTH_OIDC_NAME=Continue with Company
VIFU_AUTH_OIDC_BOOTSTRAP_EMAIL=admin@example.com
```

The provider must issue a verified email claim. Vifu uses Authorization Code
with PKCE, validates the ID token issuer, audience, signature, expiry, and nonce,
and stores the resulting Vifu web session as an opaque hash. The bootstrap email
is required only when OIDC will create the first administrator. Set
`VIFU_AUTH_PASSWORD_ENABLED=false` for OIDC-only access after verifying the OIDC
flow. Vifu does not automatically merge an OIDC identity into an existing
password account with the same email.

The three authority secrets must be independent and contain at least 16
characters when set explicitly. When left blank, Compose generates independent
values and persists them in the `vifu_secrets` Docker volume. `VIFU_ADMIN_KEY`
is available only to the Dashboard server and runtime; it is not a browser
session and is not used for daily Dashboard requests. Never give it a
`NEXT_PUBLIC_` name.

The first account is assigned the deployment `admin` role. Later local-password
signups remain open by default and are assigned the deployment `operator` role.
Passwords use bcrypt. Raw web session tokens are sent only in an HttpOnly,
host-only, SameSite=Lax cookie and only a SHA-256 hash is stored in PostgreSQL.
To close account creation, set `AUTH_DISABLE_SIGNUP=true` or
`VIFU_SIGNUP_ENABLED=false` and recreate the Dashboard container.

Day-1 self-host user management is intentionally small: users can sign up, sign
in, and hold deployment roles. Invite flows, member administration, and password
reset/change screens are not included yet.

Vifu depends on PostgreSQL, not a provider-specific API. A PostgreSQL-compatible
managed provider can be used by setting `DATABASE_URL`; the included container
is the default self-host option.

## Agent Gateway

Vifu Agent Gateway is part of the Compose stack. It opens one authenticated
WebSocket to `vifu-server`, discovers agents through configured external
providers, and carries concurrent logical endpoint channels over that
connection.

No external provider is required for the Vifu stack to start. Provider
integrations are optional and documented separately:

See [providers](providers/README.md) for available integrations. OpenClaw, for
example, is started with OpenClaw's own Docker Compose flow and then registered
in Vifu's generic provider registry at `~/.vifu/providers.json`. Provider
directories may be removed without changing Vifu's core self-host stack. When a
provider is configured but not running, the Vifu Agent Gateway stays up and
retries. The Dashboard shows no connected agents until the provider becomes
available.

Remote `VIFU_SERVER_URL` values must use HTTPS. Plain HTTP is accepted only for
loopback development and Docker-internal service names such as `backend`, so
Agent Gateway credentials are not sent over a remote plaintext WebSocket.

## Exposure

All published ports bind to `127.0.0.1` by default. For remote access, keep
PostgreSQL private and place the Dashboard and runtime behind TLS and
request-size limits. Built-in local accounts protect Dashboard operations; the
built-in OIDC adapter or an access proxy can add centralized identity. Set
`VIFU_BIND_HOST` only after those controls are in place.

The bootstrap admin key remains powerful deployment authority. Rotate it and
the Agent Gateway token if either appears in logs, shell history, screenshots, or
browser-visible configuration. Revoke affected web sessions separately.

## Upgrade And Backup

Restart services without deleting the volume:

```bash
docker compose up -d
```

After changing local source or upgrading a checkout, rebuild images first:

```bash
docker compose up -d --build
```

`vifu-server` applies embedded database migrations before accepting traffic.
The Dashboard server owns and upgrades its authentication tables. Back up
PostgreSQL before upgrading across releases. Do not use `down --volumes`
unless you intend to delete all deployment data.
