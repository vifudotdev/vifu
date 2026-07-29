# Embed The Runtime

The `vifu` crate is both the public Rust SDK and the source of the `vifu`
binary. Runtime support is included by default.

```toml
[dependencies]
vifu = "0.1"
```

One `VifuRuntime` represents one application or project. It owns the registry
of providers, agents, and named endpoints, and keeps session state behind a
host-selectable store.

## Choose A Deployment Shape

| Shape | Use it for | Contract |
| --- | --- | --- |
| Embedded Runtime | A Rust, iOS, or Android application that owns one project | The host calls `VifuRuntime` directly and registers provider implementations in process |
| Vifu Server | A deployment that operates multiple projects | Applications call project HTTP/WebSocket endpoints; Server adds keys, database state, provider configuration, and traces |
| Server with Agent Gateway | Providers that run on another machine or network boundary | Gateway connects provider resources to Server over one authenticated multiplexed connection |

Agent Gateway is a Server transport and connects to a running Vifu Server. An
embedded Runtime is self-contained at the Vifu layer: it executes directly in
the application and calls the providers registered by that host.

## Register An Agent

Providers are dynamic Rust implementations, not vendor Cargo features. The
built-in HTTP adapter can route any capability to one of its supported
protocols:

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
        persona: json!({ "instructions": "Guide the player concisely." }),
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
    name: "town-guide".into(),
    agent: "guide".into(),
    capability: "chat".into(),
    timeout_ms: 30_000,
})?;
```

A provider may expose several capabilities. Agents reference a provider by its
runtime name, and endpoints give the application a stable name even when the
provider or agent changes.

## Invoke Asynchronously

```rust
let session = runtime.session("player-42")?;
let output = session
    .invoke(InvocationInput::json(
        "town-guide",
        json!({
            "messages": [
                { "role": "user", "content": "Where is the north gate?" }
            ]
        }),
    ))
    .await?;
```

JSON and binary inputs use the same invocation contract. Sessions are
independent and updates to the same session are serialized so snapshot
revisions cannot overwrite one another.

## Invoke From A Game Loop

The game-loop API starts work on the Runtime worker and returns immediately:

```rust
let handle = runtime.start_invoke(InvocationInput::json(
    "town-guide",
    json!({ "messages": [{ "role": "user", "content": "Hello" }] }),
))?;

// Call from later frames until the operation reaches a terminal state.
let poll = runtime.poll_invocation(&handle)?;
match poll.status {
    InvocationStatus::Completed => {
        let output = poll.output.expect("completed invocation has output");
        // Apply output to application state.
    }
    InvocationStatus::Failed | InvocationStatus::Cancelled => {
        // Handle the safe public error or cancellation.
    }
    InvocationStatus::Pending | InvocationStatus::Running => {}
}

// Runtime sends cooperative cancellation to the provider.
runtime.cancel_invocation(&handle)?;
```

iOS and Android hosts use the equivalent `VifuEmbeddedRuntime` UniFFI object and
implement `VifuAgentProvider` as a native callback.

## Persist State

`VifuRuntime::new` uses `MemoryRuntimeStore`. A host can implement
`RuntimeStore` and pass it to `VifuRuntime::with_store` to persist each session
in its own database or save system.

`export_snapshot` serializes the currently loaded sessions for one project.
`restore_snapshot` validates the snapshot version and project before restoring
it. Provider definitions and credentials are not part of the snapshot. Keep
credentials inside provider implementations, not agent metadata, invocation
metadata, or runtime state.

## Add Application Behavior

The lower-level Bevy API remains available for deterministic application
behavior and custom effects:

```rust
use vifu::runtime::prelude::*;

pub struct MyRuntimePlugin;

impl Plugin for MyRuntimePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(RuntimeSchedule, handle_commands);
    }
}

fn handle_commands(
    mut commands: ResMut<RuntimeCommandQueue>,
    mut events: ResMut<RuntimeEventQueue>,
    mut effects: ResMut<EffectRequestQueue>,
    mut state: ResMut<RuntimeState>,
) {
    for command in commands.drain() {
        state.value["lastCommand"] = json!(command.name);
        events.emit("command.accepted", json!({ "commandId": command.id }));
        effects.request("agent.invoke", command.payload);
    }
}
```

`VifuRuntime::execute_effects` handles `agent.invoke` effects through the same
registered endpoint contract and returns application-defined effects to the
host.

## Features

| Feature | Adds |
| --- | --- |
| `runtime` | Providers, agents, endpoints, sessions, state, effects, and snapshots |
| `gateway` | Provider discovery and the multiplexed Agent Gateway client |
| `server` | HTTP, WebSocket, SQLite, and PostgreSQL Vifu Server |
| `full` | Runtime, Gateway, and Server library APIs |
| `binary` | The complete `vifu` executable; enabled by default |
| `local-whisper` | Optional local Whisper execution support |

Provider vendors and normal provider capabilities are registered at runtime.
Feature flags select broad binary capabilities only.

## Boundary

The crate intentionally does not define scenes, timelines, choices, character
formats, or a visual graph contract. Those concepts belong to the application
plugin that implements them.
