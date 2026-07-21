use serde::Serialize;

use crate::{GameRuntimeError, GameSourceV1};

const MAX_YAML_SOURCE_BYTES: usize = 2 * 1024 * 1024;

pub fn canonical_json_bytes(value: &impl Serialize) -> Result<Vec<u8>, GameRuntimeError> {
    serde_json_canonicalizer::to_vec(value).map_err(GameRuntimeError::Json)
}

pub fn canonical_json(value: &impl Serialize) -> Result<String, GameRuntimeError> {
    serde_json_canonicalizer::to_string(value).map_err(GameRuntimeError::Json)
}

pub fn source_from_yaml(source: &str) -> Result<GameSourceV1, GameRuntimeError> {
    if source.len() > MAX_YAML_SOURCE_BYTES {
        return Err(GameRuntimeError::Yaml(format!(
            "source exceeds the {MAX_YAML_SOURCE_BYTES}-byte YAML import limit"
        )));
    }
    serde_saphyr::from_str(source).map_err(|error| GameRuntimeError::Yaml(error.to_string()))
}

pub fn source_to_yaml(source: &GameSourceV1) -> Result<String, GameRuntimeError> {
    serde_saphyr::to_string(source).map_err(|error| GameRuntimeError::Yaml(error.to_string()))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn canonical_json_sorts_object_keys() {
        let canonical = canonical_json(&json!({"z": 1, "a": 2})).expect("canonical JSON");
        assert_eq!(canonical, r#"{"a":2,"z":1}"#);
    }

    #[test]
    fn source_round_trips_through_yaml() {
        let source = GameSourceV1::new("YAML game");
        let yaml = source_to_yaml(&source).expect("serialize YAML");
        let restored = source_from_yaml(&yaml).expect("parse YAML");
        assert_eq!(restored, source);
    }
}
