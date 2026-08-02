# crates.io Release Contract

The public packages have two explicit entry points:

- `vifu` installs the complete configuration-driven Runtime, Agent Gateway, and
  Server application.
- `vifu-runtime` is the portable Rust SDK for applications that embed Vifu.

Install the application without feature selection:

```bash
cargo install vifu
```

Embed the execution kernel directly:

```toml
[dependencies]
vifu-runtime = "0.1"
```

The application release artifacts are built with the package provider features
enabled, including the in-process llama.cpp and Local Whisper Providers. Runtime
configuration selects which roles start, while `providers.json` and project
bindings select which configured Providers are reachable.

## Package Layout

The repository keeps implementation crates separate so each boundary can be
tested and optimized independently:

| Package | Role |
| --- | --- |
| `vifu-runtime` | Portable Bevy execution kernel |
| `vifu-provider-llama` | In-process llama.cpp GGUF Provider |
| `vifu-gateway` | Provider, protocol, relay, and session implementation |
| `vifu-server` | HTTP, WebSocket, SQLite, and PostgreSQL server implementation |
| `vifu` | Complete configuration-driven application and binary |

Embedded applications should depend on `vifu-runtime`. The Gateway and Server
packages remain public implementation boundaries that can also support custom
deployments.

## Release Order

Publish one version from a clean, tagged commit in dependency order:

```bash
cargo publish -p vifu-runtime
cargo publish -p vifu-provider-llama
cargo publish -p vifu-gateway
cargo publish -p vifu-server
cargo publish -p vifu
```

Wait for each package to become available in the crates.io index before
publishing the next package.

Before uploading:

```bash
cargo fmt --all --check
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo package --list -p vifu-runtime
cargo publish --dry-run -p vifu-runtime
```

Repeat package inspection and dry-run verification for each package. A
crates.io release is permanent; never package local configuration, credentials,
generated state, or build output.
