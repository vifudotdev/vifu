# Vifu

Vifu is an open-source endpoint runtime for local AI agents. It turns one
authenticated connection to a local OpenClaw gateway into stable project
endpoints with profiles, bindings, API keys, connection status, and traces.

The runtime, Agent Gateway protocol, PostgreSQL migrations, dashboard, and Docker
deployment are all included in this repository under Apache-2.0.

## Local Demo

Start PostgreSQL, `vifu-server`, and the Dashboard:

```bash
sh scripts/init-self-hosted.sh
docker compose -f self-hosted/docker/docker-compose.yml up --build --wait
```

Open `http://localhost:6791` and create the first local administrator. The
account is stored only in this deployment's PostgreSQL database; no Vifu account
or external identity provider is required. Signup stays open by default for
additional local users. The first account receives the deployment `admin` role;
later signups receive the deployment `operator` role. Signup can be disabled
with `AUTH_DISABLE_SIGNUP=true` or `VIFU_SIGNUP_ENABLED=false`.

Vifu uses OpenClaw's OpenAI-compatible HTTP surface. Enable
`gateway.http.endpoints.chatCompletions`, then connect the Gateway in a second
terminal:

```bash
VIFU_OPENCLAW_TOKEN="$OPENCLAW_GATEWAY_TOKEN" sh scripts/dev-agent-gateway.sh
```

No token is needed when the local Gateway is intentionally configured without
authentication.

The Dashboard will show the connected OpenClaw agents. Create a Project from
one or more detected agents to get a stable project endpoint and publishable
project key. OpenClaw remains the source of truth for Agent identity, workspace,
Soul, memory, tools, and model configuration. All calls on that Agent Gateway share
one authenticated WebSocket.

## Independent Self-hosting

The self-hosted core works without an external identity provider:

```text
application -> vifu-server -> one multiplexed WebSocket -> Vifu Agent Gateway -> OpenClaw
                     |
                     +-> PostgreSQL profiles, endpoints, keys, traces

Dashboard -> PostgreSQL users and web sessions
Dashboard -> vifu-server through a server-side runtime credential
```

Each Project exposes JSON-RPC 2.0 over HTTPS and WSS at one stable address. No
SDK is required:

```http
POST http://demo.localhost:6790
Authorization: Bearer vifu_pk_...
Content-Type: application/json

{"jsonrpc":"2.0","id":1,"method":"agent.invoke","params":{"agent":"town-guide","message":"Open the north gate"}}
```

Use `rpc.discover` for the OpenRPC document and `agent.list` for the bindings
available through a Project. The development path
`/v1/projects/{slug}/rpc` provides the same protocol when wildcard localhost
names are unavailable.

Individual endpoint invocation remains available for endpoint-scoped keys:

```http
POST /v1/endpoints/{id-or-slug}/invoke
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
- the same Next.js Dashboard used by every deployment mode.

Services bind to `127.0.0.1` by default. Configuration, upgrades, persistence,
and exposure guidance are in [self-hosted/README.md](self-hosted/README.md).

## Architecture And Security

`vifu-server` owns the core runtime contract:

- routing Profile, Binding, and Endpoint CRUD;
- Project CRUD and project-scoped JSON-RPC over HTTPS/WSS;
- endpoint-scoped API keys;
- authenticated Agent Gateway sessions and heartbeat;
- one WebSocket with multiple logical channels;
- OpenClaw agent discovery and invocation through `/v1/models` and
  `/v1/chat/completions`;
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

The Agent Gateway accepts only loopback OpenClaw URLs. Remote self-host access must
still use TLS even though the Dashboard has built-in login. See
[SECURITY.md](SECURITY.md) for the full boundary.

## Repository Layout

```text
crates/
  vifu/             CLI, OpenClaw adapter, Agent Gateway, session, protocol
  vifu-server/      JSON-RPC, Agent Gateway relay, PostgreSQL runtime
npm-packages/
  dashboard/        One capability-driven Next.js Dashboard
self-hosted/docker/ PostgreSQL, vifu-server, and Dashboard images
scripts/            Development, E2E, and public-repository checks
```

## Build And Contribute

See [BUILD.md](BUILD.md) for source-development and verification commands.
Read [CONTRIBUTING.md](CONTRIBUTING.md) before opening a pull request, and use
the private process in [SECURITY.md](SECURITY.md) for vulnerability reports.

Vifu is licensed under [Apache-2.0](LICENSE). The license does not grant rights
to the Vifu name or logos; see [TRADEMARKS.md](TRADEMARKS.md).
