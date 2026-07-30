# Install From Source

## Requirements

- Rust from `rust-toolchain.toml`

## Run Vifu

```bash
cd crates/vifu
cargo run --features binary
```

The first run creates `~/.vifu/config.json` and
`~/.vifu/providers.json`. With the default configuration, one process runs the
Server and Agent Gateway roles and stores state in `~/.vifu/vifu.sqlite`.

## Run The Console

The complete Console stack uses Docker Compose and PostgreSQL. From the
repository root:

```bash
cp .env.example .env
docker compose up -d
```

Open `http://localhost:6791`.

Read the generated Admin Key and enter it in the Console:

```bash
docker compose exec backend cat /run/vifu/secrets/admin_key
```

The Console manages projects, providers, agents, API keys, connection status,
and traces. It reads runtime authority on the server side.

## Stop

Stop a source process with `Ctrl-C`. Stop the Console stack while preserving
its PostgreSQL volume with:

```bash
docker compose down
```
