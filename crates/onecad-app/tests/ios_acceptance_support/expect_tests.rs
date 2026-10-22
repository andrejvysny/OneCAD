//! Unit tests of the expectation comparison (`expect.rs`).
use super::*;

fn expectations() -> Expectations {
    Expectations::load().0
}

fn plate(volume: f64) -> Measurement {
    Measurement {
        occt_fingerprint: "0a6a1dce34181289".into(),
        document_id: "doc".into(),
        regen_outcome: "published".into(),
        geometry_source: "live".into(),
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
    shifted.bodies[0].max[1] += 1e-5;
    assert!(
        !check(case, &shifted, &all).is_empty(),
        "bounds 1e-7 abs + 1e-9 rel + f32 storage"
    );
    let mut cached = plate(reference());
    cached.geometry_source = "cached".into();
    assert!(!check(case, &cached, &all).is_empty(), "cached meshes");
    let mut noop = plate(reference());
    noop.regen_outcome = "noop".into();
    assert!(!check(case, &noop, &all).is_empty(), "no regeneration");
}

/// A plate anchored off f32-exact coordinates: its MESH1 bounds carry f32
/// rounding (2.86e-6 on a 60 mm span from 13.7), which the check allows,
/// while a real 1e-5 mm error still fails.
#[test]
fn f32_mesh_storage_is_allowed_and_nothing_more() {
    let all = expectations();
    let case = all.case("plate").unwrap();
    let stored = |value: f64| f64::from(value as f32);
    let mut measured = plate(reference());
    measured.bodies[0].min = [stored(13.7), stored(-12.345), 0.0];
    measured.bodies[0].max = [stored(13.7 + 60.0), stored(-12.345 + 40.0), 4.0];
    assert!(check(case, &measured, &all).is_empty(), "{measured:?}");
    measured.bodies[0].max[0] += 1e-5;
    assert!(!check(case, &measured, &all).is_empty());
    assert_eq!(f32_storage(60.0), 1.9073486328125e-6);
    assert_eq!(f32_storage(0.0), f64::from(f32::from_bits(1)) / 2.0);
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
