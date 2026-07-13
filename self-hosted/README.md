# Self-hosting Vifu

The included Docker Compose stack runs one Vifu deployment. It requires no Vifu
account and communicates through ordinary HTTP and WebSocket contracts.

## Services

| Service | Role | Default address |
| --- | --- | --- |
| `postgres` | Durable runtime state | `127.0.0.1:5432` |
| `backend` | `vifu-server` HTTP and WebSocket runtime | `127.0.0.1:6790` |
| `dashboard` | Next.js standalone management console | `127.0.0.1:6791` |

## Start

From the repository root:

```bash
sh scripts/init-self-hosted.sh
docker compose -f self-hosted/docker/docker-compose.yml up --build --wait
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

The initialization script creates an untracked, mode `0600` `.env` file with
independent random authority values. It refuses to overwrite an existing file.
Compose accepts these values:

| Variable | Purpose |
| --- | --- |
| `VIFU_AUTH_MODE` | Dashboard auth mode label; the included deployment uses `local-password` |
| `AUTH_DISABLE_USERNAME_PASSWORD` | Disables the Dashboard email/password provider when set to `true` |
| `AUTH_DISABLE_SIGNUP` | Disables Dashboard account creation when set to `true` |
| `VIFU_AUTH_PASSWORD_ENABLED` | Enables the built-in Dashboard email/password provider unless set to `false` |
| `VIFU_SIGNUP_ENABLED` | Enables Dashboard signup unless set to `false` |
| `VIFU_ADMIN_KEY` | Server-side Dashboard runtime credential, recovery, and automation |
| `VIFU_AGENT_GATEWAY_TOKEN` | Agent Gateway WebSocket authentication |
| `VIFU_API_KEY_PEPPER` | One-way endpoint key hashing |
| `DATABASE_URL` | PostgreSQL connection string for Dashboard auth state and `vifu-server` runtime state |
| `VIFU_BIND_HOST` | Host interface for published ports |
| `VIFU_SERVER_PORT` | Published runtime port |
| `VIFU_DASHBOARD_PORT` | Published Dashboard port |
| `VIFU_PROJECT_DOMAIN` | DNS suffix for stable Project endpoints |
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
characters. Compose refuses to start when they are missing. `VIFU_ADMIN_KEY`
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

Run the Agent Gateway on the machine that can reach the local OpenClaw gateway:

```bash
VIFU_OPENCLAW_TOKEN=replace-with-your-gateway-token \
sh scripts/dev-agent-gateway.sh
```

The Agent Gateway accepts only loopback OpenClaw URLs. It opens one authenticated
WebSocket to `vifu-server`, discovers agents through OpenClaw's enabled
OpenAI-compatible HTTP surface, and carries concurrent logical endpoint
channels over that connection. Gateways configured without authentication can
omit `VIFU_OPENCLAW_TOKEN`.

Remote `VIFU_SERVER_URL` values must use HTTPS. Plain HTTP is accepted only for
loopback development so Agent Gateway credentials are never sent over a remote
plaintext WebSocket.

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

Build new images and recreate services without deleting the volume:

```bash
docker compose -f self-hosted/docker/docker-compose.yml up -d --build --wait
```

`vifu-server` applies embedded database migrations before accepting traffic.
The Dashboard server owns and upgrades its authentication tables. Back up
PostgreSQL before upgrading across releases. Do not use `down --volumes`
unless you intend to delete all deployment data.
