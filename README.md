# Vifu

Vifu is a small local connector for AI agents.

The first preview checks a local OpenClaw Gateway over loopback.

```bash
vifu
```

## What Vifu Does

- Finds a local OpenClaw Gateway on `http://127.0.0.1:18789`.
- Keeps local agent access on your machine by default.
- Avoids public local-agent URLs in the default configuration.

## Commands

```bash
vifu                 # Start the local connector
vifu --status        # Show local connector status
vifu --doctor        # Diagnose local setup
vifu --logout        # Remove local Vifu session state
vifu --reset         # Remove all local Vifu state
```

## OpenClaw

Vifu currently targets a local OpenClaw Gateway.

```bash
openclaw gateway --port 18789
vifu --status
```

By default, Vifu accepts only loopback OpenClaw URLs. That keeps the first
version simple and avoids turning the CLI into a general remote access tool.

## Security Model

- OpenClaw credentials stay on the user's machine.
- Vifu does not expose a public listener by default.
- Vifu only accepts loopback OpenClaw URLs by default.
- Do not post logs or issue reports that include tokens, passwords, or other
  sensitive data.

## Development

Requirements:

- Rust 1.80 or newer

Common checks:

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
cargo build
cargo check
```

Run locally:

```bash
cargo run -- --status
```

## Project Status

This repository contains the Rust implementation of the public `vifu` CLI.

## License

Apache-2.0. See [LICENSE](LICENSE).
