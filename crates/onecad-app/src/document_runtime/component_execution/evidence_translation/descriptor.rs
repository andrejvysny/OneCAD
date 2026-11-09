//! Closed native worker descriptor transport; unknown spatial semantics refuse.
use super::*;
use serde_json::{Map, Value};

const REQUIRED: [&str; 11] = [
    "shapeType",
    "center",
    "size",
    "magnitude",
    "surfaceType",
    "curveType",
    "normal",
    "tangent",
    "hasNormal",
    "hasTangent",
    "adjacencyHash",
];
const OPTIONAL: [&str; 3] = ["outward", "originSlot", "generatorSlot"];

fn vector(value: &Value) -> Result<[f64; 3], ApiError> {
    let values = value
        .as_array()
        .filter(|values| values.len() == 3)
        .ok_or_else(invalid_descriptor)?;
    let mut result = [0.0; 3];
    for (out, value) in result.iter_mut().zip(values) {
        *out = value
            .as_f64()
            .filter(|value| value.is_finite())
            .ok_or_else(invalid_descriptor)?;
    }
    Ok(result)
}

fn invalid_descriptor() -> ApiError {
    invalid("unsupported native translation evidence descriptor")
}

fn direction(fields: &Map<String, Value>, key: &str, present: bool) -> Result<(), ApiError> {
    let values = vector(&fields[key])?;
    let length_squared = values.iter().map(|value| value * value).sum::<f64>();
    if !length_squared.is_finite() || (present && length_squared <= 0.0) {
        return Err(invalid_descriptor());
    }
    Ok(())
}

fn slot(value: &Value) -> Result<(), ApiError> {
    let fields = value
        .as_object()
        .filter(|fields| fields.len() == 2)
        .ok_or_else(invalid_descriptor)?;
    serde_json::from_value::<RecordId>(
        fields
            .get("recordId")
            .cloned()
            .ok_or_else(invalid_descriptor)?,
    )
    .map_err(|_| invalid_descriptor())?;
    if fields
        .get("slot")
        .and_then(Value::as_str)
        .is_none_or(str::is_empty)
    {
        return Err(invalid_descriptor());
    }
    Ok(())
}

fn validate(fields: &Map<String, Value>, kind: ElementKind) -> Result<(), ApiError> {
    if REQUIRED.iter().any(|key| !fields.contains_key(*key))
        || fields
            .keys()
            .any(|key| !REQUIRED.contains(&key.as_str()) && !OPTIONAL.contains(&key.as_str()))
    {
        return Err(invalid_descriptor());
    }
    let shape = match kind {
        ElementKind::Face => 4,
        ElementKind::Edge => 6,
        ElementKind::Vertex => 7,
    };
    if fields["shapeType"].as_i64() != Some(shape)
        || !fields["surfaceType"]
            .as_i64()
            .is_some_and(|value| (0..=10).contains(&value))
        || !fields["curveType"]
            .as_i64()
            .is_some_and(|value| (0..=8).contains(&value))
    {
        return Err(invalid_descriptor());
    }
    for key in ["size", "magnitude"] {
        if fields[key]
            .as_f64()
            .is_none_or(|value| !value.is_finite() || value < 0.0)
        {
            return Err(invalid_descriptor());
        }
    }
    for (key, flag) in [("normal", "hasNormal"), ("tangent", "hasTangent")] {
        direction(
            fields,
            key,
            fields[flag].as_bool().ok_or_else(invalid_descriptor)?,
        )?;
    }
    if fields.contains_key("outward") {
        direction(fields, "outward", true)?;
    }
    let hash = fields["adjacencyHash"]
        .as_str()
        .ok_or_else(invalid_descriptor)?;
    if hash.len() != 16
        || !hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(invalid_descriptor());
    }
    for key in ["originSlot", "generatorSlot"] {
        if let Some(value) = fields.get(key) {
            slot(value)?;
        }
    }
    Ok(())
}

pub(super) fn translated(
    value: &Value,
    kind: ElementKind,
    delta: [f64; 3],
) -> Result<Value, ApiError> {
    let fields = value.as_object().ok_or_else(invalid_descriptor)?;
    validate(fields, kind)?;
    let center = vector(&fields["center"])?;
    let mut translated = [0.0; 3];
    for axis in 0..3 {
        translated[axis] = center[axis] + delta[axis];
        if !translated[axis].is_finite() {
            return Err(invalid_descriptor());
        }
    }
    let mut result = fields.clone();
    result.insert("center".into(), serde_json::json!(translated));
    Ok(Value::Object(result))
}
