//! Negative (and the matching positive) controls of every
//! `ios-simulator-gate` reject rule (docs/ios/SIMULATOR_GATE.md).
use super::{
    captures::hash,
    ios_evidence,
    ios_release::{self, Platform},
    ios_sim, ios_verify,
};
use serde_json::{json, Value};
use std::{fs, path::Path};

pub(super) const UDID: &str = "3D9FCE1F-7F97-4D48-8301-BABA7A46CADB";
pub(super) const RUN: &str = "20261022T101500Z-f585dab0-0123456789abcdef";
pub(super) const SOURCE: &str = "f585dab0aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const NATIVE: &str = "OneCADNativeTests/TestHookPingTests/testPingThroughTestCommand()";
const UI: &str = "OneCADUITests/LaunchTests/testLaunchShowsCanvasAndFileMenu()";

/// Tests run in `xtask/`; the gate itself runs from the repository root.
pub(super) fn repo(path: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join(path)
}

fn inventory(skips: Value) -> ios_verify::Inventory {
    let text = json!({ "tests": [NATIVE, UI], "approvedSkips": skips }).to_string();
    ios_verify::inventory(&text).unwrap()
}

fn case(id: &str, result: &str, children: Value) -> Value {
    let (suite, name) = id.split_once('/').unwrap().1.split_once('/').unwrap();
    json!({ "nodeType": "Test Suite", "name": suite, "children": [{
        "nodeType": "Test Case", "name": name, "nodeIdentifier": format!("{suite}/{name}"),
        "result": result, "children": children }] })
}

fn tests(native: Value, ui: Value) -> Value {
    json!({ "testNodes": [{ "nodeType": "Test Plan", "name": "iPadAcceptance", "children": [
        { "nodeType": "Unit test bundle", "name": "OneCADNativeTests", "children": [native] },
        { "nodeType": "UI test bundle", "name": "OneCADUITests", "children": [ui] } ] }] })
}

fn passed() -> Value {
    tests(
        case(NATIVE, "Passed", json!([])),
        case(UI, "Passed", json!([])),
    )
}

fn summary(passed: u64, skipped: u64) -> Value {
    json!({ "devicesAndConfigurations": [{ "device": { "deviceId": UDID } }],
        "totalTestCount": 2, "passedTests": passed, "failedTests": 0,
        "skippedTests": skipped, "expectedFailures": 0 })
}

#[test]
fn ios_exact_inventory_passes_and_test_level_rejects_fail_closed() {
    let inventory = inventory(json!([]));
    ios_verify::verify_tests(&passed(), &summary(2, 0), &inventory, UDID).unwrap();
    let native = |result: &str, children: Value| case(NATIVE, result, children);
    let ui = || case(UI, "Passed", json!([]));
    let extra = case("OneCADUITests/LaunchTests/testExtra()", "Passed", json!([]));
    let mut duplicated = passed();
    duplicated["testNodes"][0]["children"][0]["children"]
        .as_array_mut()
        .unwrap()
        .push(native("Passed", json!([])));
    let repetitions = json!([{ "nodeType": "Repetition", "result": "Failed" },
        { "nodeType": "Repetition", "result": "Passed" }]);
    for (label, executed, totals) in [
        ("missing", tests(json!({}), ui()), summary(1, 0)),
        (
            "extra",
            tests(native("Passed", json!([])), extra),
            summary(2, 0),
        ),
        (
            "failed",
            tests(native("Failed", json!([])), ui()),
            summary(1, 0),
        ),
        (
            "skipped",
            tests(native("Skipped", json!([])), ui()),
            summary(1, 1),
        ),
        (
            "expected failure",
            tests(native("Expected Failure", json!([])), ui()),
            summary(1, 0),
        ),
        ("duplicate", duplicated, summary(2, 0)),
        (
            "retried",
            tests(native("Passed", repetitions), ui()),
            summary(2, 0),
        ),
        ("summary failed", passed(), json!({ "failedTests": 1 })),
        (
            "other simulator",
            passed(),
            summary(2, 0)
                .to_string()
                .replace(UDID, "OTHER")
                .parse()
                .unwrap(),
        ),
    ] {
        assert!(
            ios_verify::verify_tests(&executed, &totals, &inventory, UDID).is_err(),
            "{label} accepted"
        );
    }
}

#[test]
fn ios_skips_need_an_approved_reason() {
    let skipped = tests(
        case(NATIVE, "Skipped", json!([])),
        case(UI, "Passed", json!([])),
    );
    let approved = inventory(json!([{ "test": NATIVE, "reason": "simulator lacks X" }]));
    ios_verify::verify_tests(&skipped, &summary(1, 1), &approved, UDID).unwrap();
    for skips in [
        json!([{ "test": NATIVE, "reason": " " }]),
        json!([{ "test": "OneCADUITests/Unknown/test()", "reason": "x" }]),
    ] {
        let text = json!({ "tests": [NATIVE, UI], "approvedSkips": skips }).to_string();
        assert!(ios_verify::inventory(&text).is_err());
    }
    let text = json!({ "tests": [NATIVE, NATIVE], "approvedSkips": [] }).to_string();
    assert!(
        ios_verify::inventory(&text).is_err(),
        "duplicated inventory"
    );
    let real = fs::read_to_string(repo(ios_verify::INVENTORY)).unwrap();
    let real = ios_verify::inventory(&real).unwrap();
    assert!(
        real.approved_skips.is_empty(),
        "approved skip list starts empty"
    );
    assert!(real.tests.contains(NATIVE) && real.tests.contains(UI));
}

#[test]
fn ios_run_id_dirty_tree_and_oracle_log_rules() {
    ios_verify::verify_run_id(RUN, SOURCE).unwrap();
    assert!(
        ios_verify::verify_run_id(RUN, "0000000000").is_err(),
        "other source"
    );
    assert!(ios_verify::verify_run_id("20261022-f585dab0-x", SOURCE).is_err());
    let id = ios_sim::run_id(1_792_627_063, SOURCE, [1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(id, "20261021T235743Z-f585dab0-0102030405060708");
    ios_verify::verify_run_id(&id, SOURCE).unwrap();
    ios_verify::verify_clean_tree("").unwrap();
    assert!(ios_verify::verify_clean_tree(" M crates/onecad-ios/src/app.rs\n").is_err());
    let log = "running 1 test\ntest ios_simulator_artifacts ... ok\n\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 3 filtered out\n";
    ios_evidence::verify_oracle_log(log).unwrap();
    for bad in [
        log.replace("... ok", "... FAILED"),
        log.replace(
            "1 passed; 0 failed; 0 ignored",
            "0 passed; 0 failed; 1 ignored",
        ),
        format!("{log}{log}"),
        String::new(),
    ] {
        assert!(ios_evidence::verify_oracle_log(&bad).is_err());
    }
}

pub(super) fn identity(platform: &str, hooks: bool) -> Value {
    json!({ "manifestVersion": 2, "kind": "embedded", "abiVersion": 3, "protocolVersion": 1,
        "workerVersion": "0.1.0", "quantizationVersion": 1, "solverPolicyVersion": 2,
        "occt": { "version": "8.0.1", "buildId": "b", "fingerprint": "1234567890abcdef" },
        "platform": platform, "deploymentTarget": "18.0", "testHooks": hooks })
}

pub(super) fn kernel() -> Value {
    let simulator = identity("iphonesimulator", true);
    json!({ "backend": "in-process", "embedded": simulator, "linked": simulator, "epoch": 1,
        "hello": { "protocolVersion": 1, "workerVersion": "0.1.0", "quantizationVersion": 1,
            "solverPolicyVersion": 2, "occt": { "version": "8.0.1", "fingerprint": "1234567890abcdef" } } })
}

#[test]
fn ios_bundle_and_kernel_identity_must_be_the_simulator_build() {
    let simulator = identity("iphonesimulator", true);
    ios_verify::verify_bundle_identity(&simulator, "iphonesimulator", true).unwrap();
    ios_verify::verify_bundle_identity(&identity("iphoneos", false), "iphoneos", false).unwrap();
    assert!(
        ios_verify::verify_bundle_identity(&identity("iphoneos", true), "iphoneos", false).is_err()
    );
    assert!(ios_verify::verify_bundle_identity(&simulator, "iphoneos", true).is_err());
    ios_verify::verify_kernel(&kernel(), &simulator).unwrap();
    let mut stub = kernel();
    stub["backend"] = json!("stub");
    let mut other_build = kernel();
    other_build["embedded"]["occt"]["buildId"] = json!("other");
    let mut other_fingerprint = kernel();
    other_fingerprint["hello"]["occt"]["fingerprint"] = json!("0a6a1dce34181289");
    let mut no_hello = kernel();
    no_hello["hello"] = Value::Null;
    let mut no_embedded = kernel();
    no_embedded["embedded"] = Value::Null;
    let mut other_link = kernel();
    other_link["linked"]["archiveSha256"] = json!("other");
    for bad in [
        stub,
        other_build,
        other_fingerprint,
        no_hello,
        no_embedded,
        other_link,
    ] {
        assert!(
            ios_verify::verify_kernel(&bad, &simulator).is_err(),
            "{bad}"
        );
    }
}

pub(super) fn write(path: &Path, value: &Value) {
    fs::write(path, value.to_string()).unwrap();
}

fn artifacts(dir: &Path) {
    fs::write(dir.join("plate.onecad"), b"zip").unwrap();
    let provenance = json!({ "runId": RUN, "kind": "document", "artifact": "plate.onecad",
        "sha256": hash(b"zip"), "kernelIdentity": kernel() });
    write(&dir.join("plate.onecad.provenance.json"), &provenance);
    write(
        &dir.join("plate.json"),
        &json!({ "runId": RUN, "kind": "semantic", "kernelIdentity": kernel() }),
    );
}

#[test]
fn ios_artifacts_reject_stale_runs_missing_files_and_fake_kernels() {
    let simulator = identity("iphonesimulator", true);
    let required = vec!["plate".to_string()];
    let check = |dir: &Path| ios_verify::verify_artifacts(dir, RUN, &simulator, &required);
    let area = tempfile::tempdir().unwrap();
    artifacts(area.path());
    check(area.path()).unwrap();
    // A trace record sealed with its own provenance sidecar is accepted.
    fs::write(area.path().join("trace.jsonl"), b"{}").unwrap();
    let sealed = json!({ "runId": RUN, "kind": "trace", "artifact": "trace.jsonl",
        "sha256": hash(b"{}"), "kernelIdentity": kernel() });
    write(&area.path().join("trace.jsonl.provenance.json"), &sealed);
    check(area.path()).unwrap();
    type Mutation = fn(&Path);
    let mutations: [(&str, Mutation); 8] = [
        ("stale provenance", |d| {
            edit(
                d,
                "plate.onecad.provenance.json",
                "runId",
                json!("20261021T000000Z-f585dab0-0000000000000000"),
            )
        }),
        ("missing run id", |d| {
            edit(d, "plate.json", "runId", Value::Null)
        }),
        ("stale semantic", |d| {
            edit(d, "plate.json", "runId", json!("other"))
        }),
        ("fake backend", |d| {
            edit(
                d,
                "plate.json",
                "kernelIdentity",
                json!({ "backend": "fake" }),
            )
        }),
        ("hash mismatch", |d| {
            fs::write(d.join("plate.onecad"), b"changed").unwrap()
        }),
        ("missing provenance", |d| {
            fs::remove_file(d.join("plate.onecad.provenance.json")).unwrap()
        }),
        ("missing model", |d| {
            fs::remove_file(d.join("plate.onecad")).unwrap();
            fs::remove_file(d.join("plate.onecad.provenance.json")).unwrap();
        }),
        ("unprovenanced extra", |d| {
            fs::write(d.join("trace.jsonl"), b"{}").unwrap()
        }),
    ];
    for (label, mutate) in mutations {
        let area = tempfile::tempdir().unwrap();
        artifacts(area.path());
        mutate(area.path());
        assert!(check(area.path()).is_err(), "{label} accepted");
    }
}

fn edit(dir: &Path, file: &str, key: &str, value: Value) {
    let path = dir.join(file);
    let mut record: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    record[key] = value;
    write(&path, &record);
}

const OTOOL_DEVICE: &str = "Load command 9\n      cmd LC_BUILD_VERSION\n  cmdsize 32\n platform 2\n    minos 18.0\n      sdk 26.5\n   ntools 1\n";

#[test]
fn ios_release_products_reject_hooks_test_verbs_and_wrong_platforms() {
    ios_release::verify_symbols("_main\n_onecad_ios_app_create\n").unwrap();
    assert!(ios_release::verify_symbols("_main\n_onecad_ios_test_command\n").is_err());
    assert!(ios_release::verify_symbols("").is_err());
    ios_release::verify_strings(b"\0OneCAD\0Shutdown\0").unwrap();
    for verb in ios_release::TEST_VERBS {
        assert!(ios_release::verify_strings(format!("x\0{verb}\0").as_bytes()).is_err());
    }
    ios_release::verify_build_version(OTOOL_DEVICE, Platform::Device).unwrap();
    let simulator = OTOOL_DEVICE.replace("platform 2", "platform 7");
    ios_release::verify_build_version(&simulator, Platform::Simulator).unwrap();
    for (otool, platform) in [
        (simulator.clone(), Platform::Device),
        (OTOOL_DEVICE.to_string(), Platform::Simulator),
        (
            OTOOL_DEVICE.replace("minos 18.0", "minos 17.0"),
            Platform::Device,
        ),
        (format!("{OTOOL_DEVICE}{OTOOL_DEVICE}"), Platform::Device),
        (String::new(), Platform::Device),
    ] {
        assert!(ios_release::verify_build_version(&otool, platform).is_err());
    }
    let libraries = "OneCAD:\n\t/usr/lib/libc++.1.dylib (compatibility version 1.0.0)\n\t/System/Library/Frameworks/UIKit.framework/UIKit (compatibility version 1.0.0)\n";
    ios_release::verify_libraries(libraries).unwrap();
    let foreign = format!("{libraries}\t@rpath/libTKernel.dylib (compatibility version 8.0.0)\n");
    assert!(ios_release::verify_libraries(&foreign).is_err());
}

#[test]
fn ios_simulator_selection_prefers_named_gate_devices_and_newest_runtime() {
    let runtimes = json!({ "runtimes": [
        { "identifier": "rt.iOS-18-1", "version": "18.1", "platform": "iOS", "isAvailable": true },
        { "identifier": "rt.iOS-26-5", "version": "26.5", "platform": "iOS", "isAvailable": true },
        { "identifier": "rt.iOS-26-6", "version": "26.6", "platform": "iOS", "isAvailable": false },
        { "identifier": "rt.watchOS", "version": "30.0", "platform": "watchOS", "isAvailable": true } ] });
    let newest = ios_sim::runtime(&runtimes, None).unwrap();
    assert_eq!(newest.identifier, "rt.iOS-26-5");
    assert_eq!(
        ios_sim::runtime(&runtimes, Some("18.1"))
            .unwrap()
            .identifier,
        "rt.iOS-18-1"
    );
    assert!(
        ios_sim::runtime(&runtimes, Some("26.6")).is_err(),
        "unavailable runtime"
    );
    let name = ios_sim::gate_name(ios_sim::DEFAULT_DEVICE_TYPE, &newest);
    assert_eq!(name, "OneCAD Gate iPad Pro 13 M5 26.5");
    let devices = json!({ "devices": { "rt.iOS-26-5": [
        { "name": "OneCAD Lane Gate", "udid": "LANE", "isAvailable": true },
        { "name": name, "udid": UDID, "isAvailable": true } ] } });
    assert_eq!(
        ios_sim::find_named(&devices, &newest, &name).as_deref(),
        Some(UDID)
    );
    assert_eq!(
        ios_sim::find_udid(&devices, "LANE").unwrap().1,
        "OneCAD Lane Gate"
    );
    assert!(ios_sim::find_udid(&devices, "MISSING").is_none());
    let types =
        json!({ "devicetypes": [{ "name": "iPad Pro 13-inch (M5)", "identifier": "dt.M5" }] });
    assert_eq!(
        ios_sim::device_type(&types, "iPad Pro 13-inch (M5)").unwrap(),
        "dt.M5"
    );
    assert!(ios_sim::device_type(&types, "iPhone").is_err());
}
