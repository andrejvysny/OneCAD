//! Pure rules of `ios-simulator-gate` (docs/ios/SIMULATOR_GATE.md, "Verifier
//! rejects"): the executed XCTest inventory, run identity, kernel identity and
//! the simulator artifacts. Every rule has a negative test in `ios_tests.rs`.
use super::{captures::hash, Result};
use regex::Regex;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

pub(super) const INVENTORY: &str = "platforms/ios/acceptance-inventory.json";
pub(super) const BUNDLE_ID: &str = "com.onecad.ipad";

/// The checked-in test inventory and the approved skips (with reasons).
#[derive(Debug)]
pub(super) struct Inventory {
    pub tests: BTreeSet<String>,
    pub approved_skips: BTreeMap<String, String>,
}

pub(super) fn inventory(text: &str) -> Result<Inventory> {
    let value: Value = serde_json::from_str(text)?;
    let mut tests = BTreeSet::new();
    for test in value["tests"].as_array().ok_or("inventory needs tests")? {
        let test = test.as_str().ok_or("inventory tests are strings")?;
        if test.split('/').count() != 3 || !tests.insert(test.to_string()) {
            return Err(format!("inventory entry malformed or duplicated: {test}").into());
        }
    }
    if tests.is_empty() {
        return Err("inventory is empty".into());
    }
    let mut approved_skips = BTreeMap::new();
    for skip in value["approvedSkips"]
        .as_array()
        .ok_or("inventory needs approvedSkips")?
    {
        let test = skip["test"].as_str().filter(|t| tests.contains(*t));
        let reason = skip["reason"].as_str().filter(|r| !r.trim().is_empty());
        match (test, reason) {
            (Some(test), Some(reason)) => approved_skips.insert(test.into(), reason.into()),
            _ => {
                return Err(
                    format!("approved skip needs an inventoried test and a reason: {skip}").into(),
                )
            }
        };
    }
    Ok(Inventory {
        tests,
        approved_skips,
    })
}

/// One executed test case: `<bundle>/<Suite>/<test()>`, its result and how
/// many times it ran.
#[derive(Debug, PartialEq)]
pub(super) struct Executed {
    pub id: String,
    pub result: String,
    pub runs: usize,
}

const RUN_NODES: [&str; 4] = ["Repetition", "Test Case Run", "Arguments", "Device"];

fn walk(node: &Value, bundle: Option<&str>, out: &mut Vec<Executed>) -> Result<()> {
    let kind = node["nodeType"].as_str().unwrap_or_default();
    let bundle = match kind {
        "Unit test bundle" | "UI test bundle" => node["name"].as_str(),
        _ => bundle,
    };
    let children = node["children"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    if kind == "Test Case" {
        let identifier = node["nodeIdentifier"]
            .as_str()
            .ok_or("test case without identifier")?;
        let bundle = bundle.ok_or("test case outside a test bundle")?;
        let runs = children
            .iter()
            .filter(|child| RUN_NODES.contains(&child["nodeType"].as_str().unwrap_or_default()))
            .count();
        out.push(Executed {
            id: format!("{bundle}/{identifier}"),
            result: node["result"].as_str().unwrap_or_default().to_string(),
            runs: runs.max(1),
        });
        return Ok(());
    }
    for child in children {
        walk(child, bundle, out)?;
    }
    Ok(())
}

/// Test cases of `xcresulttool get test-results tests` (Xcode 26 JSON).
pub(super) fn executed(tests: &Value) -> Result<Vec<Executed>> {
    let mut out = Vec::new();
    for node in tests["testNodes"]
        .as_array()
        .ok_or("xcresult tests without testNodes")?
    {
        walk(node, None, &mut out)?;
    }
    Ok(out)
}

/// Exact inventory, every test once, passed (or an approved skip), and a
/// summary that agrees on one simulator.
pub(super) fn verify_tests(
    tests: &Value,
    summary: &Value,
    inventory: &Inventory,
    udid: &str,
) -> Result<()> {
    let executed = executed(tests)?;
    let mut seen = BTreeSet::new();
    for test in &executed {
        if !seen.insert(test.id.as_str()) || test.runs != 1 {
            return Err(format!("test executed more than once: {}", test.id).into());
        }
        if !inventory.tests.contains(&test.id) {
            return Err(format!("test not in the inventory: {}", test.id).into());
        }
        let approved = inventory.approved_skips.contains_key(&test.id);
        match test.result.as_str() {
            "Passed" => {}
            "Skipped" if approved => {}
            "Skipped" => return Err(format!("unapproved skip: {}", test.id).into()),
            other => return Err(format!("test {} result {other}", test.id).into()),
        }
    }
    let missing: Vec<_> = inventory
        .tests
        .iter()
        .filter(|t| !seen.contains(t.as_str()))
        .collect();
    if !missing.is_empty() {
        return Err(format!("inventoried tests did not run: {missing:?}").into());
    }
    verify_summary(summary, inventory, &executed, udid)
}

fn verify_summary(
    summary: &Value,
    inventory: &Inventory,
    executed: &[Executed],
    udid: &str,
) -> Result<()> {
    let skipped = executed.iter().filter(|t| t.result == "Skipped").count() as u64;
    let devices = summary["devicesAndConfigurations"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let one_device = matches!(devices, [only] if only["device"]["deviceId"] == udid);
    if !one_device
        || summary["totalTestCount"] != inventory.tests.len() as u64
        || summary["failedTests"] != 0
        || summary["expectedFailures"] != 0
        || summary["skippedTests"] != skipped
        || summary["passedTests"] != inventory.tests.len() as u64 - skipped
    {
        return Err("xcresult summary disagrees with the inventory, simulator or results".into());
    }
    Ok(())
}

/// `<yyyymmddThhmmssZ>-<sha8>-<16 hex>`.
pub(super) fn verify_run_id(id: &str, source: &str) -> Result<()> {
    let pattern = Regex::new(r"^\d{8}T\d{6}Z-([0-9a-f]{8})-[0-9a-f]{16}$")?;
    let captures = pattern.captures(id).ok_or("malformed run id")?;
    if !source.starts_with(&captures[1]) {
        return Err("run id names another source revision".into());
    }
    Ok(())
}

pub(super) fn verify_clean_tree(porcelain: &str) -> Result<()> {
    if porcelain.trim().is_empty() {
        Ok(())
    } else {
        Err("source tree is dirty (git status --porcelain=v1 is not empty)".into())
    }
}

/// Identity manifest v2 of one bundle (`scripts/build-ios-worker.sh`).
pub(super) fn verify_bundle_identity(
    identity: &Value,
    platform: &str,
    test_hooks: bool,
) -> Result<()> {
    if identity["manifestVersion"] != 2
        || identity["kind"] != "embedded"
        || identity["platform"] != platform
        || identity["testHooks"] != test_hooks
        || identity["deploymentTarget"] != "18.0"
        || !identity["occt"]["fingerprint"].is_string()
    {
        return Err(format!(
            "{platform} kernel identity is not the expected manifest v2 (testHooks {test_hooks})"
        )
        .into());
    }
    Ok(())
}

/// A semantic artifact's kernel: the in-process backend running exactly the
/// simulator bundle identity built in step 3 (never a stub or fake backend).
pub(super) fn verify_kernel(kernel: &Value, simulator: &Value) -> Result<()> {
    if kernel["backend"] != "in-process" {
        return Err(format!("backend {} is not in-process", kernel["backend"]).into());
    }
    if &kernel["embedded"] != simulator || &kernel["linked"] != simulator {
        return Err(
            "embedded or linked identity differs from the simulator bundle built by the gate"
                .into(),
        );
    }
    let hello = &kernel["hello"];
    let pairs = [
        ("protocolVersion", "protocolVersion"),
        ("workerVersion", "workerVersion"),
        ("quantizationVersion", "quantizationVersion"),
        ("solverPolicyVersion", "solverPolicyVersion"),
    ];
    let axes_match = pairs.iter().all(|(h, s)| hello[h] == simulator[s])
        && hello["occt"]["version"] == simulator["occt"]["version"]
        && hello["occt"]["fingerprint"] == simulator["occt"]["fingerprint"];
    if hello.is_null() || !axes_match {
        return Err("worker hello does not match the embedded simulator identity".into());
    }
    Ok(())
}

fn recorded_run(value: &Value, run_id: &str, what: &str) -> Result<()> {
    match value["runId"].as_str() {
        Some(id) if id == run_id => Ok(()),
        Some(id) => Err(format!("{what} belongs to stale run {id}").into()),
        None => Err(format!("{what} carries no run id").into()),
    }
}

/// Copied `Documents/acceptance/<run-id>/`: every model and STEP file has a
/// provenance sidecar of this run whose hash matches, every semantic JSON is
/// this run's, every kernel is the simulator bundle, required documents exist.
pub(super) fn verify_artifacts(
    dir: &Path,
    run_id: &str,
    simulator: &Value,
    required: &[String],
) -> Result<BTreeSet<String>> {
    let names = fs::read_dir(dir)?
        .map(|entry| {
            Ok(entry?
                .file_name()
                .into_string()
                .map_err(|_| "non-UTF-8 artifact name")?)
        })
        .collect::<Result<BTreeSet<String>>>()?;
    for name in &names {
        let path = dir.join(name);
        if !path.is_file() {
            return Err(format!("artifact is not a regular file: {name}").into());
        }
        if let Some(artifact) = name.strip_suffix(".provenance.json") {
            let record: Value = serde_json::from_slice(&fs::read(&path)?)?;
            recorded_run(&record, run_id, name)?;
            verify_kernel(&record["kernelIdentity"], simulator)?;
            let bytes = fs::read(dir.join(artifact))
                .map_err(|_| format!("{name} names a missing artifact"))?;
            if record["artifact"] != artifact || record["sha256"] != hash(&bytes) {
                return Err(format!("{name} does not describe {artifact}").into());
            }
        } else if name.ends_with(".json") {
            let record: Value = serde_json::from_slice(&fs::read(&path)?)?;
            recorded_run(&record, run_id, name)?;
            verify_kernel(&record["kernelIdentity"], simulator)?;
        } else if !names.contains(&format!("{name}.provenance.json")) {
            return Err(format!("artifact without provenance: {name}").into());
        }
    }
    let missing: Vec<_> = required
        .iter()
        .filter(|r| !names.contains(&format!("{r}.onecad")))
        .collect();
    if !missing.is_empty() {
        return Err(format!("expected model artifacts missing: {missing:?}").into());
    }
    Ok(names)
}
