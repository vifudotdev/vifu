# vifu

`vifu` is the Vifu Agent Runtime application for operating agents behind stable
product endpoints. One executable runs Vifu Server, Agent Gateway, or both
according to `~/.vifu/config.json`.

## Install the binary

The fastest path is a prebuilt archive from the
[latest Vifu release](https://github.com/vifudotdev/vifu/releases/latest).
Download the archive for your platform, extract it, then run:

```bash
./vifu
```

If you already use Cargo:

```bash
cargo install vifu
vifu
```

The default binary runs Vifu Server and Agent Gateway according to the local
Vifu configuration. Runtime and Gateway state use `~/.vifu/runtime.sqlite`; the
local Server uses `~/.vifu/vifu.sqlite`. It includes provider features shipped
by this package, including the in-process llama.cpp and Local Whisper Providers.
Configure local models in `~/.vifu/providers.json`.

Rust applications that embed the execution kernel should depend on
[`vifu-runtime`](https://crates.io/crates/vifu-runtime) instead of this
application package.

Read the [Vifu documentation](https://vifu.dev/docs) or view the
[source repository](https://github.com/vifudotdev/vifu).
