# Security Policy

## Supported Versions

Security fixes target the latest released Vifu version.

## Reporting

Report suspected vulnerabilities privately through GitHub Security Advisories
for `vifudotdev/vifu`. Do not open public issues containing access tokens,
session IDs, admin keys, project API keys, Agent Gateway credentials, provider credentials,
private endpoint URLs, database contents, or sensitive logs.

## Trust Boundaries

- The browser Dashboard is an untrusted client. Every authorization decision is
  enforced by the Dashboard server or runtime that owns the resource.
- Self-hosted passwords are hashed with bcrypt. Opaque web session tokens are
  stored only as SHA-256 hashes with expiry and revocation timestamps.
- A self-hosted Dashboard uses `VIFU_ADMIN_KEY` only on the server side for
  runtime administration, recovery, and automation. The key must never enter
  `NEXT_PUBLIC_*`, HTML, browser logs, screenshots, or client bundles.
- Project API keys authorize one project and use the OpenAI-compatible `model`
  field to select an exposed agent. A key can follow all current and future
  exposed agents or an explicit binding allowlist. `vifu-server` stores a
  peppered hash and returns the raw value only at creation.
- The Agent Gateway bootstrap token enrolls independent Gateway identities; it
  is not the normal WebSocket credential. Each Gateway stores its own credential
  in a permission-restricted local session file, while the server stores only a
  peppered hash and supports revocation. Request IDs and channel IDs are checked
  against the connection that owns the in-flight call.
- The Agent Gateway accepts only loopback OpenClaw URLs, disables HTTP redirects,
  limits request and response sizes, and does not open a local public listener.
  Provider credentials remain in the operator-controlled provider
  configuration and are not copied into the Agent Gateway session file.
- PostgreSQL is the durable source of runtime metadata. Restrict database
  network access, encrypt backups, and manage retention outside the Dashboard.
- Vifu runs locally or self-hosted without a Vifu account.

## Self-hosting

The included Compose ports bind to `127.0.0.1` by default. Before exposing Vifu
beyond a trusted host or private network:

- terminate TLS at a maintained reverse proxy;
- disable built-in signup unless actively adding local users, and add OIDC or
  an access proxy when centralized identity is required;
- keep PostgreSQL unreachable from public networks;
- replace every sample authority value with an independent random secret;
- set request and connection limits appropriate for the deployment;
- establish PostgreSQL backup and restore procedures;
- monitor failed authentication, Agent Gateway churn, backpressure, and timeouts.

Rotate affected credentials immediately if they appear in source control,
shell history, logs, images, browser-visible configuration, or support data.

## Development Requirements

New network surfaces require authentication analysis, bounded input and output,
timeouts, cancellation behavior, tests for cross-resource authorization, and
documentation before release. Run the repository hygiene and E2E checks in
[BUILD.md](BUILD.md) before publishing artifacts.
