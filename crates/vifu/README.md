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

The default binary runs Vifu Server and Agent Gateway and includes the
in-process llama.cpp and Local Whisper Providers. The first launch creates the
local files and opens the live Runtime TUI in an interactive terminal. Press
`B` to open the Dashboard. Runtime and Gateway state use
`~/.vifu/runtime.sqlite`; the local Server uses `~/.vifu/vifu.sqlite`.
Configure models in `~/.vifu/providers.json` when you are ready to run them.

For a source build, including its official Dashboard bundle, follow the
[source installation guide](https://github.com/vifudotdev/vifu/blob/main/docs/install.md#build-from-source).

Rust applications that embed the execution kernel should depend on
[`vifu-runtime`](https://crates.io/crates/vifu-runtime) instead of this
application package.

Read the [Vifu documentation](https://vifu.dev/docs) or view the
[source repository](https://github.com/vifudotdev/vifu).
