# Install Vifu

## Download A Release

Download the archive for your platform from the
[latest release](https://github.com/vifudotdev/vifu/releases/latest).

- macOS and Linux archives use `.tar.gz`.
- Windows archives use `.zip`.

Extract the archive, then start the local Server and Agent Gateway:

```bash
./vifu
```

The same process serves the local Console. Open the URL printed at startup,
normally:

```text
http://127.0.0.1:6790
```

On Windows:

```powershell
.\vifu.exe
```

## Build From Source

Use Cargo only when you want to build from source.

### Requirements

- Rust from `rust-toolchain.toml`
- Bun, when rebuilding the embedded Console assets

### Run Vifu

```bash
bun run build:console
cargo run -p vifu
```

The first run creates `~/.vifu/config.json` and
`~/.vifu/providers.json`. With the default configuration, one process runs the
Server and Agent Gateway roles. Runtime and Gateway state is stored in
`~/.vifu/runtime.sqlite`; local Server data is stored separately in
`~/.vifu/vifu.sqlite`.
Add `llama` or `local-whisper` entries to the runtime Provider registry to load
local models in the same process. The Console edits the same registry in local
mode, and project provider records only bind provider keys to projects; see
[Agent Providers](../providers/README.md).

`cargo run` embeds the assets already generated in
`target/vifu-console-assets/`. Re-run `bun run build:console` after changing
the embedded Console UI. See [Embedded Console](embedded-console.md).

## Run The Console

The PostgreSQL Console stack uses Docker Compose. From the repository root:

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

Stop a release or source process with `Ctrl-C`. Stop the Console stack while
preserving its PostgreSQL volume with:

```bash
docker compose down
```
