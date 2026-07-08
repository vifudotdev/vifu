# Contributing

Thanks for improving Vifu.

## Product Boundary

Vifu is a narrow CLI for local AI agent connectivity. Keep new features within
that boundary unless the project maintainers explicitly accept a broader scope.

Avoid adding complex command trees. Prefer:

```bash
vifu
vifu --status
vifu --doctor
```

## Development

Run the focused checks before opening a pull request:

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
cargo build
cargo check
```

## Pull Requests

Please include:

- what changed
- why it belongs in the CLI
- user-visible behavior
- tests run

Security-sensitive changes should explain what secrets stay local and what is
allowed to cross the network boundary.
