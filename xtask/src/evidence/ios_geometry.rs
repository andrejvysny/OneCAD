//! Re-check of the Mac geometry oracle's report (step 8) against
//! `fixtures/ios/acceptance/expectations.json`: the gate does not take the
//! oracle's `passed` on trust. Same rules as the oracle's own comparison
//! (crates/onecad-app/tests/ios_acceptance_support/expect.rs).
use super::{captures::hash, Result};
use serde_json::Value;
use std::{collections::BTreeSet, fs, path::Path};

pub(super) const EXPECTATIONS: &str = "fixtures/ios/acceptance/expectations.json";
/// OCCT fingerprint of the staged Mac sidecar (the one `native-host-gate` pins).
pub(super) const MAC_OCCT_FINGERPRINT: &str = "0a6a1dce34181289";

fn analytic(value: &Value) -> Option<f64> {
    Some(value["constant"].as_f64()? + value["piCoefficient"].as_f64()? * std::f64::consts::PI)
}

fn number(value: &Value) -> Result<f64> {
    value.as_f64().ok_or_else(|| "missing number".into())
}

fn triple(value: &Value) -> Result<[f64; 3]> {
    let items = value
        .as_array()
        .filter(|a| a.len() == 3)
        .ok_or("expected [x, y, z]")?;
    Ok([number(&items[0])?, number(&items[1])?, number(&items[2])?])
}

/// The case named `name` in `expectations`.
pub(super) fn case<'a>(expectations: &'a Value, name: &str) -> Option<&'a Value> {
    expectations["artifacts"]
        .as_array()?
        .iter()
        .find(|case| case["name"] == name)
}

/// Names of required artifacts the simulator must save.
pub(super) fn required_simulator(expectations: &Value) -> Vec<String> {
    expectations["artifacts"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .filter(|case| case["required"] == true && case["source"] == "simulator")
        .filter_map(|case| case["name"].as_str().map(str::to_string))
        .collect()
}

/// One measurement (`{features, bodies, sketches, regenOutcome}`) against one case.
pub(super) fn check(case: &Value, measured: &Value, expectations: &Value) -> Result<()> {
    // The Mac host's own regeneration, never cached meshes or an empty plan.
    if measured["regenOutcome"] != "published" || measured["geometrySource"] != "live" {
        return Err(format!(
            "regeneration ended {} with {} geometry",
            measured["regenOutcome"], measured["geometrySource"]
        )
        .into());
    }
    if measured["occtFingerprint"] != MAC_OCCT_FINGERPRINT {
        return Err("not measured by the Mac sidecar's OCCT".into());
    }
    sketches(case, measured)?;
    model(case, measured, expectations)
}

/// History, the one body, its volume and bounds: the rules a Mac oracle
/// measurement and the simulator kernel's own summary share.
fn model(case: &Value, measured: &Value, expectations: &Value) -> Result<()> {
    let features = measured["features"].as_array().ok_or("features")?;
    let ops: Vec<&Value> = features.iter().map(|f| &f["opType"]).collect();
    let expected: Vec<&Value> = case["features"]
        .as_array()
        .ok_or("case features")?
        .iter()
        .collect();
    if ops != expected || features.iter().any(|f| f["status"] != "ok") {
        return Err(
            format!("feature history {ops:?} differs from {expected:?} or is not ok").into(),
        );
    }
    let [body] = measured["bodies"].as_array().ok_or("bodies")?.as_slice() else {
        return Err("expected exactly one body".into());
    };
    if body["healthy"] != true || body["solids"] != case["solids"] {
        return Err("body is not one healthy solid of the expected count".into());
    }
    volume(case, body, expectations)?;
    bounds(case, body, &expectations["tolerance"])
}

/// The simulator kernel's own measurement of each required model: the
/// `write_semantic` record of the same name (`summary`, in-process mass
/// properties and tessellation) meets the case like the Mac regeneration.
pub(super) fn verify_in_process(artifacts: &Path, expectations: &Value) -> Result<()> {
    for name in required_simulator(expectations) {
        let file = artifacts.join(format!("{name}.json"));
        let record: Value = serde_json::from_slice(
            &fs::read(&file).map_err(|_| format!("{name}.json (in-process summary) missing"))?,
        )?;
        let case = case(expectations, &name).ok_or("case")?;
        if record["summary"]["document"]["geometrySource"] != "live" {
            return Err(format!("{name} in-process: geometry is not a live regeneration").into());
        }
        model(case, &record["summary"], expectations)
            .map_err(|e| format!("{name} in-process: {e}"))?;
    }
    Ok(())
}

/// Constraint integrity: every sketch solves without over-constraint or
/// conflict; a case's `sketch` fixes the one sketch's state and size.
fn sketches(case: &Value, measured: &Value) -> Result<()> {
    let sketches = measured["sketches"].as_array().ok_or("sketches")?;
    let solvable = |s: &Value| {
        matches!(
            s["status"].as_str(),
            Some("FullyConstrained" | "UnderConstrained")
        )
    };
    if !sketches.iter().all(solvable) {
        return Err("a sketch is over-constrained, conflicting or unsolved".into());
    }
    let expected = &case["sketch"];
    if expected.is_null() {
        return Ok(());
    }
    let matches = |s: &Value| {
        ["status", "dof", "entities", "constraints"]
            .iter()
            .all(|key| s[key] == expected[key])
    };
    match sketches.as_slice() {
        [only] if matches(only) => Ok(()),
        _ => Err(format!("sketches differ from one {expected}").into()),
    }
}

fn volume(case: &Value, body: &Value, expectations: &Value) -> Result<()> {
    let tolerance = &expectations["tolerance"];
    let measured = number(&body["volume"])?;
    if measured.is_nan() || measured <= 0.0 {
        return Err("volume is not positive".into());
    }
    if let Some(expected) = analytic(&case["volume"]) {
        if (measured - expected).abs() > number(&tolerance["volumeRelative"])? * expected.abs() {
            return Err(format!("volume {measured} outside tolerance of {expected}").into());
        }
    }
    if let Some(decrease) = analytic(&case["decreaseFromReference"]) {
        let reference =
            analytic(&expectations["referencePlate"]["volume"]).ok_or("reference volume")?;
        let actual = reference - measured;
        let limit = number(&tolerance["editDecreaseRelative"])? * decrease.abs();
        if measured >= reference || (actual - decrease).abs() > limit {
            return Err(format!("volume decrease {actual} outside tolerance of {decrease}").into());
        }
    }
    Ok(())
}

/// Half an f32 ulp at `value`: what MESH1's f32 storage of a coordinate may
/// add (the oracle's `expect::f32_storage`).
pub(super) fn f32_storage(value: f64) -> f64 {
    let stored = (value.abs() as f32).min(f32::MAX);
    f64::from(f32::from_bits(stored.to_bits() + 1) - stored) / 2.0
}

fn bounds(case: &Value, body: &Value, tolerance: &Value) -> Result<()> {
    let (absolute, relative) = (
        number(&tolerance["boundsAbsolute"])?,
        number(&tolerance["boundsRelative"])?,
    );
    let near =
        |a: f64, e: f64, storage: f64| (a - e).abs() <= absolute + relative * e.abs() + storage;
    let (min, max) = (triple(&body["min"])?, triple(&body["max"])?);
    if !case["bounds"].is_null() {
        let (emin, emax) = (
            triple(&case["bounds"]["min"])?,
            triple(&case["bounds"]["max"])?,
        );
        let off = |axis: usize| {
            !near(min[axis], emin[axis], f32_storage(emin[axis]))
                || !near(max[axis], emax[axis], f32_storage(emax[axis]))
        };
        if (0..3).any(off) {
            return Err(format!("bounds {min:?}..{max:?} outside tolerance").into());
        }
    }
    if !case["extents"].is_null() {
        // (extent, f32 storage of both ends)
        let mut actual: [(f64, f64); 3] = std::array::from_fn(|axis| {
            (
                max[axis] - min[axis],
                f32_storage(min[axis]) + f32_storage(max[axis]),
            )
        });
        let mut expected = triple(&case["extents"])?;
        if case["extentsXYUnordered"] == true {
            if actual[0].0 > actual[1].0 {
                actual.swap(0, 1);
            }
            if expected[0] > expected[1] {
                expected.swap(0, 1);
            }
        }
        if (0..3).any(|axis| !near(actual[axis].0, expected[axis], actual[axis].1)) {
            return Err(format!("extents {actual:?} outside tolerance of {expected:?}").into());
        }
    }
    Ok(())
}

fn checked_artifact(artifact: &Value, expectations: &Value, source: &str) -> Result<String> {
    let name = artifact["name"]
        .as_str()
        .ok_or("oracle artifact without name")?;
    let case = case(expectations, name).filter(|c| c["source"] == source);
    let case =
        case.ok_or_else(|| format!("oracle checked {name} without a {source} expectation"))?;
    if artifact["failures"]
        .as_array()
        .is_none_or(|f| !f.is_empty())
    {
        return Err(format!("oracle reported failures for {name}").into());
    }
    check(case, &artifact["measurement"], expectations).map_err(|e| format!("{name}: {e}"))?;
    Ok(name.to_string())
}

/// The author run: `mac-plate.onecad` exists with the reported hash and passes.
pub(super) fn verify_author_report(
    report: &Value,
    expectations: &Value,
    fixture: &Path,
) -> Result<()> {
    let artifact = &report["macFixture"]["artifact"];
    checked_artifact(artifact, expectations, "mac")?;
    if report["passed"] != true || artifact["sha256"] != hash(&fs::read(fixture)?) {
        return Err("Mac fixture report does not describe the authored fixture".into());
    }
    Ok(())
}

/// A re-save (`roundTripOf`) keeps the authored document (id, feature records,
/// sketch definitions) and is not its byte copy.
fn round_trip(artifact: &Value, author: &Value, fixture_sha: &str) -> Result<()> {
    let identity = |m: &Value| {
        let features: Vec<_> = m["features"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|f| (f["id"].clone(), f["opType"].clone()))
            .collect();
        let sketches: Vec<_> = m["sketches"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|s| {
                (
                    s["id"].clone(),
                    s["entities"].clone(),
                    s["constraints"].clone(),
                )
            })
            .collect();
        (m["documentId"].clone(), features, sketches)
    };
    let original = &author["macFixture"]["artifact"]["measurement"];
    let resaved = &artifact["measurement"];
    if !original["documentId"].is_string() || identity(original) != identity(resaved) {
        return Err("the re-save is not the authored Mac document".into());
    }
    if artifact["roundTrip"]["fixtureSha256"] != fixture_sha || artifact["sha256"] == fixture_sha {
        return Err("the re-save does not name the authored fixture or is its byte copy".into());
    }
    Ok(())
}

/// The verify run: exactly the copied simulator documents, hashes matching,
/// every required one present and in tolerance, re-saves keeping the authored
/// document of `author` (the author run's report).
pub(super) fn verify_simulator_report(
    report: &Value,
    author: &Value,
    expectations: &Value,
    artifacts: &Path,
) -> Result<()> {
    let simulator = &report["simulator"];
    let empty = |key: &str| simulator[key].as_array().is_some_and(Vec::is_empty);
    if report["passed"] != true || !empty("missing") || !empty("unexpected") {
        return Err("oracle verify run did not pass".into());
    }
    let mut checked = BTreeSet::new();
    for artifact in simulator["artifacts"]
        .as_array()
        .ok_or("oracle artifacts")?
    {
        let name = checked_artifact(artifact, expectations, "simulator")?;
        if case(expectations, &name).is_some_and(|c| !c["roundTripOf"].is_null()) {
            let fixture_sha = author["macFixture"]["artifact"]["sha256"]
                .as_str()
                .unwrap_or_default();
            round_trip(artifact, author, fixture_sha).map_err(|e| format!("{name}: {e}"))?;
        }
        let file = artifacts.join(format!("{name}.onecad"));
        if artifact["sha256"]
            != hash(&fs::read(&file).map_err(|_| format!("{name}.onecad not copied"))?)
        {
            return Err(format!("oracle measured another {name}.onecad").into());
        }
        checked.insert(name);
    }
    let copied: BTreeSet<String> = fs::read_dir(artifacts)?
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter_map(|name| name.strip_suffix(".onecad").map(str::to_string))
        .collect();
    let required: BTreeSet<String> = required_simulator(expectations).into_iter().collect();
    if checked != copied || !required.is_subset(&checked) {
        return Err("oracle did not check exactly the copied documents, or one is missing".into());
    }
    Ok(())
}

/// The report's provenance: these expectations and this Mac sidecar.
pub(super) fn verify_report_provenance(
    report: &Value,
    expectations_sha: &str,
    worker_sha: &str,
) -> Result<()> {
    if report["expectationsSha256"] != expectations_sha || report["workerSha256"] != worker_sha {
        return Err("oracle report was produced with other expectations or another worker".into());
    }
    Ok(())
}
