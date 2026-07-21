mod canonical;
mod compiler;
mod condition;
mod contract;
mod error;
mod registry;
mod runtime;

pub use canonical::{canonical_json, canonical_json_bytes, source_from_yaml, source_to_yaml};
pub use compiler::{CompileOutput, GameCompiler};
pub use condition::{evaluate_condition, ConditionExpression, ValueExpression};
pub use contract::*;
pub use error::{GameRuntimeError, ValidationIssue, ValidationSeverity};
pub use registry::{NodeDefinition, NodePhase, NodeRegistry, PortDefinition, PortDirection};
pub use runtime::{GameRuntime, GameRuntimePlugin};

pub const GAME_SCHEMA_VERSION: u32 = 1;
