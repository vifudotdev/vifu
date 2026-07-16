# Vifu

Vifu is an open-source endpoint runtime for local AI agents. It provides stable
project endpoints with profiles, bindings, API keys, connection status, and
traces, while external agent providers remain optional integrations.

The runtime, Agent Gateway protocol, PostgreSQL migrations, dashboard, and Docker
deployment are all included in this repository under Apache-2.0.

## Local Demo

Start Vifu:

```bash
cd self-hosted/docker
cp .env.example .env
docker compose up -d
```

After that, restarting the stack from `self-hosted/docker` is just:

```bash
docker compose up -d
```

From the repository root, `bun run self-host` is the same starter for local
development checkouts.

Open `http://localhost:6791` and create the first local administrator. The
account is stored only in this deployment's PostgreSQL database; no Vifu account
or external identity provider is required. Signup stays open by default for
additional local users. The first account receives the deployment `admin` role;
later signups receive the deployment `operator` role. Signup can be disabled
with `AUTH_DISABLE_SIGNUP=true` or `VIFU_SIGNUP_ENABLED=false`.

The Compose stack includes Vifu Agent Gateway, but no external provider is
required for Vifu to start. Provider integrations are optional and live under
`self-hosted/providers/`. If no provider is enabled, Vifu still runs normally
and the Dashboard shows no connected agents.

## Independent Self-hosting

The self-hosted core works without an external identity provider:

```text
application -> vifu-server -> one multiplexed WebSocket -> Vifu Agent Gateway -> provider
                     |
                     +-> PostgreSQL profiles, endpoints, keys, traces

Dashboard -> PostgreSQL users and web sessions
Dashboard -> vifu-server through a server-side runtime credential
```

Applications call endpoint-scoped HTTP invoke APIs. No SDK is required:

```http
POST http://localhost:6790/v1/endpoints/town-guide/invoke
Authorization: Bearer vifu_ep_...
Content-Type: application/json

{"message":"Open the north gate"}
```

API keys are scoped to one endpoint. The server stores only peppered key
hashes, and returns the raw key once when it is created.

## Docker Self-hosting

The included Compose stack runs:

- PostgreSQL for durable runtime state;
- `vifu-server` for HTTP, WebSocket routing, and database migration history;
- the same Next.js Dashboard used by every deployment mode;
- Vifu Agent Gateway for connecting configured external agent providers.

Services bind to `127.0.0.1` by default. Configuration, upgrades, persistence,
and exposure guidance are in [self-hosted/README.md](self-hosted/README.md).

## Architecture And Security

`vifu-server` owns the core runtime contract:

- routing Profile, Binding, and Endpoint CRUD;
- Project CRUD and endpoint-scoped HTTP invocation;
- endpoint-scoped API keys;
- authenticated Agent Gateway sessions and heartbeat;
- one WebSocket with multiple logical channels;
- provider agent discovery and invocation through adapter contracts;
- bounded queues, request timeout, cancellation, reconnect, and resume;
- trace correlation and PostgreSQL persistence.

The Dashboard owns web authentication and keeps deployment authority on its
server side. Local development uses no login and is restricted to loopback.
Self-hosted deployments support local email/password in the same Dashboard, with
optional self-host OIDC. Passwords use bcrypt, and opaque session hashes are
stored in PostgreSQL. The Dashboard owns the auth implementation and initializes
the auth tables it uses; append-only database history may still include older
auth table migrations. The generated admin key is a server-side runtime
credential for the Dashboard, recovery, and automation; it never enters HTML,
browser logs, or the client bundle. Optional managed infrastructure may be
introduced later, but it is not required for the day-1 runtime.

External agent providers are optional. Provider-specific setup lives under
`self-hosted/providers/` and can be removed without changing the Vifu core
stack. Remote self-host access must still use TLS even though the Dashboard has
built-in login. See
[SECURITY.md](SECURITY.md) for the full boundary.

## Repository Layout

```text
crates/
  vifu/             CLI, provider adapters, Agent Gateway, session, protocol
  vifu-server/      HTTP API, Agent Gateway relay, PostgreSQL runtime
npm-packages/
  dashboard/        One capability-driven Next.js Dashboard
self-hosted/docker/ PostgreSQL, vifu-server, Dashboard, and Agent Gateway images
scripts/            Development, E2E, and public-repository checks
```

## Build And Contribute

See [BUILD.md](BUILD.md) for source-development and verification commands.
Read [CONTRIBUTING.md](CONTRIBUTING.md) before opening a pull request, and use
the private process in [SECURITY.md](SECURITY.md) for vulnerability reports.

Vifu is licensed under [Apache-2.0](LICENSE). The license does not grant rights
to the Vifu name or logos; see [TRADEMARKS.md](TRADEMARKS.md).
