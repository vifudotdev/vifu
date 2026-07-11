# Security Policy

## Supported Versions

Security fixes target the latest released Vifu version.

## Reporting

Report suspected vulnerabilities privately through GitHub Security Advisories
for `vifudotdev/vifu`. Do not open public issues containing access tokens,
session IDs, admin keys, endpoint keys, connector tokens, provider credentials,
private endpoint URLs, database contents, or sensitive logs.

## Trust Boundaries

- The Dashboard is an untrusted client. Every authorization decision is
  enforced by the runtime or account service that owns the resource.
- A self-hosted Dashboard uses `VIFU_ADMIN_KEY` only on its server side. The key
  must never enter `NEXT_PUBLIC_*`, HTML, browser logs, screenshots, or client
  bundles.
- Endpoint API keys authorize exactly one endpoint. `vifu-server` stores a
  peppered hash and returns the raw value only at creation.
- Connector WebSockets require a bearer token. Request IDs and channel IDs are
  checked against the connection that owns the in-flight call.
- The connector accepts only loopback OpenClaw URLs, disables HTTP redirects,
  limits request and response sizes, and does not open a local public listener.
  Its Gateway token is read from the environment and is never persisted in the
  connector session file.
- PostgreSQL is the durable source of runtime metadata. Restrict database
  network access, encrypt backups, and manage retention outside the Dashboard.
- Official account sessions, team authority, billing, provisioning, and managed
  domains are server-side services. Their client contracts are not authority.

## Self-hosting

The included Compose ports bind to `127.0.0.1` by default. Before exposing Vifu
beyond a trusted host or private network:

- terminate TLS at a maintained reverse proxy;
- put the Dashboard behind an external identity layer;
- keep PostgreSQL unreachable from public networks;
- replace every sample authority value with an independent random secret;
- set request and connection limits appropriate for the deployment;
- establish PostgreSQL backup and restore procedures;
- monitor failed authentication, connector churn, backpressure, and timeouts.

Rotate affected credentials immediately if they appear in source control,
shell history, logs, images, browser-visible configuration, or support data.

## Development Requirements

New network surfaces require authentication analysis, bounded input and output,
timeouts, cancellation behavior, tests for cross-resource authorization, and
documentation before release. Run the repository hygiene and E2E checks in
[BUILD.md](BUILD.md) before publishing artifacts.
