//! Refuse unsupported data that a typed decode would silently discard.
use serde_json::Value;

pub(super) fn validate(source: &Value, decoded: &Value) -> Result<(), String> {
    visit(source, decoded, &mut Vec::new(), None)
}

fn visit<'a>(
    source: &'a Value,
    decoded: &Value,
    path: &mut Vec<String>,
    operation: Option<&'a str>,
) -> Result<(), String> {
    match source {
        Value::Object(fields) => {
            let operation = fields.get("opType").and_then(Value::as_str).or(operation);
            let output = decoded
                .as_object()
                .ok_or("document object shape changed during decode")?;
            for (key, value) in fields {
                let Some(present) = output.get(key) else {
                    if canonical_omission(path, key, value, operation, fields) {
                        continue;
                    }
                    return Err(format!(
                        "unsupported document field {}/{} would be discarded",
                        path.join("/"),
                        key
                    ));
                };
                path.push(key.clone());
                visit(value, present, path, operation)?;
                path.pop();
            }
        }
        Value::Array(items) => {
            let output = decoded
                .as_array()
                .ok_or("document array shape changed during decode")?;
            if items.len() != output.len() {
                return Err("document array entries would be discarded during decode".into());
            }
            for (index, (source, output)) in items.iter().zip(output).enumerate() {
                path.push(index.to_string());
                visit(source, output, path, operation)?;
                path.pop();
            }
        }
        _ => {}
    }
    Ok(())
}

fn canonical_omission(
    path: &[String],
    key: &str,
    value: &Value,
    operation: Option<&str>,
    fields: &serde_json::Map<String, Value>,
) -> bool {
    let names: Vec<&str> = path.iter().map(String::as_str).collect();
    let empty_array = value.as_array().is_some_and(Vec::is_empty);
    let empty_object = value.as_object().is_some_and(serde_json::Map::is_empty);
    if names.as_slice() == ["settings"] {
        return (key == "occtFingerprint" && value.is_null())
            || (key == "tolerancePolicyHash" && value == "");
    }
    let local = match names.as_slice() {
        ["rootDefinition", rest @ ..] | ["definitions", _, rest @ ..] => rest,
        _ => return false,
    };
    match local {
        [] => {
            (matches!(key, "sketchVisibility" | "elements") && empty_object)
                || (key == "namedSelections" && empty_array)
        }
        ["elements", _] => matches!(key, "anchor" | "descriptor") && value.is_null(),
        ["sketches", _] => {
            (matches!(key, "entities" | "constraints" | "regions") && empty_array)
                || (key == "projections" && empty_object)
                || (key == "resolvedPlane" && value.is_null())
        }
        ["sketches", _, "entities", _] => key == "referenceLocked" && value == false,
        ["sketches", _, "regions", _] => key == "holes" && empty_array,
        ["timeline", "records", _, "params"] => {
            parameter_omission(operation, key, value, empty_array)
        }
        ["timeline", "records", _, "determinism"] => {
            (key == "occtOptions" && empty_object)
                || (matches!(
                    key,
                    "occtOptionsHash" | "tolerancePolicyHash" | "solverPolicyHash"
                ) && value == "")
        }
        ["timeline", "records", _, "params", "distance" | "distance2" | "draftAngleDeg" | "diameter" | "depth" | "cbDiameter"
        | "cbDepth" | "csDiameter" | "csAngleDeg"] => {
            key == "expr" && value.is_null() && fields.get("value").is_some_and(Value::is_number)
        }
        _ => false,
    }
}

fn parameter_omission(
    operation: Option<&str>,
    key: &str,
    value: &Value,
    empty_array: bool,
) -> bool {
    match operation {
        Some("Sketch") => {
            (matches!(key, "entities" | "constraints") && empty_array)
                || (key == "hostFace" && value.is_null())
        }
        Some("Extrude") => {
            matches!(
                key,
                "profile" | "targetBodyId" | "targetFace" | "targetFace2"
            ) && value.is_null()
        }
        Some("Revolve") => matches!(key, "profile" | "axis" | "targetBodyId") && value.is_null(),
        Some("Hole") => {
            matches!(key, "points" | "thread" | "resultPolicyVersion") && value.is_null()
        }
        _ => false,
    }
}
