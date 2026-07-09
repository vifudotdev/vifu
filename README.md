# Vifu

Vifu is the open-source local connector for AI agents.

The first preview checks a local OpenClaw Gateway over loopback. It can also run
a self-hosted deployment for local development and private networks.

```bash
vifu
```

## Status

Vifu is an early preview. The local OpenClaw connector and self-host deployment
are the current public surface. The relay protocol may change before a stable
release.

## Install

Build from source:

```bash
cargo install --path .
vifu --doctor
```

Or run without installing:

```bash
cargo run -- --doctor
```

## Quickstart

Local OpenClaw check:

```bash
openclaw gateway --port 18789
vifu --status
```

Self-host deployment smoke test:

```bash
VIFU_DEPLOYMENT=self-host vifu deploy --listen 127.0.0.1:48989
vifu --relay 127.0.0.1:48989
```

## Self Hosting

By default, Vifu runs locally and does not require an account. To run the relay
yourself, start a self-hosted deployment:

```bash
VIFU_DEPLOYMENT=self-host vifu deploy --listen 127.0.0.1:48989
```

`VIFU_DEPLOYMENT` defaults to `local`. Use `self-host` when you operate the
relay yourself.

## What Vifu Does

- Finds a local OpenClaw Gateway on `http://127.0.0.1:18789`.
- Connects that local capability to a relay when `--relay` is set.
- Runs the selected deployment with `vifu deploy`.
- Keeps local agent access on your machine by default.
- Avoids public local-agent URLs in the default configuration.

## Commands

```bash
vifu                 # Start the local connector
vifu deploy          # Start the selected deployment
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

## Relay Preview

The preview relay transport is a small Vifu protocol over TCP. It is meant for
local development, private networks, and early self-host tests. Do not expose the
preview relay directly to the public internet without an external secure network
layer.

## Privacy And Security

- OpenClaw credentials stay on the user's machine.
- Local-only usage does not require a Vifu account.
- Vifu does not expose a public listener by default.
- Vifu only accepts loopback OpenClaw URLs by default.
- The preview relay does not require Docker socket access.
- Do not post logs or issue reports that include tokens, passwords, or other
  sensitive data.

## Docker

Build the deployment image:

```bash
docker build -t vifu:local .
docker run --rm -p 48989:48989 vifu:local
```

For a local client on the same machine:

```bash
vifu --relay 127.0.0.1:48989
```

Docker Compose:

```bash
docker compose up --build
```

To customize the listener for Docker Compose, copy `.env.example` to
`.env.local` and edit `VIFU_LISTEN_ADDR`.

## Development

Requirements:

- Rust 1.90 or newer

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
