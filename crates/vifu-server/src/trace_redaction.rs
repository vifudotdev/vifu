use serde_json::Value;

const MAX_REDACTION_DEPTH: usize = 32;
const MAX_TRACE_TEXT_CHARS: usize = 512;

pub(crate) fn redact_trace_value(value: &Value) -> Value {
    redact(value, 0)
}

pub(crate) fn redact_trace_text(value: &str) -> String {
    if contains_sensitive_trace_text(value) {
        return "[REDACTED sensitive trace error]".to_string();
    }
    value
        .chars()
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\t'))
        .take(MAX_TRACE_TEXT_CHARS)
        .collect()
}

fn redact(value: &Value, depth: usize) -> Value {
    if depth >= MAX_REDACTION_DEPTH {
        return Value::String("[REDACTED deep value]".to_string());
    }
    match value {
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, value)| {
                    let value = if sensitive_key(key) {
                        Value::String("[REDACTED]".to_string())
                    } else {
                        redact(value, depth + 1)
                    };
                    (key.clone(), value)
                })
                .collect(),
        ),
        Value::Array(values) => Value::Array(
            values
                .iter()
                .map(|value| redact(value, depth + 1))
                .collect(),
        ),
        Value::String(value) if contains_sensitive_trace_text(value) => {
            Value::String("[REDACTED sensitive value]".to_string())
        }
        _ => value.clone(),
    }
}

fn sensitive_key(key: &str) -> bool {
    let normalized = key
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    [
        "authorization",
        "apikey",
        "accesstoken",
        "refreshtoken",
        "token",
        "secret",
        "password",
        "credential",
        "cookie",
        "session",
        "sessionid",
    ]
    .iter()
    .any(|candidate| normalized == *candidate || normalized.ends_with(candidate))
}

pub(crate) fn contains_sensitive_trace_text(value: &str) -> bool {
    let lower = value.trim().to_ascii_lowercase();
    lower.starts_with("data:")
        || lower.starts_with("bearer ")
        || lower.starts_with("basic ")
        || [
            "authorization:",
            "authorization=",
            "api_key=",
            "api-key=",
            "apikey=",
            "access_token=",
            "access token:",
            "token=",
            "token:",
            "secret=",
            "secret:",
            "password=",
            "password:",
            "credential=",
            "credential:",
            "cookie=",
            "cookie:",
            "session=",
            "session:",
            "vifu_pk_",
            "vifu_gw_",
        ]
        .iter()
        .any(|marker| lower.contains(marker))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{redact_trace_text, redact_trace_value};

    #[test]
    fn recursively_redacts_sensitive_keys_and_credential_strings() {
        let redacted = redact_trace_value(&json!({
            "nested": {
                "apiKey": "sk-private",
                "messages": [{
                    "content": "Authorization: Bearer private-token",
                    "safe": "hello"
                }]
            }
        }));
        let serialized = redacted.to_string();
        assert_eq!(redacted["nested"]["apiKey"], "[REDACTED]");
        assert_eq!(
            redacted["nested"]["messages"][0]["content"],
            "[REDACTED sensitive value]"
        );
        assert_eq!(redacted["nested"]["messages"][0]["safe"], "hello");
        assert!(!serialized.contains("private-token"));
        assert!(!serialized.contains("sk-private"));
    }

    #[test]
    fn trace_errors_redact_credentials_and_preserve_bounded_diagnostics() {
        assert_eq!(
            redact_trace_text("request failed: Authorization: Bearer private-token"),
            "[REDACTED sensitive trace error]"
        );
        assert_eq!(
            redact_trace_text("model returned invalid JSON"),
            "model returned invalid JSON"
        );
        assert_eq!(redact_trace_text(&"x".repeat(700)).chars().count(), 512);
    }
}
