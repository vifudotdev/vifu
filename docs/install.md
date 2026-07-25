# Install From Source

## Requirements

- Rust from `rust-toolchain.toml`
- Bun 1.3.9
- Docker with Compose

## Start PostgreSQL

From the repository root:

```bash
docker compose -f docker-compose.yml -f docker-compose.local.yml up -d postgres
```

The local Compose override exposes PostgreSQL on `127.0.0.1:5432` and keeps its
data in the same named volume used by the self-hosted stack.

## Run Vifu

```bash
cd crates/vifu
cargo run
```

The first run creates `~/.vifu/config.json` and
`~/.vifu/providers.json`. With the default configuration, one process runs the
Server and Agent Gateway roles.

## Run The Console

In another terminal:

```bash
cd npm-packages/dashboard
bun install --frozen-lockfile
bun dev
```

Open `http://localhost:6791`.

The Console manages projects, providers, agents, API keys, connection status,
and traces. It reads runtime authority on the server side.

## Stop

Stop the Rust and Next.js processes with `Ctrl-C`. Stop PostgreSQL with:

```bash
docker compose -f docker-compose.yml -f docker-compose.local.yml down
```
