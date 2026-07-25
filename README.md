# Vifu

![Vifu](npm-packages/dashboard/public/brand/vifu-lockup.png)

Vifu is a small, fast, stateful, and portable Agent Runtime. Embed it into an
application to connect, control, coordinate, and monitor local or remote Agents.

Vifu includes the cross-platform Rust runtime, Agent Gateway, durable state,
stable application APIs, traces, and a small operations Console. Applications
define their behavior by composing plugins with the headless runtime.

## Embed Vifu

The `vifu` crate is the public Rust SDK and also produces the `vifu` binary.
Runtime support is included by default:

```toml
[dependencies]
vifu = "0.1"
```

Advanced builds can disable default features to select only `runtime`,
`gateway`, or `server`. The default build also produces the complete Vifu Server
and Agent Gateway executable.

See [Embed the runtime](docs/runtime-embedding.md) and the
[crates.io release contract](docs/crates-io.md).

## Run With Docker

```bash
cp .env.example .env
docker compose up -d
```

Open `http://localhost:6791`. The stack starts PostgreSQL, Vifu Server, Vifu
Agent Gateway, and the operations Console.

Use the same command to restart the stack. Stop it while preserving the database
volume with:

```bash
docker compose down
```

See [Self-hosting Vifu](docs/self-hosting.md) for configuration and upgrades.

## Run From Source

Start PostgreSQL:

```bash
docker compose -f docker-compose.yml -f docker-compose.local.yml up -d postgres
```

Run the Rust binary:

```bash
cd crates/vifu
cargo run
```

Run the Console in another terminal:

```bash
cd npm-packages/dashboard
bun install --frozen-lockfile
bun dev
```

On first run, `vifu` creates its local configuration under `~/.vifu/`. The
default configuration runs Server and Agent Gateway roles together. The same
binary can run either role separately when a deployment needs independent
processes.

## Architecture

```text
Application -> Vifu Server -> PostgreSQL
                     |
                     +-> Vifu Gateway A -> provider agents
                     +-> Vifu Gateway B -> provider agents
                     +-> Vifu Gateway N -> provider agents

Console -----> Vifu Server
```

Applications call a project-scoped, OpenAI-compatible endpoint:

```http
POST http://localhost:6790/my-project/v1/chat/completions
Authorization: Bearer vifu_pk_...
Content-Type: application/json

{
  "model": "town-guide",
  "messages": [{ "role": "user", "content": "Open the north gate" }]
}
```

A Vifu Gateway connects provider resources to the Server over one authenticated,
multiplexed WebSocket. Projects, profiles, API keys, provider settings, and
traces remain in PostgreSQL.

## Headless Runtime

`crates/vifu-runtime` is the Bevy-based execution kernel behind the public
`vifu` crate. It supplies:

- a deterministic runtime schedule;
- command and effect-result queues;
- event and effect-request queues;
- JSON state and revisioned snapshots;
- a standard Bevy `Plugin` extension point.

It does not prescribe a graph language, narrative schema, or editor format.
Application-specific behavior stays in application plugins. See
[Embed the runtime](docs/runtime-embedding.md).

## Repository Layout

```text
crates/
  vifu/               Single executable and Agent Gateway
  vifu-gateway/       Provider and protocol building blocks
  vifu-runtime/       Embeddable Bevy runtime primitives
  vifu-server/        HTTP API, relay, traces, and PostgreSQL
npm-packages/
  dashboard/          Lightweight operations Console
providers/            Provider integration guides
```

## Documentation

- [Install from source](docs/install.md)
- [Self-host Vifu](docs/self-hosting.md)
- [Embed the runtime](docs/runtime-embedding.md)
- [crates.io release contract](docs/crates-io.md)
- [Provider integrations](providers/README.md)
- [Build and test](BUILD.md)

Read [CONTRIBUTING.md](CONTRIBUTING.md) before opening a pull request. Report
security issues through the private process in [SECURITY.md](SECURITY.md).

Vifu is licensed under [Apache-2.0](LICENSE). The license does not grant rights
to the Vifu name and logos; see
[TRADEMARKS.md](TRADEMARKS.md).
