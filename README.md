# Vifu

![Vifu](npm-packages/dashboard/public/brand/vifu-lockup.png)

<div align="center">

<b>Agent runtime in Rust.</b>

[![Crates.io](https://img.shields.io/crates/v/vifu.svg)](https://crates.io/crates/vifu)
[![Runtime API](https://docs.rs/vifu-runtime/badge.svg)](https://docs.rs/vifu-runtime)
[![CI](https://github.com/vifudotdev/vifu/actions/workflows/ci.yml/badge.svg)](https://github.com/vifudotdev/vifu/actions/workflows/ci.yml)
[![Discord](https://img.shields.io/badge/Discord-Join-5865F2?logo=discord&logoColor=white)](https://discord.com/invite/VdqqFwJbNE)

[quick start](#quick-start) / [docs](docs/README.md) / [install](docs/install.md) / [console](docs/embedded-console.md) / [providers](providers/README.md) / [embed](docs/runtime-embedding.md) / [self-host](docs/self-hosting.md) / [build](BUILD.md)

</div>

## Quick start

Download the archive for your platform from the
[latest release](https://github.com/vifudotdev/vifu/releases/latest), extract
it, then start the local Server and Agent Gateway:

```bash
./vifu
```

Open the Console URL printed by the process, normally
`http://127.0.0.1:6790`.

For the PostgreSQL self-host Console stack:

```bash
cp .env.example .env
docker compose up -d
```

Open `http://localhost:6791`; if prompted, read the generated Admin Key:

```bash
docker compose exec backend cat /run/vifu/secrets/admin_key
```

Use the Console to connect a Provider, expose an Agent endpoint, and copy a
project key.

## Description

Vifu gives applications one stable endpoint contract for local and remote AI
agents. Product code owns state, UI, safety rules, and allowed actions. Vifu
owns projects, provider bindings, agents, endpoints, sessions, Gateway
transport, keys, and traces.

- Operate many projects from one Console.
- Connect external Providers or in-process local Providers.
- Call project endpoints through an OpenAI-compatible HTTP API.
- Embed the Rust Runtime directly inside an application.
- Sync embedded agents through Agent Gateway when a deployment needs operations.
- Manage Project Settings in the Console; JSON is an import/export artifact for
  backup, migration, and embedded targets.

## Supported surfaces

| Surface | Status | Start here |
| --- | --- | --- |
| Local Server and embedded Console | Supported | [Install](docs/install.md) or [Embedded Console](docs/embedded-console.md) |
| Self-host Server and Console | Supported | [Self-host](docs/self-hosting.md) |
| Agent Providers | Supported | [Provider integrations](providers/README.md) |
| Rust embedding | Supported | [Embed the Runtime](docs/runtime-embedding.md) |
| Swift on iOS/macOS | Supported | [Apple application guide](docs/runtime-embedding.md#add-vifu-to-an-apple-application) |
| Godot in an Apple host | Experimental | [Runtime Bridge](docs/runtime-embedding.md#connect-a-game-engine) |
| Kotlin/Android | Experimental | [Runtime embedding](docs/runtime-embedding.md) |

## Documentation

#### Use Vifu

- [Install Vifu](docs/install.md)
- [Embedded Console](docs/embedded-console.md)
- [Self-host with Docker](docs/self-hosting.md)
- [Project Settings](docs/project-settings.md)
- [Provider integrations](providers/README.md)
- [Local llama Provider](providers/llama/README.md)
- [Local Whisper Provider](providers/local-whisper/README.md)

#### Embed Vifu

- [Embed the Runtime](docs/runtime-embedding.md)
- [vifu-runtime API](https://docs.rs/vifu-runtime)
- [vifu-gateway](crates/vifu-gateway/README.md)

#### Development

- [Build and test](BUILD.md)
- [Positioning and related projects](docs/comparison.md)
- [Contributing](CONTRIBUTING.md)
- [Security](SECURITY.md)

Vifu is licensed under [Apache-2.0](LICENSE). The license does not grant rights
to the Vifu name and logos; see [TRADEMARKS.md](TRADEMARKS.md).
