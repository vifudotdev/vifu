# vifu

`vifu` is the public Rust SDK and executable for the Vifu Agent Runtime.

## Embed Vifu

Runtime support is included by default:

```toml
[dependencies]
vifu = "0.1"
```

```rust
use vifu::runtime::prelude::*;

let mut runtime = HeadlessRuntime::new();
let advance = runtime.dispatch(RuntimeCommand::new(
    "command-1",
    "application.input",
    json!({ "text": "Hello" }),
));

assert_eq!(advance.snapshot.revision, 1);
```

Available features:

| Feature | Capability |
| --- | --- |
| `runtime` | Portable command, state, event, effect, and snapshot runtime; included by default |
| `gateway` | Provider discovery and multiplexed Agent Gateway client |
| `server` | HTTP, WebSocket, and PostgreSQL Vifu Server |
| `full` | Runtime, Gateway, and Server library APIs |
| `binary` | Complete `vifu` executable; enabled by default |
| `local-whisper` | Optional local Whisper provider support |

Advanced builds can disable default features and select only the capabilities
they use.

## Install the binary

```bash
cargo install vifu
vifu
```

The default binary runs Vifu Server and Agent Gateway according to the local
Vifu configuration. Local Whisper support is opt-in:

```bash
cargo install vifu --features local-whisper
```

Read the [Vifu documentation](https://vifu.dev/docs) or view the
[source repository](https://github.com/vifudotdev/vifu).
