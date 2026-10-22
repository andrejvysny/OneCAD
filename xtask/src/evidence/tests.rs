use super::{
    captures, host,
    host_names::{HOST_TESTS, SERVICE_TESTS},
    kernelbench,
};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
struct Area(PathBuf);
impl Area {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "onecad-evidence-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Area {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn summary() -> String {
    format!(
        "test result: ok. {} passed; 0 failed; 0 ignored; 12 filtered out; finished in 1s\n",
        HOST_TESTS.len()
    )
}
fn record(name: &str) -> String {
    format!("test {name} ... Native host gate adapter: AdapterInfo {{ backend: Metal }}\nok\n")
}
fn accepted() -> String {
    HOST_TESTS
        .iter()
        .map(|name| record(name))
        .collect::<String>()
        + &summary()
}
#[test]
fn host_inventory_requires_exact_current_names() {
    assert_eq!(HOST_TESTS.len(), 97);
    let inventory = HOST_TESTS
        .iter()
        .map(|name| format!("{name}: test\n"))
        .collect::<String>();
    host::inventory(&inventory, &host::LIFECYCLE).unwrap();
    host::verify(&accepted(), &host::LIFECYCLE).unwrap();
    for output in [
        inventory.clone() + &format!("{}: test\n", HOST_TESTS[0]),
        inventory.replace(HOST_TESTS[0], HOST_TESTS[1]),
        inventory.replace(HOST_TESTS[0], "unknown"),
    ] {
        assert!(host::inventory(&output, &host::LIFECYCLE).is_err());
    }
}
#[test]
fn host_zero_skips_wrong_failed_duplicate_missing_and_nonmetal_fail_closed() {
    let accepted = accepted();
    let first = record(HOST_TESTS[0]);
    for output in [
        "test result: ok. 0 passed; 0 failed; 0 ignored;".into(),
        accepted.replace(
            &format!("{} passed; 0 failed; 0 ignored;", HOST_TESTS.len()),
            &format!("{} passed; 0 failed; 1 ignored;", HOST_TESTS.len() - 1),
        ),
        accepted.replace(HOST_TESTS[0], "unrelated_test"),
        accepted.replacen("\nok\n", "\nFAILED\n", 1),
        accepted.replacen("backend: Metal", "backend: Vulkan", 1),
        first.clone() + &accepted,
        accepted.replace(&first, &record(HOST_TESTS[1])),
        accepted.replace(&first, ""),
        accepted.clone() + &summary(),
    ] {
        assert!(
            host::verify(&output, &host::LIFECYCLE).is_err(),
            "incorrect acceptance: {output}"
        );
    }
}
#[test]
fn host_separate_stderr_and_extra_adapter_setup_valid() {
    let stdout = HOST_TESTS
        .iter()
        .map(|name| format!("test {name} ... ok\n"))
        .collect::<String>()
        + &summary();
    let stderr =
        "Native host gate adapter: AdapterInfo { backend: Metal }\n".repeat(HOST_TESTS.len() + 2);
    host::verify(&(stdout + &stderr), &host::LIFECYCLE).unwrap();
}
#[test]
fn host_service_selection_requires_its_own_exact_inventory_and_count() {
    assert_eq!(SERVICE_TESTS.len(), 4);
    let services = &host::SERVICES;
    let inventory = SERVICE_TESTS
        .iter()
        .map(|name| format!("{name}: test\n"))
        .collect::<String>();
    host::inventory(&inventory, services).unwrap();
    for output in [
        inventory.clone() + &format!("{}: test\n", SERVICE_TESTS[0]),
        inventory.clone() + &format!("{}: test\n", HOST_TESTS[0]),
        inventory.replace(SERVICE_TESTS[0], "unknown"),
        inventory.replace(&format!("{}: test\n", SERVICE_TESTS[3]), ""),
    ] {
        assert!(host::inventory(&output, services).is_err());
    }
    let summary =
        "test result: ok. 4 passed; 0 failed; 0 ignored; 58 filtered out; finished in 1s\n";
    // Service tests emit no adapter record, so none is required here.
    let passed = SERVICE_TESTS
        .iter()
        .map(|name| format!("test {name} ... ok\n"))
        .collect::<String>()
        + summary;
    host::verify(&passed, services).unwrap();
    for output in [
        "test result: ok. 0 passed; 0 failed; 0 ignored;".into(),
        passed.replacen("... ok\n", "... FAILED\n", 1),
        passed.replace(
            "4 passed; 0 failed; 0 ignored;",
            "3 passed; 0 failed; 1 ignored;",
        ),
        passed.replace(&format!("test {} ... ok\n", SERVICE_TESTS[3]), ""),
        passed.clone() + summary,
        accepted() + &passed,
    ] {
        assert!(host::verify(&output, services).is_err());
    }
    assert!(host::verify(&passed, &host::LIFECYCLE).is_err());
    assert!(host::verify(&accepted(), services).is_err());
}
#[test]
fn host_missing_worker_requires_selected_test_and_decisive_failure() {
    let name = HOST_TESTS[1];
    let failed = format!("test {name} ... FAILED\n\nfailures:\n\nfailures:\n    {name}\n\ntest result: FAILED. 0 passed; 1 failed; 0 ignored;\nnative host gate requires existing worker\n");
    host::missing_worker(Some(101), &failed).unwrap();
    for (status, output) in [
        (0, failed.clone()),
        (101, "compile failed".into()),
        (101, "native host gate requires existing worker".into()),
        (101, failed.replace(name, "app_logic_tests::wrong_case")),
        (101, format!("test {name} ... FAILED\n{failed}")),
    ] {
        assert!(host::missing_worker(Some(status), &output).is_err());
    }
}
fn fixture() -> Area {
    let area = Area::new();
    let mut png = vec![0; 33];
    png[..8].copy_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);
    png[12..16].copy_from_slice(b"IHDR");
    png[19] = 1;
    png[23] = 1;
    for depth in ["fallback", "stencil"] {
        for projection in ["perspective", "orthographic"] {
            for theme in ["dark", "light"] {
                let name = format!("{depth}-{projection}-{theme}-shaded-edges");
                fs::write(area.0.join(format!("{name}.png")), &png).unwrap();
                fs::write(area.0.join(format!("{name}.txt")),format!("case: \"{name}\"\nfeatures: test\nadapter: backend: Metal\nsource_commit: {}\nsource_status:\n","a".repeat(40))).unwrap();
            }
        }
    }
    area
}
const FIRST: &str = "fallback-perspective-dark-shaded-edges";
#[test]
fn capture_pairs_have_stable_manifest_and_download_hashes() {
    let area = fixture();
    let manifest = captures::verify(&area.0).unwrap();
    fs::write(
        area.0.join("manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    assert_eq!(captures::verify(&area.0).unwrap(), manifest);
    assert_eq!(manifest["images"], 8);
    assert_eq!(manifest["files"].as_array().unwrap().len(), 16);
    for record in manifest["files"].as_array().unwrap() {
        let bytes = fs::read(area.0.join(record["name"].as_str().unwrap())).unwrap();
        assert_eq!(record["sha256"], captures::hash(&bytes));
    }
    assert_eq!(
        captures::hash(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}
#[test]
fn capture_unsafe_incomplete_empty_invalid_provenance_and_mixed_revision_fail() {
    let changes: &[fn(&Path)] = &[
        |root| fs::write(root.join("invalid:name.txt"), "bad").unwrap(),
        |root| fs::remove_file(root.join(format!("{FIRST}.txt"))).unwrap(),
        |root| {
            for ext in ["png", "txt"] {
                fs::remove_file(root.join(format!("{FIRST}.{ext}"))).unwrap();
            }
        },
        |root| fs::write(root.join(format!("{FIRST}.png")), "not PNG").unwrap(),
        |root| fs::write(root.join(format!("{FIRST}.txt")), "").unwrap(),
        |root| fs::write(root.join(format!("{FIRST}.txt")), "missing provenance").unwrap(),
        |root| {
            let path = root.join(format!("{FIRST}.txt"));
            let text = fs::read_to_string(&path).unwrap();
            fs::write(path, text.replace(&"a".repeat(40), &"b".repeat(40))).unwrap();
        },
        |root| {
            let path = root.join(format!("{FIRST}.png"));
            let mut bytes = fs::read(&path).unwrap();
            bytes[16..20].fill(0);
            fs::write(path, bytes).unwrap();
        },
    ];
    for change in changes {
        let area = fixture();
        change(&area.0);
        assert!(captures::verify(&area.0).is_err());
    }
    let empty = Area::new();
    assert!(captures::verify(&empty.0).is_err());
}
#[test]
#[cfg(unix)]
fn capture_symlinks_and_directories_rejected() {
    for directory in [false, true] {
        let area = fixture();
        let path = area.0.join(format!("{FIRST}.png"));
        fs::remove_file(&path).unwrap();
        if directory {
            fs::create_dir(&path).unwrap();
        } else {
            std::os::unix::fs::symlink(
                area.0.join("stencil-perspective-dark-shaded-edges.png"),
                path,
            )
            .unwrap();
        }
        assert!(captures::verify(&area.0).is_err());
    }
}
#[test]
fn digest_variants_platforms_nulls_and_changes_preserved() {
    let area = Area::new();
    fs::write(area.0.join("results.jsonl"),[
        json!({"caseId":"case","backend":"occt","inputDigest":"base"}),
        json!({"caseId":"case","backend":"occt","campaignVariant":{"name":"rotated"},"normalizedDigest":"rotated"}),
    ].iter().map(Value::to_string).collect::<Vec<_>>().join("\n")).unwrap();
    let rows = kernelbench::digests(&area.0, "suite", "darwin-arm64").unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows["suite|case|occt|base|darwin-arm64"]["normalizedDigest"],
        Value::Null
    );
    let mut changed = rows.clone();
    changed.remove("suite|case|occt|base|darwin-arm64");
    changed["suite|case|occt|rotated|darwin-arm64"]["normalizedDigest"] = json!("changed");
    changed.insert("suite|extra|occt|base|darwin-arm64".into(), json!({}));
    let problems = kernelbench::digest_diff(&rows, &changed);
    for prefix in ["MISSING", "CHANGED", "UNEXPECTED"] {
        assert!(problems.iter().any(|value| value.starts_with(prefix)));
    }
    assert!(kernelbench::digest_diff(&rows, &rows).is_empty());
}
#[test]
fn semantic_projection_excludes_timing_and_reports_nested_and_absent_values() {
    let area = Area::new();
    fs::write(
        area.0.join("summary.json"),
        json!({"schemaVersion":1,"counts":{"pass":2},"timing":{"median":8}}).to_string(),
    )
    .unwrap();
    let observed = kernelbench::semantics(&area.0).unwrap();
    assert!(observed.get("timing").is_none());
    assert_eq!(observed["replay"], Value::Null);
    let mut changed = observed.clone();
    changed["counts"]["pass"] = json!(3);
    let mut problems = vec![];
    kernelbench::diff(Some(&observed), Some(&changed), "", &mut problems);
    assert_eq!(problems, ["counts.pass: expected 2, got 3"]);
    problems.clear();
    kernelbench::diff(None, Some(&Value::Null), "new", &mut problems);
    assert_eq!(problems, ["new: expected undefined, got null"]);
}
#[test]
fn kernelbench_preserves_usage_and_missing_input_exit_codes() {
    let usage = kernelbench::run(&[]).unwrap_err();
    assert_eq!(usage.downcast_ref::<super::ExitError>().unwrap().code, 2);
    let area = Area::new();
    let args = [
        "compare".into(),
        area.0.to_string_lossy().into_owned(),
        "suite".into(),
        area.0.join("missing.json").to_string_lossy().into_owned(),
    ];
    let missing = kernelbench::run(&args).unwrap_err();
    assert_eq!(missing.downcast_ref::<super::ExitError>().unwrap().code, 3);
}

#[test]
fn kernelbench_record_merge_removes_stale_rows_and_keeps_other_platforms_and_suites() {
    let area = Area::new();
    let platform = kernelbench::platform();
    let path = area.0.join("digests.json");
    let stale = format!("suite|stale|occt|base|{platform}");
    let mut baseline = serde_json::Map::new();
    baseline.insert(
        stale.clone(),
        json!({"inputDigest":"stale","normalizedDigest":null}),
    );
    baseline.insert(
        "suite|case|occt|base|foreign-platform".into(),
        json!({"inputDigest":"foreign","normalizedDigest":null}),
    );
    baseline.insert(
        format!("other-suite|case|occt|base|{platform}"),
        json!({"inputDigest":"other","normalizedDigest":null}),
    );
    fs::write(&path, serde_json::to_vec(&baseline).unwrap()).unwrap();
    fs::write(
        area.0.join("results.jsonl"),
        json!({"caseId":"case","backend":"occt","inputDigest":"current"}).to_string(),
    )
    .unwrap();
    let args = |mode: &str| {
        vec![
            mode.into(),
            area.0.to_string_lossy().into_owned(),
            "suite".into(),
            path.to_string_lossy().into_owned(),
        ]
    };
    kernelbench::run(&args("record")).unwrap();
    let recorded: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert!(recorded.get(&stale).is_none());
    assert_eq!(recorded.as_object().unwrap().len(), 3);
    assert_eq!(
        recorded["suite|case|occt|base|foreign-platform"]["inputDigest"],
        "foreign"
    );
    kernelbench::run(&args("compare")).unwrap();
    fs::write(
        area.0.join("results.jsonl"),
        json!({"caseId":"case","backend":"occt","inputDigest":"changed"}).to_string(),
    )
    .unwrap();
    let mismatch = kernelbench::run(&args("compare")).unwrap_err();
    assert_eq!(mismatch.downcast_ref::<super::ExitError>().unwrap().code, 1);
}
#[test]
fn kernelbench_semantic_record_ignores_timing_but_rejects_changed_verdict() {
    let area = Area::new();
    let path = area.0.join("semantics.json");
    let summary = area.0.join("summary.json");
    fs::write(
        &summary,
        json!({"counts":{"pass":3},"timing":1}).to_string(),
    )
    .unwrap();
    let args = |mode: &str| {
        vec![
            mode.into(),
            area.0.to_string_lossy().into_owned(),
            "suite".into(),
            path.to_string_lossy().into_owned(),
        ]
    };
    kernelbench::run(&args("semantic-record")).unwrap();
    fs::write(
        &summary,
        json!({"counts":{"pass":3},"timing":900}).to_string(),
    )
    .unwrap();
    kernelbench::run(&args("semantic-compare")).unwrap();
    fs::write(
        &summary,
        json!({"counts":{"pass":2},"timing":900}).to_string(),
    )
    .unwrap();
    let error = kernelbench::run(&args("semantic-compare")).unwrap_err();
    assert_eq!(error.downcast_ref::<super::ExitError>().unwrap().code, 1);
    assert!(error.to_string().contains("counts.pass: expected 3, got 2"));
}

#[test]
fn host_command_records_status_arguments_selected_environment_and_failure_output() {
    let area = Area::new();
    let environment = std::collections::BTreeMap::from([
        ("ONECAD_REQUIRE_GPU".into(), "1".into()),
        ("PRIVATE_TOKEN".into(), "must-not-be-recorded".into()),
    ]);
    let accepted = super::process::run(
        &area.0,
        "version",
        "rustc",
        &["--version"],
        &environment,
        true,
    )
    .unwrap();
    assert_eq!(accepted.status, Some(0));
    assert!(accepted.text.starts_with("rustc "));
    let command: Value =
        serde_json::from_slice(&fs::read(area.0.join("version.command.json")).unwrap()).unwrap();
    assert_eq!(command["args"], json!(["--version"]));
    assert_eq!(command["environment"], json!({"ONECAD_REQUIRE_GPU":"1"}));
    assert_eq!(command["status"], 0);
    let rejected = super::process::run(
        &area.0,
        "rejected",
        "rustc",
        &["--onecad-invalid-option"],
        &environment,
        false,
    )
    .unwrap();
    assert_ne!(rejected.status, Some(0));
    assert!(area.0.join("rejected.log").is_file());
    assert!(area.0.join("rejected.command.json").is_file());
    assert!(super::process::run(
        &area.0,
        "required",
        "rustc",
        &["--onecad-invalid-option"],
        &environment,
        true
    )
    .is_err());
}

#[test]
fn semantic_arrays_keep_order_and_null_is_distinct_from_missing() {
    let baseline = json!({"records":["base","rotated"],"replay":null});
    let mut problems = Vec::new();
    kernelbench::diff(Some(&baseline), Some(&baseline), "", &mut problems);
    assert!(problems.is_empty());
    kernelbench::diff(
        Some(&baseline),
        Some(&json!({"records":["rotated","base"]})),
        "",
        &mut problems,
    );
    assert_eq!(problems.len(), 2);
    assert!(problems
        .iter()
        .any(|problem| problem.starts_with("records: expected [")));
    assert!(problems
        .iter()
        .any(|problem| problem == "replay: expected null, got undefined"));
}
