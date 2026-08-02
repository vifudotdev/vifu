# Building From Source

Run each service from its owning directory. Docker Compose commands run from the
repository root because the Compose file lives there.

## Prerequisites

- Rust 1.95 or newer
- CMake, a C/C++ compiler, and libclang
- Bun 1.3.9 and Node.js 22
- Docker with Compose v2 for the self-host stack

Building the Apple XCFramework additionally requires Xcode 15 or newer.

Install the native build tools for your operating system before the first Cargo
build. The copy-pasteable commands and `LIBCLANG_PATH` troubleshooting are in
[Install native build dependencies](docs/install.md#install-native-build-dependencies).

## Source Development

The first run needs no Vifu configuration. Install the workspace dependencies,
build the official Console bundle, then start Vifu:

```bash
bun install --frozen-lockfile
bun run build:console
cargo run -p vifu
```

The default build includes the llama.cpp and Local Whisper Providers. Vifu
creates `~/.vifu/config.json` and `~/.vifu/providers.json`, starts both roles
on loopback, opens the local Dashboard, and keeps running until the user presses
`Ctrl-C`. Runtime and Gateway state is stored in `~/.vifu/runtime.sqlite`;
local Server data is stored in `~/.vifu/vifu.sqlite`.

`bun run build:console` compiles the shared React Console into
`target/vifu-console-assets/`. Cargo embeds the files already present in that
directory; it does not run Bun automatically. Re-run the Console build after UI
changes and before compiling release binaries.

The release workflow verifies this bundle and sets
`VIFU_REQUIRE_CONSOLE_ASSETS=1`, so a release build cannot silently embed the
development fallback page.

To run a Gateway-only process on a machine that already has a Server, replace
the generated runtime configuration with a gateway-only configuration:

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
VIFU_AGENT_GATEWAY_ENROLLMENT_TOKEN_FILE=/secure/path/to/one-time-token cargo run
```

The project owner issues the one-time enrollment token through
`POST /v1/project/{slug}/agent-gateway-enrollments`. Gateway consumes it on its
first successful authorization. Vifu keeps one stable Machine identity and the
Server-issued Device Token in private, server-scoped state inside
`~/.vifu/runtime.sqlite`; later starts reconnect automatically and rotate the
token when required. Remove the one-time token file after enrollment. There is
no enrollment command-line flag, and enrollment tokens are rejected in
persistent Vifu runtime configuration.

For a provider that does not require authentication, omit its `auth` block.
The repository's combined local and Docker configurations use deployment
bootstrap registration and do not require a project enrollment token.

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
cargo build --release --locked -p vifu --all-features
```

It is written to `target/release/vifu`. The binary uses embedded SQLite unless
its runtime configuration provides an explicit PostgreSQL or SQLite database
URL.

```bash
cargo fmt --all -- --check
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace
cargo build -p vifu
```

SQLite and PostgreSQL migrations are embedded in the Vifu Server role and run
at startup. The Dashboard authenticates against the runtime Admin Key and keeps
only a signed, HttpOnly browser session; it has no user or session database.
Deployments may instead configure a trusted external authority. Its Access
Tokens and the Admin Key use the same `Vifu` authorization interface,
then enter the same identity and deployment-operation checks.
SQLx uses runtime-checked queries. SQLite lifecycle and restart tests always
run; PostgreSQL integration is mandatory in CI.

## Apple Package

The root `Package.swift` is the public SwiftPM manifest. It combines the tracked
UniFFI Swift wrapper with `VifuMobileFFI.xcframework` from a version-matched
GitHub release.

For local development, the build script also installs the generated artifact at
`Frameworks/VifuMobileFFI.xcframework`; the manifest selects that copy when it
exists. `VIFU_SWIFT_LOCAL_ARTIFACT` can point SwiftPM at another generated copy.

```bash
rustup target add \
  aarch64-apple-ios \
  aarch64-apple-ios-sim \
  x86_64-apple-ios \
  x86_64-apple-darwin
scripts/build-apple-package.sh
```

The script prints a local SwiftPM checksum. Before tagging, run the **Release
Vifu binaries** workflow manually for the intended commit and use the checksum
from its job summary in `Package.swift`. The tag run rebuilds the artifact on
the same release environment, verifies its URL and checksum, links the Swift
smoke test, and uploads it with the other release files.

## Dashboard

```bash
bun run check
bun run test
bun run build
bun run test:e2e
```

`bun run check` enforces the one-Dashboard boundary, provider-neutral HTTP
contracts, public-repository hygiene, and TypeScript correctness. Unit tests
cover Console data contracts, proxy policy, protocol, and SDK contracts.
Browser tests cover Admin Key validation, sidebar session persistence across
service restarts, key non-disclosure, and signout.

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
WebSocket, verifies Project Key scopes and traces, restarts the services,
verifies PostgreSQL persistence and session resume, then removes its test
resources.

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
