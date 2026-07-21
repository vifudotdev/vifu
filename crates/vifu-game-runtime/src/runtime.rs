use std::collections::{BTreeMap, BTreeSet};

use bevy_app::{App, Plugin};
use bevy_ecs::prelude::*;
use bevy_ecs::schedule::{IntoScheduleConfigs, ScheduleLabel, SystemSet};
use serde_json::{json, Value};

use crate::condition::{evaluate_condition, ConditionExpression};
use crate::contract::{
    CompiledNode, ConversationMessage, EffectKind, EffectRequest, EffectResult, GameCommand,
    GameEvent, GamePlanV1, GameSnapshotV1, PendingHostAction, RuntimeAdvance, RuntimeFailure,
    SessionStatus,
};
use crate::error::GameRuntimeError;
use crate::GAME_SCHEMA_VERSION;

const DEFAULT_STEP_LIMIT: u32 = 10_000;

#[derive(Clone, Debug, Hash, PartialEq, Eq, ScheduleLabel)]
struct GameRuntimeSchedule;

#[derive(Clone, Debug, Hash, PartialEq, Eq, SystemSet)]
enum RuntimeStage {
    Ingress,
    Route,
    ExecuteNodes,
    EmitEffects,
    Commit,
    Egress,
}

#[derive(Component)]
struct RuntimeNodeEntity {
    ordinal: u32,
}

#[derive(Resource)]
struct RuntimeContext {
    plan: GamePlanV1,
    snapshot: GameSnapshotV1,
    command: Option<GameCommand>,
    last_command_data: Value,
    events: Vec<GameEvent>,
    effects: Vec<EffectRequest>,
    node_executions: Vec<crate::contract::NodeExecution>,
    step_limit: u32,
    error: Option<GameRuntimeError>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct GameRuntimePlugin;

impl Plugin for GameRuntimePlugin {
    fn build(&self, app: &mut App) {
        app.init_schedule(GameRuntimeSchedule)
            .configure_sets(
                GameRuntimeSchedule,
                (
                    RuntimeStage::Ingress,
                    RuntimeStage::Route,
                    RuntimeStage::ExecuteNodes,
                    RuntimeStage::EmitEffects,
                    RuntimeStage::Commit,
                    RuntimeStage::Egress,
                )
                    .chain(),
            )
            .add_systems(
                GameRuntimeSchedule,
                ingress_system.in_set(RuntimeStage::Ingress),
            )
            .add_systems(
                GameRuntimeSchedule,
                route_integrity_system.in_set(RuntimeStage::Route),
            )
            .add_systems(
                GameRuntimeSchedule,
                execute_nodes_system.in_set(RuntimeStage::ExecuteNodes),
            )
            .add_systems(
                GameRuntimeSchedule,
                retain_effects_system.in_set(RuntimeStage::EmitEffects),
            )
            .add_systems(
                GameRuntimeSchedule,
                commit_revision_system.in_set(RuntimeStage::Commit),
            )
            .add_systems(
                GameRuntimeSchedule,
                finalize_status_system.in_set(RuntimeStage::Egress),
            );
    }
}

pub struct GameRuntime {
    app: App,
}

impl GameRuntime {
    pub fn new(plan: GamePlanV1, random_seed: u64) -> Result<Self, GameRuntimeError> {
        let snapshot = GameSnapshotV1::initial(plan.entry_node, &plan.variables, random_seed);
        Self::restore(plan, snapshot)
    }

    pub fn restore(plan: GamePlanV1, snapshot: GameSnapshotV1) -> Result<Self, GameRuntimeError> {
        if plan.schema_version != GAME_SCHEMA_VERSION {
            return Err(GameRuntimeError::UnsupportedSchemaVersion(
                plan.schema_version,
            ));
        }
        if snapshot.schema_version != GAME_SCHEMA_VERSION {
            return Err(GameRuntimeError::UnsupportedSchemaVersion(
                snapshot.schema_version,
            ));
        }
        if plan
            .nodes
            .iter()
            .all(|node| node.ordinal != plan.entry_node)
        {
            return Err(GameRuntimeError::InvalidPlan(
                "entry node ordinal does not exist".to_string(),
            ));
        }
        if snapshot
            .current_nodes
            .iter()
            .any(|ordinal| plan.nodes.iter().all(|node| node.ordinal != *ordinal))
        {
            return Err(GameRuntimeError::InvalidState(
                "snapshot references a node that is not in the plan".to_string(),
            ));
        }

        let mut app = App::empty();
        app.add_plugins(GameRuntimePlugin);
        for node in &plan.nodes {
            app.world_mut().spawn(RuntimeNodeEntity {
                ordinal: node.ordinal,
            });
        }
        app.insert_resource(RuntimeContext {
            plan,
            snapshot,
            command: None,
            last_command_data: Value::Null,
            events: Vec::new(),
            effects: Vec::new(),
            node_executions: Vec::new(),
            step_limit: DEFAULT_STEP_LIMIT,
            error: None,
        });
        Ok(Self { app })
    }

    pub fn with_step_limit(mut self, step_limit: u32) -> Self {
        self.app
            .world_mut()
            .resource_mut::<RuntimeContext>()
            .step_limit = step_limit.max(1);
        self
    }

    pub fn snapshot(&self) -> &GameSnapshotV1 {
        &self.app.world().resource::<RuntimeContext>().snapshot
    }

    pub fn plan(&self) -> &GamePlanV1 {
        &self.app.world().resource::<RuntimeContext>().plan
    }

    pub fn dispatch(&mut self, command: GameCommand) -> Result<RuntimeAdvance, GameRuntimeError> {
        {
            let mut context = self.app.world_mut().resource_mut::<RuntimeContext>();
            if context.command.is_some() {
                return Err(GameRuntimeError::InvalidState(
                    "a command is already being processed".to_string(),
                ));
            }
            context.events.clear();
            context.effects.clear();
            context.node_executions.clear();
            context.error = None;
            context.command = Some(command);
        }
        self.app.world_mut().run_schedule(GameRuntimeSchedule);
        let mut context = self.app.world_mut().resource_mut::<RuntimeContext>();
        if let Some(error) = context.error.take() {
            return Err(error);
        }
        Ok(RuntimeAdvance {
            snapshot: context.snapshot.clone(),
            events: context.events.clone(),
            effects: context.effects.clone(),
            node_executions: context.node_executions.clone(),
        })
    }
}

fn ingress_system(mut context: ResMut<RuntimeContext>) {
    let Some(command) = context.command.take() else {
        context.error = Some(GameRuntimeError::InvalidState(
            "runtime schedule was invoked without a command".to_string(),
        ));
        return;
    };
    context.last_command_data = command.data.clone();

    if command.command_type == "game.cancel" {
        if is_terminal(&context.snapshot.status) {
            context.error = Some(GameRuntimeError::SessionFinished(
                context.snapshot.status.to_string(),
            ));
            return;
        }
        context.snapshot.status = SessionStatus::Cancelled;
        emit_event(
            &mut context,
            "game.session.cancelled",
            None,
            json!({"reason": command.data}),
        );
        return;
    }

    if is_terminal(&context.snapshot.status) {
        context.error = Some(GameRuntimeError::SessionFinished(
            context.snapshot.status.to_string(),
        ));
        return;
    }

    match context.snapshot.status {
        SessionStatus::WaitingInput => resume_input(&mut context, &command),
        SessionStatus::WaitingEffect => resume_effect(&mut context, &command),
        SessionStatus::WaitingHost => resume_host_action(&mut context, &command),
        SessionStatus::Running => {
            context.error = Some(GameRuntimeError::InvalidState(
                "runtime was already running when a command arrived".to_string(),
            ));
        }
        SessionStatus::Completed | SessionStatus::Failed | SessionStatus::Cancelled => {
            unreachable!("terminal states returned above")
        }
    }
}

fn resume_input(context: &mut RuntimeContext, command: &GameCommand) {
    let Some(node) = current_node(context).cloned() else {
        context.error = Some(GameRuntimeError::InvalidState(
            "waiting session has no current node".to_string(),
        ));
        return;
    };

    if context.snapshot.revision == 0 && node.node_type == "start" {
        if command.command_type != "game.start" {
            context.error = Some(GameRuntimeError::UnexpectedCommand {
                expected: "game.start".to_string(),
                actual: command.command_type.clone(),
            });
            return;
        }
        if let Err(error) = validate_value(&context.plan.inputs, &command.data) {
            context.error = Some(GameRuntimeError::InvalidState(format!(
                "game input does not match the published schema: {error}"
            )));
            return;
        }
        context.snapshot.status = SessionStatus::Running;
        return;
    }

    let expected = match node.node_type.as_str() {
        "choice" => "player.choice",
        "input" | "event" => node
            .config
            .get("commandType")
            .and_then(Value::as_str)
            .unwrap_or("player.text"),
        _ => {
            context.error = Some(GameRuntimeError::InvalidState(format!(
                "node `{}` cannot wait for player input",
                node.id
            )));
            return;
        }
    };
    if command.command_type != expected {
        context.error = Some(GameRuntimeError::UnexpectedCommand {
            expected: expected.to_string(),
            actual: command.command_type.clone(),
        });
        return;
    }
    if let Some(schema) = node.config.get("inputSchema") {
        if let Err(error) = validate_value(schema, &command.data) {
            context.error = Some(GameRuntimeError::InvalidState(format!(
                "input for node `{}` is invalid: {error}",
                node.id
            )));
            return;
        }
    }

    context
        .snapshot
        .node_outputs
        .insert(node.id.clone(), command.data.clone());
    let selected_port = if node.node_type == "choice" {
        let Some(option_id) = command.data.get("optionId").and_then(Value::as_str) else {
            context.error = Some(GameRuntimeError::InvalidState(
                "player.choice requires data.optionId".to_string(),
            ));
            return;
        };
        emit_event(
            context,
            "choice.selected",
            Some(node.id.clone()),
            json!({"optionId": option_id}),
        );
        option_id.to_string()
    } else {
        "next".to_string()
    };
    context.snapshot.status = SessionStatus::Running;
    route_from_node(context, node.ordinal, &selected_port, false);
}

fn resume_effect(context: &mut RuntimeContext, command: &GameCommand) {
    if command.command_type != "effect.completed" {
        context.error = Some(GameRuntimeError::UnexpectedCommand {
            expected: "effect.completed".to_string(),
            actual: command.command_type.clone(),
        });
        return;
    }
    let result: EffectResult = match serde_json::from_value(command.data.clone()) {
        Ok(result) => result,
        Err(error) => {
            context.error = Some(GameRuntimeError::InvalidState(format!(
                "effect result is invalid: {error}"
            )));
            return;
        }
    };
    let Some(pending) = context.snapshot.pending_effect.clone() else {
        context.error = Some(GameRuntimeError::InvalidState(
            "session has no pending effect".to_string(),
        ));
        return;
    };
    if result.effect_id != pending.effect_id {
        context.error = Some(GameRuntimeError::InvalidState(format!(
            "effect `{}` does not match pending effect `{}`",
            result.effect_id, pending.effect_id
        )));
        return;
    }
    let Some(node) = current_node(context).cloned() else {
        context.error = Some(GameRuntimeError::InvalidState(
            "pending effect has no current node".to_string(),
        ));
        return;
    };
    context.snapshot.pending_effect = None;
    context.snapshot.status = SessionStatus::Running;
    if let Some(error) = result.error {
        let event_type = if node.node_type == "agent" {
            "agent.failed"
        } else {
            "tool.failed"
        };
        emit_event(
            context,
            event_type,
            Some(node.id.clone()),
            json!({"code": error.code, "message": error.message}),
        );
        if has_route(context, node.ordinal, "error") {
            route_from_node(context, node.ordinal, "error", false);
        } else {
            fail_runtime(context, error.code, error.message, Some(node.id));
        }
        return;
    }

    let output = result.output.unwrap_or(Value::Null);
    if let Some(schema) = node.config.get("outputSchema") {
        match jsonschema::validator_for(schema) {
            Ok(validator) => {
                if let Err(error) = validator.validate(&output) {
                    fail_runtime(
                        context,
                        "agent_output_invalid",
                        error.to_string(),
                        Some(node.id),
                    );
                    return;
                }
            }
            Err(error) => {
                fail_runtime(
                    context,
                    "agent_output_schema_invalid",
                    error.to_string(),
                    Some(node.id),
                );
                return;
            }
        }
    }
    if node.node_type == "agent" {
        apply_agent_output(context, &node, &output);
    }
    context
        .snapshot
        .node_outputs
        .insert(node.id.clone(), output.clone());
    let (event_type, public_output) = if node.node_type == "agent" {
        ("agent.completed", public_agent_output(&output))
    } else {
        ("tool.completed", json!({"effectId": result.effect_id}))
    };
    emit_event(context, event_type, Some(node.id.clone()), public_output);
    route_from_node(context, node.ordinal, "next", false);
}

fn resume_host_action(context: &mut RuntimeContext, command: &GameCommand) {
    let success = match command.command_type.as_str() {
        "host.action.completed" => true,
        "host.action.failed" => false,
        _ => {
            context.error = Some(GameRuntimeError::UnexpectedCommand {
                expected: "host.action.completed or host.action.failed".to_string(),
                actual: command.command_type.clone(),
            });
            return;
        }
    };
    let Some(pending) = context.snapshot.pending_host_action.clone() else {
        context.error = Some(GameRuntimeError::InvalidState(
            "session has no pending host action".to_string(),
        ));
        return;
    };
    let actual_action_id = command
        .data
        .get("actionId")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if actual_action_id != pending.action_id {
        context.error = Some(GameRuntimeError::InvalidState(format!(
            "host action `{actual_action_id}` does not match pending action `{}`",
            pending.action_id
        )));
        return;
    }
    let Some(node) = current_node(context).cloned() else {
        context.error = Some(GameRuntimeError::InvalidState(
            "pending host action has no current node".to_string(),
        ));
        return;
    };
    context.snapshot.pending_host_action = None;
    context.snapshot.status = SessionStatus::Running;
    if success {
        emit_event(
            context,
            "host.action.completed",
            Some(node.id.clone()),
            json!({"actionId": pending.action_id}),
        );
        route_from_node(context, node.ordinal, "next", false);
    } else if has_route(context, node.ordinal, "error") {
        route_from_node(context, node.ordinal, "error", false);
    } else {
        fail_runtime(
            context,
            "host_action_failed",
            "the host reported that the action failed",
            Some(node.id),
        );
    }
}

fn route_integrity_system(
    mut context: ResMut<RuntimeContext>,
    runtime_nodes: Query<&RuntimeNodeEntity>,
) {
    if context.error.is_some() || context.snapshot.status != SessionStatus::Running {
        return;
    }
    let known: BTreeMap<_, _> = runtime_nodes
        .iter()
        .map(|node| (node.ordinal, ()))
        .collect();
    if let Some(ordinal) = context
        .snapshot
        .current_nodes
        .iter()
        .find(|ordinal| !known.contains_key(ordinal))
    {
        context.error = Some(GameRuntimeError::InvalidState(format!(
            "runtime node entity {ordinal} is missing"
        )));
    }
}

fn execute_nodes_system(mut context: ResMut<RuntimeContext>) {
    if context.error.is_some() || context.snapshot.status != SessionStatus::Running {
        return;
    }
    let mut command_steps = 0;
    while context.snapshot.status == SessionStatus::Running {
        if command_steps >= context.step_limit {
            let step_limit = context.step_limit;
            let node_id = current_node(&context).map(|node| node.id.clone());
            fail_runtime(
                &mut context,
                "step_limit_exceeded",
                format!("runtime exceeded its {step_limit}-step command limit"),
                node_id,
            );
            break;
        }
        command_steps += 1;
        context.snapshot.total_steps += 1;
        let Some(node) = current_node(&context).cloned() else {
            fail_runtime(
                &mut context,
                "route_missing",
                "runtime has no node left to execute",
                None,
            );
            break;
        };
        let sequence = context.snapshot.total_steps;
        context
            .node_executions
            .push(crate::contract::NodeExecution {
                sequence,
                ordinal: node.ordinal,
                node_id: node.id.clone(),
                node_type: node.node_type.clone(),
            });
        execute_node(&mut context, &node);
    }
}

fn execute_node(context: &mut RuntimeContext, node: &CompiledNode) {
    match node.node_type.as_str() {
        "start" => {
            emit_event(
                context,
                "game.session.started",
                Some(node.id.clone()),
                json!({}),
            );
            route_from_node(context, node.ordinal, "next", false);
        }
        "end" => {
            let output = node
                .config
                .get("output")
                .map(|value| resolve_value(value, &runtime_context_value(context)))
                .unwrap_or_else(|| {
                    context
                        .snapshot
                        .public_output
                        .clone()
                        .unwrap_or(Value::Null)
                });
            context.snapshot.public_output = Some(output.clone());
            remove_active_node(context, node.ordinal);
            if context.snapshot.current_nodes.is_empty() {
                context.snapshot.status = SessionStatus::Completed;
                emit_event(
                    context,
                    "game.session.completed",
                    Some(node.id.clone()),
                    json!({"output": output}),
                );
            }
        }
        "input" | "event" => {
            context.snapshot.status = SessionStatus::WaitingInput;
            emit_event(
                context,
                "game.session.waiting",
                Some(node.id.clone()),
                json!({
                    "for": "input",
                    "commandType": node.config.get("commandType").and_then(Value::as_str).unwrap_or("player.text")
                }),
            );
        }
        "choice" => {
            context.snapshot.status = SessionStatus::WaitingInput;
            emit_event(
                context,
                "choice.presented",
                Some(node.id.clone()),
                json!({
                    "prompt": node.config.get("prompt").cloned().unwrap_or(Value::Null),
                    "options": node.config.get("options").cloned().unwrap_or_else(|| json!([]))
                }),
            );
        }
        "agent" | "tool" => request_effect(context, node),
        "host_action" => request_host_action(context, node),
        "condition" => {
            let condition = node.config.get("condition").and_then(|value| {
                serde_json::from_value::<ConditionExpression>(value.clone()).ok()
            });
            let matches = condition.as_ref().is_some_and(|condition| {
                evaluate_condition(condition, &runtime_context_value(context))
            });
            route_from_node(
                context,
                node.ordinal,
                if matches { "true" } else { "false" },
                false,
            );
        }
        "parallel" => route_all_from_node(context, node.ordinal),
        "loop" | "for_each" => execute_bounded_loop(context, node),
        "random" => {
            let ports: Vec<_> = context
                .plan
                .edges
                .iter()
                .filter(|edge| edge.source_node == node.ordinal)
                .map(|edge| edge.source_port.clone())
                .collect();
            if ports.is_empty() {
                fail_runtime(
                    context,
                    "route_missing",
                    "Random node has no output routes",
                    Some(node.id.clone()),
                );
            } else {
                let index = deterministic_index(&mut context.snapshot, ports.len());
                route_from_node(context, node.ordinal, &ports[index], false);
            }
        }
        "state" | "character_state" | "relationship" | "memory" => {
            if let Some(key) = node.config.get("key").and_then(Value::as_str) {
                let value = node
                    .config
                    .get("value")
                    .map(|value| resolve_value(value, &runtime_context_value(context)))
                    .unwrap_or(Value::Null);
                set_state_value(&mut context.snapshot.state, key, value.clone());
                emit_event(
                    context,
                    "state.changed",
                    Some(node.id.clone()),
                    json!({"key": key, "value": value}),
                );
            }
            route_from_node(context, node.ordinal, "next", false);
        }
        "output" => {
            let value = node
                .config
                .get("value")
                .map(|value| resolve_value(value, &runtime_context_value(context)))
                .unwrap_or_else(|| context.snapshot.state.clone());
            if let Err(error) = validate_value(&context.plan.outputs, &value) {
                fail_runtime(context, "game_output_invalid", error, Some(node.id.clone()));
                return;
            }
            context.snapshot.public_output = Some(value);
            route_from_node(context, node.ordinal, "next", false);
        }
        "scene" => {
            emit_event(
                context,
                "scene.entered",
                Some(node.id.clone()),
                node.config.clone(),
            );
            route_from_node(context, node.ordinal, "next", false);
        }
        "dialogue" => {
            emit_event(
                context,
                "dialogue.started",
                Some(node.id.clone()),
                node.config.clone(),
            );
            emit_event(
                context,
                "dialogue.completed",
                Some(node.id.clone()),
                node.config.clone(),
            );
            route_from_node(context, node.ordinal, "next", false);
        }
        "ending" => {
            emit_event(
                context,
                "ending.reached",
                Some(node.id.clone()),
                node.config.clone(),
            );
            route_from_node(context, node.ordinal, "next", false);
        }
        "background" | "character_visual" | "expression" | "audio" | "video" | "voice"
        | "subtitle" | "transition" | "camera_cue" | "delay" => {
            emit_event(
                context,
                presentation_event_type(&node.node_type),
                Some(node.id.clone()),
                node.config.clone(),
            );
            route_from_node(context, node.ordinal, "next", false);
        }
        _ => route_from_node(context, node.ordinal, "next", false),
    }
}

fn request_effect(context: &mut RuntimeContext, node: &CompiledNode) {
    let effect_id = format!("effect-{}", context.snapshot.next_effect_sequence);
    context.snapshot.next_effect_sequence += 1;
    let kind = if node.node_type == "agent" {
        EffectKind::Agent
    } else {
        EffectKind::Tool
    };
    let descriptor = if kind == EffectKind::Agent {
        let agent_id = node
            .config
            .get("agentId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        context
            .plan
            .agents
            .iter()
            .find(|agent| agent.id == agent_id)
            .map(|agent| agent.execution_descriptor.clone())
            .unwrap_or_else(|| json!({"agentId": agent_id}))
    } else {
        let agent_id = node
            .config
            .get("agentId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let mut descriptor = context
            .plan
            .agents
            .iter()
            .find(|agent| agent.id == agent_id)
            .map(|agent| agent.execution_descriptor.clone())
            .unwrap_or_else(|| json!({"agentId": agent_id}));
        if let Some(object) = descriptor.as_object_mut() {
            object.insert(
                "tool".to_string(),
                node.config.get("tool").cloned().unwrap_or(Value::Null),
            );
        }
        descriptor
    };
    let input = node
        .config
        .get("input")
        .map(|value| resolve_value(value, &runtime_context_value(context)))
        .unwrap_or_else(|| context.last_command_data.clone());
    let effect = EffectRequest {
        effect_id,
        node_id: node.id.clone(),
        kind,
        descriptor,
        input,
    };
    context.snapshot.pending_effect = Some(effect.clone());
    context.snapshot.status = SessionStatus::WaitingEffect;
    context.effects.push(effect.clone());
    emit_event(
        context,
        if node.node_type == "agent" {
            "agent.requested"
        } else {
            "tool.requested"
        },
        Some(node.id.clone()),
        json!({"effectId": effect.effect_id}),
    );
}

fn request_host_action(context: &mut RuntimeContext, node: &CompiledNode) {
    let action_id = format!("action-{}", context.snapshot.next_effect_sequence);
    context.snapshot.next_effect_sequence += 1;
    let pending = PendingHostAction {
        action_id: action_id.clone(),
        node_id: node.id.clone(),
        target: node
            .config
            .get("target")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        action: node
            .config
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        arguments: node
            .config
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({})),
    };
    let completion_required = node
        .config
        .get("completion")
        .and_then(Value::as_str)
        .unwrap_or("required")
        == "required";
    emit_event(
        context,
        "host.action.requested",
        Some(node.id.clone()),
        json!({
            "actionId": pending.action_id,
            "target": pending.target,
            "action": pending.action,
            "arguments": pending.arguments,
            "completion": if completion_required { "required" } else { "optional" }
        }),
    );
    if completion_required {
        context.snapshot.pending_host_action = Some(pending);
        context.snapshot.status = SessionStatus::WaitingHost;
    } else {
        route_from_node(context, node.ordinal, "next", false);
    }
}

fn execute_bounded_loop(context: &mut RuntimeContext, node: &CompiledNode) {
    let max_iterations = node
        .config
        .get("maxIterations")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let key = format!("__vifu_loop_{}", node.id);
    let iteration = context
        .snapshot
        .state
        .get(&key)
        .and_then(Value::as_u64)
        .unwrap_or_default();
    if iteration < max_iterations {
        set_state_value(
            &mut context.snapshot.state,
            &key,
            Value::from(iteration + 1),
        );
        route_from_node(context, node.ordinal, "body", false);
    } else {
        set_state_value(&mut context.snapshot.state, &key, Value::from(0));
        route_from_node(context, node.ordinal, "done", false);
    }
}

fn route_from_node(
    context: &mut RuntimeContext,
    source_node: u32,
    source_port: &str,
    allow_multiple: bool,
) {
    let condition_context = runtime_context_value(context);
    let mut targets: Vec<_> = context
        .plan
        .edges
        .iter()
        .filter(|edge| edge.source_node == source_node && edge.source_port == source_port)
        .filter(|edge| {
            edge.condition
                .as_ref()
                .is_none_or(|condition| evaluate_condition(condition, &condition_context))
        })
        .map(|edge| edge.target_node)
        .collect();
    targets.sort_unstable();
    targets.dedup();
    if !allow_multiple {
        targets.truncate(1);
    }
    route_to_targets(context, source_node, source_port, targets, allow_multiple);
}

fn route_all_from_node(context: &mut RuntimeContext, source_node: u32) {
    let condition_context = runtime_context_value(context);
    let targets = context
        .plan
        .edges
        .iter()
        .filter(|edge| edge.source_node == source_node)
        .filter(|edge| {
            edge.condition
                .as_ref()
                .is_none_or(|condition| evaluate_condition(condition, &condition_context))
        })
        .map(|edge| edge.target_node)
        .collect();
    route_to_targets(context, source_node, "branch", targets, true);
}

fn route_to_targets(
    context: &mut RuntimeContext,
    source_node: u32,
    source_port: &str,
    mut targets: Vec<u32>,
    allow_multiple: bool,
) {
    targets.sort_unstable();
    targets.dedup();
    if !allow_multiple {
        targets.truncate(1);
    }
    if targets.is_empty() {
        let node_id = context
            .plan
            .nodes
            .iter()
            .find(|node| node.ordinal == source_node)
            .map(|node| node.id.clone());
        fail_runtime(
            context,
            "route_missing",
            format!("no `{source_port}` route is available"),
            node_id,
        );
        return;
    }

    let ready_targets = targets
        .into_iter()
        .filter(|target| register_join_arrival(context, source_node, *target))
        .collect::<Vec<_>>();
    let mut remaining = context.snapshot.current_nodes.clone();
    if let Some(index) = remaining.iter().position(|ordinal| *ordinal == source_node) {
        remaining.remove(index);
    }
    let mut next = ready_targets;
    next.extend(remaining);
    let mut seen = BTreeMap::new();
    next.retain(|ordinal| seen.insert(*ordinal, ()).is_none());
    context.snapshot.current_nodes = next;
    if context.snapshot.current_nodes.is_empty() {
        fail_runtime(
            context,
            "join_incomplete",
            "runtime reached a Join before every branch arrived",
            None,
        );
    }
}

fn register_join_arrival(context: &mut RuntimeContext, source_node: u32, target_node: u32) -> bool {
    let is_join = context
        .plan
        .nodes
        .iter()
        .any(|node| node.ordinal == target_node && node.node_type == "join");
    if !is_join {
        return true;
    }
    let required = context
        .plan
        .edges
        .iter()
        .filter(|edge| edge.target_node == target_node)
        .map(|edge| edge.source_node)
        .collect::<BTreeSet<_>>()
        .len();
    let arrivals = context
        .snapshot
        .join_arrivals
        .entry(target_node)
        .or_default();
    arrivals.insert(source_node);
    if arrivals.len() < required {
        return false;
    }
    context.snapshot.join_arrivals.remove(&target_node);
    true
}

fn remove_active_node(context: &mut RuntimeContext, ordinal: u32) {
    if let Some(index) = context
        .snapshot
        .current_nodes
        .iter()
        .position(|current| *current == ordinal)
    {
        context.snapshot.current_nodes.remove(index);
    }
}

fn has_route(context: &RuntimeContext, source_node: u32, source_port: &str) -> bool {
    context
        .plan
        .edges
        .iter()
        .any(|edge| edge.source_node == source_node && edge.source_port == source_port)
}

fn current_node(context: &RuntimeContext) -> Option<&CompiledNode> {
    let ordinal = *context.snapshot.current_nodes.first()?;
    context
        .plan
        .nodes
        .iter()
        .find(|node| node.ordinal == ordinal)
}

fn apply_agent_output(context: &mut RuntimeContext, node: &CompiledNode, output: &Value) {
    let allowed: BTreeMap<_, _> = node
        .config
        .get("allowedStateChanges")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|key| (key, ()))
        .collect();
    if let Some(changes) = output.get("stateChanges").and_then(Value::as_object) {
        for (key, value) in changes {
            if allowed.contains_key(key.as_str()) {
                set_state_value(&mut context.snapshot.state, key, value.clone());
            }
        }
    }
    if let Some(dialogue) = output.get("dialogue") {
        let agent_id = node
            .config
            .get("agentId")
            .and_then(Value::as_str)
            .unwrap_or(&node.id)
            .to_string();
        context
            .snapshot
            .conversations
            .entry(agent_id)
            .or_default()
            .push(ConversationMessage {
                role: "assistant".to_string(),
                content: dialogue.clone(),
            });
    }
}

fn public_agent_output(output: &Value) -> Value {
    let mut public = serde_json::Map::new();
    for field in ["dialogue", "emotion", "presentationIntent", "stateChanges"] {
        if let Some(value) = output.get(field) {
            public.insert(field.to_string(), value.clone());
        }
    }
    Value::Object(public)
}

fn runtime_context_value(context: &RuntimeContext) -> Value {
    json!({
        "state": context.snapshot.state,
        "input": context.last_command_data,
        "outputs": context.snapshot.node_outputs,
        "publicOutput": context.snapshot.public_output
    })
}

fn resolve_value(value: &Value, context: &Value) -> Value {
    if let Some(pointer) = value
        .as_object()
        .filter(|object| object.len() == 1)
        .and_then(|object| object.get("$ref"))
        .and_then(Value::as_str)
    {
        return context.pointer(pointer).cloned().unwrap_or(Value::Null);
    }
    value.clone()
}

fn validate_value(schema: &Value, value: &Value) -> Result<(), String> {
    let validator = jsonschema::validator_for(schema).map_err(|error| error.to_string())?;
    validator.validate(value).map_err(|error| error.to_string())
}

fn set_state_value(state: &mut Value, key: &str, value: Value) {
    if !state.is_object() {
        *state = json!({});
    }
    if let Some(object) = state.as_object_mut() {
        object.insert(key.to_string(), value);
    }
}

fn deterministic_index(snapshot: &mut GameSnapshotV1, len: usize) -> usize {
    let mut value = snapshot.random_seed ^ snapshot.random_counter;
    value ^= value << 13;
    value ^= value >> 7;
    value ^= value << 17;
    snapshot.random_counter += 1;
    (value as usize) % len
}

fn fail_runtime(
    context: &mut RuntimeContext,
    code: impl Into<String>,
    message: impl Into<String>,
    node_id: Option<String>,
) {
    let failure = RuntimeFailure {
        code: code.into(),
        message: message.into(),
        node_id,
    };
    context.snapshot.status = SessionStatus::Failed;
    context.snapshot.failure = Some(failure.clone());
    emit_event(
        context,
        "game.session.failed",
        failure.node_id.clone(),
        json!({"code": failure.code, "message": failure.message}),
    );
}

fn emit_event(
    context: &mut RuntimeContext,
    event_type: &str,
    subject: Option<String>,
    data: Value,
) {
    let sequence = context.snapshot.next_event_sequence;
    context.snapshot.next_event_sequence += 1;
    context.events.push(GameEvent {
        specversion: "1.0".to_string(),
        id: format!("event-{sequence}"),
        source: "vifu://game-runtime".to_string(),
        event_type: event_type.to_string(),
        subject,
        sequence,
        data,
    });
}

fn presentation_event_type(node_type: &str) -> &str {
    match node_type {
        "background" => "background.changed",
        "character_visual" => "character.visual.changed",
        "expression" => "character.expression.changed",
        "audio" => "audio.play",
        "video" => "video.play",
        "voice" => "voice.play",
        "subtitle" => "subtitle.show",
        "transition" => "transition.requested",
        "camera_cue" => "camera.cue.requested",
        "delay" => "timeline.delay",
        _ => "presentation.requested",
    }
}

fn is_terminal(status: &SessionStatus) -> bool {
    matches!(
        status,
        SessionStatus::Completed | SessionStatus::Failed | SessionStatus::Cancelled
    )
}

fn retain_effects_system(_context: ResMut<RuntimeContext>) {}

fn commit_revision_system(mut context: ResMut<RuntimeContext>) {
    if context.error.is_none() {
        context.snapshot.revision += 1;
    }
}

fn finalize_status_system(mut context: ResMut<RuntimeContext>) {
    if context.error.is_none()
        && context.snapshot.status == SessionStatus::Running
        && context.snapshot.current_nodes.is_empty()
    {
        fail_runtime(
            &mut context,
            "route_missing",
            "runtime reached no stable boundary",
            None,
        );
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::{
        AgentReference, GameCompiler, GameSourceV1, LogicalPresentationResource, PortReference,
        SourceEdge, SourceNode,
    };

    use super::*;

    fn edge(id: &str, source: &str, port: &str, target: &str) -> SourceEdge {
        SourceEdge {
            id: id.to_string(),
            source: PortReference {
                node_id: source.to_string(),
                port: port.to_string(),
            },
            target: PortReference {
                node_id: target.to_string(),
                port: "in".to_string(),
            },
            condition: None,
        }
    }

    fn interactive_plan() -> GamePlanV1 {
        let mut source = GameSourceV1::new("Runtime test");
        source.agents.push(AgentReference {
            id: "guide".to_string(),
            profile_id: "profile-guide".to_string(),
            profile_version_id: Some("profile-version-1".to_string()),
            capabilities: vec!["dialogue".to_string()],
            execution_descriptor: json!({"route": "profile-version-1"}),
        });
        source
            .presentation_resources
            .push(LogicalPresentationResource {
                id: "world.main-gate".to_string(),
                kind: "object".to_string(),
                required_capabilities: vec!["vifu.world.object-action.v1".to_string()],
                required: true,
                fallback: None,
            });
        source.graph.nodes.extend([
            SourceNode {
                id: "agent".to_string(),
                node_type: "agent".to_string(),
                version: 1,
                config: json!({"agentId": "guide", "allowedStateChanges": ["trust"]}),
                parent_id: None,
                label: None,
                notes: None,
            },
            SourceNode {
                id: "gate".to_string(),
                node_type: "host_action".to_string(),
                version: 1,
                config: json!({
                    "target": "world.main-gate",
                    "action": "open",
                    "completion": "required"
                }),
                parent_id: None,
                label: None,
                notes: None,
            },
            SourceNode {
                id: "choice".to_string(),
                node_type: "choice".to_string(),
                version: 1,
                config: json!({
                    "prompt": "Where next?",
                    "options": [
                        {"id": "forest", "label": "Forest"},
                        {"id": "village", "label": "Village"}
                    ]
                }),
                parent_id: None,
                label: None,
                notes: None,
            },
            SourceNode {
                id: "forest".to_string(),
                node_type: "ending".to_string(),
                version: 1,
                config: json!({"endingId": "forest"}),
                parent_id: None,
                label: None,
                notes: None,
            },
            SourceNode {
                id: "village".to_string(),
                node_type: "ending".to_string(),
                version: 1,
                config: json!({"endingId": "village"}),
                parent_id: None,
                label: None,
                notes: None,
            },
            SourceNode {
                id: "end".to_string(),
                node_type: "end".to_string(),
                version: 1,
                config: json!({}),
                parent_id: None,
                label: None,
                notes: None,
            },
        ]);
        source.graph.edges.extend([
            edge("1", "start", "next", "agent"),
            edge("2", "agent", "next", "gate"),
            edge("3", "gate", "next", "choice"),
            edge("4", "choice", "forest", "forest"),
            edge("5", "choice", "village", "village"),
            edge("6", "forest", "next", "end"),
            edge("7", "village", "next", "end"),
        ]);
        GameCompiler::default()
            .compile(&source)
            .expect("compile interactive plan")
            .plan
    }

    #[test]
    fn advances_across_effect_host_action_and_player_choice() {
        let mut runtime = GameRuntime::new(interactive_plan(), 7).expect("create runtime");
        let started = runtime
            .dispatch(GameCommand {
                idempotency_key: "start".to_string(),
                expected_revision: None,
                command_type: "game.start".to_string(),
                data: json!({}),
            })
            .expect("start game");
        assert_eq!(started.snapshot.status, SessionStatus::WaitingEffect);
        assert_eq!(
            started
                .node_executions
                .iter()
                .map(|execution| execution.node_id.as_str())
                .collect::<Vec<_>>(),
            vec!["start", "agent"]
        );
        let effect = started.effects.first().expect("Agent effect").clone();

        let resumed = runtime
            .dispatch(GameCommand {
                idempotency_key: "effect".to_string(),
                expected_revision: None,
                command_type: "effect.completed".to_string(),
                data: serde_json::to_value(EffectResult {
                    effect_id: effect.effect_id,
                    output: Some(json!({
                        "dialogue": "The gate is ready.",
                        "stateChanges": {"trust": 2, "notAllowed": true}
                    })),
                    error: None,
                })
                .expect("serialize effect"),
            })
            .expect("resume effect");
        assert_eq!(resumed.snapshot.status, SessionStatus::WaitingHost);
        assert_eq!(resumed.snapshot.state["trust"], json!(2));
        assert!(resumed.snapshot.state.get("notAllowed").is_none());
        let action = resumed
            .snapshot
            .pending_host_action
            .as_ref()
            .expect("host action")
            .action_id
            .clone();

        let choice = runtime
            .dispatch(GameCommand {
                idempotency_key: "host".to_string(),
                expected_revision: None,
                command_type: "host.action.completed".to_string(),
                data: json!({"actionId": action}),
            })
            .expect("complete host action");
        assert_eq!(choice.snapshot.status, SessionStatus::WaitingInput);
        assert!(choice
            .events
            .iter()
            .any(|event| event.event_type == "choice.presented"));

        let completed = runtime
            .dispatch(GameCommand {
                idempotency_key: "choice".to_string(),
                expected_revision: None,
                command_type: "player.choice".to_string(),
                data: json!({"optionId": "forest"}),
            })
            .expect("select ending");
        assert_eq!(completed.snapshot.status, SessionStatus::Completed);
        assert!(completed
            .events
            .iter()
            .any(|event| event.event_type == "ending.reached"));
    }

    #[test]
    fn restored_snapshot_produces_the_same_next_events() {
        let plan = interactive_plan();
        let mut runtime = GameRuntime::new(plan.clone(), 99).expect("runtime");
        let started = runtime
            .dispatch(GameCommand {
                idempotency_key: "start".to_string(),
                expected_revision: None,
                command_type: "game.start".to_string(),
                data: json!({}),
            })
            .expect("start");
        let effect = started.effects[0].clone();
        let command = GameCommand {
            idempotency_key: "effect".to_string(),
            expected_revision: None,
            command_type: "effect.completed".to_string(),
            data: serde_json::to_value(EffectResult {
                effect_id: effect.effect_id,
                output: Some(json!({"dialogue": "Hello"})),
                error: None,
            })
            .expect("serialize result"),
        };
        let mut restored =
            GameRuntime::restore(plan, started.snapshot).expect("restore runtime snapshot");

        let left = runtime.dispatch(command.clone()).expect("advance original");
        let right = restored.dispatch(command).expect("advance restored");
        assert_eq!(left, right);
    }

    #[test]
    fn parallel_branches_join_without_dropping_state_updates() {
        let mut source = GameSourceV1::new("Parallel runtime");
        source.graph.nodes.extend([
            SourceNode {
                id: "parallel".to_string(),
                node_type: "parallel".to_string(),
                version: 1,
                config: json!({}),
                parent_id: None,
                label: None,
                notes: None,
            },
            SourceNode {
                id: "left".to_string(),
                node_type: "state".to_string(),
                version: 1,
                config: json!({"key": "left", "value": true}),
                parent_id: None,
                label: None,
                notes: None,
            },
            SourceNode {
                id: "right".to_string(),
                node_type: "state".to_string(),
                version: 1,
                config: json!({"key": "right", "value": true}),
                parent_id: None,
                label: None,
                notes: None,
            },
            SourceNode {
                id: "join".to_string(),
                node_type: "join".to_string(),
                version: 1,
                config: json!({}),
                parent_id: None,
                label: None,
                notes: None,
            },
            SourceNode {
                id: "end".to_string(),
                node_type: "end".to_string(),
                version: 1,
                config: json!({}),
                parent_id: None,
                label: None,
                notes: None,
            },
        ]);
        source.graph.edges.extend([
            edge("1", "start", "next", "parallel"),
            edge("2", "parallel", "left", "left"),
            edge("3", "parallel", "right", "right"),
            edge("4", "left", "next", "join"),
            edge("5", "right", "next", "join"),
            edge("6", "join", "next", "end"),
        ]);
        let plan = GameCompiler::default()
            .compile(&source)
            .expect("compile parallel plan")
            .plan;
        let mut runtime = GameRuntime::new(plan, 1).expect("create runtime");

        let completed = runtime
            .dispatch(GameCommand {
                idempotency_key: "start".to_string(),
                expected_revision: None,
                command_type: "game.start".to_string(),
                data: json!({}),
            })
            .expect("run parallel branches");

        assert_eq!(completed.snapshot.status, SessionStatus::Completed);
        assert_eq!(completed.snapshot.state["left"], json!(true));
        assert_eq!(completed.snapshot.state["right"], json!(true));
        assert!(completed.snapshot.join_arrivals.is_empty());
    }

    #[test]
    fn runtime_validates_public_input_and_output_schemas() {
        let mut source = GameSourceV1::new("Schema runtime");
        source.inputs = json!({
            "type": "object",
            "required": ["playerId"],
            "properties": {"playerId": {"type": "string"}},
            "additionalProperties": false
        });
        source.outputs = json!({
            "type": "object",
            "required": ["ending"],
            "properties": {"ending": {"type": "string"}},
            "additionalProperties": false
        });
        source.graph.nodes.extend([
            SourceNode {
                id: "output".to_string(),
                node_type: "output".to_string(),
                version: 1,
                config: json!({"value": {"wrong": true}}),
                parent_id: None,
                label: None,
                notes: None,
            },
            SourceNode {
                id: "end".to_string(),
                node_type: "end".to_string(),
                version: 1,
                config: json!({}),
                parent_id: None,
                label: None,
                notes: None,
            },
        ]);
        source.graph.edges.extend([
            edge("1", "start", "next", "output"),
            edge("2", "output", "next", "end"),
        ]);
        let plan = GameCompiler::default()
            .compile(&source)
            .expect("compile schema plan")
            .plan;

        let mut invalid_input = GameRuntime::new(plan.clone(), 1).expect("runtime");
        assert!(invalid_input
            .dispatch(GameCommand {
                idempotency_key: "invalid-input".to_string(),
                expected_revision: None,
                command_type: "game.start".to_string(),
                data: json!({}),
            })
            .is_err());

        let mut invalid_output = GameRuntime::new(plan, 1).expect("runtime");
        let advance = invalid_output
            .dispatch(GameCommand {
                idempotency_key: "invalid-output".to_string(),
                expected_revision: None,
                command_type: "game.start".to_string(),
                data: json!({"playerId": "player-one"}),
            })
            .expect("runtime failure is committed as state");
        assert_eq!(advance.snapshot.status, SessionStatus::Failed);
        assert_eq!(
            advance
                .snapshot
                .failure
                .as_ref()
                .map(|failure| failure.code.as_str()),
            Some("game_output_invalid")
        );
    }

    #[test]
    fn tool_failures_use_tool_events_and_error_routes() {
        let mut source = GameSourceV1::new("Tool runtime");
        source.agents.push(AgentReference {
            id: "guide".to_string(),
            profile_id: "profile-guide".to_string(),
            profile_version_id: Some("profile-version-1".to_string()),
            capabilities: vec!["tool".to_string()],
            execution_descriptor: json!({
                "profileId": "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
                "profileVersionId": "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"
            }),
        });
        source.graph.nodes.extend([
            SourceNode {
                id: "tool".to_string(),
                node_type: "tool".to_string(),
                version: 1,
                config: json!({"agentId": "guide", "tool": "fixture.lookup"}),
                parent_id: None,
                label: None,
                notes: None,
            },
            SourceNode {
                id: "end".to_string(),
                node_type: "end".to_string(),
                version: 1,
                config: json!({}),
                parent_id: None,
                label: None,
                notes: None,
            },
        ]);
        source.graph.edges.extend([
            edge("1", "start", "next", "tool"),
            edge("2", "tool", "next", "end"),
            edge("3", "tool", "error", "end"),
        ]);
        let plan = GameCompiler::default()
            .compile(&source)
            .expect("compile tool plan")
            .plan;
        let mut runtime = GameRuntime::new(plan, 1).expect("runtime");
        let started = runtime
            .dispatch(GameCommand {
                idempotency_key: "start".to_string(),
                expected_revision: None,
                command_type: "game.start".to_string(),
                data: json!({}),
            })
            .expect("request tool");
        let effect = started.effects[0].clone();
        assert_eq!(effect.descriptor["tool"], "fixture.lookup");
        assert_eq!(
            effect.descriptor["profileVersionId"],
            "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"
        );
        let completed = runtime
            .dispatch(GameCommand {
                idempotency_key: "tool-error".to_string(),
                expected_revision: None,
                command_type: "effect.completed".to_string(),
                data: serde_json::to_value(EffectResult {
                    effect_id: effect.effect_id,
                    output: None,
                    error: Some(RuntimeFailure {
                        code: "tool_failed".to_string(),
                        message: "fixture failure".to_string(),
                        node_id: Some("tool".to_string()),
                    }),
                })
                .expect("effect result"),
            })
            .expect("follow error route");

        assert_eq!(completed.snapshot.status, SessionStatus::Completed);
        assert!(completed
            .events
            .iter()
            .any(|event| event.event_type == "tool.failed"));
        assert!(!completed
            .events
            .iter()
            .any(|event| event.event_type == "agent.failed"));
    }
}
