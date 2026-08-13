# Build A Rust Agent With Vifu

Rust applications embed the core Runtime directly.

## 1. Add The Crate

```toml
[dependencies]
vifu-runtime = "0.1"
```

Use `default-features = false` for the smallest core. Enable or add only the
Provider integrations required by the host.

## 2. Implement A Provider

Implement `AgentProvider`. In `invoke_with_events`, report liveness, output
deltas, and typed stages through `ProviderEventSink`. Respect the supplied
`CancellationToken`.

## 3. Register The Runtime Graph

```rust
use std::sync::Arc;
use vifu_runtime::prelude::*;

let runtime = VifuRuntime::new("my-rust-app")?;
runtime.register_provider("models", Arc::new(provider))?;
runtime.register_agent(AgentDefinition {
    id: "guide".into(),
    name: "Guide".into(),
    provider: "models".into(),
    capabilities: vec!["chat".into()],
    metadata: serde_json::json!({ "model": "my-local-model" }),
})?;
runtime.register_endpoint(EndpointDefinition {
    name: "chat".into(),
    agent: "guide".into(),
    capability: "chat".into(),
    timeout_ms: 30_000,
})?;
```

## 4. Invoke

```rust
let output = runtime
    .invoke(InvocationInput::json(
        "chat",
        serde_json::json!({ "prompt": "Hello" }),
    ))
    .await?;
```

Use `start_invoke`, `drain_invocation_events`, and `poll_invocation` for a game
loop that cannot await. Completed provider stages are also present in
`output.trace`.

## 5. Add Gateway

Use `vifu-gateway::embedded::EmbeddedRuntimeGateway` when this Runtime must
join a Vifu Server. Keep the machine private key and Server device token in the
host credential store. The complete lifecycle is in the
[Runtime embedding guide](../runtime-embedding.md#connect-an-embedded-runtime-to-vifu-server).

Run the public API proof with:

```bash
cargo test -p vifu-runtime --test public_api
```
