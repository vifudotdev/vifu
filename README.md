# Vifu

Vifu is an open-source endpoint runtime for local AI agents. It turns one
authenticated connection to a local OpenClaw gateway into independently
secured HTTP endpoints with profiles, bindings, API keys, connection status,
and traces.

The runtime, connector protocol, PostgreSQL migrations, dashboard, and Docker
deployment are all included in this repository under Apache-2.0.

## Local Demo

Start PostgreSQL, `vifu-server`, and the Dashboard:

```bash
sh scripts/init-self-hosted.sh
docker compose -f self-hosted/docker/docker-compose.yml up --build --wait
```

Open `http://localhost:6791`. No Vifu account or login is required.

Vifu uses OpenClaw's OpenAI-compatible HTTP surface. Enable
`gateway.http.endpoints.chatCompletions`, then connect the Gateway in a second
terminal:

```bash
VIFU_OPENCLAW_TOKEN="$OPENCLAW_GATEWAY_TOKEN" sh scripts/dev-connector.sh
```

No token is needed when the local Gateway is intentionally configured without
authentication.

The Dashboard will show the connected OpenClaw agents. Create an Agent Profile,
bind one of those agents, then create as many independently keyed endpoints as
you need. All endpoints on that connector share one authenticated WebSocket.

## No-account Quickstart

The self-hosted core works without an external identity provider:

```text
application -> vifu-server -> one multiplexed WebSocket -> Vifu Connector -> OpenClaw
                     |
                     +-> PostgreSQL profiles, endpoints, keys, sessions, traces
```

The public invocation contract is:

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
- `vifu-server` for HTTP, WebSocket routing, and migrations;
- the same Next.js Dashboard used by every deployment mode.

Services bind to `127.0.0.1` by default. Configuration, upgrades, persistence,
and exposure guidance are in [self-hosted/README.md](self-hosted/README.md).

## Architecture And Security

`vifu-server` owns the core runtime contract:

- Agent Profile, Binding, and Endpoint CRUD;
- endpoint-scoped API keys;
- authenticated connector sessions and heartbeat;
- one WebSocket with multiple logical channels;
- OpenClaw agent discovery and invocation through `/v1/models` and
  `/v1/chat/completions`;
- bounded queues, request timeout, cancellation, reconnect, and resume;
- trace correlation and PostgreSQL persistence.

The Dashboard reads runtime capabilities and keeps deployment authority on its
server side. In the current self-hosted release, it uses the generated admin
key without placing that key in HTML, browser logs, or the client bundle.

The connector accepts only loopback OpenClaw URLs. Self-hosted admin access
should remain on a trusted network or behind TLS and an external identity
layer. See [SECURITY.md](SECURITY.md) for the full boundary.

## Repository Layout

```text
crates/
  vifu/             CLI, OpenClaw adapter, connector, session, protocol
  vifu-server/      HTTP API, WebSocket relay, PostgreSQL runtime
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
