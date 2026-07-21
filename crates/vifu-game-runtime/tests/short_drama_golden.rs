use serde_json::{json, Value};
use vifu_game_runtime::{
    AgentReference, EffectResult, GameCharacterV1, GameCommand, GameCompiler, GamePlanV1,
    GameRuntime, GameSnapshotV1, GameSourceV1, GameVariable, LogicalPresentationResource,
    PortReference, SessionStatus, SourceEdge, SourceNode,
};

fn node(id: &str, node_type: &str, config: Value) -> SourceNode {
    SourceNode {
        id: id.to_string(),
        node_type: node_type.to_string(),
        version: 1,
        config,
        parent_id: None,
        label: Some(id.replace('-', " ")),
        notes: None,
    }
}

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
        managed_by: None,
    }
}

fn command(key: &str, command_type: &str, data: Value) -> GameCommand {
    GameCommand {
        idempotency_key: key.to_string(),
        expected_revision: None,
        command_type: command_type.to_string(),
        data,
    }
}

fn resource(id: &str, kind: &str, capability: &str, required: bool) -> LogicalPresentationResource {
    LogicalPresentationResource {
        id: id.to_string(),
        kind: kind.to_string(),
        required_capabilities: vec![capability.to_string()],
        required,
        fallback: (!required).then(|| json!({"kind": "placeholder"})),
    }
}

fn golden_source() -> GameSourceV1 {
    let mut source = GameSourceV1::new("The Gate at Dusk");
    source.metadata.description = Some("A three-scene interactive short drama.".to_string());
    source.variables = vec![
        GameVariable {
            id: "metMira".to_string(),
            initial_value: json!(false),
            public: true,
        },
        GameVariable {
            id: "trust".to_string(),
            initial_value: json!(0),
            public: true,
        },
    ];
    source.agents = vec![
        AgentReference {
            id: "mira".to_string(),
            profile_id: "profile.mira".to_string(),
            profile_version_id: Some("profile.mira.v3".to_string()),
            capabilities: vec!["dialogue".to_string(), "emotion".to_string()],
            execution_descriptor: json!({"selectionKey": "mira"}),
        },
        AgentReference {
            id: "kai".to_string(),
            profile_id: "profile.kai".to_string(),
            profile_version_id: Some("profile.kai.v2".to_string()),
            capabilities: vec!["dialogue".to_string()],
            execution_descriptor: json!({"selectionKey": "kai"}),
        },
    ];
    source.localization.source_messages.extend([
        ("character.mira.name".to_string(), "Mira".to_string()),
        ("character.kai.name".to_string(), "Kai".to_string()),
    ]);
    source.characters = vec![
        GameCharacterV1 {
            id: "mira".to_string(),
            name_message_id: "character.mira.name".to_string(),
            role_message_id: None,
            agent_id: Some("mira".to_string()),
            portrait_resource_id: Some("character.mira.portrait".to_string()),
            player: false,
        },
        GameCharacterV1 {
            id: "kai".to_string(),
            name_message_id: "character.kai.name".to_string(),
            role_message_id: None,
            agent_id: Some("kai".to_string()),
            portrait_resource_id: None,
            player: false,
        },
    ];
    source.presentation_resources = vec![
        resource(
            "scene.opening.background",
            "image",
            "vifu.presentation.image.v1",
            false,
        ),
        resource(
            "character.mira.portrait",
            "image",
            "vifu.presentation.image.v1",
            false,
        ),
        resource(
            "subtitle.opening.en",
            "subtitle",
            "vifu.presentation.subtitle.v1",
            false,
        ),
        resource(
            "scene.forest.video",
            "video",
            "vifu.presentation.video.v1",
            false,
        ),
        resource(
            "world.main-gate",
            "object",
            "vifu.world.object-action.v1",
            true,
        ),
    ];
    source.graph.nodes.extend([
        node(
            "scene-opening",
            "scene",
            json!({"name": "Dusk at the gate", "sequenceId": "opening", "startMs": 0, "durationMs": 9000}),
        ),
        node(
            "opening-background",
            "background",
            json!({"logicalResourceId": "scene.opening.background", "sequenceId": "opening", "startMs": 0, "durationMs": 9000}),
        ),
        node(
            "opening-subtitle",
            "subtitle",
            json!({"locale": "en", "logicalResourceId": "subtitle.opening.en", "sequenceId": "opening", "startMs": 500, "durationMs": 2500}),
        ),
        node(
            "mira-visual",
            "character_visual",
            json!({"logicalResourceId": "character.mira.portrait", "sequenceId": "opening", "startMs": 900, "durationMs": 5000}),
        ),
        node(
            "mira-agent",
            "agent",
            json!({
                "agentId": "mira",
                "input": {"beat": "greeting"},
                "allowedStateChanges": ["metMira"],
                "fallback": {"dialogue": "The road is dangerous after sunset."},
                "blocking": false,
                "sequenceId": "opening",
                "startMs": 1200,
                "durationMs": 2500
            }),
        ),
        node(
            "player-name",
            "input",
            json!({"commandType": "player.text", "prompt": "What should Mira call you?", "sequenceId": "opening", "startMs": 4000, "durationMs": 1200}),
        ),
        node("remember-mira", "state", json!({"key": "metMira", "op": "set", "value": true})),
        node(
            "route-choice",
            "choice",
            json!({
                "prompt": "Where will you go?",
                "options": [
                    {"id": "forest", "label": "Follow the forest trail"},
                    {"id": "village", "label": "Return to the village"},
                    {"id": "ask", "label": "Ask the villagers first"}
                ],
                "sequenceId": "opening",
                "startMs": 5400,
                "durationMs": 1500
            }),
        ),
        node(
            "scene-forest",
            "scene",
            json!({"name": "Forest crossing", "sequenceId": "forest", "startMs": 0, "durationMs": 8000}),
        ),
        node(
            "forest-video",
            "video",
            json!({"logicalResourceId": "scene.forest.video", "sequenceId": "forest", "startMs": 0, "inMs": 1200, "durationMs": 6500, "volume": 0.7}),
        ),
        node(
            "kai-agent",
            "agent",
            json!({
                "agentId": "kai",
                "input": {"beat": "gate-warning"},
                "allowedStateChanges": ["trust"],
                "fallback": {"dialogue": "I can hold the gate, but not for long."},
                "blocking": false,
                "sequenceId": "forest",
                "startMs": 2200,
                "durationMs": 2600
            }),
        ),
        node(
            "trust-kai",
            "relationship",
            json!({"key": "trust", "op": "set", "value": 4}),
        ),
        node(
            "open-gate",
            "host_action",
            json!({
                "target": "world.main-gate",
                "action": "open",
                "arguments": {"durationMs": 800},
                "completion": "required",
                "sequenceId": "forest",
                "startMs": 5000,
                "durationMs": 800
            }),
        ),
        node(
            "forest-choice",
            "choice",
            json!({
                "prompt": "The gate is open. What now?",
                "options": [
                    {"id": "help", "label": "Help Kai"},
                    {"id": "leave", "label": "Leave before it closes"}
                ],
                "sequenceId": "forest",
                "startMs": 6200,
                "durationMs": 1200
            }),
        ),
        node(
            "scene-village",
            "scene",
            json!({"name": "Village square", "sequenceId": "village", "startMs": 0, "durationMs": 5000}),
        ),
        node(
            "village-dialogue",
            "dialogue",
            json!({"speaker": "Mira", "text": "The lanterns will stay lit for you.", "blocking": false, "sequenceId": "village", "startMs": 400, "durationMs": 2200}),
        ),
        node(
            "village-choice",
            "choice",
            json!({
                "prompt": "Stay in the square?",
                "options": [
                    {"id": "wait", "label": "Wait for dawn"},
                    {"id": "follow", "label": "Follow the forest lights"}
                ],
                "sequenceId": "village",
                "startMs": 2800,
                "durationMs": 1200
            }),
        ),
        node(
            "ending-hope",
            "ending",
            json!({"endingId": "gatekeepers", "title": "The New Gatekeepers"}),
        ),
        node(
            "ending-home",
            "ending",
            json!({"endingId": "safe-at-dawn", "title": "Safe at Dawn"}),
        ),
        node(
            "end-hope",
            "end",
            json!({"output": {"endingId": "gatekeepers"}}),
        ),
        node(
            "end-home",
            "end",
            json!({"output": {"endingId": "safe-at-dawn"}}),
        ),
    ]);
    source.graph.edges.extend([
        edge("01", "start", "next", "scene-opening"),
        edge("02", "scene-opening", "next", "opening-background"),
        edge("03", "opening-background", "next", "opening-subtitle"),
        edge("04", "opening-subtitle", "next", "mira-visual"),
        edge("05", "mira-visual", "next", "mira-agent"),
        edge("06", "mira-agent", "next", "player-name"),
        edge("07", "player-name", "next", "remember-mira"),
        edge("08", "remember-mira", "next", "route-choice"),
        edge("09", "route-choice", "forest", "scene-forest"),
        edge("10", "route-choice", "village", "scene-village"),
        edge("11", "route-choice", "ask", "scene-village"),
        edge("12", "scene-village", "next", "village-dialogue"),
        edge("13", "village-dialogue", "next", "village-choice"),
        edge("14", "village-choice", "wait", "ending-home"),
        edge("15", "village-choice", "follow", "scene-forest"),
        edge("16", "scene-forest", "next", "forest-video"),
        edge("17", "forest-video", "next", "kai-agent"),
        edge("18", "kai-agent", "next", "trust-kai"),
        edge("19", "trust-kai", "next", "open-gate"),
        edge("20", "open-gate", "next", "forest-choice"),
        edge("21", "open-gate", "error", "ending-home"),
        edge("22", "forest-choice", "help", "ending-hope"),
        edge("23", "forest-choice", "leave", "ending-home"),
        edge("24", "ending-hope", "next", "end-hope"),
        edge("25", "ending-home", "next", "end-home"),
    ]);
    source
}

fn golden_plan() -> GamePlanV1 {
    let compiled = GameCompiler::default()
        .compile(&golden_source())
        .expect("compile golden short drama");
    assert_eq!(compiled.manifest.scenes.len(), 3);
    assert_eq!(compiled.manifest.characters.len(), 2);
    assert!(compiled
        .manifest
        .required_host_capabilities
        .contains(&"vifu.world.object-action.v1".to_string()));
    compiled.plan
}

fn complete_agent(runtime: &mut GameRuntime, key: &str, dialogue: &str, state: Value) {
    let pending = runtime
        .snapshot()
        .pending_effect
        .as_ref()
        .expect("pending Agent effect")
        .clone();
    runtime
        .dispatch(command(
            key,
            "effect.completed",
            serde_json::to_value(EffectResult {
                effect_id: pending.effect_id,
                output: Some(json!({
                    "dialogue": dialogue,
                    "emotion": "focused",
                    "stateChanges": state.as_object().into_iter().flat_map(|changes| {
                        changes.iter().map(|(key, value)| json!({
                            "key": key,
                            "op": "set",
                            "value": value
                        }))
                    }).collect::<Vec<_>>()
                })),
                error: None,
            })
            .expect("effect result"),
        ))
        .expect("complete Agent effect");
}

#[test]
fn three_scene_drama_resumes_agents_input_host_actions_and_reaches_an_ending() {
    let plan = golden_plan();
    let mut runtime = GameRuntime::new(plan.clone(), 41).expect("create runtime");

    let opening = runtime
        .dispatch(command("start", "game.start", json!({})))
        .expect("start drama");
    assert_eq!(opening.snapshot.status, SessionStatus::WaitingEffect);
    assert_eq!(
        opening
            .events
            .iter()
            .map(|event| event.event_type.as_str())
            .collect::<Vec<_>>(),
        vec![
            "game.session.started",
            "scene.entered",
            "background.changed",
            "subtitle.show",
            "character.visual.changed",
            "agent.requested",
        ]
    );

    complete_agent(
        &mut runtime,
        "mira-response",
        "Tell me your name before we choose a road.",
        json!({"metMira": true}),
    );
    assert_eq!(runtime.snapshot().status, SessionStatus::WaitingInput);

    let named = runtime
        .dispatch(command(
            "player-name",
            "player.text",
            json!({"text": "Ari"}),
        ))
        .expect("submit free text");
    assert_eq!(named.snapshot.status, SessionStatus::WaitingInput);
    assert_eq!(named.snapshot.state["metMira"], json!(true));
    assert_eq!(named.snapshot.node_outputs["player-name"]["text"], "Ari");
    assert!(named
        .events
        .iter()
        .any(|event| event.event_type == "choice.presented"));

    let forest = runtime
        .dispatch(command(
            "route-forest",
            "player.choice",
            json!({"optionId": "forest"}),
        ))
        .expect("choose forest");
    assert_eq!(forest.snapshot.status, SessionStatus::WaitingEffect);
    assert!(forest
        .events
        .iter()
        .any(|event| event.event_type == "video.play"));

    complete_agent(
        &mut runtime,
        "kai-response",
        "Help me open it, and we both make it through.",
        json!({"trust": 3}),
    );
    assert_eq!(runtime.snapshot().status, SessionStatus::WaitingHost);
    assert_eq!(runtime.snapshot().state["trust"], json!(4));

    let encoded = serde_json::to_vec(runtime.snapshot()).expect("serialize snapshot");
    let restored_snapshot: GameSnapshotV1 =
        serde_json::from_slice(&encoded).expect("deserialize snapshot");
    let mut restored =
        GameRuntime::restore(plan, restored_snapshot).expect("restore durable session");
    let action_id = runtime
        .snapshot()
        .pending_host_action
        .as_ref()
        .expect("gate action")
        .action_id
        .clone();
    let acknowledge = command(
        "gate-opened",
        "host.action.completed",
        json!({"actionId": action_id}),
    );
    let original_ack = runtime
        .dispatch(acknowledge.clone())
        .expect("acknowledge original");
    let restored_ack = restored
        .dispatch(acknowledge)
        .expect("acknowledge restored");
    assert_eq!(original_ack, restored_ack);
    assert_eq!(original_ack.snapshot.status, SessionStatus::WaitingInput);

    let completed = runtime
        .dispatch(command(
            "help-kai",
            "player.choice",
            json!({"optionId": "help"}),
        ))
        .expect("reach hopeful ending");
    assert_eq!(completed.snapshot.status, SessionStatus::Completed);
    assert_eq!(
        completed.snapshot.public_output,
        Some(json!({"endingId": "gatekeepers"}))
    );
    assert!(completed.events.iter().any(|event| {
        event.event_type == "ending.reached" && event.data["endingId"] == "gatekeepers"
    }));
    assert_eq!(completed.snapshot.conversations.len(), 2);
}

#[test]
fn alternate_branch_reaches_the_second_ending_without_running_forest_agent() {
    let mut runtime = GameRuntime::new(golden_plan(), 41).expect("create runtime");
    runtime
        .dispatch(command("start", "game.start", json!({})))
        .expect("start drama");
    complete_agent(
        &mut runtime,
        "mira-response",
        "Choose carefully.",
        json!({}),
    );
    runtime
        .dispatch(command(
            "player-name",
            "player.text",
            json!({"text": "Ari"}),
        ))
        .expect("submit free text");
    runtime
        .dispatch(command(
            "route-village",
            "player.choice",
            json!({"optionId": "village"}),
        ))
        .expect("choose village");

    let completed = runtime
        .dispatch(command(
            "wait-for-dawn",
            "player.choice",
            json!({"optionId": "wait"}),
        ))
        .expect("reach village ending");
    assert_eq!(completed.snapshot.status, SessionStatus::Completed);
    assert_eq!(
        completed.snapshot.public_output,
        Some(json!({"endingId": "safe-at-dawn"}))
    );
    assert!(completed.snapshot.conversations.contains_key("mira"));
    assert!(!completed.snapshot.conversations.contains_key("kai"));
}
