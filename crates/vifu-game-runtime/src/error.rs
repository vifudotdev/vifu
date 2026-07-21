use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ValidationSeverity {
    Error,
    Warning,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationIssue {
    pub severity: ValidationSeverity,
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edge_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

impl ValidationIssue {
    pub fn error(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            severity: ValidationSeverity::Error,
            code: code.into(),
            message: message.into(),
            node_id: None,
            edge_id: None,
            path: None,
        }
    }

    pub fn warning(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            severity: ValidationSeverity::Warning,
            code: code.into(),
            message: message.into(),
            node_id: None,
            edge_id: None,
            path: None,
        }
    }

    pub fn for_node(mut self, node_id: impl Into<String>) -> Self {
        self.node_id = Some(node_id.into());
        self
    }

    pub fn for_edge(mut self, edge_id: impl Into<String>) -> Self {
        self.edge_id = Some(edge_id.into());
        self
    }

    pub fn at_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }
}

#[derive(Debug, Error)]
pub enum GameRuntimeError {
    #[error("game source failed validation")]
    Validation(Vec<ValidationIssue>),
    #[error("unsupported game schema version {0}")]
    UnsupportedSchemaVersion(u32),
    #[error("locale `{0}` is not supported by this game")]
    UnsupportedLocale(String),
    #[error("node `{node_type}` version {version} is not registered")]
    UnknownNode { node_type: String, version: u32 },
    #[error("runtime is waiting for `{expected}`, not `{actual}`")]
    UnexpectedCommand { expected: String, actual: String },
    #[error("runtime session has already reached terminal status `{0}`")]
    SessionFinished(String),
    #[error("runtime exceeded its {0}-step command limit")]
    StepLimit(u32),
    #[error("runtime plan is invalid: {0}")]
    InvalidPlan(String),
    #[error("runtime state is invalid: {0}")]
    InvalidState(String),
    #[error("JSON serialization failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("YAML serialization failed: {0}")]
    Yaml(String),
}
