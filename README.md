# Vifu

Vifu is an open-source endpoint runtime for local AI agents. It provides stable
project endpoints with profiles, bindings, API keys, connection status, and
traces, and connects agent providers through Vifu Agent Gateway.

The runtime, Agent Gateway protocol, PostgreSQL migrations, dashboard, and Docker
deployment are all included in this repository under Apache-2.0.

## Run Vifu

A complete Vifu installation has three layers:

| Layer | Processes |
| --- | --- |
| Dashboard | The Next.js management console |
| Runtime | One `vifu` binary configured as Server, Agent Gateway, or both |
| Database | PostgreSQL |

Vifu ships one Rust executable: `vifu`. On first run it creates
`~/.vifu/config.json` and `~/.vifu/providers.json`, then starts a loopback
Server and Agent Gateway. The runtime configuration is only needed when a
deployment changes addresses or runs the two roles separately. A Gateway always
connects to its configured Server. Agent providers integrate through the Gateway
role.

`local` and `self-hosted` are deployment modes. Native processes and Docker are
two ways to run a self-hosted deployment; Docker is not a separate product mode.

### Local Development

Run PostgreSQL, the configured Vifu runtime, and the Dashboard as separate
native processes. PostgreSQL stays in Docker so local data survives restarts.
Install Dashboard dependencies once:

```bash
bun install --frozen-lockfile
```

Then use separate terminals:

```bash
# Terminal 1: PostgreSQL
docker compose -f docker-compose.yml -f docker-compose.local.yml up -d postgres
```

```bash
# Terminal 2: Vifu Server and Agent Gateway
cd crates/vifu
cargo run
```

```bash
# Terminal 3: Dashboard
cd npm-packages/dashboard
bun dev
```

Open `http://localhost:6791` to work through the local Dashboard. Stop the Rust
runtime and Dashboard with `Ctrl-C`; PostgreSQL remains available for the next
run. Stop the local database with:

```bash
docker compose -f docker-compose.yml -f docker-compose.local.yml down
```

### Docker Self-hosting

Docker Compose starts the Dashboard, PostgreSQL, and two containers that reuse
the same Vifu runtime image with separate role configurations:

```bash
cp .env.example .env
docker compose up -d
```

After that, restarting the stack from the repository root is just:

```bash
docker compose up -d
```

Open `http://localhost:6791` and create the first local administrator. Dashboard
identities and sessions stay in this deployment's PostgreSQL database. Signup
stays open by default for additional local users. The first account receives the
deployment `admin` role; later signups receive the deployment `operator` role.
Signup can be disabled with `AUTH_DISABLE_SIGNUP=true` or
`VIFU_SIGNUP_ENABLED=false`.

The Compose stack includes Vifu Agent Gateway. Connect a supported provider
under `providers/` to discover agents, expose them through a project endpoint,
and invoke them from your game. The Dashboard shows connected agents as the
Gateway discovers them.

Configuration, upgrades, persistence, and network exposure are documented in
[Self-hosting Vifu](docs/self-hosting.md).

### Native Self-hosting

Build the Vifu executable without Docker:

```bash
cargo build --release --locked -p vifu
```

Run `target/release/vifu` directly. It creates its loopback configuration on
first use. To split the Server and Agent Gateway, or change their addresses,
edit `~/.vifu/config.json` using
[config/runtime.example.json](config/runtime.example.json) as a reference. A
complete native installation also runs the Next.js Dashboard and PostgreSQL. See
[BUILD.md](BUILD.md) for verification.

## Runtime Architecture

A self-hosted deployment keeps Dashboard identity and session state inside the
deployment:

```text
application -> vifu (Server role) -> one multiplexed WebSocket -> vifu (Gateway role) -> provider
                           |
                           +-> PostgreSQL profiles, endpoints, keys, traces

Dashboard -> PostgreSQL users and web sessions
Dashboard -> Vifu Server through a server-side runtime credential
```

Applications call the OpenAI-compatible API. Existing AI SDKs only need a Vifu
project base URL, a project API key, and the target agent slug or ID as `model`:

```http
POST http://localhost:6790/my-project/v1/chat/completions
Authorization: Bearer vifu_pk_...
Content-Type: application/json

{
  "model": "town-guide",
  "messages": [{ "role": "user", "content": "Open the north gate" }],
  "stream": false
}
```

Project keys can follow every exposed agent in one project or an explicit set
of agent bindings. The server stores only peppered key hashes and returns a raw
key once when it is created. Removing an agent from the Gameplay canvas or
turning off its exposure makes it unavailable through the project API.

## Architecture And Security

The Vifu Server role owns the core runtime contract:

- routing Profile, Binding, and Endpoint CRUD;
- Project CRUD and project-scoped HTTP invocation;
- project API keys with all-agent or selected-agent access;
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
browser logs, or the client bundle.

Provider-specific setup lives under `providers/`. Register a supported provider
to make its agents available to Vifu projects; each registration uses the same
Vifu Server, Dashboard, database, and Agent Gateway stack. Remote self-host
access must still use TLS even though the Dashboard has built-in login. See
[SECURITY.md](SECURITY.md) for the full boundary.

## Repository Layout

```text
crates/
  vifu/             The single executable, configuration, and Agent Gateway
  vifu-server/      Internal HTTP API, Agent Gateway relay, PostgreSQL runtime library
npm-packages/
  dashboard/        One capability-driven Next.js Dashboard
providers/          Agent provider guides and examples
docker-compose.yml PostgreSQL, one Vifu runtime image, Dashboard, and Agent Gateway stack
scripts/            E2E fixtures and public-repository checks
```

## Build And Contribute

See [BUILD.md](BUILD.md) for source-development and verification commands.
Read [CONTRIBUTING.md](CONTRIBUTING.md) before opening a pull request, and use
the private process in [SECURITY.md](SECURITY.md) for vulnerability reports.

Vifu is licensed under [Apache-2.0](LICENSE). The license does not grant rights
to the Vifu name or logos; see [TRADEMARKS.md](TRADEMARKS.md).
