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

For the normal local loop, start the local server, Dashboard, Agent Gateway, and
PostgreSQL together:

```bash
bun run local
```

Open `http://localhost:6791`. Press `Ctrl-C` to stop the local server,
Dashboard, and Agent Gateway. The PostgreSQL container is kept so data survives
between runs.

If the self-host stack is already running, `bun run local` stops only the
self-host app containers and reuses the running self-host PostgreSQL container.
Return to Docker self-host mode with:

```bash
docker compose up -d
```

The lower-level commands remain available when you need to run pieces
independently.

Start only PostgreSQL:

```bash
bun run dev:database
```

This uses `.env.local` and a separate `vifu-local` Docker Compose database
volume.

Start the Rust server and Dashboard in separate terminals:

```bash
bun run dev:server
bun run dev:dashboard
```

Open `http://localhost:6791`. The server listens on
`http://127.0.0.1:6790`.

To run only the local Vifu Agent Gateway:

```bash
mkdir -p ~/.vifu
cat > ~/.vifu/providers.json <<'JSON'
{
  "providers": [
    {
      "key": "openclaw-local",
      "type": "openclaw",
      "url": "http://127.0.0.1:18789",
      "auth": { "token": "replace-with-openclaw-gateway-token" }
    }
  ]
}
JSON
bun run dev:agent-gateway
```

The Gateway must have `gateway.http.endpoints.chatCompletions.enabled` set to
`true`. An intentionally unauthenticated local Gateway does not need the
`auth` block.

For an isolated adapter test, run the included mock on another port and point
the Agent Gateway at it:

```bash
mkdir -p ~/.vifu
cat > ~/.vifu/providers.json <<'JSON'
{
  "providers": [
    {
      "key": "openclaw-mock",
      "type": "openclaw",
      "url": "http://127.0.0.1:18790"
    }
  ]
}
JSON
OPENCLAW_MOCK_PORT=18790 bun run dev:openclaw-mock
bun run dev:agent-gateway
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
cd self-hosted/docker
cp .env.example .env
docker compose build --pull --no-cache
docker compose up -d --wait
curl --fail --silent http://127.0.0.1:6790/health
curl --fail --silent http://127.0.0.1:6790/v1/status
curl --fail --silent http://127.0.0.1:6791/project > /dev/null
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
can target an already-running OpenClaw Gateway instead. If that Gateway requires
auth, set OpenClaw's own `OPENCLAW_GATEWAY_TOKEN` in the shell before running
the test; the harness writes it into a temporary `providers.json`.

```bash
VIFU_E2E_USE_EXISTING_OPENCLAW=1 \
bun run test:self-hosted
```

Stop the stack after testing:

```bash
cd self-hosted/docker
docker compose down --volumes
```

Generated `.next`, `.next-e2e`, `test-results`, screenshots, credentials, and
local environment files must not be committed.
