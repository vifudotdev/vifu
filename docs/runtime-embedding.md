# Embed The Runtime

The `vifu` crate is both the public Rust SDK and the source of the `vifu`
binary. Runtime support is included by default.

Add Vifu to an application:

```toml
[dependencies]
vifu = "0.1"
```

| Feature | Adds |
| --- | --- |
| `runtime` | Portable command, state, event, effect, and snapshot runtime |
| `gateway` | Agent Provider discovery and the multiplexed Agent Gateway client |
| `server` | HTTP, WebSocket, SQLite, and PostgreSQL Vifu Server |
| `full` | Runtime, Gateway, and Server library APIs |
| `binary` | The complete `vifu` executable; enabled by default |
| `local-whisper` | Optional local Whisper provider support |

The lower-level `vifu-runtime` package supplies the headless execution kernel.
Application code should normally import it through `vifu::runtime`.

Advanced builds with strict dependency-size requirements can disable default
features and select only the capabilities they use.

## Add A Plugin

Register normal Bevy systems on `RuntimeSchedule`:

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

## Run Headlessly

```rust
use vifu::runtime::prelude::{json, HeadlessRuntime, RuntimeCommand};

let mut runtime = HeadlessRuntime::new();
runtime.app_mut().add_plugins(MyRuntimePlugin);

let advance = runtime.dispatch(RuntimeCommand::new(
    "command-1",
    "player.message",
    json!({ "text": "Hello" }),
));

println!("{}", advance.snapshot.revision);
```

The host executes requested effects and returns results through
`HeadlessRuntime::complete_effect`. Persist `RuntimeSnapshot` when sessions must
survive process restarts.

## Boundary

The crate intentionally does not define scenes, timelines, choices, character
formats, or a visual graph contract. Those concepts belong to the application
plugin that implements them.
