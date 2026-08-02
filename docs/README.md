# Vifu Documentation

Vifu is an Agent Runtime in Rust for embedding and operating agents inside
products. Product code owns domain state, UI, safety, and allowed actions;
Vifu owns provider connections, agent identity, Project Settings, sessions,
stable endpoints, Gateway transport, and traces.

## Start

- [Install Vifu](install.md)
- [Embedded Console](embedded-console.md)
- [Self-host a full or headless deployment with Docker](self-hosting.md)
- [Project Settings](project-settings.md)

## Operate

- [ARM optimization TUI](arm-optimization-tui.md)
- [Provider integrations](../providers/README.md)
- [Local llama Provider](../providers/llama/README.md)
- [Local Whisper Provider](../providers/local-whisper/README.md)
- [Security](../SECURITY.md)

## Embed

- [Embed the Runtime](runtime-embedding.md)
- [Runtime API](https://docs.rs/vifu-runtime)
- [Gateway crate](../crates/vifu-gateway/README.md)

## Reference

- [Runtime configuration example](../config/runtime.example.json)
- [Positioning and related projects](comparison.md)
- [Build and test](../BUILD.md)
