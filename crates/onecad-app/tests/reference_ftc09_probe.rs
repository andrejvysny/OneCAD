//! Explicit real-worker feasibility probe; not a completed FTC09 qualification.
#[allow(dead_code)]
mod native_plate_support;
mod reference_ftc09_support;

use reference_ftc09_support::{countersinks, pocket, verify_geometry, verify_source};

use native_plate_support::Harness;
use onecad_app::{api, worker::ElementQuery};
use onecad_core::{
    document::{
        datum::DatumPlane,
        hole_placement::HolePlacement,
        record::{BooleanMode, HoleParams, HoleType, KnownOperation, Operation, OperationRecord},
        refs::{AnchorIntent, ElementKind, ElementRef, PrimaryRef},
        variables::Scalar,
    },
    edit::{EditCommand, SketchEditOp},
    ids::{BodyId, DatumPlaneId, ElementId, EntityId, RecordId, RegionId, SketchId, TopoKey},
    math::{Vec2, Vec3},
    sketch::{Sketch, SketchAttachment, SketchEntity, WorldPlane},
};
use serde_json::Value;
use std::path::{Path, PathBuf};
use uuid::Uuid;

fn id(n: u128) -> EntityId {
    EntityId(Uuid::from_u128(0xf7c09000 + n))
}
fn point(v: &Value) -> Vec2 {
    Vec2::new_unchecked(-v[1].as_f64().unwrap(), -v[0].as_f64().unwrap())
}
fn entities(wires: &Value) -> Vec<SketchEditOp> {
    let mut result = Vec::new();
    let mut n = 0u128;
    for wire in wires.as_array().unwrap() {
        for curve in wire.as_array().unwrap() {
            let kind = curve["kind"].as_str().unwrap();
            let first = id(n);
            n += 1;
            let coordinate = if kind == "line" {
                &curve["start"]
            } else {
                &curve["center"]
            };
            result.push(SketchEditOp::AddEntity {
                entity: SketchEntity::point(first, point(coordinate), false, false),
            });
            let second = id(n);
            n += 1;
            let entity = if kind == "line" {
                result.push(SketchEditOp::AddEntity {
                    entity: SketchEntity::point(second, point(&curve["end"]), false, false),
                });
                let line = id(n);
                n += 1;
                SketchEntity::line(line, first, second, false)
            } else if kind == "arc" {
                SketchEntity::arc(
                    second,
                    first,
                    curve["radius"].as_f64().unwrap(),
                    -std::f64::consts::FRAC_PI_2 - curve["end"].as_f64().unwrap(),
                    -std::f64::consts::FRAC_PI_2 - curve["start"].as_f64().unwrap(),
                    false,
                )
                .unwrap()
            } else {
                assert_eq!(kind, "circle");
                SketchEntity::circle(second, first, curve["radius"].as_f64().unwrap(), false)
                    .unwrap()
            };
            result.push(SketchEditOp::AddEntity { entity });
        }
    }
    result
}

#[test]
fn source_alignment_is_a_proper_rotation() {
    let plane = WorldPlane::XY.plane();
    let map = |p: &Value| {
        let p = point(p);
        [
            plane.x_axis.x * p.x + plane.y_axis.x * p.y,
            plane.x_axis.y * p.x + plane.y_axis.y * p.y,
            0.0,
        ]
    };
    let x = map(&serde_json::json!([1.0, 0.0]));
    let y = [plane.normal.x, plane.normal.y, plane.normal.z];
    let z = map(&serde_json::json!([0.0, 1.0]));
    assert_eq!(
        (x, y, z),
        ([1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0])
    );
    let cross = [
        y[1] * z[2] - y[2] * z[1],
        y[2] * z[0] - y[0] * z[2],
        y[0] * z[1] - y[1] * z[0],
    ];
    assert_eq!(x[0] * cross[0] + x[1] * cross[1] + x[2] * cross[2], 1.0);
}

#[test]
fn ftc09_editable_base_profile_probe() {
    native_plate_support::required_worker();
    if let Ok(phase) = std::env::var("ONECAD_NATIVE_PLATE_TEST_PHASE") {
        let root = PathBuf::from(std::env::var_os("ONECAD_NATIVE_PLATE_TEST_ROOT").unwrap());
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                match phase.as_str() {
                    "create" => create(&root).await,
                    "reopen" => reopen(&root).await,
                    _ => panic!("unknown phase"),
                }
            });
        return;
    }
    let root = tempfile::tempdir().unwrap();
    for phase in ["create", "reopen"] {
        native_plate_support::run_isolated_phase(
            root.path(),
            phase,
            "ftc09_editable_base_profile_probe",
        );
    }
}

async fn create(root: &Path) {
    verify_source();
    let harness = Harness::new(&root.join("create-host")).await;
    api::new_document(&harness.state, harness.host.clone())
        .await
        .unwrap();
    assert!(
        harness
            .state
            .live_worker()
            .unwrap()
            .wait_ready(std::time::Duration::from_secs(15))
            .await
    );
    let sid = SketchId(Uuid::from_u128(0xf7c09001));
    let sketch = Sketch::on_world_plane(sid, "NIST FTC09 editable base", WorldPlane::XY);
    api::apply_edit_command(
        &harness.state,
        harness.host.clone(),
        EditCommand::AddSketch { sketch },
    )
    .await
    .unwrap();
    api::enter_sketch(
        &harness.state,
        harness.host.clone(),
        sid.to_string(),
        Some(true),
    )
    .await
    .unwrap();
    let recipe: Value = serde_json::from_str(include_str!(
        "../../../fixtures/reference-histories/nist-ftc09-curves.json"
    ))
    .unwrap();
    let centers: Vec<_> = recipe["countersinks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| &c["center"])
        .collect();
    let wires: Vec<_> = recipe["bottomWires"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|wire| {
            let curves = wire.as_array().unwrap();
            !(curves.len() == 1
                && curves[0]["kind"] == "circle"
                && centers.contains(&&curves[0]["center"]))
        })
        .cloned()
        .collect();
    assert_eq!(wires.len(), 31);
    let ops = entities(&Value::Array(wires));
    eprintln!("FTC09 editable entities: {}", ops.len());
    let solve = api::sketch_upsert(&harness.state, harness.host.clone(), sid.to_string(), ops)
        .await
        .unwrap();
    eprintln!("FTC09 solve: {:?}, DOF {}", solve.status, solve.dof);
    let result = api::finish_sketch(&harness.state, harness.host.clone(), sid.to_string())
        .await
        .unwrap();
    eprintln!(
        "FTC09 regions: {}, holes: {:?}",
        result.regions.len(),
        result
            .regions
            .iter()
            .map(|r| r.holes.len())
            .collect::<Vec<_>>()
    );
    assert!(
        result.regions.iter().any(|r| r.holes.len() == 30),
        "FTC09 base region must retain 30 holes before drilling countersinks"
    );
    harness.settled(&harness.projection().await).await;
    let base = result.regions.iter().find(|r| r.holes.len() == 30).unwrap();
    let mut operation = native_plate_support::profile::extrude(&result, 3.03784);
    let Operation::Known(KnownOperation::Extrude(params)) = &mut operation else {
        unreachable!()
    };
    let profile = params.profile.as_mut().unwrap();
    profile.sketch = sid;
    profile.region = RegionId::new(&base.region_id);
    params.distance = Scalar::new(3.03784);
    let applied = api::apply_operation(
        &harness.state,
        harness.host.clone(),
        vec![EditCommand::AddOperation {
            record: OperationRecord::new(
                RecordId(Uuid::from_u128(0xf7c09002)),
                1,
                "FTC09 base",
                operation,
            ),
            at_cursor: true,
        }],
        None,
    )
    .await
    .unwrap();
    harness.settled(&applied.projection).await;
    let projection = harness.projection().await;
    assert_eq!(
        projection.bodies.len(),
        1,
        "FTC09 base must regenerate one solid"
    );
    let body: BodyId = projection.bodies.keys().next().unwrap().parse().unwrap();
    let mass = api::query_mass_properties(
        &harness.state,
        onecad_app::worker::wire::body_id_wire(body),
        None,
    )
    .await
    .unwrap();
    eprintln!(
        "FTC09 base volume: {} mm³; area: {} mm²",
        mass.volume, mass.surface_area
    );
    for index in 0..2 {
        pocket(
            &harness,
            &recipe["blindPocketWires"][index],
            body,
            index as u128,
        )
        .await;
    }
    let mass = api::query_mass_properties(
        &harness.state,
        onecad_app::worker::wire::body_id_wire(body),
        None,
    )
    .await
    .unwrap();
    eprintln!(
        "FTC09 base with pockets volume: {} mm³; area: {} mm²",
        mass.volume, mass.surface_area
    );
    countersinks(&harness, &recipe, body).await;
    verify_geometry(&harness, body).await;
    let mass = api::query_mass_properties(
        &harness.state,
        onecad_app::worker::wire::body_id_wire(body),
        None,
    )
    .await
    .unwrap();
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../fixtures/reference-oracles/nist-ftc09.json"
    ))
    .unwrap();
    eprintln!(
        "FTC09 native volume: {} mm³; area: {} mm²; oracle: {}",
        mass.volume, mass.surface_area, oracle
    );
    let tolerance = oracle["trialRelativeTolerance"].as_f64().unwrap();
    for (actual, expected) in [
        (mass.volume, oracle["volumeMm3"].as_f64().unwrap()),
        (
            mass.surface_area,
            oracle["surfaceAreaMm2"].as_f64().unwrap(),
        ),
    ] {
        assert!(
            (actual - expected).abs() <= expected * tolerance,
            "actual {actual}, expected {expected}, relative tolerance {tolerance}"
        );
    }
    let undone = api::undo(&harness.state, harness.host.clone())
        .await
        .unwrap();
    harness.settled(&undone).await;
    let redone = api::redo(&harness.state, harness.host.clone())
        .await
        .unwrap();
    harness.settled(&redone).await;
    check_mass(&harness, body).await;
    let saved = api::save_document(
        &harness.state,
        harness.host.clone(),
        Some(root.join("ftc09.onecad").to_string_lossy().into_owned()),
        None,
    )
    .await
    .unwrap();
    assert!(saved.clean);
    harness.close().await;
}

async fn check_mass(h: &Harness, body: BodyId) {
    let mass =
        api::query_mass_properties(&h.state, onecad_app::worker::wire::body_id_wire(body), None)
            .await
            .unwrap();
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../fixtures/reference-oracles/nist-ftc09.json"
    ))
    .unwrap();
    let tolerance = oracle["trialRelativeTolerance"].as_f64().unwrap();
    for (actual, expected) in [
        (mass.volume, oracle["volumeMm3"].as_f64().unwrap()),
        (
            mass.surface_area,
            oracle["surfaceAreaMm2"].as_f64().unwrap(),
        ),
    ] {
        assert!(
            (actual - expected).abs() <= expected * tolerance,
            "actual {actual}, expected {expected}, relative tolerance {tolerance}"
        );
    }
}

async fn reopen(root: &Path) {
    let h = Harness::new(&root.join("reopen-host")).await;
    api::open_document(
        &h.state,
        h.host.clone(),
        root.join("ftc09.onecad").to_string_lossy().into_owned(),
        None,
    )
    .await
    .unwrap();
    h.settled(&h.projection().await).await;
    let projection = h.projection().await;
    assert_eq!(projection.bodies.len(), 1);
    let kinds: Vec<_> = projection
        .features
        .iter()
        .map(|feature| feature.op_type.as_str())
        .collect();
    assert_eq!(kinds.iter().filter(|kind| **kind == "Sketch").count(), 3);
    assert_eq!(kinds.iter().filter(|kind| **kind == "Extrude").count(), 3);
    assert_eq!(kinds.iter().filter(|kind| **kind == "Hole").count(), 1);
    assert!(kinds
        .iter()
        .all(|kind| ["Sketch", "Extrude", "Hole"].contains(kind)));
    assert!(h
        .state
        .runtime
        .lock()
        .await
        .as_ref()
        .unwrap()
        .import_blob_shas()
        .is_empty());
    let body = projection.bodies.keys().next().unwrap().parse().unwrap();
    check_mass(&h, body).await;
    let id = RecordId(Uuid::from_u128(0xf7c09400));
    let mut operation = api::get_operation_record(&h.state, id).await.unwrap().op;
    let mut invalid = operation.clone();
    let Operation::Known(KnownOperation::Hole(params)) = &mut invalid else {
        panic!("expected native Hole")
    };
    params.cs_diameter = Some(Scalar::new(1.0));
    let before = h.projection().await;
    assert!(api::apply_operation(
        &h.state,
        h.host.clone(),
        vec![EditCommand::UpdateOperationParams {
            record: id,
            op: invalid
        }],
        None
    )
    .await
    .is_err());
    let after = h.projection().await;
    assert_eq!(before.revision, after.revision);
    check_mass(&h, body).await;
    let Operation::Known(KnownOperation::Hole(params)) = &mut operation else {
        panic!("expected native Hole")
    };
    params.cs_diameter = Some(Scalar::new(8.0));
    let edited = api::apply_operation(
        &h.state,
        h.host.clone(),
        vec![EditCommand::UpdateOperationParams {
            record: id,
            op: operation,
        }],
        None,
    )
    .await
    .unwrap();
    h.settled(&edited.projection).await;
    let edited_mass =
        api::query_mass_properties(&h.state, onecad_app::worker::wire::body_id_wire(body), None)
            .await
            .unwrap();
    assert!(edited_mass.volume < 136445.10481135413);
    let undone = api::undo(&h.state, h.host.clone()).await.unwrap();
    h.settled(&undone).await;
    check_mass(&h, body).await;
    let redone = api::redo(&h.state, h.host.clone()).await.unwrap();
    h.settled(&redone).await;
    let redone_mass =
        api::query_mass_properties(&h.state, onecad_app::worker::wire::body_id_wire(body), None)
            .await
            .unwrap();
    assert_eq!(redone_mass.volume, edited_mass.volume);
    h.close().await;
}
