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

1. Download the archive for your platform from the
   [latest release](https://github.com/vifudotdev/vifu/releases/latest).
2. Extract the archive.
3. Start Vifu:

```bash
./vifu
```

4. Install the
   [optimized Android Starter APK](https://github.com/vifudotdev/vifu/releases/download/android-starter-v0.1.1/vifu-android-starter.apk).
   You can also [build the Starter from source](examples/android-starter/README.md#build-from-source).

Vifu creates a local Runtime profile and opens the Runtime TUI. It stores local
state in SQLite under `~/.vifu`.

Press `B` to open the Dashboard. The default Dashboard address is
`http://127.0.0.1:6790`. Vifu continues to serve requests until you stop the
TUI.

Pair the Android Starter with Vifu to inspect its inference stages. Install the
[baseline APK](https://github.com/vifudotdev/vifu/releases/download/android-starter-v0.1.1/vifu-android-starter-baseline.apk)
beside the optimized APK. Pair both applications with the same Vifu project to
compare their traces on one device. Use a new one-time pairing code for each
application. For iOS and other integrations, use the [examples guide](examples/README.md).

## Description

Vifu gives products stable endpoints for local and remote AI Agents. Product
code owns the state, UI, safety rules, and allowed actions.

Vifu manages the provider connections, Agents, endpoints, sessions, keys,
routes, and traces for each App.

- Connect local or external Providers to named endpoints.
- Call endpoints through an OpenAI-compatible HTTP API.
- Inspect Agents and traces in the TUI or Console.
- Embed the Rust Runtime in an application.
- Connect an embedded Runtime to a Server through Agent Gateway.
- Import or export Project Settings as JSON.

## Available surfaces

| Surface | Distribution | Start here |
| --- | --- | --- |
| Local Server and embedded Console | Release binary | [Install](docs/install.md) or [Embedded Console](docs/embedded-console.md) |
| Self-host Server and Console | Docker Compose | [Self-host](docs/self-hosting.md) |
| Agent Providers | Built-in and configurable adapters | [Provider integrations](providers/README.md) |
| Rust embedding | crates.io package | [Embed the Runtime](docs/runtime-embedding.md) |
| Swift on iOS/macOS | SwiftPM package | [Apple application guide](docs/runtime-embedding.md#add-vifu-to-an-apple-application) |
| Android Starter | Release APK and Kotlin source project | [Android Starter](examples/android-starter/README.md) |
| iOS Starter | SwiftUI source and optional TestFlight beta | [iOS Starter](examples/ios-starter/README.md) |
| Godot in an Apple host | VifuGodot SwiftPM package | [VifuGodot guide](integrations/godot/apple/README.md) |
| Kotlin/Android | Modular Core, llama, and Whisper ARM64 Maven AARs | [Android Starter](examples/android-starter/README.md) |

## Documentation

- [Dashboard architecture](docs/dashboard-architecture.md)

#### Use Vifu

- [Install Vifu](docs/install.md)
- [Apps and App IDs](docs/apps-and-app-ids.md)
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
- [Runnable examples and mobile starters](examples/README.md)
- [Android Starter](examples/android-starter/README.md)
- [iOS Starter](examples/ios-starter/README.md)
- [Godot iOS Starter](examples/godot-ios-starter/README.md)
- [Android AAR reference](integrations/android/README.md)

#### Development

- [Build and test](BUILD.md)
- [Positioning and related projects](docs/comparison.md)
- [Contributing](CONTRIBUTING.md)
- [Security](SECURITY.md)

Vifu is licensed under [Apache-2.0](LICENSE). The license does not grant rights
to the Vifu name and logos. See [TRADEMARKS.md](TRADEMARKS.md).
