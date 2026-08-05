# Changelog

All notable changes to Vifu are documented here.

## 0.1.9 - 2026-08-05

- Rebuilt the Apple Runtime binary with the Xcode 15 toolchain so iOS 17 and
  macOS 14 package consumers link against the supported Metal SDK surface.

## 0.1.8 - 2026-08-05

- Added paired Apple Runtime Gateway connectivity so embedded iOS runtimes can
  keep running independently while Vifu Server monitors and configures them.
- Added the separate VifuGodot Apple distribution with the complete Vifu
  Runtime, in-process bridge, maintained SwiftGodot SDK, and pinned prebuilt
  libgodot binaries for iOS and macOS.
- Added a focused Godot iOS embedding example and aligned the released
  VifuMobileFFI binary with the current enrollment API.

## 0.1.7 - 2026-08-03

- Added the live ARM optimization TUI with hundreds-of-agent monitoring,
  provider-stage traces, operating-system CPU/RSS evidence, and editor export.
- Added bounded local-model combination measurements with repeat statistics,
  contract validation, session route activation, and one-key Undo.
- Added shared lazy llama.cpp model residency, memory admission, and idle-model
  eviction for resource-constrained local inference.
- Added typed application feedback and the StarDojo adapter so model delivery,
  response parsing, action execution, and frame presentation remain distinct.
- Added the embedded Trace Explorer and Comparison History Dashboard to the
  default zero-configuration Vifu startup path.

## 0.1.6 - 2026-08-01

- Added anonymous Gateway bootstrap for servers that enable temporary projects.
- Reconciled discovered agents into projects assigned to their Gateway so a
  newly issued endpoint is immediately usable.
- Kept guest bootstrap responses compatible across Server and Gateway versions
  and deferred runtime sync until initial registration completes.

## 0.1.5 - 2026-08-01

- Added portable Runtime deployments and release selection across the Server,
  Agent Gateway, and Dashboard.
- Added the shared Runtime Bridge for embedded Apple hosts and in-process game
  engine communication.
- Added durable Gateway enrollment, reconnect, resume, and deployment state
  coverage to the self-hosted end-to-end gate.
- Updated the Apple package for the current UniFFI bridge API.

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
