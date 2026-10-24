use super::{captures, enum_variants, finish, input, json, list, read, string, Result};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const REQUIRED: &[&str] = &[
    "operation",
    "mode",
    "supportStatus",
    "uiExposure",
    "cppTest",
    "rustRealWorkerTest",
    "frontendTest",
    "playwrightTest",
    "corpusCase",
    "kernelbenchSuite",
    "ciJob",
    "notes",
];
const LANES: &[(&str, &[&str])] = &[
    ("cppTest", &["worker-8-0-1", "linux-worker", "s3-worker"]),
    ("rustRealWorkerTest", &["rust-8-0-1"]),
    ("frontendTest", &["rust-8-0-1", "native-host-lifecycle"]),
    ("playwrightTest", &["e2e-chromium", "e2e-webkit"]),
    ("kernelbenchSuite", &["linux-kernelbench", "s4-kernelbench"]),
];

fn workflow_jobs(source: &str) -> Result<BTreeSet<String>> {
    let lines: Vec<_> = source.lines().collect();
    let Some(start) = lines.iter().position(|line| *line == "jobs:") else {
        return Ok(BTreeSet::new());
    };
    let block = lines[start + 1..]
        .iter()
        .take_while(|line| !line.starts_with(|c: char| c.is_ascii_alphabetic()))
        .copied()
        .collect::<Vec<_>>()
        .join("\n");
    captures(&block, r"(?m)^ {2}([A-Za-z0-9_-]+):\s*$")
}

pub(super) fn validate(root: &Path, manifest: &Value) -> Result<(usize, usize, usize)> {
    let rows = manifest["rows"]
        .as_array()
        .ok_or("expected version 1 rows array and corpusCases object")?;
    let classifications = manifest["corpusCases"]
        .as_object()
        .ok_or("expected version 1 rows array and corpusCases object")?;
    if manifest["version"] != 1 {
        return Err("expected version 1 rows array and corpusCases object".into());
    }
    let non_operation = manifest["nonOperationRows"].as_array().ok_or(
        "expected a nonOperationRows array (rows that are deliberately not KnownOperations)",
    )?;
    let mut errors = Vec::new();
    let mut jobs = BTreeSet::new();
    for workflow in ["ci.yml", "self-hosted.yml", "native-renderer.yml"] {
        jobs.extend(workflow_jobs(&read(
            root,
            &format!(".github/workflows/{workflow}"),
        )?)?);
    }
    if jobs.is_empty() {
        errors.push(
            "no CI jobs parsed from .github/workflows — refusing to validate vacuously".to_owned(),
        );
    }
    let mut corpus = BTreeSet::new();
    for entry in std::fs::read_dir(root.join("corpus/cases"))? {
        let path = entry?.path();
        if path.extension().is_some_and(|v| v == "json") {
            let case = json(&path)?;
            corpus.insert(
                case["id"]
                    .as_str()
                    .ok_or("corpus case needs string id")?
                    .to_owned(),
            );
        }
    }
    let mut covered = BTreeMap::<String, usize>::new();
    let mut operations = BTreeSet::new();
    let mut tuples = BTreeSet::new();
    for (index, row) in rows.iter().enumerate() {
        check_row(root, index, row, &jobs, &mut errors);
        let operation = string(row, "operation");
        operations.insert(operation.to_owned());
        let key = format!("{operation}/{}", string(row, "mode"));
        if !tuples.insert(key.clone()) {
            errors.push(format!("row {index} duplicates {key}"));
        }
        let case = string(row, "corpusCase");
        if !case.is_empty() {
            if !corpus.contains(case) {
                errors.push(format!("row {index} names unknown corpus case {case}"));
            }
            *covered.entry(case.to_owned()).or_default() += 1;
        }
    }
    check_corpus(&corpus, &covered, classifications, &mut errors);
    let registry = check_registries(root, rows, &operations, non_operation, &mut errors)?;
    finish("modeling coverage", errors)?;
    Ok((corpus.len(), jobs.len(), registry))
}

fn check_row(
    root: &Path,
    index: usize,
    row: &Value,
    jobs: &BTreeSet<String>,
    errors: &mut Vec<String>,
) {
    for field in REQUIRED {
        if !row[*field].is_string() {
            errors.push(format!("row {index} missing string {field}"));
        }
    }
    let status = string(row, "supportStatus");
    let exposure = string(row, "uiExposure");
    if !["supported", "deferred", "unsupported"].contains(&status) {
        errors.push(format!("row {index} has invalid supportStatus"));
    }
    if !["exposed", "hidden", "mixed", "unsupported"].contains(&exposure) {
        errors.push(format!("row {index} has invalid uiExposure"));
    }
    let where_ = format!("{}/{}", string(row, "operation"), string(row, "mode"));
    if string(row, "operation").is_empty() || string(row, "mode").is_empty() {
        errors.push(format!("row {index} needs operation and mode"));
    }
    if status == "unsupported" && exposure != "unsupported" {
        errors.push(format!(
            "{where_} unsupported capability must declare uiExposure unsupported"
        ));
    }
    if status == "supported" && ["exposed", "mixed"].contains(&exposure) {
        if list(string(row, "rustRealWorkerTest")).is_empty() {
            errors.push(format!("{where_} lacks a real-worker lane"));
        }
        if list(string(row, "playwrightTest")).is_empty()
            && list(string(row, "frontendTest")).is_empty()
        {
            errors.push(format!("{where_} lacks a native UI or browser lane"));
        }
    }
    if exposure == "hidden" && !list(string(row, "playwrightTest")).is_empty() {
        errors.push(format!(
            "{where_} is UI-hidden but cites a browser exposure lane"
        ));
    }
    for field in [
        "cppTest",
        "rustRealWorkerTest",
        "frontendTest",
        "playwrightTest",
    ] {
        for path in list(string(row, field)) {
            if !root.join(path).exists() {
                errors.push(format!("{where_} {field} names a missing file {path}"));
            }
        }
    }
    check_native_ui_paths(row, &where_, errors);
    let named = list(string(row, "ciJob"));
    for job in &named {
        if !jobs.contains(*job) {
            errors.push(format!(
                "{where_} names ciJob {job}, which is not a job in any workflow"
            ));
        }
    }
    for (lane, owners) in LANES {
        if !list(string(row, lane)).is_empty() && !named.iter().any(|job| owners.contains(job)) {
            errors.push(format!(
                "{where_} cites {lane} but names no job that runs it (expected one of {})",
                owners.join(", ")
            ));
        }
    }
}

fn check_native_ui_paths(row: &Value, where_: &str, errors: &mut Vec<String>) {
    // Native host unit tests run in both the full workspace and native-host jobs.
    // Backend tests cannot substitute for an exposed command's UI evidence.
    for path in list(string(row, "frontendTest")) {
        let native = Path::new(path)
            .strip_prefix("crates/onecad-ui/src")
            .is_ok_and(|relative| {
                relative
                    .extension()
                    .is_some_and(|extension| extension == "rs")
                    && relative
                        .components()
                        .all(|component| matches!(component, std::path::Component::Normal(_)))
            });
        if !native {
            errors.push(format!(
                "{where_} frontendTest must name native host Rust tests under crates/onecad-ui/src: {path}"
            ));
        }
    }
}

fn check_corpus(
    corpus: &BTreeSet<String>,
    covered: &BTreeMap<String, usize>,
    classifications: &serde_json::Map<String, Value>,
    errors: &mut Vec<String>,
) {
    for id in corpus {
        if covered.get(id) != Some(&1) {
            errors.push(format!("corpus case {id} needs exactly one manifest row"));
        }
        if !["operation", "solver", "antiGoal", "unsupported"].contains(
            &classifications
                .get(id)
                .and_then(Value::as_str)
                .unwrap_or(""),
        ) {
            errors.push(format!("corpus case {id} needs an explicit classification"));
        }
    }
    for id in classifications.keys() {
        if !corpus.contains(id) {
            errors.push(format!("classification names unknown corpus case {id}"));
        }
    }
}

fn cross_check(
    label: &str,
    names: &BTreeSet<String>,
    operations: &BTreeSet<String>,
    errors: &mut Vec<String>,
) {
    if names.is_empty() {
        errors.push(format!(
            "{label}: scanned zero entries — the source moved and this check went vacuous"
        ));
    }
    for name in names {
        if !operations.contains(name) {
            errors.push(format!("{label} knows {name}, which has no manifest row"));
        }
    }
}

fn check_registries(
    root: &Path,
    rows: &[Value],
    operations: &BTreeSet<String>,
    non_operation: &[Value],
    errors: &mut Vec<String>,
) -> Result<usize> {
    let mut registry = enum_variants(
        &read(root, "crates/onecad-core/src/document/record.rs")?,
        "KnownOperation",
        r"\(",
    )?;
    cross_check("KnownOperation", &registry, operations, errors);
    let dispatch = captures(
        &read(root, "worker/src/session/PlanExecutor.cpp")?,
        r#"op_type == "([A-Za-z]+)""#,
    )?;
    cross_check("worker dispatch", &dispatch, operations, errors);
    registry.extend(dispatch);
    let schema = captures(
        &read(root, "protocol/SCHEMA.md")?,
        r"(?m)^\*\*([A-Za-z]+)\*\* \(`op\.[A-Za-z]+`\)",
    )?;
    cross_check("SCHEMA §7.3 op catalogue", &schema, operations, errors);
    registry.extend(schema);
    let families = enum_variants(
        &read(root, "crates/onecad-kernelbench/src/case_v2.rs")?,
        "OperationFamily",
        ",",
    )?;
    cross_check("kernelbench families", &families, operations, errors);
    for family in families {
        if !rows.iter().any(|r| {
            string(r, "operation") == family && !list(string(r, "kernelbenchSuite")).is_empty()
        }) {
            errors.push(format!(
                "kernelbench family {family} has manifest rows but none names a kernelbenchSuite"
            ));
        }
    }
    let mut declared = BTreeSet::new();
    for (index, value) in non_operation.iter().enumerate() {
        if let Some(name) = value.as_str() {
            declared.insert(name);
        } else {
            errors.push(format!("nonOperationRows entry {index} must be a string"));
        }
    }
    for operation in operations {
        if !registry.contains(operation) && !declared.contains(operation.as_str()) {
            errors.push(format!("row operation {operation} is in no registry and is not declared in nonOperationRows"));
        }
    }
    for name in declared {
        if !operations.contains(name) {
            errors.push(format!(
                "nonOperationRows declares {name}, which has no row"
            ));
        }
    }
    Ok(registry.len())
}

pub(super) fn run(root: &Path) -> Result<()> {
    let manifest = json(&input(
        root,
        "MODELING_COVERAGE_MANIFEST",
        "docs/qa/modeling-operation-coverage.json",
    ))?;
    let (corpus, jobs, registry) = validate(root, &manifest)?;
    println!("modeling coverage: {} rows, {corpus} corpus cases, {jobs} CI jobs, {registry} registry operations", manifest["rows"].as_array().map_or(0, Vec::len));
    Ok(())
}
