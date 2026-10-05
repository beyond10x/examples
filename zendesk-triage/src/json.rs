//! Conversions between `serde_json` values and the JSON value of Commission's generated model,
//! which keeps a number in the spelling it arrived in.

use loom_sdk::commission::model::json::Value as Model;
use serde_json::Value;

/// `value` as the model's JSON value.
pub fn to_model(value: &Value) -> Model {
    match value {
        Value::Null => Model::Null,
        Value::Bool(flag) => Model::Bool(*flag),
        Value::Number(number) => Model::Number(number.to_string()),
        Value::String(text) => Model::Text(text.clone()),
        Value::Array(items) => Model::Array(items.iter().map(to_model).collect()),
        Value::Object(members) => Model::Object(
            members
                .iter()
                .map(|(name, member)| (name.clone(), to_model(member)))
                .collect(),
        ),
    }
}

/// The model's JSON value as a `serde_json` value; a number whose spelling `serde_json` does not
/// read becomes its text.
pub fn from_model(value: &Model) -> Value {
    match value {
        Model::Null => Value::Null,
        Model::Bool(flag) => Value::Bool(*flag),
        Model::Number(spelling) => serde_json::from_str::<serde_json::Number>(spelling)
            .map_or_else(|_| Value::String(spelling.clone()), Value::Number),
        Model::Text(text) => Value::String(text.clone()),
        Model::Array(items) => Value::Array(items.iter().map(from_model).collect()),
        Model::Object(members) => Value::Object(
            members
                .iter()
                .map(|(name, member)| (name.clone(), from_model(member)))
                .collect(),
        ),
    }
}

/// The lowercase hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_round_trip() {
        let value = serde_json::json!({"a": [1, 2.5, "x", null, true], "b": {"c": -3}});
        assert_eq!(from_model(&to_model(&value)), value);
    }
}
