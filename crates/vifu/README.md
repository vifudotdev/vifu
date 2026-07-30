# vifu

`vifu` is the public Rust SDK and executable for the Vifu Agent Runtime.

## Embed Vifu

Runtime support is included by default:

```toml
[dependencies]
vifu = "0.1"
```

`VifuRuntime` registers providers, agents, and stable named endpoints directly
inside the host process. It supports both async invocation and a non-blocking
start/poll/cancel API for game loops. See the
[embedding guide](https://vifu.dev/docs/runtime-embedding).

Available features:

| Feature | Capability |
| --- | --- |
| `runtime` | Providers, agents, endpoints, sessions, state, effects, and snapshots; included by default |
| `gateway` | Provider discovery and multiplexed Agent Gateway client |
| `server` | HTTP, WebSocket, SQLite, and PostgreSQL Vifu Server |
| `full` | Runtime, Gateway, and Server library APIs |
| `binary` | Complete `vifu` executable |
| `local-whisper` | Optional local Whisper provider support |

Advanced builds can disable default features and select only the capabilities
they use. Provider integrations are registered dynamically and do not require
vendor-specific feature flags.

## Install the binary

```bash
cargo install vifu --features binary
vifu
```

The default binary runs Vifu Server and Agent Gateway according to the local
Vifu configuration and stores local state in `~/.vifu/vifu.sqlite`. Local
Whisper support is opt-in:

```bash
cargo install vifu --features binary,local-whisper
```

Read the [Vifu documentation](https://vifu.dev/docs) or view the
[source repository](https://github.com/vifudotdev/vifu).
