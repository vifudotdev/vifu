# Vifu Documentation

Vifu is an Agent Runtime in Rust for embedding and operating agents inside
products. Product code owns domain state, UI, safety, and allowed actions;
Vifu owns provider connections, agent identity, Project Settings, sessions,
stable endpoints, Gateway transport, and traces.

## Start

- [Install Vifu](install.md)
- [Apps and App IDs](apps-and-app-ids.md)
- [Runtime topology, monitoring, and Gateway enrollment](topology-and-pairing.md)
- [Embedded Console](embedded-console.md)
- [Self-host a full or headless deployment with Docker](self-hosting.md)
- [Project Settings](project-settings.md)

## Operate

- [Topology protocol live testing](topology-live-testing.md)
- [ARM optimization TUI](arm-optimization-tui.md)
- [Provider integrations](../providers/README.md)
- [Local llama Provider](../providers/llama/README.md)
- [Local Whisper Provider](../providers/local-whisper/README.md)
- [Security](../SECURITY.md)

## Embed

- [Embed the Runtime](runtime-embedding.md)
- [Runtime API](https://docs.rs/vifu-runtime)
- [Gateway crate](../crates/vifu-gateway/README.md)
- [VifuGodot for Apple hosts](../integrations/godot/apple/README.md)
- [Mobile FFI for Apple and Android hosts](../crates/vifu-mobile-ffi/README.md)
- [Ten-minute Android starter](../examples/android-starter/README.md)
- [Android AAR reference](../integrations/android/README.md)

## Reference

- [Runtime configuration example](../config/runtime.example.toml)
- [Positioning and related projects](comparison.md)
- [Build and test](../BUILD.md)
