# Vifu

Vifu is a small local connector and relay for AI agents.

The first preview checks a local OpenClaw Gateway over loopback. It can also
run a self-hosted relay for local development and private networks.

```bash
vifu
```

## What Vifu Does

- Finds a local OpenClaw Gateway on `http://127.0.0.1:18789`.
- Connects that local capability to a Vifu relay when `--relay` is set.
- Runs a self-hosted Vifu relay with `vifu server`.
- Keeps local agent access on your machine by default.
- Avoids public local-agent URLs in the default configuration.

## Commands

```bash
vifu                 # Start the local connector
vifu server          # Start a self-hosted relay
vifu --status        # Show local connector status
vifu --doctor        # Diagnose local setup
vifu --logout        # Remove local Vifu session state
vifu --reset         # Remove all local Vifu state
```

Self-hosted relay smoke test:

```bash
vifu server --listen 127.0.0.1:48989
vifu --relay 127.0.0.1:48989
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
local development, private networks, and early self-hosting tests. Do not expose
the preview relay directly to the public internet without an external secure
network layer.

Future managed relay deployments will add account authorization, relay session
tokens, and service-side permissions before accepting public traffic.

## Security Model

- OpenClaw credentials stay on the user's machine.
- Vifu does not expose a public listener by default.
- Vifu only accepts loopback OpenClaw URLs by default.
- The preview relay does not require Docker socket access.
- Do not post logs or issue reports that include tokens, passwords, or other
  sensitive data.

## Docker

Build the self-hosted relay image:

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
cp .env.example .env.local
docker compose up --build
```

## Vifu Cloud Control Plane

`vifu server` can optionally register itself with a Vifu API control plane. This
is meant for managed relay deployments where the relay runs as a service
process and uses service credentials supplied by the deployment environment.

Cloud registration is disabled by default. To enable it, set:

```bash
VIFU_CLOUD_ENABLED=1
VIFU_API_BASE_URL=https://api.example.test
VIFU_SERVICE_ID=vifu-relay-dev
VIFU_SERVICE_USERNAME=service@example.test
VIFU_SERVICE_PASSWORD=replace-me
VIFU_RELAY_ID=local-relay
VIFU_RELAY_ENDPOINT=tcp://127.0.0.1:48989
```

When enabled, the relay:

- logs in through `POST /v1/auth/service/login`;
- registers through `POST /v1/relays/register`;
- sends heartbeat updates through `POST /v1/relays/heartbeat`.

Real service credentials belong in your runtime secret store or a local
untracked `.env.local` file, never in source control.

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
