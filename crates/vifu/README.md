# vifu

`vifu` is the complete Vifu Agent Runtime application. One executable runs
Vifu Server, Agent Gateway, or both according to `~/.vifu/config.json`.

## Install the binary

```bash
cargo install vifu
vifu
```

The default binary runs Vifu Server and Agent Gateway according to the local
Vifu configuration and stores local state in `~/.vifu/vifu.sqlite`. Local
Whisper support is opt-in:

```bash
cargo install vifu --features local-whisper
```

Rust applications that embed the execution kernel should depend on
[`vifu-runtime`](https://crates.io/crates/vifu-runtime) instead of this
application package.

Read the [Vifu documentation](https://vifu.dev/docs) or view the
[source repository](https://github.com/vifudotdev/vifu).
