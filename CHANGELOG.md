# Changelog

All notable changes to Vifu are documented here.

## Unreleased

- Added `vifu-server`, a PostgreSQL-backed Agent Endpoint Runtime with Profile,
  Binding, Endpoint, API key, connection, invocation, and trace APIs.
- Added an authenticated, resumable connector WebSocket with logical channels,
  heartbeat, bounded queues, timeout, cancellation, and concurrent calls.
- Added OpenClaw agent discovery and invocation through its OpenAI-compatible
  HTTP API.
- Consolidated Cloud, local, and self-host views into one capability-driven
  Next.js Dashboard with separate authority adapters.
- Added a three-service self-host stack for PostgreSQL, `vifu-server`, and the
  standalone Dashboard.
- Added endpoint key-isolation tests and a ten-endpoint concurrency,
  persistence, restart, and connector-resume E2E gate.
- Adopted Apache-2.0 and documented Vifu trademark use.
