use super::{captures, enum_variants, finish, input, json, read, string, Result};
use regex::Regex;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const REQUIRED: &[&str] = &[
    "operation",
    "mode",
    "supportStatus",
    "uiExposure",
    "inputTopLevelShapes",
    "requiresExactlyOneSolid",
    "emptyResultSemantics",
    "multiSolidResultSemantics",
    "bodyLifecycle",
    "bodyIdPolicy",
    "authoritativeHistory",
    "historyAbsenceBehavior",
    "validationTier",
    "toleranceEvidence",
    "cancellationPolicy",
    "previewFidelity",
    "persistenceBehavior",
    "userRecoveryBehavior",
];
const TIERS: &[(&str, &str)] = &[
    ("Extrude", "ExtrudeOp.cpp"),
    ("Revolve", "RevolveOp.cpp"),
    ("Fillet", "FilletChamferOp.cpp"),
    ("Chamfer", "FilletChamferOp.cpp"),
    ("Shell", "ShellOp.cpp"),
    ("Boolean", "BooleanOp.cpp"),
    ("LinearPattern", "PatternOp.cpp"),
    ("CircularPattern", "PatternOp.cpp"),
    ("MirrorBody", "MirrorOp.cpp"),
    ("TransformBody", "TransformOp.cpp"),
    ("Hole", "HoleOp.cpp"),
    ("OffsetFace", "OffsetFaceOp.cpp"),
    ("Gear", "GearOp.cpp"),
    ("PlaceComponent", "ComponentOp.cpp"),
    ("DetachComponent", "ComponentOp.cpp"),
];

pub(super) fn validate(root: &Path, contracts: &Value, coverage: &Value) -> Result<usize> {
    let rows = contracts["rows"]
        .as_array()
        .ok_or("expected version 1 rows array")?;
    if contracts["version"] != 1 {
        return Err("expected version 1 rows array".into());
    }
    let coverage_rows = coverage["rows"]
        .as_array()
        .ok_or("coverage manifest needs rows array")?;
    let coverage_ops: BTreeSet<_> = coverage_rows
        .iter()
        .map(|r| string(r, "operation"))
        .collect();
    let mut errors = Vec::new();
    let mut keys = BTreeSet::new();
    let mut operations = BTreeSet::new();
    for (index, row) in rows.iter().enumerate() {
        check_row(index, row, &mut errors);
        let operation = string(row, "operation");
        operations.insert(operation);
        let key = format!("{operation}::{}", string(row, "mode"));
        if !keys.insert(key.clone()) {
            errors.push(format!("row {index} duplicates {key}"));
        }
    }
    for operation in &operations {
        if !coverage_ops.contains(operation) {
            errors.push(format!(
                "contract operation {operation} is missing from coverage manifest"
            ));
        }
    }
    check_tiers(root, rows, &mut errors)?;
    check_dispatch(root, rows, &mut errors)?;
    finish("modeling contracts", errors)?;
    Ok(operations.len())
}

fn check_row(index: usize, row: &Value, errors: &mut Vec<String>) {
    for field in REQUIRED {
        if row.get(*field).is_none() {
            errors.push(format!("row {index} missing {field}"));
        }
    }
    if !row["operation"].is_string() || !row["mode"].is_string() {
        errors.push(format!("row {index} needs operation and mode strings"));
    }
    let status = string(row, "supportStatus");
    let exposure = string(row, "uiExposure");
    if !["supported", "deferred", "unsupported"].contains(&status) {
        errors.push(format!("row {index} has invalid supportStatus {status}"));
    }
    if !["exposed", "hidden", "unsupported"].contains(&exposure) {
        errors.push(format!("row {index} has invalid uiExposure {exposure}"));
    }
    if status == "unsupported" && exposure != "unsupported" {
        errors.push(format!(
            "row {index} unsupported operation must declare uiExposure unsupported"
        ));
    }
    for field in ["authoritativeHistory", "historyAbsenceBehavior"] {
        let value = string(row, field).to_lowercase();
        if value.contains("deferred") && !value.contains("owner") && !value.contains("dependency") {
            errors.push(format!(
                "row {index} {field} is deferred but missing owner/dependency"
            ));
        }
    }
}

fn check_tiers(root: &Path, rows: &[Value], errors: &mut Vec<String>) -> Result<()> {
    let leading = Regex::new(r"\bTier (A|B)\b")?;
    let mut cache = BTreeMap::new();
    for row in rows {
        let Some((_, file)) = TIERS
            .iter()
            .find(|(operation, _)| *operation == string(row, "operation"))
        else {
            continue;
        };
        let path = format!("worker/src/ops/{file}");
        if !cache.contains_key(&path) {
            let tiers = match read(root, &path) {
                Err(_) => {
                    errors.push(format!("validationTier: {path} does not exist — the source moved and this check went vacuous"));
                    None
                }
                Ok(source) => {
                    let a = source.contains("PublicationTier::TierA");
                    let b = source.contains("PublicationTier::TierB");
                    if !a && !b {
                        errors.push(format!("validationTier: {path} names no PublicationTier at all — this check went vacuous"));
                        None
                    } else {
                        Some((a, b))
                    }
                }
            };
            cache.insert(path.clone(), tiers);
        }
        let Some((a, b)) = cache[&path] else {
            continue;
        };
        let claim = leading.captures(string(row, "validationTier"));
        let tier = claim.as_ref().map(|m| &m[1]);
        let where_ = format!("{}::{}", string(row, "operation"), string(row, "mode"));
        if tier == Some("A") && b && !a {
            errors.push(format!(
                "{where_} validationTier claims Tier A, but {path} only ever requests Tier B"
            ));
        }
        if tier == Some("B") && !b {
            errors.push(format!(
                "{where_} validationTier claims Tier B, but {path} never requests it"
            ));
        }
    }
    Ok(())
}

fn check_dispatch(root: &Path, rows: &[Value], errors: &mut Vec<String>) -> Result<()> {
    let record = root.join("crates/onecad-core/src/document/record.rs");
    let known = if record.exists() {
        enum_variants(&std::fs::read_to_string(record)?, "KnownOperation", r"\(")?
    } else {
        errors.push("supportStatus: record.rs does not exist — this check went vacuous".to_owned());
        BTreeSet::new()
    };
    if known.is_empty() {
        errors.push(
            "supportStatus: scanned zero KnownOperation variants — this check went vacuous"
                .to_owned(),
        );
    }
    let plan = root.join("worker/src/session/PlanExecutor.cpp");
    if !plan.exists() {
        errors.push(
            "supportStatus: PlanExecutor.cpp does not exist — this check went vacuous".to_owned(),
        );
        return Ok(());
    }
    let dispatch = captures(
        &std::fs::read_to_string(plan)?,
        r#"op_type == "([A-Za-z]+)""#,
    )?;
    if dispatch.is_empty() {
        errors
            .push("supportStatus: scanned zero dispatch arms — this check went vacuous".to_owned());
        return Ok(());
    }
    for row in rows {
        let operation = string(row, "operation");
        let where_ = format!("{operation}::{}", string(row, "mode"));
        if string(row, "supportStatus") == "unsupported" && dispatch.contains(operation) {
            errors.push(format!(
                "{where_} is marked unsupported but the worker dispatches {operation}"
            ));
        }
        if string(row, "supportStatus") == "supported"
            && !dispatch.contains(operation)
            && known.contains(operation)
        {
            errors.push(format!(
                "{where_} is marked supported but the worker has no {operation} dispatch arm"
            ));
        }
    }
    Ok(())
}

pub(super) fn run(root: &Path) -> Result<()> {
    let contracts = json(&input(
        root,
        "MODELING_CONTRACTS_PATH",
        "docs/qa/modeling-operation-contracts.json",
    ))?;
    let coverage = json(&input(
        root,
        "MODELING_COVERAGE_MANIFEST",
        "docs/qa/modeling-operation-coverage.json",
    ))?;
    let operations = validate(root, &contracts, &coverage)?;
    println!(
        "modeling contracts: {} rows, {operations} operations, {} tier-checked operations",
        contracts["rows"].as_array().map_or(0, Vec::len),
        TIERS.len()
    );
    Ok(())
}
