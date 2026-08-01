# Embed The Runtime

The `vifu-runtime` crate is the portable Rust SDK for embedding Vifu directly
inside an application.

```toml
[dependencies]
vifu-runtime = "0.1"
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

## Add Vifu To An Apple Application

In Xcode, choose **File > Add Package Dependencies** and enter:

```text
https://github.com/vifudotdev/vifu
```

Choose a released version and add the `Vifu` product to the application target.
The package supports iOS 17 or newer and macOS 14 or newer.

```swift
import Vifu

let runtime = try VifuEmbeddedRuntime(projectId: "my-application")
let snapshot = try runtime.exportSnapshot()
```

The Swift source API is generated from the same UniFFI contract used by the
Rust mobile adapter. The package downloads a checksum-verified XCFramework
containing device, simulator, and macOS libraries.

Open a SQLite-backed Runtime when project configuration and session state must
survive application restarts:

```swift
let runtime = try VifuEmbeddedRuntime.open(
    projectId: "my-application",
    databasePath: runtimeDatabaseURL.path
)
```

## Register An Agent

Providers are dynamic Rust implementations, not vendor Cargo features. The
built-in HTTP adapter can route any capability to one of its supported
protocols:

```rust
use std::sync::Arc;
use vifu_runtime::prelude::*;

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
// take_invocation removes a terminal result after returning it.
let poll = runtime.take_invocation(&handle)?;
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

## Connect An Embedded Runtime To Vifu Server

Network access is optional. The application continues to invoke its embedded
Runtime directly when no Gateway is running. To pair the same Runtime with a
Server deployment:

1. Create a one-time Gateway enrollment in the project's **Deployments** page.
2. Generate a device identity and keep it in the Apple Keychain.
3. Start `VifuEmbeddedGateway` with that identity and the enrollment token.

```swift
let generated = generateVifuGatewayIdentity()
let identity = VifuGatewayIdentity(generated: generated)
let identityStore = VifuGatewayIdentityStore()
try identityStore.save(identity, for: "my-application")

let gateway = try VifuEmbeddedGateway(
    runtime: runtime,
    config: VifuEmbeddedGatewayConfig(
        serverUrl: "https://runtime.example.com",
        gatewayId: identity.gatewayId,
        runtimeDatabasePath: runtimeDatabaseURL.path
    )
)
try gateway.start(identity: identity, enrollmentToken: enrollmentToken)
```

The enrollment token is consumed once. Later starts load the same device
identity from Keychain and omit the token. Rust keeps the credential in memory;
it is not written to the Runtime SQLite database or portable manifest.

The first connection to an empty deployment imports the embedded manifest as
release 1. Later release activation, configuration sync, trace upload, and
remote invocation follow the deployment policies selected in the Dashboard.
Remote invocation is disabled by default, while configuration sync and summary
trace upload are enabled.

When native UI and an embedded game engine share one Runtime, use
`VifuRuntimeBridgeSession` as the host-facing bridge. Attach one
`VifuRuntimeBridgeConnection` to route engine `runtime.*` requests into the
embedded Runtime. Application-defined frames remain available to the host, and
each subscriber receives the same Runtime events without competing for a
single stream.

## Stream Provider Output

Providers that produce incremental output override `invoke_with_events` and
emit each text fragment through the supplied event sink:

```rust
fn invoke_with_events<'a>(
    &'a self,
    request: ProviderRequest,
    cancellation: CancellationToken,
    events: ProviderEventSink,
) -> ProviderFuture<'a> {
    Box::pin(async move {
        events.output_delta(InvocationData::Json(json!("Hello ")));
        events.output_delta(InvocationData::Json(json!("world")));
        Ok(ProviderResponse::json(json!({ "text": "Hello world" })))
    })
}
```

Direct embedded hosts can call `drain_invocation_events` with an invocation
handle. Runtime Bridge clients receive the same ordered events as encoded
frames. Each invocation emits `started`, zero or more `outputDelta` events, and
one terminal event. The Runtime bounds each invocation queue and coalesces
adjacent output deltas so a fast provider cannot grow host memory without
limit.

## Connect A Game Engine

`RuntimeBridge` exposes the Runtime through transport-neutral `req`, `res`, and
`event` frames. Engine integrations only move encoded frames; they do not
reimplement invocation, session, provider, or cancellation behavior.

```text
Godot / Unity / Unreal
          |
    engine adapter
          |
      transport
          |
 Runtime Bridge session
      /           \
 embedded       application
 Runtime         messages
```

Use `VifuInProcessBridgeTransport` when Vifu is embedded in the same
application. A WebSocket transport can implement the same
`VifuRuntimeBridgeTransport` protocol when the engine and Runtime are separate
processes or devices. Both shapes preserve the same frame contract, so moving
execution does not require rewriting game logic.

The initial Godot frame adapter is in `integrations/godot/apple/`. It only
connects `GlobalState` signals to `VifuInProcessBridgeTransport` and attaches
to an already-started `GodotInstance`; the host application retains Godot's
creation, rendering, frame-loop, restart, and destruction lifecycle. Runtime
routing belongs to `VifuRuntimeBridgeSession`, while application-specific
message decoding stays in the host. Unity and Unreal adapters can implement the
same small frame transport against their native plugin systems.

The optional `vifu-provider-llama` crate implements this contract for local
GGUF models through llama.cpp. Apple applications can enable the same provider
through the `local-llama` feature of `vifu-mobile-ffi`. The provider crate's
[`chat` example](../crates/vifu-provider-llama/examples/chat.rs) demonstrates
the same runtime registration and invocation path.

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
use vifu_runtime::prelude::*;

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

Provider vendors and normal provider capabilities are registered at runtime.
Optional local Whisper execution is available through the `local-whisper`
feature.

## Boundary

The crate intentionally does not define scenes, timelines, choices, character
formats, or a visual graph contract. Those concepts belong to the application
plugin that implements them.
