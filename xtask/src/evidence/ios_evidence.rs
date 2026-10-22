//! Verification of a complete `ios-simulator-gate` evidence directory from the
//! files the run left there, so `--verify-only` re-checks a downloaded or
//! archived run with exactly the rules the live run applied.
use super::{
    captures::hash,
    ios_geometry::{self, EXPECTATIONS},
    ios_release::{self, Platform},
    ios_verify::{self, INVENTORY},
    Result,
};
use regex::Regex;
use serde_json::Value;
use std::{fs, path::Path};

/// Embedded kernel bundles built in step 3: (evidence file, platform, test hooks).
pub(super) const BUNDLES: [(&str, &str, &str, bool); 3] = [
    (
        "iphonesimulator-Debug-test-hooks",
        "iphonesimulator",
        "Debug",
        true,
    ),
    ("iphoneos-Release", "iphoneos", "Release", false),
    (
        "iphonesimulator-Release",
        "iphonesimulator",
        "Release",
        false,
    ),
];
pub(super) const SIMULATOR_BUNDLE: &str = "iphonesimulator-Debug-test-hooks";
pub(super) const MAC_FIXTURE: &str = "mac-authored/mac-plate.onecad";

pub(super) fn json(path: &Path) -> Result<Value> {
    let bytes = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))?)
}

fn text(path: &Path) -> Result<String> {
    Ok(fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?)
}

/// The ignored oracle test ran alone and passed. Under `--nocapture` its own
/// output may sit between `test ... ` and `ok`, so the decisive record is the
/// single summary line (as `native-host-gate` tolerates, host.rs).
pub(super) fn verify_oracle_log(log: &str) -> Result<()> {
    let record = Regex::new(r"(?m)^test ios_simulator_artifacts \.\.\. ")?;
    let passed = Regex::new(r"(?m)(?:\.\.\. |^)ok$")?;
    let summary = Regex::new(r"(?m)^test result: ok\. 1 passed; 0 failed; 0 ignored;")?;
    let any_summary = Regex::new(r"(?m)^test result: ")?;
    if record.find_iter(log).count() != 1
        || passed.find_iter(log).count() != 1
        || summary.find_iter(log).count() != 1
        || any_summary.find_iter(log).count() != 1
    {
        return Err("geometry oracle did not run exactly once and pass".into());
    }
    Ok(())
}

/// Every rule of docs/ios/SIMULATOR_GATE.md over `dir`; the failures found.
/// `root` is the repository whose inventory and expectations apply.
pub(super) fn verify(dir: &Path, root: &Path) -> Vec<String> {
    let mut failures = Vec::new();
    let mut note = |step: &str, result: Result<()>| {
        if let Err(error) = result {
            failures.push(format!("{step}: {error}"));
        }
    };
    note("provenance", provenance(dir));
    let run = json(&dir.join("run.json"));
    let simulator = json(&dir.join("kernel").join(format!("{SIMULATOR_BUNDLE}.json")));
    note("kernel bundles", bundles(dir));
    let (run, simulator) = match (run, simulator) {
        (Ok(run), Ok(simulator)) => (run, simulator),
        (run, simulator) => {
            note("run identity", run.map(drop));
            note("kernel identity", simulator.map(drop));
            return failures;
        }
    };
    let run_id = run["runId"].as_str().unwrap_or_default().to_string();
    note("tests", tests(dir, root, &run));
    note("artifacts", artifacts(dir, root, &run_id, &simulator));
    note("oracle", oracle(dir, root, &run));
    for platform in [Platform::Device, Platform::Simulator] {
        note(
            &format!("release {}", platform.sdk()),
            release(dir, platform),
        );
    }
    failures
}

fn provenance(dir: &Path) -> Result<()> {
    let source = text(&dir.join("source.log"))?;
    let source = source.trim();
    if !Regex::new(r"^[0-9a-f]{40}$")?.is_match(source) {
        return Err("source.log is not a commit hash".into());
    }
    let run = json(&dir.join("run.json"))?;
    ios_verify::verify_run_id(run["runId"].as_str().ok_or("run.json runId")?, source)?;
    if run["source"] != source {
        return Err("run.json names another source revision".into());
    }
    ios_verify::verify_clean_tree(&text(&dir.join("source-status.log"))?)
}

fn bundles(dir: &Path) -> Result<()> {
    for (name, platform, _, hooks) in BUNDLES {
        let identity = json(&dir.join("kernel").join(format!("{name}.json")))?;
        ios_verify::verify_bundle_identity(&identity, platform, hooks)?;
    }
    Ok(())
}

fn tests(dir: &Path, root: &Path, run: &Value) -> Result<()> {
    let inventory = ios_verify::inventory(&text(&root.join(INVENTORY))?)?;
    let udid = run["udid"].as_str().ok_or("run.json udid")?;
    ios_verify::verify_tests(
        &json(&dir.join("xcresult-tests.json"))?,
        &json(&dir.join("xcresult-summary.json"))?,
        &inventory,
        udid,
    )
}

fn artifacts(dir: &Path, root: &Path, run_id: &str, simulator: &Value) -> Result<()> {
    let expectations = json(&root.join(EXPECTATIONS))?;
    let required = ios_geometry::required_simulator(&expectations);
    ios_verify::verify_artifacts(&dir.join("artifacts"), run_id, simulator, &required).map(drop)
}

fn oracle(dir: &Path, root: &Path, run: &Value) -> Result<()> {
    let expectations_bytes = fs::read(root.join(EXPECTATIONS))?;
    let expectations: Value = serde_json::from_slice(&expectations_bytes)?;
    let expectations_sha = hash(&expectations_bytes);
    let worker_sha = run["workerSha256"]
        .as_str()
        .ok_or("run.json workerSha256")?;
    verify_oracle_log(&text(&dir.join("oracle-author.log"))?)?;
    let author = json(&dir.join("oracle-author.json"))?;
    ios_geometry::verify_report_provenance(&author, &expectations_sha, worker_sha)?;
    ios_geometry::verify_author_report(&author, &expectations, &dir.join(MAC_FIXTURE))?;
    verify_oracle_log(&text(&dir.join("oracle-verify.log"))?)?;
    let verified = json(&dir.join("oracle-verify.json"))?;
    ios_geometry::verify_report_provenance(&verified, &expectations_sha, worker_sha)?;
    ios_geometry::verify_simulator_report(&verified, &author, &expectations, &dir.join("artifacts"))
}

pub(super) fn release_product(dir: &Path, platform: Platform) -> std::path::PathBuf {
    dir.join("DerivedData-release/Build/Products")
        .join(format!("Release-{}", platform.sdk()))
        .join("OneCAD.app/OneCAD")
}

fn release(dir: &Path, platform: Platform) -> Result<()> {
    let logs = dir.join("release").join(platform.sdk());
    ios_release::verify_symbols(&text(&logs.join("nm.log"))?)?;
    ios_release::verify_strings(&fs::read(release_product(dir, platform))?)?;
    ios_release::verify_build_version(&text(&logs.join("otool-load-commands.log"))?, platform)?;
    ios_release::verify_libraries(&text(&logs.join("otool-libraries.log"))?)
}
