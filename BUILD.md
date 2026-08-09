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
then build the official Console bundle and start Vifu with one Cargo command:

```bash
bun install --frozen-lockfile
cargo vifu
```

The default build includes the llama.cpp and Local Whisper Providers. Vifu
creates `~/.vifu/config.toml` and `~/.vifu/providers.json`.

The process starts the Server and Gateway on loopback. It also opens the Runtime
TUI in an interactive terminal.

Press `B` to open the local Dashboard. Press `Q` to stop Vifu. Vifu asks for
confirmation if requests, a comparison, or a route override is active.

Vifu stores Runtime and Gateway state in `~/.vifu/runtime.sqlite`. It stores
local Server data in `~/.vifu/vifu.sqlite`.

`cargo vifu` is the repository's complete source-development command. It runs
`bun run build:console` and then `cargo run -p vifu`. Cargo run options are
forwarded, so `cargo vifu --release` starts a release build. Pass Vifu options
after `--`, for example `cargo vifu -- -c server.address=https://192.0.2.10:6790`.

`bun run build:console` compiles the shared React Console into
`target/vifu-console-assets/`. Cargo embeds the files in that directory. Cargo
does not run Bun automatically.

Build the Console again after UI changes. Also build it before you compile a
release binary.

The release workflow verifies this bundle and sets
`VIFU_REQUIRE_CONSOLE_ASSETS=1`, so a release build cannot silently embed the
development fallback page.

To use a Server on another machine, configure explicit Server and Gateway
addresses:

```bash
mkdir -p ~/.vifu
cat > ~/.vifu/config.toml <<'TOML'
[server]
address = "https://runtime.example.com"

[gateway]
address = "http://localhost:6790"
TOML
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

The project owner creates a one-time token through
`POST /v1/project/{slug}/agent-gateway-enrollments`. The Gateway consumes this
token after its first successful authorization.

Vifu stores the Machine identity and Device Token in private Server state. This
state is in `~/.vifu/runtime.sqlite`. On later starts, Vifu reconnects with the
stored identity. It rotates the token when required.

Remove the one-time token file after enrollment. Vifu rejects enrollment tokens
in persistent Runtime configuration. There is no enrollment command-line flag.

For a provider that does not require authentication, omit its `auth` block.
The repository's combined local and Docker configurations use deployment
bootstrap registration and do not require a project enrollment token.

For an isolated adapter test, run the included mock on another port and point
the Agent Gateway at it:

```bash
mkdir -p ~/.vifu
cat > ~/.vifu/config.toml <<'TOML'
[server]
address = "http://127.0.0.1:6790"

[gateway]
address = "http://127.0.0.1:6790"
TOML
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

Run the isolated protocol-level topology matrix with one command:

```bash
scripts/test-topologies.sh
```

It uses real loopback HTTP and WebSocket connections. Each case has independent
temporary files, ports, and a SQLite database. The case also has an independent
Vifu home. See
[Topology protocol live testing](docs/topology-live-testing.md) for its test
matrix, reports, and the separate Docker release gate.

SQLite and PostgreSQL migrations are embedded in the Vifu Server role and run
at startup. The Dashboard authenticates against the Runtime Admin Key. It keeps
only a signed, HttpOnly browser session. It has no user or session database.

A deployment can instead configure a trusted external authority. Its Access
Tokens and the Admin Key use the same `Vifu` authorization interface,
then enter the same identity and deployment authorization rules.
SQLx uses runtime-checked queries. SQLite lifecycle and restart tests always
run. PostgreSQL integration is mandatory in CI.

## Apple Package

The root `Package.swift` is the public SwiftPM manifest. It combines the tracked
UniFFI Swift wrapper with `VifuMobileFFI.xcframework` from a version-matched
GitHub release.

For local development, the build script also installs the generated artifact at
`Frameworks/VifuMobileFFI.xcframework`. The manifest selects that copy when it
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
Vifu binaries** workflow for the intended commit. Put the checksum from its job
summary in `Package.swift`. The tag run rebuilds the artifact in
the same release environment. It verifies the URL and checksum. Then it links
the Swift smoke test and uploads all release files.

Godot hosts consume the separate `VifuGodot` package at
`integrations/godot/apple`. Keeping it outside the root package preserves the
Godot-free `Vifu` and `VifuMobileFFI` dependency graph. Its compatibility and
verification instructions are documented in
[`integrations/godot/apple/README.md`](integrations/godot/apple/README.md).
The VifuGodot product includes the complete Vifu Agent Runtime, Vifu's
maintained SwiftGodot forks, and the platform-appropriate prebuilt libgodot
binary. Sibling paths are explicit development overrides for testing
unpublished commits and release assets.

The large libgodot runtime uses a low-frequency maintainer release. Build it
once on a known Apple development machine from an exact public
`vifudotdev/libgodot` commit:

```bash
git clone --branch vifu-4.5 \
  https://github.com/vifudotdev/libgodot.git ../libgodot-release-source
git -C ../libgodot-release-source checkout 235560cb32a5265092f7a35c7b376526cbe12cc5
git -C ../libgodot-release-source submodule update --init --depth 1 godot
scripts/prepare-libgodot-apple-release.sh \
  ../libgodot-release-source \
  libgodot-4.5.1-vifu.1 \
  ../libgodot-release-source/build/vifu-release/libgodot-4.5.1-vifu.1
```

The command builds only the release iOS device, iOS Simulator, and macOS
slices. It does not compile the Godot Editor because this binary release does
not regenerate the separately versioned SwiftGodot API. It then creates
separate deterministic archives, SwiftPM checksums, source metadata, and Godot
notices.

Build output stays under the libgodot checkout and the explicit output
directory. The script removes temporary verification directories automatically.
SCons uses the machine's logical CPU count by default. Set
`LIBGODOT_BUILD_JOBS` to a positive integer when the build machine needs a lower
concurrency limit.

After inspecting those files, create a draft—not a public release—from the Vifu
commit containing the matching verification workflow:

```bash
scripts/create-libgodot-apple-draft.sh \
  ../libgodot-release-source/build/vifu-release/libgodot-4.5.1-vifu.1 \
  libgodot-4.5.1-vifu.1
```

If the VifuGodot manifest follows the binary release, use the release-tool
commit as the third argument. Push this commit before you create the draft. The
draft tag targets this commit instead of the newer local HEAD.

Finally run `.github/workflows/release-libgodot.yml` with the exact libgodot
commit and draft tag. The GitHub macOS job does not compile Godot. It verifies
the complete asset set, public source commits, manifest, checksums, framework
slices, exact architectures, and release markers. With `publish=true`, it makes
the verified draft public. Normal Vifu releases and consuming applications
reuse that pinned artifact rather than recompiling Godot.

The **Release Vifu binaries** workflow publishes the installable VifuGodot
package after the matching Vifu release assets succeed. It exports the exact
`integrations/godot/apple` tree from the Vifu tag, advances
`vifudotdev/VifuGodot` `main`, creates the same semantic tag, and creates the
matching GitHub Release. A rerun verifies and reuses an already-published
snapshot instead of overwriting it.

The workflow uses the release GitHub App credentials configured as
`VIFU_RELEASE_APP_ID` (repository variable or secret) and
`VIFU_RELEASE_APP_PRIVATE_KEY` (repository secret). The App installation must
grant Contents write access to `vifudotdev/VifuGodot`. The workflow scopes its
installation token to that repository. `VIFUGODOT_RELEASE_REPOSITORY` is an
optional repository variable for an intentional distribution-repository move.

The release script rejects personal tokens. GitHub creates the snapshot commit
as the App. The published commit receives the verified bot signature. A
separate commit-signing key is not necessary.

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
curl --fail --silent http://127.0.0.1:6790/project > /dev/null
```

The full Agent Gateway test creates an isolated stack on random loopback ports.
The test also verifies persistence. It removes the stack afterward:

```bash
sh scripts/run-self-hosted-e2e.sh
```

The test creates ten endpoints. It invokes them concurrently through one Agent
Gateway WebSocket. Then it verifies Project Key scopes and traces.

The test restarts the services and verifies PostgreSQL persistence. It also
verifies session resume. Finally, it removes its test resources.

By default the test starts a protocol-compatible fixture. Release verification
can target an already-running OpenClaw Gateway instead. If that Gateway requires
auth, set OpenClaw's own `OPENCLAW_GATEWAY_TOKEN` in the shell before running
the test. The harness writes it into a temporary `providers.json`.

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
