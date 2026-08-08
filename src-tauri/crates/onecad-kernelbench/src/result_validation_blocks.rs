use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Component, Path};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Diagnostic {
    code: String,
    phase: Phase,
    message: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
enum Phase {
    Request,
    Generation,
    Selection,
    Operation,
    Audit,
    Validation,
    Supervisor,
}

pub fn audit(value: &Value) -> Result<(), String> {
    exact(
        value,
        &[
            "productionAudit",
            "exactValid",
            "selfInterferenceFree",
            "closedManifold",
            "counts",
            "massProperties",
            "bounds",
            "tolerances",
            "microTopology",
        ],
    )?;
    enumeration(&value["productionAudit"], &["pass", "fail", "notRun"])?;
    booleans(
        value,
        &["exactValid", "selfInterferenceFree", "closedManifold"],
    )?;
    counts(&value["counts"])?;
    mass(&value["massProperties"])?;
    bounds(&value["bounds"])?;
    tolerances(&value["tolerances"])?;
    micro(&value["microTopology"])
}

fn counts(value: &Value) -> Result<(), String> {
    exact(value, &["solids", "shells", "faces", "edges", "vertices"])?;
    value
        .as_object()
        .unwrap()
        .values()
        .try_for_each(|item| bounded_u64(item, 10_000_000))
}

fn mass(value: &Value) -> Result<(), String> {
    exact(value, &["volume", "area", "centroid", "inertia"])?;
    nonnegative(&value["volume"])?;
    nonnegative(&value["area"])?;
    vector(&value["centroid"], 3)?;
    vector(&value["inertia"], 9)
}

fn bounds(value: &Value) -> Result<(), String> {
    exact(value, &["minimum", "maximum", "diagonal"])?;
    vector(&value["minimum"], 3)?;
    vector(&value["maximum"], 3)?;
    nonnegative(&value["diagonal"])
}

fn tolerances(value: &Value) -> Result<(), String> {
    exact(value, &["vertex", "edge", "face"])?;
    value
        .as_object()
        .unwrap()
        .values()
        .try_for_each(distribution)
}

fn distribution(value: &Value) -> Result<(), String> {
    exact(value, &["count", "maximum", "mean", "p95"])?;
    bounded_u64(&value["count"], 10_000_000)?;
    for key in ["maximum", "mean", "p95"] {
        nonnegative(&value[key])?;
    }
    Ok(())
}

fn micro(value: &Value) -> Result<(), String> {
    exact(
        value,
        &[
            "microEdgeCount",
            "minimumEdgeRatio",
            "sliverFaceCount",
            "minimumFaceRatio",
        ],
    )?;
    bounded_u64(&value["microEdgeCount"], 10_000_000)?;
    bounded_u64(&value["sliverFaceCount"], 10_000_000)?;
    nonnegative(&value["minimumEdgeRatio"])?;
    nonnegative(&value["minimumFaceRatio"])
}

pub fn validator(value: &Value) -> Result<(), String> {
    exact(
        value,
        &["kind", "required", "status", "metrics", "diagnostics"],
    )?;
    string(&value["kind"], 128)?;
    boolean(&value["required"])?;
    enumeration(
        &value["status"],
        &["pass", "fail", "notApplicable", "notRun"],
    )?;
    array(&value["metrics"], 32)?.iter().try_for_each(metric)?;
    for item in array(&value["diagnostics"], 16)? {
        diagnostic(item)?;
    }
    Ok(())
}

fn metric(value: &Value) -> Result<(), String> {
    exact_allowed(value, &["name", "value", "unit"], &["name", "value"])?;
    string(&value["name"], 128)?;
    finite(&value["value"])?;
    if let Some(unit) = value.get("unit") {
        string(unit, 32)?;
    }
    Ok(())
}

fn diagnostic(value: &Value) -> Result<(), String> {
    let parsed: Diagnostic =
        serde_json::from_value(value.clone()).map_err(|error| error.to_string())?;
    let _ = parsed.phase;
    if parsed.message.len() > 4096 || !safe_id(&parsed.code) {
        Err("invalid validator diagnostic".into())
    } else {
        Ok(())
    }
}

pub fn selection(value: &Value) -> Result<(), String> {
    exact(
        value,
        &[
            "provenance",
            "topologyRole",
            "anchor",
            "surface",
            "adjacency",
        ],
    )?;
    string(&value["provenance"], 256)?;
    string(&value["topologyRole"], 64)?;
    string(&value["surface"], 128)?;
    vector(&value["anchor"], 3)?;
    for item in array(&value["adjacency"], 32)? {
        string(item, 128)?;
    }
    Ok(())
}

pub fn metamorph(value: &Value) -> Result<(), String> {
    exact(
        value,
        &[
            "variant",
            "status",
            "normalizedPropertiesMatch",
            "semanticEvidenceMatch",
            "surfaceSamplesMatch",
            "pointClassificationMatch",
        ],
    )?;
    string(&value["variant"], 128)?;
    enumeration(&value["status"], &["pass", "fail", "notRun"])?;
    booleans(
        value,
        &[
            "normalizedPropertiesMatch",
            "semanticEvidenceMatch",
            "surfaceSamplesMatch",
            "pointClassificationMatch",
        ],
    )
}

pub fn search(value: &Value) -> Result<(), String> {
    exact(
        value,
        &[
            "parameter",
            "probeRadius",
            "probeCount",
            "monotonicObserved",
            "transitions",
        ],
    )?;
    if value["parameter"] != "operation.radius" {
        return Err("invalid search parameter".into());
    }
    positive(&value["probeRadius"])?;
    bounded_u64(&value["probeCount"], 4096)?;
    boolean(&value["monotonicObserved"])?;
    for item in array(&value["transitions"], 256)? {
        exact(item, &["lower", "upper", "lowerState", "upperState"])?;
        nonnegative(&item["lower"])?;
        nonnegative(&item["upper"])?;
        enumeration(&item["lowerState"], &["success", "rejected", "invalid"])?;
        enumeration(&item["upperState"], &["success", "rejected", "invalid"])?;
    }
    Ok(())
}

pub fn shape_signature(value: &Value) -> Result<(), String> {
    exact(
        value,
        &["rotationCenter", "faceSamples", "pointClassifications"],
    )?;
    vector(&value["rotationCenter"], 3)?;
    for item in array(&value["faceSamples"], 256)? {
        exact(item, &["point", "surfaceKind"])?;
        vector(&item["point"], 3)?;
        enumeration(
            &item["surfaceKind"],
            &["plane", "cylinder", "cone", "other"],
        )?;
    }
    for item in array(&value["pointClassifications"], 64)? {
        exact(item, &["point", "state"])?;
        vector(&item["point"], 3)?;
        enumeration(&item["state"], &["in", "out", "on", "unknown"])?;
    }
    Ok(())
}

pub fn artifacts<'a>(values: impl IntoIterator<Item = &'a Option<String>>) -> Result<(), String> {
    for path in values.into_iter().flatten() {
        let safe = !path.is_empty()
            && path.len() <= 240
            && !Path::new(path).is_absolute()
            && Path::new(path)
                .components()
                .all(|item| matches!(item, Component::Normal(_)));
        if !safe {
            return Err("unsafe artifact path".into());
        }
    }
    Ok(())
}

pub fn exact(value: &Value, fields: &[&str]) -> Result<(), String> {
    exact_allowed(value, fields, fields)
}

fn exact_allowed(value: &Value, allowed: &[&str], required: &[&str]) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| "required block is not object".to_string())?;
    let actual: BTreeSet<_> = object.keys().map(String::as_str).collect();
    if actual.iter().any(|key| !allowed.contains(key))
        || required.iter().any(|key| !actual.contains(key))
    {
        Err("unknown or missing result field".into())
    } else {
        Ok(())
    }
}

fn array(value: &Value, max: usize) -> Result<&Vec<Value>, String> {
    value
        .as_array()
        .filter(|items| items.len() <= max)
        .ok_or_else(|| "invalid bounded array".into())
}
fn booleans(value: &Value, fields: &[&str]) -> Result<(), String> {
    fields.iter().try_for_each(|key| boolean(&value[*key]))
}
pub fn boolean(value: &Value) -> Result<(), String> {
    value
        .is_boolean()
        .then_some(())
        .ok_or_else(|| "expected bool".into())
}
pub fn enumeration(value: &Value, choices: &[&str]) -> Result<(), String> {
    value
        .as_str()
        .filter(|item| choices.contains(item))
        .map(|_| ())
        .ok_or_else(|| "invalid enum".into())
}
fn bounded_u64(value: &Value, max: u64) -> Result<(), String> {
    value
        .as_u64()
        .filter(|item| *item <= max)
        .map(|_| ())
        .ok_or_else(|| "integer out of bounds".into())
}
fn finite(value: &Value) -> Result<(), String> {
    value
        .as_f64()
        .filter(|item| item.is_finite() && item.abs() <= 1e15)
        .map(|_| ())
        .ok_or_else(|| "non-finite number".into())
}
pub fn nonnegative(value: &Value) -> Result<(), String> {
    value
        .as_f64()
        .filter(|item| item.is_finite() && *item >= 0.0 && *item <= 1e15)
        .map(|_| ())
        .ok_or_else(|| "negative or non-finite number".into())
}
fn positive(value: &Value) -> Result<(), String> {
    value
        .as_f64()
        .filter(|item| item.is_finite() && *item > 0.0 && *item <= 1e15)
        .map(|_| ())
        .ok_or_else(|| "non-positive or non-finite number".into())
}
fn vector(value: &Value, size: usize) -> Result<(), String> {
    let items = value
        .as_array()
        .filter(|items| items.len() == size)
        .ok_or_else(|| "invalid vector".to_string())?;
    items.iter().try_for_each(finite)
}
fn string(value: &Value, max: usize) -> Result<(), String> {
    value
        .as_str()
        .filter(|item| !item.is_empty() && item.len() <= max)
        .map(|_| ())
        .ok_or_else(|| "invalid string".into())
}
fn safe_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric() || (index > 0 && b"._-".contains(&byte))
        })
}
