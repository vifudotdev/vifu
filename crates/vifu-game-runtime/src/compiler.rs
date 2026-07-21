use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

use petgraph::algo::is_cyclic_directed;
use petgraph::graphmap::DiGraphMap;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::canonical::canonical_json_bytes;
use crate::contract::{
    ClientCompatibility, CompiledEdge, CompiledNode, GameManifestV1, GamePlanV1, GameSourceV1,
    ManifestItem, PinnedAgent, PinnedResource,
};
use crate::error::{GameRuntimeError, ValidationIssue, ValidationSeverity};
use crate::registry::{NodePhase, NodeRegistry};
use crate::GAME_SCHEMA_VERSION;

#[derive(Clone, Debug, PartialEq)]
pub struct CompileOutput {
    pub plan: GamePlanV1,
    pub manifest: GameManifestV1,
    pub content_hash: String,
    pub warnings: Vec<ValidationIssue>,
}

#[derive(Clone, Debug)]
pub struct GameCompiler {
    registry: NodeRegistry,
}

impl Default for GameCompiler {
    fn default() -> Self {
        Self::new(NodeRegistry::default())
    }
}

impl GameCompiler {
    pub fn new(registry: NodeRegistry) -> Self {
        Self { registry }
    }

    pub fn registry(&self) -> &NodeRegistry {
        &self.registry
    }

    pub fn validate(&self, source: &GameSourceV1) -> Vec<ValidationIssue> {
        let mut issues = Vec::new();
        if source.schema_version != GAME_SCHEMA_VERSION {
            issues.push(ValidationIssue::error(
                "unsupported_schema_version",
                format!(
                    "schema version {} is not supported; expected {GAME_SCHEMA_VERSION}",
                    source.schema_version
                ),
            ));
        }
        if source.metadata.name.trim().is_empty() {
            issues.push(
                ValidationIssue::error("name_required", "game name is required")
                    .at_path("/metadata/name"),
            );
        }
        validate_public_schema(&source.inputs, "input", "/inputs", &mut issues);
        validate_public_schema(&source.outputs, "output", "/outputs", &mut issues);

        let mut variable_ids = HashSet::new();
        for variable in &source.variables {
            if variable.id.trim().is_empty() {
                issues.push(
                    ValidationIssue::error("variable_id_required", "every variable requires an ID")
                        .at_path("/variables"),
                );
            } else if variable.id.starts_with("__vifu_") {
                issues.push(
                    ValidationIssue::error(
                        "variable_id_reserved",
                        format!(
                            "variable ID `{}` uses Vifu's reserved namespace",
                            variable.id
                        ),
                    )
                    .at_path(format!("/variables/{}", variable.id)),
                );
            } else if !variable_ids.insert(variable.id.as_str()) {
                issues.push(
                    ValidationIssue::error(
                        "duplicate_variable_id",
                        format!("variable ID `{}` is duplicated", variable.id),
                    )
                    .at_path(format!("/variables/{}", variable.id)),
                );
            }
        }

        let mut node_ids = HashSet::new();
        let mut definitions = HashMap::new();
        for node in &source.graph.nodes {
            if node.id.trim().is_empty() {
                issues.push(ValidationIssue::error(
                    "node_id_required",
                    "every node requires a stable ID",
                ));
                continue;
            }
            if !node_ids.insert(node.id.as_str()) {
                issues.push(
                    ValidationIssue::error(
                        "duplicate_node_id",
                        format!("node ID `{}` is duplicated", node.id),
                    )
                    .for_node(&node.id),
                );
            }
            let Some(definition) = self.registry.definition(&node.node_type, node.version) else {
                issues.push(
                    ValidationIssue::error(
                        "unknown_node_type",
                        format!(
                            "node type `{}` version {} is not registered",
                            node.node_type, node.version
                        ),
                    )
                    .for_node(&node.id),
                );
                continue;
            };
            definitions.insert(node.id.as_str(), definition);
            match jsonschema::validator_for(&definition.config_schema) {
                Ok(validator) => {
                    for error in validator.iter_errors(&node.config) {
                        issues.push(
                            ValidationIssue::error("invalid_node_config", error.to_string())
                                .for_node(&node.id)
                                .at_path(format!(
                                    "/graph/nodes/{}/config{}",
                                    node.id,
                                    error.instance_path()
                                )),
                        );
                    }
                }
                Err(error) => issues.push(
                    ValidationIssue::error(
                        "invalid_node_definition",
                        format!("registered config schema is invalid: {error}"),
                    )
                    .for_node(&node.id),
                ),
            }
            if matches!(node.node_type.as_str(), "loop" | "for_each") {
                let max_iterations = node
                    .config
                    .get("maxIterations")
                    .and_then(Value::as_u64)
                    .unwrap_or_default();
                if !(1..=1_000).contains(&max_iterations) {
                    issues.push(
                        ValidationIssue::error(
                            "loop_bound_required",
                            "loop nodes require maxIterations between 1 and 1000",
                        )
                        .for_node(&node.id)
                        .at_path("/config/maxIterations"),
                    );
                }
            }
            if let Some(schema) = node.config.get("outputSchema") {
                validate_node_schema(
                    schema,
                    "Agent output",
                    "/config/outputSchema",
                    node.id.as_str(),
                    &mut issues,
                );
            }
            if let Some(schema) = node.config.get("inputSchema") {
                validate_node_schema(
                    schema,
                    "node input",
                    "/config/inputSchema",
                    node.id.as_str(),
                    &mut issues,
                );
            }
        }

        let runtime_nodes: HashSet<&str> = source
            .graph
            .nodes
            .iter()
            .filter(|node| {
                definitions
                    .get(node.id.as_str())
                    .is_some_and(|definition| definition.phase == NodePhase::Runtime)
            })
            .map(|node| node.id.as_str())
            .collect();

        let starts: Vec<_> = source
            .graph
            .nodes
            .iter()
            .filter(|node| node.node_type == "start")
            .collect();
        if starts.len() != 1 {
            issues.push(ValidationIssue::error(
                "unique_entry_required",
                "a game requires exactly one Start node",
            ));
        }
        if starts
            .first()
            .is_some_and(|node| node.id != source.entry_node_id)
        {
            issues.push(ValidationIssue::error(
                "entry_node_mismatch",
                "entryNodeId must identify the Start node",
            ));
        }
        if !runtime_nodes.contains(source.entry_node_id.as_str()) {
            issues.push(
                ValidationIssue::error(
                    "entry_node_missing",
                    "entryNodeId must identify a registered runtime node",
                )
                .at_path("/entryNodeId"),
            );
        }

        let mut edge_ids = HashSet::new();
        let mut runtime_adjacency: HashMap<&str, Vec<&str>> = HashMap::new();
        let mut acyclic_graph = DiGraphMap::<&str, ()>::new();
        for node_id in &runtime_nodes {
            acyclic_graph.add_node(node_id);
        }
        for edge in &source.graph.edges {
            if !edge_ids.insert(edge.id.as_str()) {
                issues.push(
                    ValidationIssue::error(
                        "duplicate_edge_id",
                        format!("edge ID `{}` is duplicated", edge.id),
                    )
                    .for_edge(&edge.id),
                );
            }
            let Some(source_definition) = definitions.get(edge.source.node_id.as_str()) else {
                issues.push(
                    ValidationIssue::error(
                        "edge_source_missing",
                        format!("source node `{}` does not exist", edge.source.node_id),
                    )
                    .for_edge(&edge.id),
                );
                continue;
            };
            let Some(target_definition) = definitions.get(edge.target.node_id.as_str()) else {
                issues.push(
                    ValidationIssue::error(
                        "edge_target_missing",
                        format!("target node `{}` does not exist", edge.target.node_id),
                    )
                    .for_edge(&edge.id),
                );
                continue;
            };
            if source_definition.phase != target_definition.phase {
                issues.push(
                    ValidationIssue::error(
                        "cross_phase_edge",
                        "production and runtime nodes cannot share an execution edge",
                    )
                    .for_edge(&edge.id),
                );
                continue;
            }
            if !source_definition.has_output(&edge.source.port) {
                issues.push(
                    ValidationIssue::error(
                        "invalid_source_port",
                        format!(
                            "node `{}` has no output port `{}`",
                            edge.source.node_id, edge.source.port
                        ),
                    )
                    .for_edge(&edge.id),
                );
            }
            if !target_definition.has_input(&edge.target.port) {
                issues.push(
                    ValidationIssue::error(
                        "invalid_target_port",
                        format!(
                            "node `{}` has no input port `{}`",
                            edge.target.node_id, edge.target.port
                        ),
                    )
                    .for_edge(&edge.id),
                );
            }
            if source_definition.phase == NodePhase::Runtime {
                runtime_adjacency
                    .entry(edge.source.node_id.as_str())
                    .or_default()
                    .push(edge.target.node_id.as_str());
                let source_is_loop =
                    matches!(source_definition.node_type.as_str(), "loop" | "for_each");
                let target_is_loop =
                    matches!(target_definition.node_type.as_str(), "loop" | "for_each");
                if !source_is_loop && !target_is_loop {
                    acyclic_graph.add_edge(
                        edge.source.node_id.as_str(),
                        edge.target.node_id.as_str(),
                        (),
                    );
                }
            }
        }

        if is_cyclic_directed(&acyclic_graph) {
            issues.push(ValidationIssue::error(
                "unbounded_cycle",
                "runtime control flow contains a cycle outside a bounded Loop or ForEach node",
            ));
        }

        let mut reached = HashSet::new();
        if runtime_nodes.contains(source.entry_node_id.as_str()) {
            let mut queue = VecDeque::from([source.entry_node_id.as_str()]);
            while let Some(node_id) = queue.pop_front() {
                if !reached.insert(node_id) {
                    continue;
                }
                if let Some(targets) = runtime_adjacency.get(node_id) {
                    queue.extend(targets.iter().copied());
                }
            }
            for node_id in runtime_nodes.difference(&reached) {
                issues.push(
                    ValidationIssue::error(
                        "unreachable_node",
                        format!("runtime node `{node_id}` is unreachable from Start"),
                    )
                    .for_node(*node_id),
                );
            }
        }

        let outgoing_ports = source.graph.edges.iter().fold(
            HashMap::<&str, HashSet<&str>>::new(),
            |mut ports, edge| {
                if runtime_nodes.contains(edge.source.node_id.as_str())
                    && runtime_nodes.contains(edge.target.node_id.as_str())
                {
                    ports
                        .entry(edge.source.node_id.as_str())
                        .or_default()
                        .insert(edge.source.port.as_str());
                }
                ports
            },
        );
        for node in source
            .graph
            .nodes
            .iter()
            .filter(|node| runtime_nodes.contains(node.id.as_str()))
        {
            let ports = outgoing_ports.get(node.id.as_str());
            let has_port = |port: &str| ports.is_some_and(|ports| ports.contains(port));
            match node.node_type.as_str() {
                "end" if ports.is_some_and(|ports| !ports.is_empty()) => issues.push(
                    ValidationIssue::error(
                        "end_has_outgoing_route",
                        "End nodes cannot have outgoing routes",
                    )
                    .for_node(&node.id),
                ),
                "end" => {}
                "choice" => {
                    let mut option_ids = HashSet::new();
                    for option in node
                        .config
                        .get("options")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                    {
                        let option_id =
                            option.get("id").and_then(Value::as_str).unwrap_or_default();
                        if !option_ids.insert(option_id) {
                            issues.push(
                                ValidationIssue::error(
                                    "duplicate_choice_option",
                                    format!("Choice option `{option_id}` is duplicated"),
                                )
                                .for_node(&node.id),
                            );
                        }
                        if !has_port(option_id) {
                            issues.push(
                                ValidationIssue::error(
                                    "choice_route_missing",
                                    format!("Choice option `{option_id}` has no route"),
                                )
                                .for_node(&node.id),
                            );
                        }
                    }
                    for port in ports.into_iter().flatten() {
                        if !option_ids.contains(port) {
                            issues.push(
                                ValidationIssue::error(
                                    "choice_route_unknown",
                                    format!("Choice route `{port}` has no matching option"),
                                )
                                .for_node(&node.id),
                            );
                        }
                    }
                }
                "condition" => {
                    for port in ["true", "false"] {
                        if !has_port(port) {
                            issues.push(
                                ValidationIssue::error(
                                    "condition_route_missing",
                                    format!("Condition requires a `{port}` route"),
                                )
                                .for_node(&node.id),
                            );
                        }
                    }
                }
                "loop" | "for_each" => {
                    for port in ["body", "done"] {
                        if !has_port(port) {
                            issues.push(
                                ValidationIssue::error(
                                    "loop_route_missing",
                                    format!("{} requires a `{port}` route", node.node_type),
                                )
                                .for_node(&node.id),
                            );
                        }
                    }
                }
                "parallel" if ports.map_or(0, |ports| ports.len()) < 2 => issues.push(
                    ValidationIssue::error(
                        "parallel_branches_required",
                        "Parallel requires at least two branch routes",
                    )
                    .for_node(&node.id),
                ),
                "parallel" => {}
                "random" if ports.is_none_or(HashSet::is_empty) => issues.push(
                    ValidationIssue::error(
                        "random_route_required",
                        "Random requires at least one output route",
                    )
                    .for_node(&node.id),
                ),
                "random" => {}
                _ if !has_port("next") => issues.push(
                    ValidationIssue::error(
                        "next_route_missing",
                        format!("node `{}` requires a `next` route", node.id),
                    )
                    .for_node(&node.id),
                ),
                _ => {}
            }
        }

        let mut reverse_adjacency: HashMap<&str, Vec<&str>> = HashMap::new();
        for edge in &source.graph.edges {
            if runtime_nodes.contains(edge.source.node_id.as_str())
                && runtime_nodes.contains(edge.target.node_id.as_str())
            {
                reverse_adjacency
                    .entry(edge.target.node_id.as_str())
                    .or_default()
                    .push(edge.source.node_id.as_str());
            }
        }
        let mut can_reach_end = HashSet::new();
        let mut end_queue = source
            .graph
            .nodes
            .iter()
            .filter(|node| node.node_type == "end" && runtime_nodes.contains(node.id.as_str()))
            .map(|node| node.id.as_str())
            .collect::<VecDeque<_>>();
        while let Some(node_id) = end_queue.pop_front() {
            if !can_reach_end.insert(node_id) {
                continue;
            }
            if let Some(sources) = reverse_adjacency.get(node_id) {
                end_queue.extend(sources.iter().copied());
            }
        }
        for node_id in reached.difference(&can_reach_end) {
            issues.push(
                ValidationIssue::error(
                    "path_does_not_end",
                    format!("runtime node `{node_id}` cannot reach an End node"),
                )
                .for_node(*node_id),
            );
        }

        if !source
            .graph
            .nodes
            .iter()
            .any(|node| node.node_type == "end" && runtime_nodes.contains(node.id.as_str()))
        {
            issues.push(ValidationIssue::error(
                "ending_required",
                "a published game requires at least one End node",
            ));
        }

        let mut agent_ids = HashSet::new();
        for agent in &source.agents {
            if !agent_ids.insert(agent.id.as_str()) {
                issues.push(
                    ValidationIssue::error(
                        "duplicate_agent_id",
                        format!("Agent ID `{}` is duplicated", agent.id),
                    )
                    .at_path(format!("/agents/{}", agent.id)),
                );
            }
            if agent
                .profile_version_id
                .as_deref()
                .unwrap_or_default()
                .is_empty()
            {
                issues.push(
                    ValidationIssue::error(
                        "agent_version_required",
                        format!("Agent `{}` is not pinned to a Profile version", agent.id),
                    )
                    .at_path(format!("/agents/{}/profileVersionId", agent.id)),
                );
            }
            if !agent.execution_descriptor.is_object() {
                issues.push(
                    ValidationIssue::error(
                        "agent_execution_descriptor_invalid",
                        format!("Agent `{}` has an invalid execution descriptor", agent.id),
                    )
                    .at_path(format!("/agents/{}/executionDescriptor", agent.id)),
                );
            }
        }
        for node in source
            .graph
            .nodes
            .iter()
            .filter(|node| matches!(node.node_type.as_str(), "agent" | "tool"))
        {
            let agent_id = node
                .config
                .get("agentId")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if !agent_ids.contains(agent_id) {
                issues.push(
                    ValidationIssue::error(
                        "agent_reference_missing",
                        format!(
                            "{} node references unknown Agent `{agent_id}`",
                            node.node_type
                        ),
                    )
                    .for_node(&node.id)
                    .at_path("/config/agentId"),
                );
            }
        }
        let mut resource_ids = HashSet::new();
        for resource in &source.resources {
            if !resource_ids.insert(resource.id.as_str()) {
                issues.push(
                    ValidationIssue::error(
                        "duplicate_resource_id",
                        format!("resource ID `{}` is duplicated", resource.id),
                    )
                    .at_path(format!("/resources/{}", resource.id)),
                );
            }
            if !resource.approved {
                issues.push(
                    ValidationIssue::error(
                        "resource_not_approved",
                        format!("resource `{}` is not approved", resource.id),
                    )
                    .at_path(format!("/resources/{}", resource.id)),
                );
            }
        }

        let mut presentation_resource_ids = HashSet::new();
        for resource in &source.presentation_resources {
            if resource.id.trim().is_empty() {
                issues.push(
                    ValidationIssue::error(
                        "presentation_resource_id_required",
                        "logical presentation resources require an ID",
                    )
                    .at_path("/presentationResources"),
                );
            }
            if resource.kind.trim().is_empty() {
                issues.push(
                    ValidationIssue::error(
                        "presentation_resource_kind_required",
                        format!("logical resource `{}` requires a kind", resource.id),
                    )
                    .at_path(format!("/presentationResources/{}/kind", resource.id)),
                );
            }
            if !presentation_resource_ids.insert(resource.id.as_str()) {
                issues.push(
                    ValidationIssue::error(
                        "duplicate_presentation_resource_id",
                        format!(
                            "logical presentation resource `{}` is duplicated",
                            resource.id
                        ),
                    )
                    .at_path(format!("/presentationResources/{}", resource.id)),
                );
            }
            let mut capabilities = HashSet::new();
            for capability in &resource.required_capabilities {
                if capability.trim().is_empty() || !capabilities.insert(capability.as_str()) {
                    issues.push(
                        ValidationIssue::error(
                            "presentation_capability_invalid",
                            format!(
                                "logical resource `{}` contains an empty or duplicate capability",
                                resource.id
                            ),
                        )
                        .at_path(format!(
                            "/presentationResources/{}/requiredCapabilities",
                            resource.id
                        )),
                    );
                }
            }
            if !resource.required
                && !resource.required_capabilities.is_empty()
                && resource.fallback.is_none()
            {
                issues.push(
                    ValidationIssue::warning(
                        "optional_capability_fallback_missing",
                        format!(
                            "optional logical resource `{}` will be omitted when its host capability is unavailable",
                            resource.id
                        ),
                    )
                    .at_path(format!("/presentationResources/{}/fallback", resource.id)),
                );
            }
        }
        let mut declared_locales = HashSet::new();
        for locale in &source.locales {
            if locale.trim().is_empty() {
                issues.push(
                    ValidationIssue::error("locale_invalid", "locale identifiers cannot be empty")
                        .at_path("/locales"),
                );
            } else if !declared_locales.insert(locale.as_str()) {
                issues.push(
                    ValidationIssue::error(
                        "duplicate_locale",
                        format!("locale `{locale}` is declared more than once"),
                    )
                    .at_path("/locales"),
                );
            }
        }
        let mut subtitle_groups = HashMap::<String, (String, HashSet<String>)>::new();
        for node in source
            .graph
            .nodes
            .iter()
            .filter(|node| runtime_nodes.contains(node.id.as_str()))
        {
            if let Some(logical_id) = node.config.get("logicalResourceId").and_then(Value::as_str) {
                if !presentation_resource_ids.contains(logical_id) {
                    issues.push(
                        ValidationIssue::error(
                            "logical_resource_missing",
                            format!("node references undeclared logical resource `{logical_id}`"),
                        )
                        .for_node(&node.id)
                        .at_path("/config/logicalResourceId"),
                    );
                }
            }
            if node.node_type == "host_action" {
                let target = node
                    .config
                    .get("target")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if !presentation_resource_ids.contains(target) {
                    issues.push(
                        ValidationIssue::error(
                            "host_action_target_missing",
                            format!("Host Action target `{target}` is not declared as a logical resource"),
                        )
                        .for_node(&node.id)
                        .at_path("/config/target"),
                    );
                }
            }
            if node.node_type == "subtitle" {
                let locale = node
                    .config
                    .get("locale")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if locale.is_empty() {
                    issues.push(
                        ValidationIssue::error(
                            "subtitle_locale_required",
                            "Subtitle nodes require a locale",
                        )
                        .for_node(&node.id)
                        .at_path("/config/locale"),
                    );
                } else if !declared_locales.contains(locale) {
                    issues.push(
                        ValidationIssue::error(
                            "subtitle_locale_undeclared",
                            format!("Subtitle locale `{locale}` is not declared by the Game"),
                        )
                        .for_node(&node.id)
                        .at_path("/config/locale"),
                    );
                }
                if declared_locales.len() > 1 {
                    let group = node
                        .config
                        .get("subtitleKey")
                        .or_else(|| node.config.get("logicalResourceId"))
                        .and_then(Value::as_str)
                        .filter(|value| !value.trim().is_empty());
                    if let Some(group) = group {
                        subtitle_groups
                            .entry(group.to_string())
                            .or_insert_with(|| (node.id.clone(), HashSet::new()))
                            .1
                            .insert(locale.to_string());
                    } else {
                        issues.push(
                            ValidationIssue::error(
                                "subtitle_group_required",
                                "multilingual Subtitle nodes require subtitleKey or logicalResourceId",
                            )
                            .for_node(&node.id)
                            .at_path("/config/subtitleKey"),
                        );
                    }
                }
            }
            if node.config.get("assetVersionId").is_some() {
                issues.push(
                    ValidationIssue::error(
                        "managed_asset_in_gameplay",
                        "Game nodes must reference logical resources, not managed asset versions",
                    )
                    .for_node(&node.id)
                    .at_path("/config/assetVersionId"),
                );
            }
        }

        for (group, (node_id, locales)) in subtitle_groups {
            for locale in &source.locales {
                if !locales.contains(locale) {
                    issues.push(
                        ValidationIssue::error(
                            "subtitle_locale_missing",
                            format!("subtitle group `{group}` has no `{locale}` version"),
                        )
                        .for_node(&node_id)
                        .at_path("/config/locale"),
                    );
                }
            }
        }

        if source.locales.is_empty() {
            issues.push(ValidationIssue::warning(
                "locale_defaulted",
                "no locale is declared; clients will use their own default",
            ));
        }

        issues
    }

    pub fn compile(&self, source: &GameSourceV1) -> Result<CompileOutput, GameRuntimeError> {
        let issues = self.validate(source);
        let errors: Vec<_> = issues
            .iter()
            .filter(|issue| issue.severity == ValidationSeverity::Error)
            .cloned()
            .collect();
        if !errors.is_empty() {
            return Err(GameRuntimeError::Validation(errors));
        }

        let runtime_nodes: Vec<_> = source
            .graph
            .nodes
            .iter()
            .filter(|node| {
                self.registry
                    .definition(&node.node_type, node.version)
                    .is_some_and(|definition| definition.phase == NodePhase::Runtime)
            })
            .collect();
        let mut sorted_nodes = runtime_nodes;
        sorted_nodes.sort_by(|left, right| left.id.cmp(&right.id));
        let ordinals: BTreeMap<_, _> = sorted_nodes
            .iter()
            .enumerate()
            .map(|(ordinal, node)| (node.id.as_str(), ordinal as u32))
            .collect();
        let nodes = sorted_nodes
            .iter()
            .enumerate()
            .map(|(ordinal, node)| CompiledNode {
                ordinal: ordinal as u32,
                id: node.id.clone(),
                node_type: node.node_type.clone(),
                version: node.version,
                config: node.config.clone(),
            })
            .collect();
        let mut edges: Vec<_> = source
            .graph
            .edges
            .iter()
            .filter_map(|edge| {
                Some(CompiledEdge {
                    id: edge.id.clone(),
                    source_node: *ordinals.get(edge.source.node_id.as_str())?,
                    source_port: edge.source.port.clone(),
                    target_node: *ordinals.get(edge.target.node_id.as_str())?,
                    target_port: edge.target.port.clone(),
                    condition: edge.condition.clone(),
                })
            })
            .collect();
        edges.sort_by(|left, right| {
            (
                left.source_node,
                left.source_port.as_str(),
                left.target_node,
                left.id.as_str(),
            )
                .cmp(&(
                    right.source_node,
                    right.source_port.as_str(),
                    right.target_node,
                    right.id.as_str(),
                ))
        });

        let mut agents: Vec<_> = source
            .agents
            .iter()
            .map(|agent| PinnedAgent {
                id: agent.id.clone(),
                profile_id: agent.profile_id.clone(),
                profile_version_id: agent.profile_version_id.clone().unwrap_or_default(),
                capabilities: agent.capabilities.clone(),
                execution_descriptor: agent.execution_descriptor.clone(),
            })
            .collect();
        agents.sort_by(|left, right| left.id.cmp(&right.id));
        let mut resources: Vec<_> = source
            .resources
            .iter()
            .map(|resource| PinnedResource {
                id: resource.id.clone(),
                version_id: resource.version_id.clone(),
                kind: resource.kind.clone(),
                content_hash: resource.content_hash.clone(),
            })
            .collect();
        resources.sort_by(|left, right| left.id.cmp(&right.id));

        let plan = GamePlanV1 {
            schema_version: GAME_SCHEMA_VERSION,
            entry_node: *ordinals.get(source.entry_node_id.as_str()).ok_or_else(|| {
                GameRuntimeError::InvalidPlan("entry node is missing".to_string())
            })?,
            nodes,
            edges,
            inputs: source.inputs.clone(),
            outputs: source.outputs.clone(),
            variables: sorted_by_id(&source.variables, |variable| &variable.id),
            agents,
            resources,
            presentation_resources: sorted_by_id(&source.presentation_resources, |resource| {
                &resource.id
            }),
            locales: sorted_strings(&source.locales),
        };
        let manifest = self.build_manifest(source);
        let content_hash = content_hash(&(&plan, &manifest))?;
        let warnings = issues
            .into_iter()
            .filter(|issue| issue.severity == ValidationSeverity::Warning)
            .collect();

        Ok(CompileOutput {
            plan,
            manifest,
            content_hash,
            warnings,
        })
    }

    fn build_manifest(&self, source: &GameSourceV1) -> GameManifestV1 {
        let scenes = source
            .graph
            .nodes
            .iter()
            .filter(|node| node.node_type == "scene")
            .map(|node| ManifestItem {
                id: node.id.clone(),
                name: node
                    .config
                    .get("name")
                    .and_then(Value::as_str)
                    .or(node.label.as_deref())
                    .unwrap_or(&node.id)
                    .to_string(),
            })
            .collect();
        let characters = source
            .agents
            .iter()
            .map(|agent| ManifestItem {
                id: agent.id.clone(),
                name: agent.profile_id.clone(),
            })
            .collect();
        let required_host_capabilities: BTreeSet<_> = source
            .presentation_resources
            .iter()
            .filter(|resource| resource.required)
            .flat_map(|resource| resource.required_capabilities.iter().cloned())
            .collect();
        let optional_host_capabilities: BTreeSet<_> = source
            .presentation_resources
            .iter()
            .filter(|resource| !resource.required)
            .flat_map(|resource| resource.required_capabilities.iter().cloned())
            .collect();

        GameManifestV1 {
            schema_version: GAME_SCHEMA_VERSION,
            name: source.metadata.name.clone(),
            description: source.metadata.description.clone(),
            commands: default_command_schemas(),
            events: default_event_schemas(),
            inputs: source.inputs.clone(),
            outputs: source.outputs.clone(),
            scenes,
            characters,
            logical_resources: sorted_by_id(&source.presentation_resources, |resource| {
                &resource.id
            }),
            required_host_capabilities: required_host_capabilities.into_iter().collect(),
            optional_host_capabilities: optional_host_capabilities.into_iter().collect(),
            locales: sorted_strings(&source.locales),
            compatibility: ClientCompatibility {
                protocol: "vifu.game.v1".to_string(),
                minimum_version: 1,
            },
        }
    }
}

fn sorted_by_id<T: Clone>(items: &[T], id: impl Fn(&T) -> &str) -> Vec<T> {
    let mut sorted = items.to_vec();
    sorted.sort_by(|left, right| id(left).cmp(id(right)));
    sorted
}

fn sorted_strings(items: &[String]) -> Vec<String> {
    let mut sorted = items.to_vec();
    sorted.sort();
    sorted.dedup();
    sorted
}

fn content_hash(value: &impl Serialize) -> Result<String, GameRuntimeError> {
    let bytes = canonical_json_bytes(value)?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

fn validate_public_schema(
    schema: &Value,
    label: &str,
    path: &str,
    issues: &mut Vec<ValidationIssue>,
) {
    if schema.get("type").and_then(Value::as_str) != Some("object") {
        issues.push(
            ValidationIssue::error(
                format!("invalid_{label}_schema"),
                format!("public {label} schema must describe a JSON object"),
            )
            .at_path(path),
        );
    }
    validate_schema(schema, label, path, None, issues);
}

fn validate_node_schema(
    schema: &Value,
    label: &str,
    path: &str,
    node_id: &str,
    issues: &mut Vec<ValidationIssue>,
) {
    validate_schema(schema, label, path, Some(node_id), issues);
}

fn validate_schema(
    schema: &Value,
    label: &str,
    path: &str,
    node_id: Option<&str>,
    issues: &mut Vec<ValidationIssue>,
) {
    let issue = |code: &str, message: String| {
        let issue = ValidationIssue::error(code, message).at_path(path);
        node_id.map_or(issue.clone(), |node_id| issue.for_node(node_id))
    };
    if schema_contains_reference(schema) {
        issues.push(issue(
            "schema_reference_not_supported",
            format!("{label} schema cannot use $ref"),
        ));
    }
    if let Err(error) = jsonschema::validator_for(schema) {
        issues.push(issue(
            "invalid_json_schema",
            format!("{label} schema is invalid: {error}"),
        ));
    }
}

fn schema_contains_reference(value: &Value) -> bool {
    match value {
        Value::Object(object) => object
            .iter()
            .any(|(key, value)| key == "$ref" || schema_contains_reference(value)),
        Value::Array(values) => values.iter().any(schema_contains_reference),
        _ => false,
    }
}

fn default_command_schemas() -> BTreeMap<String, Value> {
    [
        "game.start",
        "player.choice",
        "player.text",
        "player.action",
        "host.ready",
        "host.signal",
        "host.action.completed",
        "host.action.failed",
        "game.cancel",
    ]
    .into_iter()
    .map(|command| (command.to_string(), json!({"type": "object"})))
    .collect()
}

fn default_event_schemas() -> BTreeMap<String, Value> {
    [
        "game.session.started",
        "game.session.waiting",
        "game.session.completed",
        "game.session.failed",
        "game.session.cancelled",
        "scene.entered",
        "dialogue.started",
        "dialogue.completed",
        "choice.presented",
        "choice.selected",
        "state.changed",
        "host.action.requested",
        "host.action.completed",
        "agent.requested",
        "agent.completed",
        "agent.failed",
        "tool.requested",
        "tool.completed",
        "tool.failed",
        "ending.reached",
        "background.changed",
        "character.visual.changed",
        "character.expression.changed",
        "audio.play",
        "video.play",
        "voice.play",
        "subtitle.show",
        "transition.requested",
        "camera.cue.requested",
        "timeline.delay",
        "presentation.requested",
    ]
    .into_iter()
    .map(|event| (event.to_string(), json!({"type": "object"})))
    .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use pretty_assertions::assert_eq;
    use serde_json::json;

    use crate::{PortReference, SourceEdge, SourceNode};

    use super::*;

    fn source() -> GameSourceV1 {
        let mut source = GameSourceV1::new("Branching game");
        source.graph.nodes.extend([
            SourceNode {
                id: "scene".to_string(),
                node_type: "scene".to_string(),
                version: 1,
                config: json!({"name": "Opening", "startMs": 0}),
                parent_id: None,
                label: Some("Opening".to_string()),
                notes: None,
            },
            SourceNode {
                id: "end".to_string(),
                node_type: "end".to_string(),
                version: 1,
                config: json!({"endingId": "ending.one"}),
                parent_id: None,
                label: Some("Ending".to_string()),
                notes: None,
            },
        ]);
        source.graph.edges.extend([
            edge("start-scene", "start", "next", "scene", "in"),
            edge("scene-end", "scene", "next", "end", "in"),
        ]);
        source
    }

    fn edge(id: &str, source: &str, output: &str, target: &str, input: &str) -> SourceEdge {
        SourceEdge {
            id: id.to_string(),
            source: PortReference {
                node_id: source.to_string(),
                port: output.to_string(),
            },
            target: PortReference {
                node_id: target.to_string(),
                port: input.to_string(),
            },
            condition: None,
        }
    }

    #[test]
    fn view_metadata_does_not_change_release_hash() {
        let compiler = GameCompiler::default();
        let mut left = source();
        let mut right = left.clone();
        left.views = BTreeMap::from([("canvas".to_string(), json!({"x": 20, "y": 30}))]);
        right.views = BTreeMap::from([("shortDrama".to_string(), json!({"track": 4, "zoom": 2}))]);

        let left = compiler.compile(&left).expect("compile left");
        let right = compiler.compile(&right).expect("compile right");
        assert_eq!(left.content_hash, right.content_hash);
        assert_eq!(left.plan, right.plan);
    }

    #[test]
    fn semantic_node_change_changes_release_hash() {
        let compiler = GameCompiler::default();
        let left = source();
        let mut right = left.clone();
        right.graph.nodes[1].config["startMs"] = json!(500);

        assert_ne!(
            compiler.compile(&left).expect("compile left").content_hash,
            compiler
                .compile(&right)
                .expect("compile right")
                .content_hash
        );
    }

    #[test]
    fn compiler_assigns_stable_ordinals_independent_of_source_order() {
        let compiler = GameCompiler::default();
        let left = source();
        let mut right = left.clone();
        right.graph.nodes.reverse();
        right.graph.edges.reverse();

        assert_eq!(
            compiler.compile(&left).expect("compile left").plan,
            compiler.compile(&right).expect("compile right").plan
        );
    }

    #[test]
    fn compiler_reports_unreachable_nodes() {
        let compiler = GameCompiler::default();
        let mut source = source();
        source.graph.nodes.push(SourceNode {
            id: "orphan".to_string(),
            node_type: "dialogue".to_string(),
            version: 1,
            config: json!({}),
            parent_id: None,
            label: None,
            notes: None,
        });

        let GameRuntimeError::Validation(issues) = compiler
            .compile(&source)
            .expect_err("orphan should not compile")
        else {
            panic!("expected validation error");
        };
        assert!(issues.iter().any(|issue| issue.code == "unreachable_node"));
    }

    #[test]
    fn compiler_requires_every_choice_option_to_have_a_route() {
        let compiler = GameCompiler::default();
        let mut source = source();
        source.graph.nodes.push(SourceNode {
            id: "choice".to_string(),
            node_type: "choice".to_string(),
            version: 1,
            config: json!({
                "options": [
                    {"id": "left", "label": "Left"},
                    {"id": "right", "label": "Right"}
                ]
            }),
            parent_id: None,
            label: None,
            notes: None,
        });
        source.graph.edges = vec![
            edge("start-scene", "start", "next", "scene", "in"),
            edge("scene-choice", "scene", "next", "choice", "in"),
            edge("choice-end", "choice", "left", "end", "in"),
        ];

        let GameRuntimeError::Validation(issues) = compiler
            .compile(&source)
            .expect_err("incomplete Choice should not compile")
        else {
            panic!("expected validation error");
        };
        assert!(issues.iter().any(|issue| {
            issue.code == "choice_route_missing" && issue.message.contains("right")
        }));
    }

    #[test]
    fn compiler_requires_host_actions_to_use_declared_logical_targets() {
        let compiler = GameCompiler::default();
        let mut source = source();
        source.graph.nodes.push(SourceNode {
            id: "gate".to_string(),
            node_type: "host_action".to_string(),
            version: 1,
            config: json!({"target": "world.main-gate", "action": "open"}),
            parent_id: None,
            label: None,
            notes: None,
        });
        source.graph.edges = vec![
            edge("start-scene", "start", "next", "scene", "in"),
            edge("scene-gate", "scene", "next", "gate", "in"),
            edge("gate-end", "gate", "next", "end", "in"),
        ];

        let GameRuntimeError::Validation(issues) = compiler
            .compile(&source)
            .expect_err("undeclared Host Action target should not compile")
        else {
            panic!("expected validation error");
        };
        assert!(issues
            .iter()
            .any(|issue| issue.code == "host_action_target_missing"));
    }

    #[test]
    fn compiler_rejects_reachable_paths_that_cannot_end() {
        let compiler = GameCompiler::default();
        let mut source = source();
        source.graph.edges = vec![edge("start-scene", "start", "next", "scene", "in")];

        let GameRuntimeError::Validation(issues) = compiler
            .compile(&source)
            .expect_err("dead execution path should not compile")
        else {
            panic!("expected validation error");
        };
        assert!(issues.iter().any(|issue| issue.code == "path_does_not_end"));
    }

    #[test]
    fn compiler_rejects_unsafe_public_schemas_and_reserved_variables() {
        let compiler = GameCompiler::default();
        let mut source = source();
        source.inputs =
            json!({"type": "array", "items": {"$ref": "https://example.invalid/schema"}});
        source.variables.push(crate::GameVariable {
            id: "__vifu_internal".to_string(),
            initial_value: Value::Null,
            public: false,
        });

        let issues = compiler.validate(&source);
        assert!(issues
            .iter()
            .any(|issue| issue.code == "invalid_input_schema"));
        assert!(issues
            .iter()
            .any(|issue| issue.code == "schema_reference_not_supported"));
        assert!(issues
            .iter()
            .any(|issue| issue.code == "variable_id_reserved"));
    }

    #[test]
    fn optional_host_capabilities_without_fallback_are_readiness_warnings() {
        let compiler = GameCompiler::default();
        let mut source = source();
        source
            .presentation_resources
            .push(crate::LogicalPresentationResource {
                id: "scene.background".to_string(),
                kind: "image".to_string(),
                required_capabilities: vec!["vifu.presentation.image.v1".to_string()],
                required: false,
                fallback: None,
            });

        assert!(compiler.validate(&source).iter().any(|issue| {
            issue.code == "optional_capability_fallback_missing"
                && issue.severity == ValidationSeverity::Warning
        }));
    }

    #[test]
    fn multilingual_subtitles_require_complete_locale_coverage() {
        let compiler = GameCompiler::default();
        let mut source = source();
        source.locales = vec!["en".to_string(), "ja".to_string()];
        source.graph.nodes.push(SourceNode {
            id: "subtitle-en".to_string(),
            node_type: "subtitle".to_string(),
            version: 1,
            config: json!({"subtitleKey": "opening", "locale": "en", "text": "Hello"}),
            parent_id: None,
            label: None,
            notes: None,
        });
        source.graph.edges = vec![
            edge("start-subtitle", "start", "next", "subtitle-en", "in"),
            edge("subtitle-end", "subtitle-en", "next", "end", "in"),
        ];

        assert!(compiler.validate(&source).iter().any(|issue| {
            issue.code == "subtitle_locale_missing" && issue.message.contains("ja")
        }));
    }
}
