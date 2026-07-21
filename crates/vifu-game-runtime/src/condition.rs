use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ValueExpression {
    Pointer { pointer: String },
    Literal { value: Value },
}

impl ValueExpression {
    fn resolve<'a>(&'a self, context: &'a Value) -> Option<&'a Value> {
        match self {
            Self::Pointer { pointer } => context.pointer(pointer),
            Self::Literal { value } => Some(value),
        }
    }
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum ConditionExpression {
    Eq {
        left: ValueExpression,
        right: ValueExpression,
    },
    Ne {
        left: ValueExpression,
        right: ValueExpression,
    },
    Gt {
        left: ValueExpression,
        right: ValueExpression,
    },
    Gte {
        left: ValueExpression,
        right: ValueExpression,
    },
    Lt {
        left: ValueExpression,
        right: ValueExpression,
    },
    Lte {
        left: ValueExpression,
        right: ValueExpression,
    },
    Exists {
        value: ValueExpression,
    },
    And {
        conditions: Vec<ConditionExpression>,
    },
    Or {
        conditions: Vec<ConditionExpression>,
    },
    Not {
        condition: Box<ConditionExpression>,
    },
}

pub fn evaluate_condition(condition: &ConditionExpression, context: &Value) -> bool {
    match condition {
        ConditionExpression::Eq { left, right } => {
            compare_values(left, right, context, |ordering| ordering == 0)
        }
        ConditionExpression::Ne { left, right } => {
            compare_values(left, right, context, |ordering| ordering != 0)
        }
        ConditionExpression::Gt { left, right } => {
            compare_values(left, right, context, |ordering| ordering > 0)
        }
        ConditionExpression::Gte { left, right } => {
            compare_values(left, right, context, |ordering| ordering >= 0)
        }
        ConditionExpression::Lt { left, right } => {
            compare_values(left, right, context, |ordering| ordering < 0)
        }
        ConditionExpression::Lte { left, right } => {
            compare_values(left, right, context, |ordering| ordering <= 0)
        }
        ConditionExpression::Exists { value } => value.resolve(context).is_some(),
        ConditionExpression::And { conditions } => conditions
            .iter()
            .all(|condition| evaluate_condition(condition, context)),
        ConditionExpression::Or { conditions } => conditions
            .iter()
            .any(|condition| evaluate_condition(condition, context)),
        ConditionExpression::Not { condition } => !evaluate_condition(condition, context),
    }
}

fn compare_values(
    left: &ValueExpression,
    right: &ValueExpression,
    context: &Value,
    predicate: impl FnOnce(i8) -> bool,
) -> bool {
    let (Some(left), Some(right)) = (left.resolve(context), right.resolve(context)) else {
        return false;
    };

    let ordering = match (left, right) {
        (Value::Number(left), Value::Number(right)) => {
            let (Some(left), Some(right)) = (left.as_f64(), right.as_f64()) else {
                return false;
            };
            if left < right {
                -1
            } else if left > right {
                1
            } else {
                0
            }
        }
        (Value::String(left), Value::String(right)) => match left.cmp(right) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        },
        (Value::Bool(left), Value::Bool(right)) => match left.cmp(right) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        },
        _ if left == right => 0,
        _ => return false,
    };
    predicate(ordering)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn evaluates_nested_pointer_conditions() {
        let context = json!({"state": {"trust": 3, "met": true}});
        let condition = ConditionExpression::And {
            conditions: vec![
                ConditionExpression::Gte {
                    left: ValueExpression::Pointer {
                        pointer: "/state/trust".to_string(),
                    },
                    right: ValueExpression::Literal { value: json!(3) },
                },
                ConditionExpression::Eq {
                    left: ValueExpression::Pointer {
                        pointer: "/state/met".to_string(),
                    },
                    right: ValueExpression::Literal { value: json!(true) },
                },
            ],
        };

        assert!(evaluate_condition(&condition, &context));
    }
}
