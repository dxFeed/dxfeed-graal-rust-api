//! The kit of the NetTest specification: the vectors that the SDK archive ships next to the specification. The build
//! script of the tools hands over its directory.

use std::path::PathBuf;

use serde_json::Value;

/// The path of a kit file.
fn kit_path(name: &str) -> PathBuf {
    PathBuf::from(env!("NETTEST_KIT_DIR")).join(name)
}

/// Reads a kit file as text; the message of a failure names the path.
pub fn kit_text(name: &str) -> String {
    let path = kit_path(name);
    std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "cannot read the kit file {}: {error}; the kit is part of the SDK archive (docs/wrapper-guide/nettest)",
            path.display()
        )
    })
}

/// Reads a kit file as JSON; the message of a failure names the path.
pub fn kit_json(name: &str) -> Value {
    serde_json::from_str(&kit_text(name))
        .unwrap_or_else(|error| panic!("the kit file {} is not JSON: {error}", kit_path(name).display()))
}

/// A 64-bit value of the kit: a hexadecimal string with the prefix `0x`.
pub fn kit_hex(value: &Value) -> u64 {
    let text = value
        .as_str()
        .unwrap_or_else(|| panic!("a hexadecimal string was expected: {value}"));
    let digits = text
        .strip_prefix("0x")
        .unwrap_or_else(|| panic!("the prefix 0x was expected: {text}"));
    u64::from_str_radix(digits, 16).unwrap_or_else(|error| panic!("not a hexadecimal value: {text}: {error}"))
}

/// The keywords of JSON Schema that the schema of the summary uses. A keyword outside this list is an error, so that a
/// kit whose schema grows is not passed silently.
const SCHEMA_KEYWORDS: &[&str] = &[
    "$schema",
    "$id",
    "title",
    "type",
    "const",
    "enum",
    "minimum",
    "maximum",
    "minLength",
    "required",
    "properties",
    "additionalProperties",
    "items",
];

/// The errors of the instance against the schema, each with the path of the value (`.config.symbols`); empty when it
/// is valid. Only the keywords of `SCHEMA_KEYWORDS` are checked, and an unknown one is an error.
pub fn schema_errors(schema: &Value, instance: &Value) -> Vec<String> {
    let mut errors = Vec::new();
    check(schema, instance, "", &mut errors);
    errors
}

/// The errors of a summary text against `nettest-summary.schema.json` of the kit; a text that is not JSON is one error.
pub fn summary_errors(text: &str) -> Vec<String> {
    match serde_json::from_str::<Value>(text) {
        Ok(summary) => schema_errors(&kit_json("nettest-summary.schema.json"), &summary),
        Err(error) => vec![format!("the summary is not JSON: {error}")],
    }
}

fn type_matches(name: &str, value: &Value) -> bool {
    match name {
        "null" => value.is_null(),
        "boolean" => value.is_boolean(),
        "string" => value.is_string(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        "number" => value.is_number(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        _ => false,
    }
}

fn check(schema: &Value, value: &Value, path: &str, errors: &mut Vec<String>) {
    let Some(schema) = schema.as_object() else {
        errors.push(format!("{path}: the schema is not an object"));
        return;
    };
    let shown = if path.is_empty() { "." } else { path };
    for (keyword, argument) in schema {
        match keyword.as_str() {
            "$schema" | "$id" | "title" => {}
            "type" => {
                let names: Vec<&str> = match argument {
                    Value::String(name) => vec![name.as_str()],
                    Value::Array(names) => names.iter().filter_map(Value::as_str).collect(),
                    _ => Vec::new(),
                };
                if !names.iter().any(|name| type_matches(name, value)) {
                    errors.push(format!("{shown}: is not of the type {names:?}: {value}"));
                }
            }
            "const" => {
                if value != argument {
                    errors.push(format!("{shown}: is not the constant {argument}: {value}"));
                }
            }
            "enum" => {
                if !argument.as_array().is_some_and(|options| options.contains(value)) {
                    errors.push(format!("{shown}: is not one of {argument}: {value}"));
                }
            }
            "minimum" | "maximum" => {
                if let (Some(number), Some(limit)) = (value.as_f64(), argument.as_f64()) {
                    let below = keyword == "minimum" && number < limit;
                    let above = keyword == "maximum" && number > limit;
                    if below || above {
                        errors.push(format!("{shown}: breaks the {keyword} {limit}: {number}"));
                    }
                }
            }
            "minLength" => {
                if let (Some(text), Some(limit)) = (value.as_str(), argument.as_u64())
                    && (text.chars().count() as u64) < limit
                {
                    errors.push(format!("{shown}: shorter than the minLength {limit}"));
                }
            }
            "required" => {
                if let (Some(object), Some(names)) = (value.as_object(), argument.as_array()) {
                    for name in names.iter().filter_map(Value::as_str) {
                        if !object.contains_key(name) {
                            errors.push(format!("{path}.{name}: is required"));
                        }
                    }
                }
            }
            "properties" => {
                if let (Some(object), Some(properties)) = (value.as_object(), argument.as_object()) {
                    for (name, property_schema) in properties {
                        if let Some(property) = object.get(name) {
                            check(property_schema, property, &format!("{path}.{name}"), errors);
                        }
                    }
                }
            }
            "additionalProperties" => {
                if let Some(object) = value.as_object() {
                    let known = schema.get("properties").and_then(Value::as_object);
                    for (name, property) in object {
                        if known.is_some_and(|known| known.contains_key(name)) {
                            continue;
                        }
                        match argument {
                            Value::Bool(false) => errors.push(format!("{path}.{name}: is not allowed")),
                            Value::Bool(true) => {}
                            other => check(other, property, &format!("{path}.{name}"), errors),
                        }
                    }
                }
            }
            "items" => {
                if let Some(items) = value.as_array() {
                    for (index, item) in items.iter().enumerate() {
                        check(argument, item, &format!("{path}[{index}]"), errors);
                    }
                }
            }
            unknown => errors.push(format!(
                "{shown}: the schema keyword '{unknown}' is not supported by the checker"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn the_checker_applies_each_keyword_of_the_schema_of_the_summary() {
        let schema = json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["name", "count"],
            "properties": {
                "name": {"type": "string", "minLength": 1},
                "count": {"type": ["integer", "null"], "minimum": 1, "maximum": 5},
                "kind": {"enum": ["a", "b", null]},
                "fixed": {"const": 7},
                "list": {"type": "array", "items": {"type": "integer"}}
            }
        });
        assert!(schema_errors(&schema, &json!({"name": "x", "count": 3, "kind": null, "fixed": 7})).is_empty());
        assert!(schema_errors(&schema, &json!({"name": "x", "count": null, "list": [1, 2]})).is_empty());
        let broken = |instance: Value, message: &str| {
            let errors = schema_errors(&schema, &instance);
            assert!(
                errors.iter().any(|error| error.contains(message)),
                "{instance}: {message} not in {errors:?}"
            );
        };
        broken(json!({"name": "", "count": 3}), ".name: shorter than the minLength");
        broken(json!({"name": "x", "count": 0}), ".count: breaks the minimum");
        broken(json!({"name": "x", "count": 6}), ".count: breaks the maximum");
        broken(json!({"name": "x", "count": 1.5}), ".count: is not of the type");
        broken(json!({"name": "x"}), ".count: is required");
        broken(json!({"name": "x", "count": 1, "other": 1}), ".other: is not allowed");
        broken(json!({"name": "x", "count": 1, "kind": "c"}), ".kind: is not one of");
        broken(
            json!({"name": "x", "count": 1, "fixed": 8}),
            ".fixed: is not the constant",
        );
        broken(
            json!({"name": "x", "count": 1, "list": [1, "2"]}),
            ".list[1]: is not of the type",
        );
        broken(json!([]), ".: is not of the type");
    }

    #[test]
    fn an_additional_property_is_checked_against_its_schema() {
        let schema = json!({"type": "object", "additionalProperties": {"type": "integer"}});
        assert!(schema_errors(&schema, &json!({"a": 1, "b": 2})).is_empty());
        assert!(!schema_errors(&schema, &json!({"a": "1"})).is_empty());
    }

    #[test]
    fn an_unknown_schema_keyword_is_an_error_at_any_depth() {
        let top = schema_errors(&json!({"type": "object", "pattern": "x"}), &json!({}));
        assert_eq!(top.len(), 1, "{top:?}");
        assert!(top[0].contains("'pattern'"), "{top:?}");
        let nested = json!({"type": "object", "properties": {"a": {"type": "string", "format": "uri"}}});
        let errors = schema_errors(&nested, &json!({"a": "x"}));
        assert!(errors.iter().any(|error| error.contains("'format'")), "{errors:?}");
        // The keywords of the schema of the kit are all known to the checker.
        let kit = kit_json("nettest-summary.schema.json");
        let mut keywords = Vec::new();
        collect_keywords(&kit, &mut keywords);
        for keyword in keywords {
            assert!(SCHEMA_KEYWORDS.contains(&keyword.as_str()), "{keyword}");
        }
    }

    /// The keywords of a schema and of the schemas below it; the names under `properties` are not keywords.
    fn collect_keywords(schema: &Value, keywords: &mut Vec<String>) {
        let Some(object) = schema.as_object() else { return };
        for (keyword, argument) in object {
            keywords.push(keyword.clone());
            match keyword.as_str() {
                "properties" => argument
                    .as_object()
                    .into_iter()
                    .flat_map(|properties| properties.values())
                    .for_each(|property| collect_keywords(property, keywords)),
                "items" | "additionalProperties" => collect_keywords(argument, keywords),
                _ => {}
            }
        }
    }

    #[test]
    fn a_text_that_is_not_json_is_an_error() {
        assert_eq!(summary_errors("{").len(), 1);
    }
}
