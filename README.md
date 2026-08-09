# Vifu

![Vifu](npm-packages/dashboard/public/brand/vifu-lockup.png)

<div align="center">

<b>Agent runtime in Rust.</b>

[![Crates.io](https://img.shields.io/crates/v/vifu.svg)](https://crates.io/crates/vifu)
[![Runtime API](https://docs.rs/vifu-runtime/badge.svg)](https://docs.rs/vifu-runtime)
[![CI](https://github.com/vifudotdev/vifu/actions/workflows/ci.yml/badge.svg)](https://github.com/vifudotdev/vifu/actions/workflows/ci.yml)
[![Discord](https://img.shields.io/badge/Discord-Join-5865F2?logo=discord&logoColor=white)](https://discord.com/invite/VdqqFwJbNE)

[quick start](#quick-start) / [docs](docs/README.md) / [install](docs/install.md) / [topology](docs/topology-and-pairing.md) / [console](docs/embedded-console.md) / [providers](providers/README.md) / [embed](docs/runtime-embedding.md) / [self-host](docs/self-hosting.md) / [build](BUILD.md)

</div>

## Quick start

Download the archive for your platform from the
[latest release](https://github.com/vifudotdev/vifu/releases/latest), extract
it, then run Vifu:

```bash
./vifu
```

Vifu creates its local Runtime profile, stores state in SQLite under `~/.vifu`,
and opens the live Runtime TUI. Press `B` to open the Dashboard for persistent
Traces and Comparisons. Vifu keeps serving until you quit the TUI. The local
Dashboard is normally available at `http://127.0.0.1:6790`.

To run from a source checkout, install the workspace dependencies once and use
the repository command that builds the Console before starting Vifu:

```bash
bun install --frozen-lockfile
cargo vifu
```

## Description

Vifu gives applications one stable endpoint contract for local and remote AI
agents. Product code owns state, UI, safety rules, and allowed actions. Vifu
owns projects, provider bindings, agents, endpoints, sessions, Gateway
transport, keys, and traces.

- Operate many projects from one Console.
- Connect external Providers or in-process local Providers.
- Monitor concurrent Agent lanes and inspect typed Trace boundaries in one TUI.
- Compare configured local models, activate a measured session route, and Undo.
- Call project endpoints through an OpenAI-compatible HTTP API.
- Embed the Rust Runtime directly inside an application.
- Sync embedded agents through Agent Gateway when a deployment needs operations.
- Manage Project Settings in the Console; JSON is an import/export artifact for
  backup, migration, and embedded targets.

## Available surfaces

| Surface | Distribution | Start here |
| --- | --- | --- |
| Local Server and embedded Console | Release binary | [Install](docs/install.md) or [Embedded Console](docs/embedded-console.md) |
| Self-host Server and Console | Docker Compose | [Self-host](docs/self-hosting.md) |
| Agent Providers | Built-in and configurable adapters | [Provider integrations](providers/README.md) |
| Rust embedding | crates.io package | [Embed the Runtime](docs/runtime-embedding.md) |
| Swift on iOS/macOS | SwiftPM package | [Apple application guide](docs/runtime-embedding.md#add-vifu-to-an-apple-application) |
| Godot in an Apple host | VifuGodot SwiftPM package | [VifuGodot guide](integrations/godot/apple/README.md) |
| Kotlin/Android | Buildable Kotlin/JNI source set | [Mobile FFI guide](crates/vifu-mobile-ffi/README.md) |

## Documentation

- [Dashboard architecture](docs/dashboard-architecture.md)

#### Use Vifu

- [Install Vifu](docs/install.md)
- [Runtime topology and Gateway enrollment](docs/topology-and-pairing.md)
- [Embedded Console](docs/embedded-console.md)
- [ARM optimization TUI](docs/arm-optimization-tui.md)
- [Self-host with Docker](docs/self-hosting.md)
- [Project Settings](docs/project-settings.md)
- [Provider integrations](providers/README.md)
- [Local llama Provider](providers/llama/README.md)
- [Local Whisper Provider](providers/local-whisper/README.md)

#### Embed Vifu

- [Embed the Runtime](docs/runtime-embedding.md)
- [vifu-runtime API](https://docs.rs/vifu-runtime)
- [vifu-gateway](crates/vifu-gateway/README.md)
- [VifuGodot for Apple hosts](integrations/godot/apple/README.md)
- [Mobile FFI for Apple and Android hosts](crates/vifu-mobile-ffi/README.md)

#### Development

- [Build and test](BUILD.md)
- [Positioning and related projects](docs/comparison.md)
- [Contributing](CONTRIBUTING.md)
- [Security](SECURITY.md)

Vifu is licensed under [Apache-2.0](LICENSE). The license does not grant rights
to the Vifu name and logos; see [TRADEMARKS.md](TRADEMARKS.md).
