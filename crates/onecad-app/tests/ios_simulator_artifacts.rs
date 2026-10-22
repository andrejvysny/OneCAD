//! Independent Mac geometry oracle for the iPad simulator gate
//! (docs/ios/SIMULATOR_GATE.md step 8). Ignored: `cargo xtask
//! ios-simulator-gate` runs it with the real sidecar.
//!
//! * `ONECAD_IOS_MAC_FIXTURE_OUT=<dir>` authors `mac-plate.onecad` there (the
//!   file the simulator round-trip test opens and re-saves) and checks it.
//! * `ONECAD_IOS_ACCEPTANCE_DIR=<dir>` opens every simulator-authored
//!   `.onecad` there, regenerates it on macOS and checks volume, bounds, solid
//!   count, feature history and sketch constraint status against
//!   `fixtures/ios/acceptance/expectations.json`; required artifacts must exist
//!   and every file must have an expectation. A case with `roundTripOf` is
//!   also compared with the authored file named by `ONECAD_IOS_MAC_FIXTURE`:
//!   same document, feature records and sketch, different bytes.
//! * `ONECAD_IOS_ORACLE_REPORT=<file>` receives the measurements for the
//!   gate's own re-check. It is written whenever every document could be
//!   opened and measured; a document that cannot fails the test without one.
//!
//! Requires `ONECAD_REQUIRE_WORKER=1` and `ONECAD_WORKER_PATH`.
mod ios_acceptance_support;

use ios_acceptance_support::{
    expect::{check, same_document, Expectations, Measurement},
    host::{measure_file, required_worker},
    plate, sha256,
};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const MAC_FIXTURE_OUT: &str = "ONECAD_IOS_MAC_FIXTURE_OUT";
const ACCEPTANCE_DIR: &str = "ONECAD_IOS_ACCEPTANCE_DIR";
const MAC_FIXTURE_FILE: &str = "ONECAD_IOS_MAC_FIXTURE";
const REPORT: &str = "ONECAD_IOS_ORACLE_REPORT";
const MAC_FIXTURE: &str = "mac-plate";

#[test]
#[ignore = "iPad simulator gate step 8: needs ONECAD_REQUIRE_WORKER=1 and the Mac sidecar"]
fn ios_simulator_artifacts() {
    let worker = required_worker();
    let fixture_out = std::env::var_os(MAC_FIXTURE_OUT).map(PathBuf::from);
    let acceptance = std::env::var_os(ACCEPTANCE_DIR).map(PathBuf::from);
    assert!(
        fixture_out.is_some() || acceptance.is_some(),
        "set {MAC_FIXTURE_OUT} and/or {ACCEPTANCE_DIR}"
    );
    let report_path = PathBuf::from(std::env::var_os(REPORT).expect("ONECAD_IOS_ORACLE_REPORT"));
    let scratch = tempfile::Builder::new()
        .prefix("onecad-ios-oracle-")
        .tempdir()
        .unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let mut report = runtime.block_on(oracle(
        fixture_out.as_deref(),
        acceptance.as_deref(),
        scratch.path(),
    ));
    report["workerSha256"] = json!(sha256(&std::fs::read(worker).unwrap()));
    std::fs::write(&report_path, serde_json::to_string_pretty(&report).unwrap()).unwrap();
    assert_eq!(
        report["passed"],
        json!(true),
        "oracle failures: {}",
        report_path.display()
    );
}

async fn oracle(fixture_out: Option<&Path>, acceptance: Option<&Path>, scratch: &Path) -> Value {
    let (expectations, expectations_sha) = Expectations::load();
    let mac = match fixture_out {
        Some(dir) => Some(author_fixture(dir, scratch, &expectations).await),
        None => None,
    };
    let simulator = match acceptance {
        Some(dir) => Some(verify_directory(dir, scratch, &expectations).await),
        None => None,
    };
    let passed = [&mac, &simulator]
        .into_iter()
        .flatten()
        .all(|part| part["passed"] == json!(true));
    json!({
        "oracle": "crates/onecad-app/tests/ios_simulator_artifacts.rs",
        "expectationsSha256": expectations_sha,
        "macFixture": mac,
        "simulator": simulator,
        "passed": passed,
    })
}

/// One document measured on macOS and checked against its case.
struct Checked {
    name: String,
    file: PathBuf,
    measurement: Measurement,
    failures: Vec<String>,
}

impl Checked {
    async fn new(name: &str, file: &Path, scratch: &Path, expectations: &Expectations) -> Self {
        let case = expectations.case(name).expect("caller resolved the case");
        let measurement = measure_file(file, &scratch.join(name)).await;
        let failures = check(case, &measurement, expectations);
        Self {
            name: name.to_string(),
            file: file.to_path_buf(),
            measurement,
            failures,
        }
    }

    fn report(&self, round_trip: Option<Value>) -> Value {
        json!({
            "name": self.name,
            "file": self.file.file_name().map(|n| n.to_string_lossy().into_owned()),
            "sha256": sha256(&std::fs::read(&self.file).unwrap()),
            "measurement": self.measurement,
            "roundTrip": round_trip,
            "failures": self.failures,
        })
    }
}

async fn author_fixture(dir: &Path, scratch: &Path, expectations: &Expectations) -> Value {
    std::fs::create_dir_all(dir).unwrap();
    let target = dir.join(format!("{MAC_FIXTURE}.onecad"));
    plate::author(&target, &scratch.join("author")).await;
    let checked = Checked::new(MAC_FIXTURE, &target, scratch, expectations).await;
    let passed = checked.failures.is_empty();
    json!({ "path": target, "artifact": checked.report(None), "passed": passed })
}

/// Compares a re-save with the authored file it must be (`roundTripOf`).
async fn round_trip(resaved: &mut Checked, scratch: &Path) -> Value {
    let Some(fixture) = std::env::var_os(MAC_FIXTURE_FILE).map(PathBuf::from) else {
        resaved
            .failures
            .push(format!("{MAC_FIXTURE_FILE} must name the authored fixture"));
        return Value::Null;
    };
    let scratch = scratch.join(format!("{}-original", resaved.name));
    let original = measure_file(&fixture, &scratch).await;
    resaved
        .failures
        .extend(same_document(&original, &resaved.measurement));
    let fixture_sha = sha256(&std::fs::read(&fixture).unwrap());
    if fixture_sha == sha256(&std::fs::read(&resaved.file).unwrap()) {
        resaved
            .failures
            .push("the re-save is a byte copy of the authored file, not a save".into());
    }
    json!({ "fixtureSha256": fixture_sha, "original": original })
}

async fn verify_directory(dir: &Path, scratch: &Path, expectations: &Expectations) -> Value {
    let mut documents: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "onecad"))
        .collect();
    documents.sort();
    let mut artifacts = Vec::new();
    let mut unexpected = Vec::new();
    for document in &documents {
        let name = document.file_stem().unwrap().to_string_lossy().into_owned();
        let Some(case) = expectations
            .case(&name)
            .filter(|case| case.source == "simulator")
        else {
            unexpected.push(name);
            continue;
        };
        let mut checked = Checked::new(&name, document, scratch, expectations).await;
        let trip = match case.round_trip_of {
            Some(_) => Some(round_trip(&mut checked, scratch).await),
            None => None,
        };
        artifacts.push(checked.report(trip));
    }
    let missing = missing(expectations, &artifacts);
    let passed = missing.is_empty()
        && unexpected.is_empty()
        && artifacts
            .iter()
            .all(|artifact| artifact["failures"].as_array().is_some_and(Vec::is_empty));
    json!({
        "directory": dir,
        "artifacts": artifacts,
        "missing": missing,
        "unexpected": unexpected,
        "passed": passed,
    })
}

fn missing(expectations: &Expectations, artifacts: &[Value]) -> Vec<String> {
    expectations
        .artifacts
        .iter()
        .filter(|case| case.required && case.source == "simulator")
        .filter(|case| !artifacts.iter().any(|a| a["name"] == json!(case.name)))
        .map(|case| case.name.clone())
        .collect()
}
