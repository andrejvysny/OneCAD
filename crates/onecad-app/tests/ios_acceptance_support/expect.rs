//! `fixtures/ios/acceptance/expectations.json` and the pure comparison of one
//! regenerated document against it (docs/ios/SIMULATOR_GATE.md, "Geometry
//! tolerance"). `cargo xtask ios-simulator-gate` re-checks the same rules on
//! the oracle report, so a stale or edited report cannot pass on its own.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const EXPECTATIONS: &str = "fixtures/ios/acceptance/expectations.json";

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Analytic {
    pub constant: f64,
    pub pi_coefficient: f64,
}

impl Analytic {
    pub fn value(self) -> f64 {
        self.constant + self.pi_coefficient * std::f64::consts::PI
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Tolerance {
    pub volume_relative: f64,
    pub edit_decrease_relative: f64,
    pub bounds_absolute: f64,
    pub bounds_relative: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Bounds {
    pub min: [f64; 3],
    pub max: [f64; 3],
}

/// The one sketch a case expects: solve state and size of its definition.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SketchExpect {
    pub status: String,
    pub dof: u32,
    pub entities: usize,
    pub constraints: usize,
}

/// One artifact's expectation. Unknown keys are refused: a misspelt rule must
/// never be silently ignored.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Case {
    pub name: String,
    pub source: String,
    pub required: bool,
    #[serde(rename = "description")]
    _description: Option<String>,
    /// What produced the geometry (edge count, length, radius or distance);
    /// the analytic values below are what is checked, and a unit test keeps
    /// them derived from these.
    pub parameters: Option<serde_json::Value>,
    pub features: Vec<String>,
    pub volume: Option<Analytic>,
    pub decrease_from_reference: Option<Analytic>,
    pub bounds: Option<Bounds>,
    pub extents: Option<[f64; 3]>,
    #[serde(default, rename = "extentsXYUnordered")]
    pub extents_xy_unordered: bool,
    pub solids: u32,
    pub sketch: Option<SketchExpect>,
    /// The Mac-authored case this simulator re-save must keep the identity of.
    pub round_trip_of: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReferencePlate {
    pub width: f64,
    pub height: f64,
    pub thickness: f64,
    pub hole_diameter: f64,
    pub volume: Analytic,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Expectations {
    pub version: u32,
    #[serde(rename = "description")]
    _description: String,
    pub tolerance: Tolerance,
    pub reference_plate: ReferencePlate,
    pub artifacts: Vec<Case>,
}

impl Expectations {
    pub fn load() -> (Self, String) {
        let path = repo_root().join(EXPECTATIONS);
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let parsed: Self = serde_json::from_slice(&bytes).expect("expectations.json parses");
        assert_eq!(parsed.version, 1, "expectations version");
        (parsed, super::sha256(&bytes))
    }

    pub fn case(&self, name: &str) -> Option<&Case> {
        self.artifacts.iter().find(|case| case.name == name)
    }
}

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crates/onecad-app sits two levels below the repository root")
        .to_path_buf()
}

/// What the oracle measured on one regenerated document.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Measurement {
    pub occt_fingerprint: String,
    pub document_id: String,
    pub regen_outcome: String,
    /// `live` once a regeneration on this host published the geometry.
    pub geometry_source: String,
    pub features: Vec<FeatureMeasure>,
    pub bodies: Vec<BodyMeasure>,
    pub sketches: Vec<SketchMeasure>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeatureMeasure {
    pub id: String,
    pub op_type: String,
    pub status: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BodyMeasure {
    pub id: String,
    pub healthy: bool,
    pub solids: u32,
    pub faces: u32,
    pub volume: f64,
    pub min: [f64; 3],
    pub max: [f64; 3],
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SketchMeasure {
    pub id: String,
    pub status: String,
    pub dof: Option<u32>,
    pub entities: usize,
    pub constraints: usize,
}

/// Every rule `case` states about `measured`; empty when all hold.
pub fn check(case: &Case, measured: &Measurement, expectations: &Expectations) -> Vec<String> {
    let mut failures = history(case, measured);
    failures.extend(sketch(case, measured));
    let [body] = measured.bodies.as_slice() else {
        failures.push(format!(
            "expected one body, found {}",
            measured.bodies.len()
        ));
        return failures;
    };
    failures.extend(solid(case, body));
    failures.extend(volume(case, body, expectations));
    failures.extend(bounds(case, body, &expectations.tolerance));
    failures
}

fn history(case: &Case, measured: &Measurement) -> Vec<String> {
    let mut failures = Vec::new();
    // Measured geometry must be this host's own regeneration, never the
    // container's cached meshes or an empty plan.
    if measured.regen_outcome != "published" || measured.geometry_source != "live" {
        failures.push(format!(
            "regeneration ended {} with {} geometry",
            measured.regen_outcome, measured.geometry_source
        ));
    }
    let ops: Vec<&str> = measured
        .features
        .iter()
        .map(|f| f.op_type.as_str())
        .collect();
    if ops != case.features {
        failures.push(format!("feature history {ops:?} != {:?}", case.features));
    }
    for feature in measured.features.iter().filter(|f| f.status != "ok") {
        failures.push(format!("feature {} is {}", feature.op_type, feature.status));
    }
    for sketch in measured
        .sketches
        .iter()
        .filter(|s| !matches!(s.status.as_str(), "FullyConstrained" | "UnderConstrained"))
    {
        failures.push(format!(
            "sketch {} constraint status {}",
            sketch.id, sketch.status
        ));
    }
    failures
}

fn sketch(case: &Case, measured: &Measurement) -> Vec<String> {
    let Some(expected) = &case.sketch else {
        return Vec::new();
    };
    match measured.sketches.as_slice() {
        [only]
            if only.status == expected.status
                && only.dof == Some(expected.dof)
                && only.entities == expected.entities
                && only.constraints == expected.constraints =>
        {
            Vec::new()
        }
        sketches => vec![format!("sketches {sketches:?} != one {expected:?}")],
    }
}

/// A re-save keeps the authored document: its id, its feature records and its
/// sketch definitions. Empty when `resaved` is `original` re-saved.
pub fn same_document(original: &Measurement, resaved: &Measurement) -> Vec<String> {
    let features = |m: &Measurement| -> Vec<(String, String)> {
        let pairs = m.features.iter().map(|f| (f.id.clone(), f.op_type.clone()));
        pairs.collect()
    };
    let sketches = |m: &Measurement| -> Vec<(String, usize, usize)> {
        let rows = m
            .sketches
            .iter()
            .map(|s| (s.id.clone(), s.entities, s.constraints));
        rows.collect()
    };
    let mut failures = Vec::new();
    if original.document_id.is_empty() || original.document_id != resaved.document_id {
        failures.push(format!(
            "document id {} is not the authored {}",
            resaved.document_id, original.document_id
        ));
    }
    if features(original) != features(resaved) {
        failures.push("feature records differ from the authored document".into());
    }
    if sketches(original) != sketches(resaved) {
        failures.push("sketch definitions differ from the authored document".into());
    }
    failures
}

fn solid(case: &Case, body: &BodyMeasure) -> Vec<String> {
    let mut failures = Vec::new();
    if !body.healthy {
        failures.push(format!("body {} is not healthy", body.id));
    }
    if body.solids != case.solids {
        failures.push(format!(
            "body has {} solids, expected {}",
            body.solids, case.solids
        ));
    }
    if body.volume.is_nan() || body.volume <= 0.0 {
        failures.push(format!("body volume {} is not positive", body.volume));
    }
    failures
}

fn volume(case: &Case, body: &BodyMeasure, expectations: &Expectations) -> Vec<String> {
    let tolerance = &expectations.tolerance;
    let mut failures = Vec::new();
    if let Some(expected) = case.volume {
        let expected = expected.value();
        if (body.volume - expected).abs() > tolerance.volume_relative * expected.abs() {
            failures.push(format!(
                "volume {} != {expected} within {} relative",
                body.volume, tolerance.volume_relative
            ));
        }
    }
    if let Some(decrease) = case.decrease_from_reference {
        let reference = expectations.reference_plate.volume.value();
        let (measured, expected) = (reference - body.volume, decrease.value());
        if body.volume >= reference {
            failures.push(format!(
                "volume {} is no decrease from {reference}",
                body.volume
            ));
        } else if (measured - expected).abs() > tolerance.edit_decrease_relative * expected.abs() {
            failures.push(format!(
                "volume decrease {measured} != {expected} within {} relative",
                tolerance.edit_decrease_relative
            ));
        }
    }
    failures
}

/// Half an f32 ulp at `value`: what MESH1's f32 storage of a coordinate may
/// add (the next-up gap is never smaller than the rounding step).
pub fn f32_storage(value: f64) -> f64 {
    let stored = (value.abs() as f32).min(f32::MAX);
    f64::from(f32::from_bits(stored.to_bits() + 1) - stored) / 2.0
}

/// Within the declared tolerance plus `storage`, the f32 rounding the
/// measured coordinates carry (docs/ios/SIMULATOR_GATE.md).
fn near(actual: f64, expected: f64, storage: f64, tolerance: &Tolerance) -> bool {
    (actual - expected).abs()
        <= tolerance.bounds_absolute + tolerance.bounds_relative * expected.abs() + storage
}

fn bounds(case: &Case, body: &BodyMeasure, tolerance: &Tolerance) -> Vec<String> {
    let mut failures = Vec::new();
    if let Some(expected) = &case.bounds {
        for axis in 0..3 {
            let (min, max) = (expected.min[axis], expected.max[axis]);
            if !near(body.min[axis], min, f32_storage(min), tolerance)
                || !near(body.max[axis], max, f32_storage(max), tolerance)
            {
                failures.push(format!(
                    "bounds {:?}..{:?} != {:?}..{:?}",
                    body.min, body.max, expected.min, expected.max
                ));
                break;
            }
        }
    }
    if let Some(expected) = case.extents {
        // (extent, f32 storage of both ends)
        let mut actual: [(f64, f64); 3] = std::array::from_fn(|axis| {
            let (min, max) = (body.min[axis], body.max[axis]);
            (max - min, f32_storage(min) + f32_storage(max))
        });
        let mut expected = expected;
        if case.extents_xy_unordered {
            if actual[0].0 > actual[1].0 {
                actual.swap(0, 1);
            }
            if expected[0] > expected[1] {
                expected.swap(0, 1);
            }
        }
        if (0..3).any(|axis| !near(actual[axis].0, expected[axis], actual[axis].1, tolerance)) {
            failures.push(format!("extents {actual:?} != {expected:?}"));
        }
    }
    failures
}

#[cfg(test)]
#[path = "expect_tests.rs"]
mod tests;
