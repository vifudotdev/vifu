use std::collections::VecDeque;
use std::fmt;

use bevy_app::{App, Plugin};
use bevy_ecs::prelude::*;
use bevy_ecs::schedule::ScheduleLabel;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Hash, PartialEq, Eq, ScheduleLabel)]
pub struct RuntimeSchedule;

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeCommand {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub payload: Value,
}

impl fmt::Debug for RuntimeCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeCommand")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("payload", &"[REDACTED]")
            .finish()
    }
}

impl RuntimeCommand {
    pub fn new(id: impl Into<String>, name: impl Into<String>, payload: Value) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            payload,
        }
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeEvent {
    pub sequence: u64,
    pub name: String,
    #[serde(default)]
    pub payload: Value,
}

impl fmt::Debug for RuntimeEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeEvent")
            .field("sequence", &self.sequence)
            .field("name", &self.name)
            .field("payload", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectRequest {
    pub id: String,
    pub kind: String,
    #[serde(default)]
    pub payload: Value,
}

impl fmt::Debug for EffectRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EffectRequest")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("payload", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectResult {
    pub effect_id: String,
    pub succeeded: bool,
    #[serde(default)]
    pub output: Value,
}

impl fmt::Debug for EffectResult {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EffectResult")
            .field("effect_id", &self.effect_id)
            .field("succeeded", &self.succeeded)
            .field("output", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeSnapshot {
    pub revision: u64,
    #[serde(default)]
    pub state: Value,
}

impl fmt::Debug for RuntimeSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeSnapshot")
            .field("revision", &self.revision)
            .field("state", &"[REDACTED]")
            .finish()
    }
}

impl Default for RuntimeSnapshot {
    fn default() -> Self {
        Self {
            revision: 0,
            state: Value::Object(Default::default()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeAdvance {
    pub snapshot: RuntimeSnapshot,
    pub events: Vec<RuntimeEvent>,
    pub effects: Vec<EffectRequest>,
}

#[derive(Resource, Default)]
pub struct RuntimeCommandQueue {
    commands: VecDeque<RuntimeCommand>,
}

impl RuntimeCommandQueue {
    pub fn push(&mut self, command: RuntimeCommand) {
        self.commands.push_back(command);
    }

    pub fn pop_front(&mut self) -> Option<RuntimeCommand> {
        self.commands.pop_front()
    }

    pub fn drain(&mut self) -> impl Iterator<Item = RuntimeCommand> + '_ {
        self.commands.drain(..)
    }

    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }
}

#[derive(Resource, Default)]
pub struct RuntimeEventQueue {
    next_sequence: u64,
    events: Vec<RuntimeEvent>,
}

impl RuntimeEventQueue {
    pub fn emit(&mut self, name: impl Into<String>, payload: Value) -> u64 {
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.saturating_add(1);
        self.events.push(RuntimeEvent {
            sequence,
            name: name.into(),
            payload,
        });
        sequence
    }

    pub fn drain(&mut self) -> impl Iterator<Item = RuntimeEvent> + '_ {
        self.events.drain(..)
    }
}

#[derive(Resource, Default)]
pub struct EffectRequestQueue {
    next_id: u64,
    effects: Vec<EffectRequest>,
}

impl EffectRequestQueue {
    pub fn request(&mut self, kind: impl Into<String>, payload: Value) -> String {
        let id = format!("effect-{}", self.next_id);
        self.next_id = self.next_id.saturating_add(1);
        self.effects.push(EffectRequest {
            id: id.clone(),
            kind: kind.into(),
            payload,
        });
        id
    }

    pub fn drain(&mut self) -> impl Iterator<Item = EffectRequest> + '_ {
        self.effects.drain(..)
    }
}

#[derive(Resource, Default)]
pub struct EffectResultQueue {
    results: VecDeque<EffectResult>,
}

impl EffectResultQueue {
    pub fn push(&mut self, result: EffectResult) {
        self.results.push_back(result);
    }

    pub fn pop_front(&mut self) -> Option<EffectResult> {
        self.results.pop_front()
    }

    pub fn drain(&mut self) -> impl Iterator<Item = EffectResult> + '_ {
        self.results.drain(..)
    }
}

#[derive(Resource, Clone)]
pub struct RuntimeState {
    pub revision: u64,
    pub value: Value,
}

impl fmt::Debug for RuntimeState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeState")
            .field("revision", &self.revision)
            .field("value", &"[REDACTED]")
            .finish()
    }
}

impl Default for RuntimeState {
    fn default() -> Self {
        Self {
            revision: 0,
            value: Value::Object(Default::default()),
        }
    }
}

impl RuntimeState {
    pub fn snapshot(&self) -> RuntimeSnapshot {
        RuntimeSnapshot {
            revision: self.revision,
            state: self.value.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct VifuRuntimePlugin;

impl Plugin for VifuRuntimePlugin {
    fn build(&self, app: &mut App) {
        app.init_schedule(RuntimeSchedule)
            .init_resource::<RuntimeCommandQueue>()
            .init_resource::<RuntimeEventQueue>()
            .init_resource::<EffectRequestQueue>()
            .init_resource::<EffectResultQueue>()
            .init_resource::<RuntimeState>();
    }
}

pub struct HeadlessRuntime {
    app: App,
}

impl Default for HeadlessRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl HeadlessRuntime {
    pub fn new() -> Self {
        let mut app = App::empty();
        app.add_plugins(VifuRuntimePlugin);
        Self { app }
    }

    pub fn restore(snapshot: RuntimeSnapshot) -> Self {
        let mut runtime = Self::new();
        runtime.app.insert_resource(RuntimeState {
            revision: snapshot.revision,
            value: snapshot.state,
        });
        runtime
    }

    pub fn app(&self) -> &App {
        &self.app
    }

    pub fn app_mut(&mut self) -> &mut App {
        &mut self.app
    }

    pub fn snapshot(&self) -> RuntimeSnapshot {
        self.app.world().resource::<RuntimeState>().snapshot()
    }

    pub fn complete_effect(&mut self, result: EffectResult) {
        self.app
            .world_mut()
            .resource_mut::<EffectResultQueue>()
            .push(result);
    }

    pub fn enqueue_command(&mut self, command: RuntimeCommand) {
        self.app
            .world_mut()
            .resource_mut::<RuntimeCommandQueue>()
            .push(command);
    }

    pub fn run_schedule(&mut self, schedule: impl ScheduleLabel) -> RuntimeAdvance {
        self.app.world_mut().run_schedule(schedule);
        self.take_advance()
    }

    pub fn dispatch(&mut self, command: RuntimeCommand) -> RuntimeAdvance {
        self.enqueue_command(command);
        self.app.world_mut().run_schedule(RuntimeSchedule);

        {
            let mut state = self.app.world_mut().resource_mut::<RuntimeState>();
            state.revision = state.revision.saturating_add(1);
        }

        self.take_advance()
    }

    fn take_advance(&mut self) -> RuntimeAdvance {
        let snapshot = self.snapshot();
        let events = self
            .app
            .world_mut()
            .resource_mut::<RuntimeEventQueue>()
            .drain()
            .collect();
        let effects = self
            .app
            .world_mut()
            .resource_mut::<EffectRequestQueue>()
            .drain()
            .collect();

        RuntimeAdvance {
            snapshot,
            events,
            effects,
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy_ecs::prelude::{ResMut, Resource};
    use serde_json::json;

    use super::*;

    #[derive(Resource, Default)]
    struct ProcessedCommands(u64);

    fn echo_commands(
        mut commands: ResMut<RuntimeCommandQueue>,
        mut events: ResMut<RuntimeEventQueue>,
        mut effects: ResMut<EffectRequestQueue>,
        mut state: ResMut<RuntimeState>,
        mut processed: ResMut<ProcessedCommands>,
    ) {
        for command in commands.drain() {
            processed.0 += 1;
            state.value["lastCommand"] = Value::String(command.name.clone());
            events.emit("command.processed", json!({ "commandId": command.id }));
            effects.request("agent.invoke", command.payload);
        }
    }

    #[test]
    fn plugins_define_runtime_behavior() {
        let mut runtime = HeadlessRuntime::new();
        runtime
            .app_mut()
            .init_resource::<ProcessedCommands>()
            .add_systems(RuntimeSchedule, echo_commands);

        let advance = runtime.dispatch(RuntimeCommand::new(
            "command-1",
            "player.message",
            json!({ "text": "Hello" }),
        ));

        assert_eq!(advance.snapshot.revision, 1);
        assert_eq!(advance.snapshot.state["lastCommand"], "player.message");
        assert_eq!(advance.events[0].name, "command.processed");
        assert_eq!(advance.effects[0].kind, "agent.invoke");
    }

    #[test]
    fn snapshots_can_be_restored() {
        let runtime = HeadlessRuntime::restore(RuntimeSnapshot {
            revision: 7,
            state: json!({ "scene": "platform" }),
        });

        assert_eq!(runtime.snapshot().revision, 7);
        assert_eq!(runtime.snapshot().state["scene"], "platform");
    }

    #[derive(Clone, Debug, Hash, PartialEq, Eq, ScheduleLabel)]
    struct ExtensionSchedule;

    fn run_extension(
        mut commands: ResMut<RuntimeCommandQueue>,
        mut events: ResMut<RuntimeEventQueue>,
        mut state: ResMut<RuntimeState>,
    ) {
        let command = commands.pop_front().expect("extension command");
        state.revision = 9;
        state.value = command.payload;
        events.emit("extension.completed", json!({ "commandId": command.id }));
    }

    #[test]
    fn extensions_run_on_the_shared_runtime_host() {
        let mut runtime = HeadlessRuntime::new();
        runtime
            .app_mut()
            .init_schedule(ExtensionSchedule)
            .add_systems(ExtensionSchedule, run_extension);
        runtime.enqueue_command(RuntimeCommand::new(
            "command-9",
            "extension.run",
            json!({ "result": "ok" }),
        ));

        let advance = runtime.run_schedule(ExtensionSchedule);

        assert_eq!(advance.snapshot.revision, 9);
        assert_eq!(advance.snapshot.state["result"], "ok");
        assert_eq!(advance.events[0].name, "extension.completed");
    }
}
