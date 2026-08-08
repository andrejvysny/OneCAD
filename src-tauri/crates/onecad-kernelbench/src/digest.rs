use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

const EXCLUDED: &[&str] = &[
    "timing",
    "resource",
    "stderr",
    "message",
    "messages",
    "path",
    "paths",
    "artifacts",
    "timestamp",
    "timestamps",
    "normalizedDigest",
    "replay",
    "metamorph",
    "differential",
    "search",
];

pub fn sha256_value(value: &Value) -> String {
    let canonical = canonicalize(value);
    let bytes = serde_json::to_vec(&canonical).expect("Value serialization cannot fail");
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

pub fn normalized_digest(value: &Value) -> String {
    sha256_value(&normalize(value))
}

fn normalize(value: &Value) -> Value {
    match value {
        Value::Object(map) => normalize_object(map),
        Value::Array(values) => Value::Array(values.iter().map(normalize).collect()),
        Value::Number(number) => quantize(number),
        other => other.clone(),
    }
}

fn normalize_object(map: &Map<String, Value>) -> Value {
    let mut sorted = Map::new();
    let mut keys: Vec<_> = map.keys().collect();
    keys.sort_unstable();
    for key in keys {
        if !EXCLUDED.contains(&key.as_str()) {
            sorted.insert(key.clone(), normalize(&map[key]));
        }
    }
    Value::Object(sorted)
}

fn canonicalize(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut sorted = Map::new();
            let mut keys: Vec<_> = map.keys().collect();
            keys.sort_unstable();
            for key in keys {
                sorted.insert(key.clone(), canonicalize(&map[key]));
            }
            Value::Object(sorted)
        }
        Value::Array(values) => Value::Array(values.iter().map(canonicalize).collect()),
        other => other.clone(),
    }
}

fn quantize(number: &serde_json::Number) -> Value {
    let Some(value) = number.as_f64() else {
        return Value::Number(number.clone());
    };
    let quantized = (value * 1e9).round() / 1e9;
    serde_json::Number::from_f64(quantized)
        .map(Value::Number)
        .unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn volatility_does_not_change_digest() {
        let a = json!({"verdict":"pass","timing":{"wallMs":1},"diagnostics":{"message":"a"}});
        let b = json!({"diagnostics":{"message":"b"},"timing":{"wallMs":99},"verdict":"pass"});
        assert_eq!(normalized_digest(&a), normalized_digest(&b));
    }

    #[test]
    fn semantic_status_changes_digest() {
        assert_ne!(
            normalized_digest(&json!({"verdict":"pass"})),
            normalized_digest(&json!({"verdict":"fail"}))
        );
    }

    #[test]
    fn canonical_object_order_is_stable() {
        assert_eq!(
            sha256_value(&json!({"a":1,"b":{"x":2,"y":3}})),
            sha256_value(&json!({"b":{"y":3,"x":2},"a":1}))
        );
    }
}
