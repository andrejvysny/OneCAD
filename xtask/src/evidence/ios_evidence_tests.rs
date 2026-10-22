//! The gate's re-check of the Mac oracle reports and the whole verifier over a
//! synthetic evidence directory (docs/ios/SIMULATOR_GATE.md steps 8-10): a
//! complete directory passes and gets `acceptance.json`; any rejected rule
//! leaves it absent.
use super::{
    captures::hash,
    ios_evidence::{self, BUNDLES, MAC_FIXTURE},
    ios_gate, ios_geometry,
    ios_release::Platform,
    ios_replay,
    ios_tests::{identity, kernel, repo, write, RUN, SOURCE, UDID},
    ios_verify,
};
use serde_json::{json, Value};
use std::{
    f64::consts::PI,
    fs,
    path::{Path, PathBuf},
};

const REFERENCE: f64 = 9600.0 - 36.0 * PI;
const WORKER_SHA: &str = "d394e96ea371b7bec50b778bdeb52fb1d77751cb56f9b75415f70188c144675d";
const ORACLE_LOG: &str = "running 1 test\ntest ios_simulator_artifacts ... ok\n\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 6 filtered out\n";
const MAC_BYTES: &[u8] = b"mac-authored container";

fn root() -> PathBuf {
    repo("")
}

fn expectations() -> Value {
    serde_json::from_slice(&fs::read(repo(ios_geometry::EXPECTATIONS)).unwrap()).unwrap()
}

fn plate(volume: f64, features: &[&str]) -> Value {
    json!({ "occtFingerprint": ios_geometry::MAC_OCCT_FINGERPRINT, "documentId": "doc",
        "regenOutcome": "published", "geometrySource": "live",
        "features": features.iter()
            .map(|op| json!({ "id": format!("{op}-id"), "opType": op, "status": "ok" }))
            .collect::<Vec<_>>(),
        "bodies": [{ "id": "b", "healthy": true, "solids": 1, "faces": 7, "volume": volume,
            "min": [-40.0, 0.0, 0.0], "max": [0.0, 60.0, 4.0] }],
        "sketches": [{ "id": "s", "status": "FullyConstrained", "dof": 0,
            "entities": 14, "constraints": 13 }] })
}

/// The measurement a correct regeneration of `case` reports.
fn measured(case: &Value) -> Value {
    let analytic = |v: &Value| {
        v["constant"].as_f64().unwrap_or(0.0) + v["piCoefficient"].as_f64().unwrap_or(0.0) * PI
    };
    let features: Vec<&str> = case["features"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap())
        .collect();
    plate(
        REFERENCE - analytic(&case["decreaseFromReference"]),
        &features,
    )
}

fn author_report(all: &Value) -> Value {
    let case = ios_geometry::case(all, "mac-plate").unwrap();
    let artifact = json!({ "name": "mac-plate", "sha256": hash(MAC_BYTES),
        "measurement": measured(case), "failures": [] });
    json!({ "passed": true, "expectationsSha256": hash(&fs::read(repo(ios_geometry::EXPECTATIONS)).unwrap()),
        "workerSha256": WORKER_SHA, "macFixture": { "passed": true, "artifact": artifact } })
}

/// Copies every required simulator document into `artifacts` and returns the
/// verify report the oracle would write for them.
fn simulator_report(all: &Value, artifacts: &Path) -> Value {
    let mut checked = Vec::new();
    for name in ios_geometry::required_simulator(all) {
        let bytes = format!("{name} container");
        let file = format!("{name}.onecad");
        fs::write(artifacts.join(&file), &bytes).unwrap();
        let provenance = json!({ "runId": RUN, "kind": "document", "artifact": file,
            "sha256": hash(bytes.as_bytes()), "kernelIdentity": kernel() });
        write(
            &artifacts.join(format!("{file}.provenance.json")),
            &provenance,
        );
        let case = ios_geometry::case(all, &name).unwrap();
        let mut summary = measured(case);
        summary["document"] = json!({ "geometrySource": "live" });
        let semantic = json!({ "runId": RUN, "kind": "semantic", "name": name,
            "kernelIdentity": kernel(), "summary": summary });
        write(&artifacts.join(format!("{name}.json")), &semantic);
        let round_trip =
            (!case["roundTripOf"].is_null()).then(|| json!({ "fixtureSha256": hash(MAC_BYTES) }));
        checked.push(json!({ "name": name, "sha256": hash(bytes.as_bytes()),
            "measurement": measured(case), "roundTrip": round_trip, "failures": [] }));
    }
    let author = author_report(all);
    json!({ "passed": true, "expectationsSha256": author["expectationsSha256"],
        "workerSha256": WORKER_SHA,
        "simulator": { "artifacts": checked, "missing": [], "unexpected": [] } })
}

#[test]
fn ios_geometry_outside_tolerance_is_rejected() {
    let all = expectations();
    let mac = ios_geometry::case(&all, "mac-plate").unwrap();
    ios_geometry::check(mac, &measured(mac), &all).unwrap();
    let fillet = ios_geometry::case(&all, "plate-fillet").unwrap();
    ios_geometry::check(fillet, &measured(fillet), &all).unwrap();
    let edit = |m: &mut Value, path: &[&str], value: Value| {
        let mut target = m;
        for key in path {
            target = match key.parse::<usize>() {
                Ok(index) => &mut target[index],
                Err(_) => &mut target[*key],
            };
        }
        *target = value;
    };
    let mutations: [(&Value, &[&str], Value); 10] = [
        (
            mac,
            &["bodies", "0", "volume"],
            json!(REFERENCE * (1.0 + 2e-6)),
        ),
        (mac, &["bodies", "0", "max", "1"], json!(60.0 + 1e-5)),
        (mac, &["regenOutcome"], json!("noop")),
        (mac, &["geometrySource"], json!("cached")),
        (mac, &["bodies", "0", "healthy"], json!(false)),
        (mac, &["features", "1", "opType"], json!("Revolve")),
        (mac, &["sketches", "0", "status"], json!("OverConstrained")),
        (mac, &["sketches", "0", "dof"], json!(1)),
        (mac, &["occtFingerprint"], json!("1235abe6387959f8")),
        (fillet, &["bodies", "0", "volume"], json!(REFERENCE)),
    ];
    for (case, path, value) in mutations {
        let mut bad = measured(case);
        edit(&mut bad, path, value);
        assert!(ios_geometry::check(case, &bad, &all).is_err(), "{path:?}");
    }
    let mut off = measured(fillet);
    off["bodies"][0]["volume"] = json!(REFERENCE - (4.0 - PI) * 1.001);
    assert!(
        ios_geometry::check(fillet, &off, &all).is_err(),
        "1e-4 decrease"
    );
    // A plate off f32-exact coordinates carries MESH1's f32 rounding
    // (2.86e-6 on 13.7..73.7), which is allowed; 1e-5 more is not.
    let plate = ios_geometry::case(&all, "plate").unwrap();
    let stored = |value: f64| f64::from(value as f32);
    let mut anchored = measured(plate);
    anchored["bodies"][0]["min"] = json!([stored(13.7), stored(-12.345), 0.0]);
    anchored["bodies"][0]["max"] = json!([stored(73.7), stored(27.655), 4.0]);
    ios_geometry::check(plate, &anchored, &all).unwrap();
    anchored["bodies"][0]["max"][0] = json!(stored(73.7) + 1e-5);
    assert!(ios_geometry::check(plate, &anchored, &all).is_err());
    assert_eq!(ios_geometry::f32_storage(60.0), 1.9073486328125e-6);
}

#[test]
fn ios_oracle_reports_must_match_documents_round_trip_and_provenance() {
    let all = expectations();
    let area = tempfile::tempdir().unwrap();
    let author = author_report(&all);
    let report = simulator_report(&all, area.path());
    let check = |report: &Value, author: &Value| {
        ios_geometry::verify_simulator_report(report, author, &all, area.path())
    };
    check(&report, &author).unwrap();
    let sha = author["expectationsSha256"].as_str().unwrap();
    ios_geometry::verify_report_provenance(&report, sha, WORKER_SHA).unwrap();
    assert!(ios_geometry::verify_report_provenance(&report, "other", WORKER_SHA).is_err());
    assert!(ios_geometry::verify_report_provenance(&report, sha, "other").is_err());
    let resaved = report["simulator"]["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .position(|a| a["name"] == "mac-plate-resaved")
        .unwrap();
    let mut lying = report.clone();
    lying["simulator"]["artifacts"][0]["measurement"]["bodies"][0]["volume"] = json!(1.0);
    let mut dropped = report.clone();
    dropped["simulator"]["artifacts"]
        .as_array_mut()
        .unwrap()
        .pop();
    let mut failed = report.clone();
    failed["passed"] = json!(false);
    let mut fresh = report.clone();
    fresh["simulator"]["artifacts"][resaved]["measurement"]["documentId"] = json!("fresh");
    let mut copy = report.clone();
    copy["simulator"]["artifacts"][resaved]["sha256"] = json!(hash(MAC_BYTES));
    let mut other_fixture = report.clone();
    other_fixture["simulator"]["artifacts"][resaved]["roundTrip"] = Value::Null;
    for bad in [lying, dropped, failed, fresh, copy, other_fixture] {
        assert!(check(&bad, &author).is_err(), "{bad}");
    }
    fs::write(area.path().join("plate.onecad"), b"replaced").unwrap();
    assert!(check(&report, &author).is_err(), "stale hash");
}

/// Executed-test JSON for exactly the checked-in inventory.
fn xcresult(inventory: &ios_verify::Inventory) -> (Value, Value) {
    let mut bundles: Vec<Value> = Vec::new();
    for id in &inventory.tests {
        let mut parts = id.split('/');
        let (bundle, suite, test) = (parts.next(), parts.next(), parts.next());
        let (bundle, suite, test) = (bundle.unwrap(), suite.unwrap(), test.unwrap());
        let kind = if bundle.ends_with("UITests") {
            "UI test bundle"
        } else {
            "Unit test bundle"
        };
        bundles.push(json!({ "nodeType": kind, "name": bundle, "children": [{
            "nodeType": "Test Suite", "name": suite, "children": [{ "nodeType": "Test Case",
            "name": test, "nodeIdentifier": format!("{suite}/{test}"), "result": "Passed" }] }] }));
    }
    let count = inventory.tests.len();
    let tests = json!({ "testNodes": [{ "nodeType": "Test Plan", "children": bundles }] });
    let summary = json!({ "devicesAndConfigurations": [{ "device": { "deviceId": UDID } }],
        "totalTestCount": count, "passedTests": count, "failedTests": 0,
        "skippedTests": 0, "expectedFailures": 0 });
    (tests, summary)
}

/// A complete evidence directory of a run that passed every rule.
fn evidence(dir: &Path) {
    fs::write(dir.join("source.log"), format!("{SOURCE}\n")).unwrap();
    fs::write(dir.join("source-status.log"), "").unwrap();
    fs::write(dir.join("source-final.log"), format!("{SOURCE}\n")).unwrap();
    fs::write(dir.join("source-status-final.log"), "").unwrap();
    write(
        &dir.join("run.json"),
        &json!({ "runId": RUN, "source": SOURCE, "udid": UDID,
        "workerSha256": WORKER_SHA }),
    );
    fs::create_dir_all(dir.join("kernel")).unwrap();
    for (name, platform, _, hooks) in BUNDLES {
        write(
            &dir.join("kernel").join(format!("{name}.json")),
            &identity(platform, hooks),
        );
    }
    let text = fs::read_to_string(repo(ios_verify::INVENTORY)).unwrap();
    let (tests, summary) = xcresult(&ios_verify::inventory(&text).unwrap());
    write(&dir.join("xcresult-tests.json"), &tests);
    write(&dir.join("xcresult-summary.json"), &summary);
    let all = expectations();
    fs::create_dir_all(dir.join("mac-authored")).unwrap();
    fs::write(dir.join(MAC_FIXTURE), MAC_BYTES).unwrap();
    fs::create_dir_all(dir.join("artifacts")).unwrap();
    let replay = dir.join("artifacts").join(ios_replay::ARTIFACT);
    write(&replay, &ios_replay::tests::valid(RUN, &kernel()));
    write(
        &replay.with_file_name(format!("{}.provenance.json", ios_replay::ARTIFACT)),
        &json!({"runId":RUN,"kind":"trace","artifact":ios_replay::ARTIFACT,
            "sha256":hash(&fs::read(&replay).unwrap()),"kernelIdentity":kernel()}),
    );
    write(&dir.join("oracle-author.json"), &author_report(&all));
    write(
        &dir.join("oracle-verify.json"),
        &simulator_report(&all, &dir.join("artifacts")),
    );
    for log in ["oracle-author.log", "oracle-verify.log"] {
        fs::write(dir.join(log), ORACLE_LOG).unwrap();
    }
    for (platform, number) in [(Platform::Device, 2), (Platform::Simulator, 7)] {
        let logs = dir.join("release").join(platform.sdk());
        fs::create_dir_all(&logs).unwrap();
        fs::write(logs.join("nm.log"), "_main\n_onecad_ios_app_create\n").unwrap();
        let otool = format!("Load command 9\n cmd LC_BUILD_VERSION\n cmdsize 32\n platform {number}\n minos 18.0\n sdk 26.5\n");
        fs::write(logs.join("otool-load-commands.log"), otool).unwrap();
        fs::write(
            logs.join("otool-libraries.log"),
            "OneCAD:\n\t/usr/lib/libSystem.B.dylib (compatibility version 1.0.0)\n",
        )
        .unwrap();
        let product = ios_evidence::release_product(dir, platform);
        fs::create_dir_all(product.parent().unwrap()).unwrap();
        fs::write(product, b"\0OneCAD\0").unwrap();
    }
}

#[test]
fn ios_synthetic_evidence_passes_and_acceptance_is_written_last() {
    let area = tempfile::tempdir().unwrap();
    evidence(area.path());
    assert_eq!(
        ios_evidence::verify(area.path(), &root()),
        Vec::<String>::new()
    );
    ios_gate::report(area.path(), &root(), true).unwrap();
    let acceptance = ios_evidence::json(&area.path().join("acceptance.json")).unwrap();
    assert_eq!(acceptance["run"]["runId"], RUN);
    type Mutation = fn(&Path);
    let mutations: [(&str, Mutation); 13] = [
        ("no Pencil replay", |d| {
            fs::remove_file(d.join("artifacts/pencil-replay.trace")).unwrap();
            fs::remove_file(d.join("artifacts/pencil-replay.trace.provenance.json")).unwrap();
        }),
        ("unsealed Pencil replay", |d| {
            fs::remove_file(d.join("artifacts/pencil-replay.trace.provenance.json")).unwrap();
        }),
        ("sealed replay with bad transitions", |d| {
            let file = d.join("artifacts/pencil-replay.trace");
            let mut record = ios_evidence::json(&file).unwrap();
            record["scenarios"][3]["observations"][4]["response"]["liveContacts"] = json!(1);
            write(&file, &record);
            let sidecar = d.join("artifacts/pencil-replay.trace.provenance.json");
            let mut provenance = ios_evidence::json(&sidecar).unwrap();
            provenance["sha256"] = json!(hash(&fs::read(&file).unwrap()));
            write(&sidecar, &provenance);
        }),
        ("dirty tree", |d| {
            fs::write(d.join("source-status.log"), " M x\n").unwrap()
        }),
        ("edited during the run", |d| {
            fs::write(d.join("source-status-final.log"), " M x\n").unwrap()
        }),
        ("committed during the run", |d| {
            fs::write(d.join("source-final.log"), "0".repeat(40)).unwrap()
        }),
        ("in-process volume off", |d| {
            let file = d.join("artifacts/plate.json");
            let mut record = ios_evidence::json(&file).unwrap();
            record["summary"]["bodies"][0]["volume"] = json!(REFERENCE * 1.01);
            write(&file, &record);
        }),
        ("no in-process summary", |d| {
            fs::remove_file(d.join("artifacts/plate-chamfer.json")).unwrap()
        }),
        ("no summary", |d| {
            fs::remove_file(d.join("xcresult-summary.json")).unwrap()
        }),
        ("unprovenanced artifact", |d| {
            fs::write(d.join("artifacts/extra.step"), b"x").unwrap()
        }),
        ("release hook", |d| {
            fs::write(
                d.join("release/iphoneos/nm.log"),
                "_onecad_ios_test_command\n",
            )
            .unwrap()
        }),
        ("hook bundle", |d| {
            let file = d.join("kernel/iphoneos-Release.json");
            write(&file, &identity("iphoneos", true));
        }),
        ("oracle did not run", |d| {
            fs::write(d.join("oracle-verify.log"), "").unwrap()
        }),
    ];
    for (label, mutate) in mutations {
        let area = tempfile::tempdir().unwrap();
        evidence(area.path());
        mutate(area.path());
        assert!(
            ios_gate::report(area.path(), &root(), true).is_err(),
            "{label} accepted"
        );
        assert!(
            !area.path().join("acceptance.json").exists(),
            "{label} wrote acceptance"
        );
        let verdict = ios_evidence::json(&area.path().join("verification.json")).unwrap();
        assert_eq!(verdict["passed"], false, "{label}");
    }
}

#[test]
fn ios_gate_refuses_a_used_evidence_directory() {
    let area = tempfile::tempdir().unwrap();
    let verdict = json!({ "passed": true, "failures": [] });
    write(&area.path().join("verification.json"), &verdict);
    let dir = area.path().to_string_lossy().into_owned();
    for args in [vec![dir.clone()], vec!["--provenance-only".into(), dir]] {
        let error = ios_gate::run(&args).unwrap_err().to_string();
        assert!(error.contains("not empty"), "{error}");
    }
    assert!(area.path().join("verification.json").is_file());
}
