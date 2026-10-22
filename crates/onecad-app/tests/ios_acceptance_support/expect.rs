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
    if !matches!(measured.regen_outcome.as_str(), "published" | "noop") {
        failures.push(format!("regeneration ended {}", measured.regen_outcome));
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

fn near(actual: f64, expected: f64, tolerance: &Tolerance) -> bool {
    (actual - expected).abs()
        <= tolerance.bounds_absolute + tolerance.bounds_relative * expected.abs()
}

fn bounds(case: &Case, body: &BodyMeasure, tolerance: &Tolerance) -> Vec<String> {
    let mut failures = Vec::new();
    if let Some(expected) = &case.bounds {
        for axis in 0..3 {
            if !near(body.min[axis], expected.min[axis], tolerance)
                || !near(body.max[axis], expected.max[axis], tolerance)
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
        let mut actual: [f64; 3] = std::array::from_fn(|axis| body.max[axis] - body.min[axis]);
        let mut expected = expected;
        if case.extents_xy_unordered {
            sort_xy(&mut actual);
            sort_xy(&mut expected);
        }
        if (0..3).any(|axis| !near(actual[axis], expected[axis], tolerance)) {
            failures.push(format!("extents {actual:?} != {expected:?}"));
        }
    }
    failures
}

fn sort_xy(extents: &mut [f64; 3]) {
    if extents[0] > extents[1] {
        extents.swap(0, 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expectations() -> Expectations {
        Expectations::load().0
    }

    fn plate(volume: f64) -> Measurement {
        Measurement {
            occt_fingerprint: "0a6a1dce34181289".into(),
            document_id: "doc".into(),
            regen_outcome: "published".into(),
            features: ["Sketch", "Extrude"]
                .map(|op| FeatureMeasure {
                    id: format!("{op}-id"),
                    op_type: op.into(),
                    status: "ok".into(),
                })
                .to_vec(),
            bodies: vec![BodyMeasure {
                id: "body".into(),
                healthy: true,
                solids: 1,
                faces: 7,
                volume,
                min: [-40.0, 0.0, 0.0],
                max: [0.0, 60.0, 4.0],
            }],
            sketches: vec![SketchMeasure {
                id: "sketch".into(),
                status: "FullyConstrained".into(),
                dof: Some(0),
                entities: 14,
                constraints: 13,
            }],
        }
    }

    fn reference() -> f64 {
        9600.0 - 36.0 * std::f64::consts::PI
    }

    #[test]
    fn the_reference_plate_passes_within_tolerance_and_fails_outside_it() {
        let all = expectations();
        let case = all.case("mac-plate").unwrap();
        assert!(check(case, &plate(reference()), &all).is_empty());
        assert!(check(case, &plate(reference() * (1.0 + 5e-7)), &all).is_empty());
        assert!(!check(case, &plate(reference() * (1.0 + 2e-6)), &all).is_empty());
        let mut shifted = plate(reference());
        shifted.bodies[0].max[1] += 2e-7;
        assert!(
            !check(case, &shifted, &all).is_empty(),
            "bounds 1e-7 abs + 1e-9 rel"
        );
    }

    #[test]
    fn edits_need_a_strict_analytic_decrease() {
        let all = expectations();
        let fillet = all.case("plate-fillet").unwrap();
        let decrease = 4.0 - std::f64::consts::PI;
        let mut measured = plate(reference() - decrease);
        measured.features.push(FeatureMeasure {
            id: "fillet".into(),
            op_type: "Fillet".into(),
            status: "ok".into(),
        });
        assert!(check(fillet, &measured, &all).is_empty());
        measured.bodies[0].volume = reference() - decrease * (1.0 + 2e-4);
        assert!(!check(fillet, &measured, &all).is_empty());
        measured.bodies[0].volume = reference();
        assert!(!check(fillet, &measured, &all).is_empty(), "no decrease");
    }

    #[test]
    fn history_solid_and_constraint_integrity_are_required() {
        let all = expectations();
        let case = all.case("plate").unwrap();
        let mut measured = plate(reference());
        measured.bodies[0].min = [0.0, -40.0, 0.0];
        measured.bodies[0].max = [60.0, 0.0, 4.0];
        assert!(
            check(case, &measured, &all).is_empty(),
            "xy extents unordered"
        );
        let mut broken = measured.clone();
        broken.features[1].status = "needsRepair".into();
        assert!(!check(case, &broken, &all).is_empty());
        let mut broken = measured.clone();
        broken.bodies[0].solids = 2;
        assert!(!check(case, &broken, &all).is_empty());
        let mut broken = measured.clone();
        broken.sketches[0].status = "OverConstrained".into();
        assert!(!check(case, &broken, &all).is_empty());
        let mut broken = measured;
        broken.bodies.push(broken.bodies[0].clone());
        assert!(!check(case, &broken, &all).is_empty());
        let mac = all.case("mac-plate").unwrap();
        assert!(check(mac, &plate(reference()), &all).is_empty());
        let mut under = plate(reference());
        under.sketches[0].status = "UnderConstrained".into();
        under.sketches[0].dof = Some(1);
        assert!(
            !check(mac, &under, &all).is_empty(),
            "authored sketch is fully constrained"
        );
    }

    #[test]
    fn a_resave_must_keep_the_authored_document() {
        let original = plate(reference());
        assert!(same_document(&original, &original.clone()).is_empty());
        let mut other = original.clone();
        other.document_id = "fresh".into();
        assert!(
            !same_document(&original, &other).is_empty(),
            "a fresh plate"
        );
        let mut other = original.clone();
        other.features[1].id = "rebuilt".into();
        assert!(!same_document(&original, &other).is_empty());
        let mut other = original.clone();
        other.sketches[0].constraints = 12;
        assert!(!same_document(&original, &other).is_empty());
    }

    /// The fixture's analytic values are the closed forms of its recorded
    /// geometry, independently of the oracle's measurements.
    #[test]
    fn analytic_expectations_follow_from_the_recorded_parameters() {
        use std::f64::consts::PI;
        let all = expectations();
        let plate = &all.reference_plate;
        let radius = plate.hole_diameter / 2.0;
        let volume = (plate.width * plate.height - PI * radius * radius) * plate.thickness;
        assert!((plate.volume.value() - volume).abs() < 1e-9);
        let sizes = [plate.width, plate.height, plate.thickness];
        assert_eq!(
            sizes,
            [
                super::super::plate::WIDTH,
                super::super::plate::HEIGHT,
                super::super::plate::THICKNESS
            ]
        );
        assert_eq!(plate.hole_diameter, super::super::plate::HOLE_DIAMETER);
        for case in all
            .artifacts
            .iter()
            .filter(|c| c.decrease_from_reference.is_some())
        {
            let p = case
                .parameters
                .as_ref()
                .expect("an edit case records its parameters");
            let length = p["edgeLength"].as_f64().unwrap() * p["edges"].as_f64().unwrap();
            let expected = match (p["radius"].as_f64(), p["distance"].as_f64()) {
                (Some(r), None) => (1.0 - PI / 4.0) * r * r * length,
                (None, Some(d)) => d * d * length / 2.0,
                _ => panic!("{}: radius or distance", case.name),
            };
            let declared = case.decrease_from_reference.unwrap().value();
            assert!((declared - expected).abs() < 1e-12, "{}", case.name);
        }
    }

    #[test]
    fn misspelt_rules_are_refused() {
        let text = std::fs::read_to_string(repo_root().join(EXPECTATIONS)).unwrap();
        let misspelt = text.replace("extentsXYUnordered", "extentsXyUnordered");
        assert!(serde_json::from_str::<Expectations>(&misspelt).is_err());
        let case = expectations();
        assert!(case.case("plate").unwrap().extents_xy_unordered);
    }
}
