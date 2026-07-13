# Contributing

Vifu welcomes focused changes to the Agent Endpoint Runtime, Dashboard, CLI,
Agent Gateway protocol, PostgreSQL contract, and self-host packaging.

## Architecture

Keep these ownership boundaries explicit:

- `crates/vifu` owns the CLI, loopback OpenClaw adapter, resumable Agent Gateway
  session, and public Agent Gateway protocol.
- `crates/vifu-server` owns HTTP APIs, WebSocket multiplexing, runtime
  authorization, routing, traces, and PostgreSQL migrations.
- `npm-packages/dashboard` is the only Dashboard application. Core views use a
  `DeploymentClient`; local and self-host authority enter through the same
  `AuthorityAdapter` path selected from server capabilities.
- `self-hosted/docker` owns the supported PostgreSQL, server, and standalone
  Dashboard deployment.

The browser is never an authority boundary. Profile, endpoint, key, Agent Gateway,
session, and trace decisions must be enforced by the server that owns them. Do
not expose admin keys, Agent Gateway tokens, provider credentials, or session tokens
through client-visible environment variables.

Public runtime code must remain provider-neutral. Core local and self-host
behavior uses PostgreSQL, HTTP, and WebSocket contracts and must not require an
official Vifu account.

## Product Terminology

Use `Vifu Dashboard`, `Agent Profile`, `Agent Endpoint`, `Agent Runtime`,
`Binding`, and `Vifu Agent Gateway` consistently. Do not introduce a second product
name for functionality already owned by the Dashboard or runtime.

## Pull Requests

Before opening a pull request, run the checks in [BUILD.md](BUILD.md). Include:

- the user-visible behavior and API contract affected;
- the runtime, Dashboard, or authority boundary changed;
- local and self-host behavior tested;
- exact verification commands and remaining test gaps.

Keep generated output, local environment files, screenshots, credentials,
private planning records, infrastructure identifiers, and internal operating
notes out of commits. Security reports belong in private GitHub Security
Advisories, not public issues.

Contributions are accepted under Apache-2.0. Use of Vifu branding remains
subject to [TRADEMARKS.md](TRADEMARKS.md).
