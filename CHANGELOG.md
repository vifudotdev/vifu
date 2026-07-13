# Changelog

All notable changes to Vifu are documented here.

## Unreleased

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
- Added a three-service self-host stack for PostgreSQL, `vifu-server`, and the
  standalone Dashboard.
- Added endpoint key-isolation tests and a ten-endpoint concurrency,
  persistence, restart, and Agent Gateway resume E2E gate.
- Adopted Apache-2.0 and documented Vifu trademark use.
