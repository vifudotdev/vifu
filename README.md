# Vifu

![Vifu](npm-packages/dashboard/public/brand/vifu-lockup.png)

[![Crates.io](https://img.shields.io/crates/v/vifu.svg)](https://crates.io/crates/vifu)
[![Downloads](https://img.shields.io/crates/d/vifu.svg)](https://crates.io/crates/vifu)
[![Docs](https://docs.rs/vifu/badge.svg)](https://docs.rs/vifu)
[![CI](https://github.com/vifudotdev/vifu/actions/workflows/ci.yml/badge.svg)](https://github.com/vifudotdev/vifu/actions/workflows/ci.yml)
[![Discord](https://img.shields.io/badge/Discord-Join-5865F2?logo=discord&logoColor=white)](https://discord.com/invite/VdqqFwJbNE)

Vifu is a small, fast, stateful, and portable runtime for agents.

Connect applications to agents from local or remote providers through one
runtime for stable APIs, durable state, access control, routing, and traces.

Vifu includes the cross-platform Rust runtime, Agent Gateway, durable state,
stable application APIs, traces, and a small operations Console. A runtime can
live directly inside an application, or the same invocation model can sit behind
Vifu Server for multi-project deployments.

## Embed Vifu

The `vifu` crate is the public Rust SDK and also produces the `vifu` binary.
Runtime support is included by default:

```toml
[dependencies]
vifu = "0.1"
```

Advanced builds can disable default features and select the broad capabilities
they use: `runtime`, `gateway`, or `server`. Provider capabilities are registered
at runtime rather than selected with vendor-specific Cargo features. Enable the
`binary` feature to build the complete Vifu Server and Agent Gateway executable.

See [Embed the runtime](docs/runtime-embedding.md) and the
[crates.io release contract](docs/crates-io.md).

Apple applications can add this repository directly as a Swift Package:

```text
https://github.com/vifudotdev/vifu
```

Select the `Vifu` product, then use `import Vifu`. SwiftPM downloads the
versioned XCFramework from the matching GitHub release, so application
developers do not need a Rust toolchain.

## Run With Docker

```bash
cp .env.example .env
docker compose up -d
```

Open `http://localhost:6791`. The stack starts PostgreSQL, Vifu Server, Vifu
Agent Gateway, and the operations Console. Read the generated Admin Key, then
enter it in the Console:

```bash
docker compose exec backend cat /run/vifu/secrets/admin_key
```

Use the same command to restart the stack. Stop it while preserving the database
volume with:

```bash
docker compose down
```

See [Self-hosting Vifu](docs/self-hosting.md) for configuration and upgrades.

## Run From Source

```bash
cd crates/vifu
cargo run --features binary
```

On first run, `vifu` creates its local configuration under `~/.vifu/`. The
default configuration runs Server and Agent Gateway roles together and stores
durable state in `~/.vifu/vifu.sqlite`. The API listens on
`http://127.0.0.1:6790`.

The same binary can run either role separately when a deployment needs
independent processes. Use Docker Compose when you also need the operations
Console and PostgreSQL-backed self-hosting.

## Architecture

Vifu has one runtime model and two optional deployment components:

| Component | Primary resource | Role |
| --- | --- | --- |
| Embedded Runtime | One application or project | Registers providers, agents, named endpoints, sessions, state, and effects inside the host process |
| Vifu Server | Many projects | Adds HTTP/WebSocket APIs, project keys, provider configuration, database persistence, and traces |
| Agent Gateway | One Server connection | Connects provider resources on another machine to Server over an authenticated multiplexed transport |
| Operations Console | One Server deployment | Operates Server projects, providers, keys, gateways, and traces |

Embedded applications call `VifuRuntime` directly:

```text
Application -> VifuRuntime -> local or remote provider
```

Multi-project deployments use Server and can add any number of Gateways:

```text
Application -> Vifu Server -> SQLite or PostgreSQL
                     |
                     +-> Vifu Gateway A -> provider agents
                     +-> Vifu Gateway B -> provider agents
                     +-> Vifu Gateway N -> provider agents

Console -----> Vifu Server
```

Agent Gateway is a Server transport, so it connects to a running Vifu Server.
The embedded Runtime is independently usable: a host registers its provider
implementations directly and owns its snapshot storage.

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
traces use embedded SQLite locally and PostgreSQL in the Docker self-hosted
stack.

## Embedded Runtime

`crates/vifu-runtime` is the Bevy-based execution kernel behind the public
`vifu` crate. It supplies:

- dynamic provider, agent, and named endpoint registration;
- async invocation and non-blocking start/poll/cancel APIs for game loops;
- independent session state with host-provided storage;
- portable project snapshot export and restore;
- a deterministic runtime schedule;
- command and effect-result queues;
- event and effect-request queues;
- JSON state and revisioned snapshots;
- a standard Bevy `Plugin` extension point.

It does not prescribe a graph language, narrative schema, or editor format.
Application-specific behavior stays in provider adapters and application
plugins. See [Embed the runtime](docs/runtime-embedding.md).

## Repository Layout

```text
crates/
  vifu/               Single executable and Agent Gateway
  vifu-gateway/       Provider and protocol building blocks
  vifu-runtime/       Embeddable Bevy runtime primitives
  vifu-server/        HTTP API, relay, traces, and durable storage
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

## Related Agent Runtimes

These open-source projects solve adjacent parts of the agent runtime stack. The
comparison describes each project's primary model, not an exhaustive feature
checklist.

| Project | Primary model | How Vifu differs |
| --- | --- | --- |
| [ADK-Rust](https://github.com/zavora-ai/adk-rust) | Rust framework and execution runtime for defining agents, tools, workflows, sessions, memory, and servers | Vifu centers the project-level contract through which applications access agents exposed by local or remote providers |
| [Google Agent Development Kit](https://github.com/google/adk-python) | Code-first framework for building, evaluating, orchestrating, and deploying agent systems | Vifu centers stable project endpoints, provider connections, access, durable state, and traces at the application boundary |
| [LangGraph](https://github.com/langchain-ai/langgraph) | Graph orchestration framework and runtime for long-running, stateful agents and workflows | Vifu organizes agents and access around a project rather than a workflow graph |
| [Cloudflare Agents SDK](https://github.com/cloudflare/agents) | Cloudflare-hosted runtime for durable agent instances, state, sessions, connections, and scheduling | Vifu keeps the application contract portable across agents running on local or remote providers |

**Vifu does not replace agent providers. It gives applications a stable,
stateful runtime contract for accessing agents across local and remote
providers.**

Read [CONTRIBUTING.md](CONTRIBUTING.md) before opening a pull request. Report
security issues through the private process in [SECURITY.md](SECURITY.md).

Vifu is licensed under [Apache-2.0](LICENSE). The license does not grant rights
to the Vifu name and logos; see
[TRADEMARKS.md](TRADEMARKS.md).
