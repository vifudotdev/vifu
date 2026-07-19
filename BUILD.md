# Building From Source

Run each service from its owning directory. Docker Compose commands run from the
repository root because the Compose file lives there.

## Prerequisites

- Rust 1.90 or newer
- Bun 1.3.9
- Node.js 22
- Docker with Compose v2

Install Dashboard dependencies:

```bash
bun install --frozen-lockfile
```

Source development uses loopback defaults. Docker self-hosting uses the root
`.env.example` template described below.

## Source Development

For local development, run PostgreSQL, the Vifu runtime, and the Dashboard in
separate terminals. Install Dashboard dependencies once:

```bash
bun install --frozen-lockfile
```

Then start each service directly:

```bash
# Terminal 1: PostgreSQL
docker compose -f docker-compose.yml -f docker-compose.local.yml up -d postgres
```

```bash
# Terminal 2: Vifu Server and Agent Gateway
cd crates/vifu
cargo run
```

```bash
# Terminal 3: Dashboard
cd npm-packages/dashboard
bun dev
```

Open `http://localhost:6791`. The runtime and Dashboard stop with `Ctrl-C`;
the local PostgreSQL container keeps its data. Stop it when needed:

```bash
docker compose -f docker-compose.yml -f docker-compose.local.yml down
```

The local Compose override uses its own `vifu-local` PostgreSQL volume and
trust authentication on the loopback-only development port. The server listens
on `http://127.0.0.1:6790`.

The first `cargo run` creates `~/.vifu/config.json` and
`~/.vifu/providers.json`. Its generated configuration starts both roles on
loopback. To run a Gateway-only process on a machine that already has a Server,
replace the generated runtime configuration with a gateway-only configuration:

```bash
mkdir -p ~/.vifu
cat > ~/.vifu/config.json <<'JSON'
{
  "version": 1,
  "gateway": {
    "serverUrl": "https://runtime.example.com"
  }
}
JSON
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
cd crates/vifu
cargo run
```

For a provider that does not require authentication, omit the `auth` block.

For an isolated adapter test, run the included mock on another port and point
the Agent Gateway at it:

```bash
mkdir -p ~/.vifu
cat > ~/.vifu/config.json <<'JSON'
{
  "version": 1,
  "gateway": {
    "serverUrl": "http://127.0.0.1:6790"
  }
}
JSON
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
OPENCLAW_MOCK_PORT=18790 node scripts/mock-openclaw.mjs
cd crates/vifu
cargo run
```

## Rust Workspace

The Rust workspace produces one runtime executable. Its configuration selects
the Server role, Agent Gateway role, or both:

```bash
cargo build --release --locked -p vifu
```

It is written to `target/release/vifu`. PostgreSQL and the Dashboard are still
required for the complete local console.

```bash
cargo fmt --all -- --check
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace
```

Database migrations are embedded in the Vifu Server role and run at startup. Dashboard
authentication is implemented in the Dashboard server, which also initializes
and upgrades the auth tables it uses. SQLx uses runtime-checked queries. The
database integration test runs when PostgreSQL is available and is mandatory in
CI; compilation and pure unit tests do not require a live database.

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
cp .env.example .env
docker compose build --pull --no-cache
docker compose up -d --wait
curl --fail --silent http://127.0.0.1:6790/health
curl --fail --silent http://127.0.0.1:6790/v1/status
curl --fail --silent http://127.0.0.1:6791/project > /dev/null
```

The full Agent Gateway and persistence test creates an isolated stack on random
loopback ports, exercises it, and removes it afterward:

```bash
sh scripts/run-self-hosted-e2e.sh
```

It creates ten endpoints, invokes them concurrently over one Agent Gateway
WebSocket, verifies Project Key scopes, Canvas exposure, and traces, restarts
the services, verifies PostgreSQL persistence and session resume, then removes
its test resources.

By default the test starts a protocol-compatible fixture. Release verification
can target an already-running OpenClaw Gateway instead. If that Gateway requires
auth, set OpenClaw's own `OPENCLAW_GATEWAY_TOKEN` in the shell before running
the test; the harness writes it into a temporary `providers.json`.

```bash
VIFU_E2E_USE_EXISTING_OPENCLAW=1 \
VIFU_E2E_OPENCLAW_PORT=18789 \
sh scripts/run-self-hosted-e2e.sh
```

Stop the stack after testing:

```bash
docker compose down --volumes
```

Generated `.next`, `.next-e2e`, `test-results`, screenshots, credentials, and
local environment files must not be committed.
