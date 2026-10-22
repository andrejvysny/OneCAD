use super::{
    captures::hash,
    host_names::{HOST_TESTS, SERVICE_TESTS},
    process, Result,
};
use regex::Regex;
use serde_json::{json, Value};
use std::{collections::BTreeMap, fs, path::Path};

/// One explicitly filtered `--ignored` cargo invocation with its own exact inventory.
pub(super) struct Selection {
    inventory_log: &'static str,
    run_log: &'static str,
    filters: &'static [&'static str],
    pub(super) tests: &'static [&'static str],
    metal_evidence: bool,
}
pub(super) const LIFECYCLE: Selection = Selection {
    inventory_log: "inventory",
    run_log: "lifecycle",
    filters: &["app_logic_tests::"],
    tests: HOST_TESTS,
    metal_evidence: true,
};
pub(super) const SERVICES: Selection = Selection {
    inventory_log: "service-inventory",
    run_log: "service-lifecycle",
    filters: &[
        "document_services::tests::home_lifecycle_tests::",
        "bridge::tests::",
    ],
    tests: SERVICE_TESTS,
    metal_evidence: false,
};

fn exact(names: &[&str], expected: &[&str]) -> Result<()> {
    let mut names = names.to_vec();
    let mut expected = expected.to_vec();
    names.sort_unstable();
    expected.sort_unstable();
    if names != expected {
        return Err(
            "required native host test inventory changed, duplicated, or incomplete".into(),
        );
    }
    Ok(())
}
pub(super) fn inventory(output: &str, selection: &Selection) -> Result<()> {
    let names: Vec<_> = output
        .lines()
        .filter_map(|line| line.strip_suffix(": test"))
        .collect();
    exact(&names, selection.tests)
}
fn records(output: &str) -> Result<Vec<(&str, &str)>> {
    let starts = Regex::new(r"(?m)^test (\S+) \.\.\. ")?;
    let matches: Vec<_> = starts.captures_iter(output).collect();
    Ok(matches
        .iter()
        .enumerate()
        .map(|(index, start)| {
            let offset = start.get(0).unwrap().start();
            let mut end = matches
                .get(index + 1)
                .map_or(output.len(), |next| next.get(0).unwrap().start());
            for marker in ["\ntest result:", "\nfailures:"] {
                if let Some(found) = output[offset..].find(marker) {
                    end = end.min(offset + found);
                }
            }
            (start.get(1).unwrap().as_str(), output[offset..end].trim())
        })
        .collect())
}
pub(super) fn verify(output: &str, selection: &Selection) -> Result<()> {
    let tests = selection.tests;
    let records = records(output)?;
    exact(
        &records.iter().map(|record| record.0).collect::<Vec<_>>(),
        tests,
    )?;
    let passed = Regex::new(r"(?:\.\.\. |\n)ok$")?;
    for (name, text) in records {
        if !passed.is_match(text) {
            return Err(format!("required test did not pass: {name}").into());
        }
    }
    let summary =
        Regex::new(r"(?m)^test result: (\w+)\. (\d+) passed; (\d+) failed; (\d+) ignored;")?;
    let summaries: Vec<_> = summary.captures_iter(output).collect();
    if summaries.len() != 1
        || &summaries[0][1] != "ok"
        || summaries[0][2].parse::<usize>()? != tests.len()
        || &summaries[0][3] != "0"
        || &summaries[0][4] != "0"
    {
        return Err(format!(
            "native host gate must run exactly {} tests with no skips",
            tests.len()
        )
        .into());
    }
    if selection.metal_evidence
        && Regex::new(r"Native host gate adapter: .*backend: Metal")?
            .find_iter(output)
            .count()
            < tests.len()
    {
        return Err("required Metal adapter evidence missing".into());
    }
    Ok(())
}
pub(super) fn missing_worker(status: Option<i32>, output: &str) -> Result<()> {
    let records = records(output)?;
    exact(
        &records.iter().map(|record| record.0).collect::<Vec<_>>(),
        &[HOST_TESTS[1]],
    )?;
    if status == Some(0)
        || !output.contains("native host gate requires existing worker")
        || !Regex::new(r"(?:\.\.\. |\n)FAILED$")?.is_match(records[0].1)
        || !Regex::new(r"(?m)^test result: FAILED\. 0 passed; 1 failed; 0 ignored;")?
            .is_match(output)
    {
        return Err("missing-worker negative check did not execute and fail as required".into());
    }
    Ok(())
}

pub(super) fn run(args: &[String]) -> Result<()> {
    let directory = std::path::absolute(
        args.first()
            .ok_or("usage: native-host-gate OUTPUT_DIRECTORY")?,
    )?;
    fs::create_dir_all(&directory)?;
    match fs::remove_file(directory.join("acceptance.json")) {
        Ok(()) => (),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
        Err(error) => return Err(error.into()),
    }
    let mut environment: BTreeMap<String, String> = std::env::vars().collect();
    let run = |name, program, args: &[&str]| {
        process::run(&directory, name, program, args, &environment, true)
    };
    let source = run("source", "git", &["rev-parse", "HEAD"])?
        .text
        .trim()
        .to_string();
    run("source-status", "git", &["status", "--porcelain=v1"])?;
    run("toolchain", "rustc", &["-Vv"])?;
    run("cargo", "cargo", &["-V"])?;
    run("platform", "sw_vers", &[])?;
    let (worker, digest) = worker(&directory, &environment)?;
    environment.insert("ONECAD_REQUIRE_WORKER".into(), "1".into());
    environment.insert("ONECAD_REQUIRE_GPU".into(), "1".into());
    environment.insert("ONECAD_WORKER_PATH".into(), worker);
    run_tests(&directory, &environment)?;
    fs::write(
        directory.join("acceptance.json"),
        serde_json::to_string_pretty(&json!({
            "classification":"automated native-host lifecycle with production OCCT worker and Metal; no physical input or AppKit teardown qualification",
            "source":source,"workerSha256":digest,"tests":HOST_TESTS,"testCount":HOST_TESTS.len(),
            "serviceTests":SERVICE_TESTS,"serviceTestCount":SERVICE_TESTS.len(),
            "serviceAdapterEvidence":"service tests emit no adapter record; Metal provenance comes from the lifecycle selection",
            "inventoryVerified":true,"executedExactlyOnce":true,"missingWorkerRejected":true
        }))?,
    )?;
    Ok(())
}
/// The staged Mac sidecar: manifest hash, OCCT 8.0.1 fingerprint and selftest.
pub(super) fn worker(
    directory: &Path,
    environment: &BTreeMap<String, String>,
) -> Result<(String, String)> {
    let path = environment
        .get("ONECAD_WORKER_PATH")
        .filter(|path| !path.is_empty())
        .ok_or("explicit ONECAD_WORKER_PATH required")?;
    let worker = std::path::absolute(path)?;
    let manifest: Value =
        serde_json::from_slice(&fs::read("packaging/binaries/onecad-worker-manifest.json")?)?;
    let digest = hash(&fs::read(&worker)?);
    if manifest["binarySha256"] != digest
        || manifest["occt"]["version"] != "8.0.1"
        || manifest["occt"]["fingerprint"] != "0a6a1dce34181289"
    {
        return Err("staged worker manifest mismatch".into());
    }
    fs::write(
        directory.join("worker-manifest.json"),
        serde_json::to_string_pretty(&manifest)?,
    )?;
    let mut environment = environment.clone();
    environment.insert("ONECAD_WORKER_LOG".into(), "debug".into());
    let path = worker
        .to_str()
        .ok_or("worker path must be UTF-8")?
        .to_string();
    let output = process::run(
        directory,
        "worker-selftest",
        &path,
        &["--selftest"],
        &environment,
        true,
    )?
    .text;
    for expected in [
        "selftest OK (occt 8.0.1,",
        "build onecad-occt-8.0.1-b8f597c67781-kp1,",
        "fingerprint 0a6a1dce34181289",
    ] {
        if !output.contains(expected) {
            return Err(format!("worker selftest missing {expected}").into());
        }
    }
    Ok((path, digest))
}
fn selection_args<'a>(selection: &Selection, extra: &[&'a str]) -> Vec<&'a str> {
    let mut args = vec![
        "test",
        "--locked",
        "-p",
        "onecad-ui",
        "--lib",
        "--",
        "--ignored",
    ];
    args.extend(selection.filters);
    args.extend(extra);
    args
}
fn run_tests(directory: &Path, environment: &BTreeMap<String, String>) -> Result<()> {
    for selection in [&LIFECYCLE, &SERVICES] {
        let args = selection_args(selection, &["--list"]);
        let output = process::run(
            directory,
            selection.inventory_log,
            "cargo",
            &args,
            environment,
            true,
        )?;
        inventory(&output.text, selection)?;
    }
    for selection in [&LIFECYCLE, &SERVICES] {
        let args = selection_args(selection, &["--nocapture", "--test-threads=1"]);
        let output = process::run(
            directory,
            selection.run_log,
            "cargo",
            &args,
            environment,
            true,
        )?;
        verify(&output.text, selection)?;
    }
    let mut environment = environment.clone();
    environment.insert(
        "ONECAD_WORKER_PATH".into(),
        directory
            .join("missing-worker")
            .to_string_lossy()
            .into_owned(),
    );
    let negative = process::run(
        directory,
        "missing-worker",
        "cargo",
        &[
            "test",
            "--locked",
            "-p",
            "onecad-ui",
            "--lib",
            HOST_TESTS[1],
            "--",
            "--exact",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ],
        &environment,
        false,
    )?;
    missing_worker(negative.status, &negative.text)
}
