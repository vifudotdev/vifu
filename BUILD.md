# Building From Source

Run commands from the repository root.

## Prerequisites

- Rust 1.90 or newer
- Bun 1.3.9
- Node.js 22
- Docker with Compose v2

Install Dashboard dependencies and create local configuration:

```bash
bun install --frozen-lockfile
cp .env.example .env.local
```

The sample values are for loopback development only. Replace all authority
values before exposing a deployment.

## Source Development

Start PostgreSQL:

```bash
bun run dev:database
```

Start the Rust server and Dashboard in separate terminals:

```bash
bun run dev:server
bun run dev:dashboard
```

Open `http://localhost:6791`. The server listens on
`http://127.0.0.1:6790`.

To connect a local OpenClaw gateway:

```bash
VIFU_OPENCLAW_TOKEN="$OPENCLAW_GATEWAY_TOKEN" bun run dev:agent-gateway
```

The Gateway must have `gateway.http.endpoints.chatCompletions.enabled` set to
`true`. An intentionally unauthenticated local Gateway does not need the token
variable.

For an isolated adapter test, run the included mock on another port and point
the Agent Gateway at it:

```bash
OPENCLAW_MOCK_PORT=18790 bun run dev:openclaw-mock
VIFU_OPENCLAW_URL=http://127.0.0.1:18790 bun run dev:agent-gateway
```

## Rust Workspace

```bash
cargo fmt --all -- --check
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace
```

Database migrations are embedded in `vifu-server` and run at startup. Dashboard
authentication is implemented in the Dashboard server, which also initializes
and upgrades the auth tables it uses. SQLx uses runtime-checked queries, so a
live database is not required for normal compilation or unit tests.

## Dashboard

```bash
bun run check
bun run build
bun run test:e2e
```

`bun run check` enforces the one-Dashboard boundary, provider-neutral HTTP
contracts, public-repository hygiene, and TypeScript correctness. Browser tests
cover self-host login, first-admin bootstrap, open signup for additional users,
sidebar session persistence, and signout.

## Clean Docker Verification

```bash
sh scripts/init-self-hosted.sh
docker compose -f self-hosted/docker/docker-compose.yml build --pull --no-cache
docker compose -f self-hosted/docker/docker-compose.yml up -d --wait
curl --fail --silent http://127.0.0.1:6790/health
curl --fail --silent http://127.0.0.1:6790/v1/status
curl --fail --silent http://127.0.0.1:6791/dashboard > /dev/null
```

With the stack running, the full Agent Gateway and persistence test is:

```bash
bun run test:self-hosted
```

It creates ten endpoints, invokes them concurrently over one Agent Gateway
WebSocket, verifies endpoint key isolation and traces, restarts all three
services, verifies PostgreSQL persistence and session resume, then removes its
test resources.

By default the test starts a protocol-compatible fixture. Release verification
can target an already-running OpenClaw Gateway instead:

```bash
VIFU_E2E_USE_EXISTING_OPENCLAW=1 \
VIFU_OPENCLAW_TOKEN="$OPENCLAW_GATEWAY_TOKEN" \
bun run test:self-hosted
```

Stop the stack after testing:

```bash
docker compose -f self-hosted/docker/docker-compose.yml down --volumes
```

Generated `.next`, `.next-e2e`, `test-results`, screenshots, credentials, and
local environment files must not be committed.
