# crates.io Release Contract

The public Rust entry point is the `vifu` crate. It provides both:

- a library SDK for embedding selected Vifu capabilities; and
- the `vifu` binary, which runs Vifu Server and Agent Gateway from the same
  configuration model used by repository builds.

## Feature Contract

```toml
# Default SDK with Runtime, Gateway, and Server support
vifu = "0.1"

# Advanced: Runtime plus Agent Gateway without the default binary feature
vifu = { version = "0.1", default-features = false, features = ["runtime", "gateway"] }

# Advanced: smallest Runtime-only dependency graph
vifu = { version = "0.1", default-features = false, features = ["runtime"] }
```

The default `binary` feature enables `full` and builds the executable:

```bash
cargo install vifu
```

Cargo cannot choose different default features based on whether `vifu` is
installed as a binary or added as a dependency. The default favors a direct
first experience; advanced embedded builds can disable default features when
dependency size matters.

## Package Layout

The repository keeps implementation crates separate so each boundary can be
tested and optimized independently:

| Package | Role |
| --- | --- |
| `vifu-runtime` | Portable Bevy execution kernel |
| `vifu-gateway` | Provider, protocol, relay, and session implementation |
| `vifu-server` | HTTP, WebSocket, and PostgreSQL server implementation |
| `vifu` | Stable public facade, feature selection, and binary |

Users should depend on `vifu`. The implementation packages are published only
because crates.io must resolve every dependency in a published package.

## Release Order

Publish one version from a clean, tagged commit in dependency order:

```bash
cargo publish -p vifu-runtime
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
