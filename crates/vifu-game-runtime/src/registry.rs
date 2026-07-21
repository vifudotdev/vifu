use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NodePhase {
    Production,
    Runtime,
}

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PortDirection {
    Input,
    Output,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PortDefinition {
    pub name: String,
    pub direction: PortDirection,
    pub value_schema: Value,
    #[serde(default)]
    pub required: bool,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NodeDefinition {
    #[serde(rename = "type")]
    pub node_type: String,
    pub version: u32,
    pub phase: NodePhase,
    pub title: String,
    pub category: String,
    pub config_schema: Value,
    pub ports: Vec<PortDefinition>,
    #[serde(default)]
    pub dynamic_inputs: bool,
    #[serde(default)]
    pub dynamic_outputs: bool,
    #[serde(default)]
    pub timeline_compatible: bool,
}

impl NodeDefinition {
    pub fn has_input(&self, name: &str) -> bool {
        self.dynamic_inputs
            || self
                .ports
                .iter()
                .any(|port| port.direction == PortDirection::Input && port.name == name)
    }

    pub fn has_output(&self, name: &str) -> bool {
        self.dynamic_outputs
            || self
                .ports
                .iter()
                .any(|port| port.direction == PortDirection::Output && port.name == name)
    }
}

#[derive(Clone, Debug)]
pub struct NodeRegistry {
    definitions: BTreeMap<(String, u32), NodeDefinition>,
}

impl NodeRegistry {
    pub fn new(definitions: impl IntoIterator<Item = NodeDefinition>) -> Self {
        let definitions = definitions
            .into_iter()
            .map(|definition| {
                (
                    (definition.node_type.clone(), definition.version),
                    definition,
                )
            })
            .collect();
        Self { definitions }
    }

    pub fn definition(&self, node_type: &str, version: u32) -> Option<&NodeDefinition> {
        self.definitions.get(&(node_type.to_string(), version))
    }

    pub fn definitions(&self) -> impl Iterator<Item = &NodeDefinition> {
        self.definitions.values()
    }
}

impl Default for NodeRegistry {
    fn default() -> Self {
        Self::new(default_definitions())
    }
}

fn default_definitions() -> Vec<NodeDefinition> {
    let mut definitions = Vec::new();

    for (node_type, title) in [
        ("story_brief", "Story Brief"),
        ("script_import", "Script Import"),
        ("story_outline", "Story Outline"),
        ("scene_breakdown", "Scene Breakdown"),
        ("dialogue_draft", "Dialogue Draft"),
        ("choice_draft", "Choice Draft"),
        ("asset_import", "Asset Import"),
        ("media_probe", "Media Probe"),
        ("thumbnail", "Thumbnail"),
        ("asset_transform", "Asset Transform"),
        ("subtitle_import", "Subtitle Import"),
        ("approval", "Approval"),
        ("build", "Build"),
        ("qa", "QA"),
    ] {
        definitions.push(definition(
            node_type,
            title,
            NodePhase::Production,
            "Production",
            passthrough_ports(),
            object_schema(),
            NodeBehavior::NONE,
        ));
    }

    definitions.push(definition(
        "start",
        "Start",
        NodePhase::Runtime,
        "Flow",
        vec![output("next")],
        object_schema(),
        NodeBehavior::NONE,
    ));
    definitions.push(definition(
        "end",
        "End",
        NodePhase::Runtime,
        "Flow",
        vec![input("in")],
        object_schema(),
        NodeBehavior::timeline(),
    ));
    definitions.push(definition(
        "input",
        "Player Input",
        NodePhase::Runtime,
        "Flow",
        passthrough_ports(),
        json!({
            "type": "object",
            "properties": {"commandType": {"type": "string"}},
            "additionalProperties": true
        }),
        NodeBehavior::timeline(),
    ));
    definitions.push(definition(
        "choice",
        "Choice",
        NodePhase::Runtime,
        "Narrative",
        vec![input("in")],
        json!({
            "type": "object",
            "required": ["options"],
            "properties": {
                "prompt": {"type": "string"},
                "options": {
                    "type": "array",
                    "minItems": 1,
                    "items": {
                        "type": "object",
                        "required": ["id", "label"],
                        "properties": {
                            "id": {"type": "string", "minLength": 1},
                            "label": {"type": "string", "minLength": 1}
                        },
                        "additionalProperties": false
                    }
                }
            },
            "additionalProperties": true
        }),
        NodeBehavior {
            dynamic_inputs: false,
            dynamic_outputs: true,
            timeline_compatible: true,
        },
    ));
    definitions.push(definition(
        "condition",
        "Condition",
        NodePhase::Runtime,
        "Logic",
        vec![input("in"), output("true"), output("false")],
        object_schema(),
        NodeBehavior::NONE,
    ));
    definitions.push(definition(
        "agent",
        "Agent",
        NodePhase::Runtime,
        "Characters",
        vec![input("in"), output("next"), output("error")],
        json!({
            "type": "object",
            "required": ["agentId"],
            "properties": {
                "agentId": {"type": "string", "minLength": 1},
                "input": {},
                "allowedStateChanges": {"type": "array", "items": {"type": "string"}},
                "fallback": {}
            },
            "additionalProperties": true
        }),
        NodeBehavior::timeline(),
    ));
    definitions.push(definition(
        "tool",
        "Tool",
        NodePhase::Runtime,
        "Integration",
        vec![input("in"), output("next"), output("error")],
        json!({
            "type": "object",
            "required": ["agentId", "tool"],
            "properties": {
                "agentId": {"type": "string", "minLength": 1},
                "tool": {"type": "string", "minLength": 1},
                "input": {}
            },
            "additionalProperties": true
        }),
        NodeBehavior::NONE,
    ));
    definitions.push(definition(
        "host_action",
        "Host Action",
        NodePhase::Runtime,
        "Presentation",
        vec![input("in"), output("next"), output("error")],
        json!({
            "type": "object",
            "required": ["target", "action"],
            "properties": {
                "target": {"type": "string", "minLength": 1},
                "action": {"type": "string", "minLength": 1},
                "arguments": {},
                "completion": {"enum": ["required", "optional"]}
            },
            "additionalProperties": true
        }),
        NodeBehavior::timeline(),
    ));
    definitions.push(definition(
        "subtitle",
        "Subtitle",
        NodePhase::Runtime,
        "Presentation",
        passthrough_ports(),
        json!({
            "type": "object",
            "required": ["locale"],
            "properties": {
                "locale": {"type": "string", "minLength": 1, "title": "Locale"},
                "subtitleKey": {"type": "string", "minLength": 1, "title": "Subtitle key"},
                "logicalResourceId": {"type": "string", "minLength": 1, "title": "Logical resource"},
                "text": {"type": "string", "title": "Subtitle text"}
            },
            "additionalProperties": true
        }),
        NodeBehavior::timeline(),
    ));

    for (node_type, title, category, timeline_compatible) in [
        ("event", "Event", "Flow", false),
        ("subscene", "Subscene", "Flow", false),
        ("merge", "Merge", "Flow", false),
        ("join", "Join", "Flow", false),
        ("delay", "Delay", "Flow", true),
        ("random", "Random", "Flow", false),
        ("output", "Output", "Flow", false),
        ("episode", "Episode", "Narrative", true),
        ("scene", "Scene", "Narrative", true),
        ("dialogue", "Dialogue", "Narrative", true),
        ("ending", "Ending", "Narrative", true),
        ("character_state", "Character State", "Characters", false),
        ("relationship", "Relationship", "Characters", false),
        ("memory", "Memory", "Characters", false),
        ("background", "Background", "Presentation", true),
        ("character_visual", "Character Visual", "Presentation", true),
        ("expression", "Expression", "Presentation", true),
        ("audio", "Audio", "Presentation", true),
        ("video", "Video", "Presentation", true),
        ("voice", "Voice", "Presentation", true),
        ("transition", "Transition", "Presentation", true),
        ("camera_cue", "Camera Cue", "Presentation", true),
        ("resource", "Resource", "Integration", false),
        ("asset", "Asset", "Integration", true),
        ("state", "State", "Logic", false),
        ("transform", "Transform", "Logic", false),
    ] {
        definitions.push(definition(
            node_type,
            title,
            NodePhase::Runtime,
            category,
            passthrough_ports(),
            object_schema(),
            NodeBehavior {
                timeline_compatible,
                ..NodeBehavior::NONE
            },
        ));
    }

    for (node_type, title) in [("loop", "Loop"), ("for_each", "For Each")] {
        definitions.push(definition(
            node_type,
            title,
            NodePhase::Runtime,
            "Flow",
            vec![input("in"), output("body"), output("done")],
            json!({
                "type": "object",
                "required": ["maxIterations"],
                "properties": {
                    "maxIterations": {"type": "integer", "minimum": 1, "maximum": 1000}
                },
                "additionalProperties": true
            }),
            NodeBehavior::NONE,
        ));
    }

    definitions.push(definition(
        "parallel",
        "Parallel",
        NodePhase::Runtime,
        "Flow",
        vec![input("in")],
        object_schema(),
        NodeBehavior {
            dynamic_inputs: false,
            dynamic_outputs: true,
            timeline_compatible: false,
        },
    ));

    definitions
}

fn definition(
    node_type: &str,
    title: &str,
    phase: NodePhase,
    category: &str,
    ports: Vec<PortDefinition>,
    config_schema: Value,
    behavior: NodeBehavior,
) -> NodeDefinition {
    NodeDefinition {
        node_type: node_type.to_string(),
        version: 1,
        phase,
        title: title.to_string(),
        category: category.to_string(),
        config_schema,
        ports,
        dynamic_inputs: behavior.dynamic_inputs,
        dynamic_outputs: behavior.dynamic_outputs,
        timeline_compatible: behavior.timeline_compatible,
    }
}

#[derive(Clone, Copy)]
struct NodeBehavior {
    dynamic_inputs: bool,
    dynamic_outputs: bool,
    timeline_compatible: bool,
}

impl NodeBehavior {
    const NONE: Self = Self {
        dynamic_inputs: false,
        dynamic_outputs: false,
        timeline_compatible: false,
    };

    const fn timeline() -> Self {
        Self {
            timeline_compatible: true,
            ..Self::NONE
        }
    }
}

fn passthrough_ports() -> Vec<PortDefinition> {
    vec![input("in"), output("next")]
}

fn input(name: &str) -> PortDefinition {
    PortDefinition {
        name: name.to_string(),
        direction: PortDirection::Input,
        value_schema: Value::Bool(true),
        required: false,
    }
}

fn output(name: &str) -> PortDefinition {
    PortDefinition {
        name: name.to_string(),
        direction: PortDirection::Output,
        value_schema: Value::Bool(true),
        required: false,
    }
}

fn object_schema() -> Value {
    json!({"type": "object", "additionalProperties": true})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_registry_exposes_versioned_runtime_nodes() {
        let registry = NodeRegistry::default();
        let agent = registry.definition("agent", 1).expect("agent definition");
        assert_eq!(agent.phase, NodePhase::Runtime);
        assert!(agent.has_input("in"));
        assert!(agent.has_output("error"));
        assert!(agent.timeline_compatible);
    }
}
