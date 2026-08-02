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
- A self-hosted Dashboard verifies the deployment Admin Key, then stores a
  short-lived signed session in an HttpOnly, SameSite cookie. The session is
  stateless, contains no Admin Key, and is invalidated when the key rotates.
- A deployment may also trust signed Access Tokens from an
  external authority configured with an exact issuer, audience, and signing
  key. The runtime converts either credential into an identity, then applies
  the same deployment read/write checks.
- Dashboard management requests send `Authorization: Vifu <credential>`.
  Admin Keys and Access Tokens are authenticated on every HTTP request.
  `/v1/admin/verify` validates the presented credential only; it does not create
  a runtime login session.
- Keep `VIFU_ADMIN_KEY` out of `NEXT_PUBLIC_*`, HTML, browser logs,
  screenshots, client bundles, and source control.
- Project API keys authorize one project and use the OpenAI-compatible `model`
  field to select an exposed agent. A key can follow all current and future
  exposed agents or an explicit binding allowlist. `vifu-server` stores a
  peppered hash and returns the raw value only at creation.
- The Agent Gateway bootstrap token enrolls independent Gateway identities; it
  is not the normal WebSocket credential. Each Gateway stores its own credential
  in permission-restricted local state, while native applications may keep it
  in the platform credential store. The server stores only a peppered hash and
  supports revocation. Request IDs and channel IDs are checked against the
  connection that owns the in-flight call.
- The Agent Gateway accepts only loopback OpenClaw URLs, disables HTTP redirects,
  limits request and response sizes, and does not open a local public listener.
  Provider credentials remain in the operator-controlled provider
  configuration and are not copied into Agent Gateway session records in
  `runtime.sqlite`.
- SQLite or PostgreSQL stores runtime metadata. Restrict database access,
  encrypt backups, and manage retention outside the Dashboard.
- Vifu is designed for local and self-hosted deployments, with Dashboard
  authority and runtime state managed by the operator's deployment.

## Self-hosting

The included Compose ports bind to `127.0.0.1` by default. Before exposing Vifu
beyond a trusted host or private network:

- terminate TLS at a maintained reverse proxy;
- add an access proxy or private-network policy when centralized identity is
  required;
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
