# Embed The Runtime

`vifu-game-runtime` supplies headless runtime primitives for applications that
want to own their behavior while sharing the same command, event, effect, state,
and snapshot model.

## Add A Plugin

Register normal Bevy systems on `RuntimeSchedule`:

```rust
use bevy_app::{App, Plugin};
use bevy_ecs::prelude::*;
use serde_json::json;
use vifu_game_runtime::{
    EffectRequestQueue, RuntimeCommandQueue, RuntimeEventQueue, RuntimeSchedule,
    RuntimeState,
};

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
use serde_json::json;
use vifu_game_runtime::{HeadlessRuntime, RuntimeCommand};

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
