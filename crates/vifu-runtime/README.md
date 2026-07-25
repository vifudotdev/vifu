# vifu-runtime

`vifu-runtime` is the small, stateful execution kernel at the center of Vifu.
Most applications should depend on the `vifu` crate and enable its `runtime`
feature. This lower-level crate remains available for hosts that only need the
kernel implementation.

The crate uses Bevy App and ECS primitives without the renderer, windowing
stack, database, HTTP server, or Vifu Console.

## Use the public Vifu SDK

```toml
[dependencies]
vifu = { version = "0.1", default-features = false, features = ["runtime"] }
```

## Embed a runtime

```rust
use vifu::runtime::prelude::*;

struct ApplicationPlugin;

impl Plugin for ApplicationPlugin {
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

let mut runtime = HeadlessRuntime::new();
runtime.app_mut().add_plugins(ApplicationPlugin);

let advance = runtime.dispatch(RuntimeCommand::new(
    "command-1",
    "application.input",
    json!({ "text": "Hello" }),
));

assert_eq!(advance.snapshot.revision, 1);
```

The host executes requested effects and returns their results through
`HeadlessRuntime::complete_effect`. Persist `RuntimeSnapshot` when state must
survive restarts.

Vifu Server, Agent Gateway, PostgreSQL, and the web Console are separate
components in the [Vifu repository](https://github.com/vifudotdev/vifu).
