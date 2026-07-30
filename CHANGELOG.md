# Changelog

All notable changes to Vifu are documented here.

## 0.1.4 - 2026-07-30

- Made `cargo install vifu` and `cargo run -p vifu` build the complete
  configuration-driven application without a feature flag.
- Established `vifu-runtime` as the direct dependency for embedded Rust
  applications.
- Removed the unused broad application feature matrix; `local-whisper` remains
  the only optional Vifu application capability.

## 0.1.3 - 2026-07-30

- Added embedded SQLite as the default local runtime store while retaining
  PostgreSQL for the complete Compose stack.
- Replaced Dashboard accounts and identity providers with deployment Admin Key
  access and a signed, stateless HttpOnly browser session.
- Added bounded, non-blocking embedded invocation APIs with native provider
  callbacks for Apple and Android hosts.
- Added a GitHub-installable Swift Package with device, simulator, and macOS
  XCFramework slices.
- Added Agent Gateway enrollment lifecycle and revoked-credential handling.
- Kept the default `vifu` crate focused on the embedded Runtime; binary
  installation now opts into the `binary` feature.

## 0.1.1 - 2026-07-26

- Exposed provider and runtime-extension APIs through the public
  `vifu::gateway` module.
- Added crates.io, docs.rs, CI, and Discord links to the repository overview.

## 0.1.0 - 2026-07-26

- Added `vifu-server`, a PostgreSQL-backed Agent Endpoint Runtime with Profile,
  Binding, Endpoint, API key, connection, invocation, and trace APIs.
- Added an authenticated, resumable Agent Gateway WebSocket with logical channels,
  heartbeat, bounded queues, timeout, cancellation, and concurrent calls.
- Added OpenClaw agent discovery and invocation through its OpenAI-compatible
  HTTP API.
- Consolidated local and self-host views into one capability-driven Next.js
  Dashboard with a local/self-host authority path.
- Added self-hosted email/password authentication with bcrypt credentials,
  PostgreSQL-backed opaque sessions, deployment roles, and first-admin setup.
- Kept day-1 authentication local/self-hosted: first-admin bootstrap, local
  email/password sessions, and optional self-host OIDC.
- Added a self-host stack for PostgreSQL, `vifu-server`, the standalone
  Dashboard, and the Agent Gateway.
- Added project API key scope tests and a ten-endpoint concurrency,
  persistence, restart, and Agent Gateway resume E2E gate.
- Adopted Apache-2.0 and documented Vifu trademark use.
