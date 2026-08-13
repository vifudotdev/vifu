# Vifu Documentation

Vifu is an on-device Agent Runtime and local operations platform. Product code
owns domain state, UI, safety, and allowed actions. Vifu owns Provider
connections, Agent identity, Project Settings, sessions, stable endpoints,
Gateway transport, and attributable traces.

## Start

- [Build with Vifu in Python, TypeScript, Swift, Kotlin, Godot, or Rust](get-started/README.md)
- [Install Vifu](install.md)
- [Apps and App IDs](apps-and-app-ids.md)
- [Runtime topology, monitoring, and Gateway enrollment](topology-and-pairing.md)
- [Embedded Console](embedded-console.md)
- [Self-host a full or headless deployment with Docker](self-hosting.md)
- [Project Settings](project-settings.md)

## Operate

- [Trace data, privacy, and performance comparisons](observability.md)
- [Topology protocol live testing](topology-live-testing.md)
- [ARM optimization TUI](arm-optimization-tui.md)
- [Provider integrations](../providers/README.md)
- [Local llama Provider](../providers/llama/README.md)
- [Local Whisper Provider](../providers/local-whisper/README.md)
- [Security](../SECURITY.md)

## Embed

- [Embed the Runtime](runtime-embedding.md)
- [Python tutorial](get-started/python.md)
- [TypeScript tutorial](get-started/typescript.md)
- [Swift tutorial](get-started/swift.md)
- [Kotlin tutorial](get-started/kotlin.md)
- [Godot tutorial](get-started/godot.md)
- [Rust tutorial](get-started/rust.md)
- [Framework and model integrations](integrations/README.md)
- [Google ADK integration](integrations/google-adk.md)
- [Foundry Local integration](integrations/foundry-local.md)
- [Runtime API](https://docs.rs/vifu-runtime)
- [Gateway crate](../crates/vifu-gateway/README.md)
- [VifuGodot for Apple hosts](../integrations/godot/apple/README.md)
- [Mobile FFI for Apple and Android hosts](../crates/vifu-mobile-ffi/README.md)
- [Runnable examples and mobile starters](../examples/README.md)
- [Android Starter](../examples/android-starter/README.md)
- [iOS Starter](../examples/ios-starter/README.md)
- [Godot iOS Starter](../examples/godot-ios-starter/README.md)
- [Android AAR reference](../integrations/android/README.md)

## Reference

- [Runtime configuration example](../config/runtime.example.toml)
- [Release binaries and mobile signing](releases.md)
- [Positioning and related projects](comparison.md)
- [Build and test](../BUILD.md)
