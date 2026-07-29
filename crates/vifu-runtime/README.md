# vifu-runtime

`vifu-runtime` is the small, stateful execution kernel at the center of Vifu.
Most applications should depend on the `vifu` crate, which includes Runtime
support by default. This lower-level crate remains available for hosts that only
need the kernel implementation.

The crate uses Bevy App and ECS primitives without the renderer, windowing
stack, database, HTTP server, or Vifu Console. It also provides the higher-level
`VifuRuntime` API used by applications and Vifu Server.

## Use the public Vifu SDK

```toml
[dependencies]
vifu = "0.1"
```

## Register providers, agents, and endpoints

```rust
use std::sync::Arc;
use vifu::runtime::prelude::*;

let mut provider = HttpCapabilityProvider::new(
    "models",
    "https://provider.example.com/v1",
    Some(std::env::var("MODEL_API_TOKEN")?),
)?;
provider.add_route(
    "chat",
    HttpCapabilityRoute::OpenAiChat {
        model: "model-name".into(),
        persona: json!({ "instructions": "Be concise." }),
    },
)?;

let runtime = VifuRuntime::new("my-application")?;
runtime.register_provider("models", Arc::new(provider))?;
runtime.register_agent(AgentDefinition {
    id: "guide".into(),
    name: "Guide".into(),
    provider: "models".into(),
    capabilities: vec!["chat".into()],
    metadata: json!({}),
})?;
runtime.register_endpoint(EndpointDefinition {
    name: "guide".into(),
    agent: "guide".into(),
    capability: "chat".into(),
    timeout_ms: 30_000,
})?;

let output = runtime
    .invoke(InvocationInput::json(
        "guide",
        json!({ "messages": [{ "role": "user", "content": "Hello" }] }),
    ))
    .await?;
```

Providers implement `AgentProvider` and are registered dynamically, so an
application can connect an in-process agent, an HTTP model service, or its own
transport without changing Cargo features. `MemoryRuntimeStore` is the default.
Pass a host `RuntimeStore` to `VifuRuntime::with_store` for durable sessions, or
use project snapshot export and restore.

`start_invoke`, `poll_invocation`, and `cancel_invocation` provide the same
result contract to synchronous game loops. The lower-level `HeadlessRuntime`
remains available for Bevy plugins, command/event processing, and custom
effects.

Vifu Server reuses this invocation model and adds multi-project HTTP APIs,
access control, database persistence, and traces. Agent Gateway is an optional
Server transport for providers running on another machine.
