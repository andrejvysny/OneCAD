//! Source-derived bottom-footprint stock; not a reconstructed Prusa box.
#[allow(dead_code)]
#[path = "native_plate_support/host.rs"]
mod host;

use host::Harness;
use onecad_app::{
    api,
    worker::{
        SolverEngine,
        wire::{body_id_wire, sketch_wire},
    },
};
use onecad_core::{
    document::{
        record::{
            BooleanMode, ExtrudeMode, ExtrudeParams, KnownOperation, Operation, OperationRecord,
            PlaneKind, SketchOpParams, SketchPlaneRef,
        },
        refs::SketchRegionRef,
        variables::Scalar,
    },
    edit::EditCommand,
    ids::{BodyId, EntityId, RecordId, RegionId, SketchId},
    math::{Vec2, Vec3},
    sketch::{Sketch, SketchEntity, WorldPlane},
};
use sha2::{Digest, Sha256};
use uuid::Uuid;

fn id(n: u128) -> Uuid {
    Uuid::from_u128(0x531200 + n)
}
fn point(value: &serde_json::Value) -> Vec2 {
    Vec2::new(value[0].as_f64().unwrap(), value[1].as_f64().unwrap()).unwrap()
}

fn stock_sketch(recipe: &serde_json::Value) -> Sketch {
    let mut sketch = Sketch::on_world_plane(SketchId(id(1)), "Box bottom contour", WorldPlane::XY);
    for (i, curve) in recipe["curves"].as_array().unwrap().iter().enumerate() {
        let base = 100 + i as u128 * 3;
        let first = EntityId(id(base));
        let second = EntityId(id(base + 1));
        let entity = EntityId(id(base + 2));
        match curve["kind"].as_str().unwrap() {
            "line" => {
                sketch
                    .add_entity(SketchEntity::point(
                        first,
                        point(&curve["start"]),
                        false,
                        false,
                    ))
                    .unwrap();
                sketch
                    .add_entity(SketchEntity::point(
                        second,
                        point(&curve["end"]),
                        false,
                        false,
                    ))
                    .unwrap();
                sketch
                    .add_entity(SketchEntity::line(entity, first, second, false))
                    .unwrap();
            }
            "arc" => {
                sketch
                    .add_entity(SketchEntity::point(
                        first,
                        point(&curve["center"]),
                        false,
                        false,
                    ))
                    .unwrap();
                sketch
                    .add_entity(
                        SketchEntity::arc(
                            entity,
                            first,
                            curve["radius"].as_f64().unwrap(),
                            curve["startAngle"].as_f64().unwrap(),
                            curve["endAngle"].as_f64().unwrap(),
                            false,
                        )
                        .unwrap(),
                    )
                    .unwrap();
            }
            kind => panic!("unsupported source contour curve: {kind}"),
        }
    }
    sketch
}

async fn add(h: &Harness, n: u128, op: Operation) {
    let response = api::apply_operation(
        &h.state,
        h.host.clone(),
        vec![EditCommand::AddOperation {
            record: OperationRecord::new(RecordId(id(n)), 0, "Box stock diagnostic", op),
            at_cursor: true,
        }],
        None,
    )
    .await
    .unwrap();
    h.settled(&response.projection).await;
}

async fn stock_profile(h: &Harness, recipe: &serde_json::Value) -> SketchRegionRef {
    let sketch = stock_sketch(recipe);
    let worker = h.state.live_worker().unwrap();
    worker.sketch_upsert(&sketch).await.unwrap();
    let regions = worker.sketch_regions(&sketch.id.to_string()).await.unwrap();
    assert_eq!(
        regions.regions.len(),
        1,
        "source bottom outline must form one region"
    );
    let profile = SketchRegionRef {
        sketch: sketch.id,
        region: RegionId::new(&regions.regions[0].region_id),
        region_identity_version: Some(regions.region_identity_version),
        region_anchor: None,
        extra: Default::default(),
    };
    let (_, entities, constraints) = sketch_wire(&sketch);
    let plane = SketchPlaneRef {
        kind: PlaneKind::Custom,
        origin: Vec3::new(0.0, 0.0, recipe["planeOrigin"][2].as_f64().unwrap()).unwrap(),
        x_axis: Vec3::new(1.0, 0.0, 0.0).unwrap(),
        y_axis: Vec3::new(0.0, 1.0, 0.0).unwrap(),
        normal: Vec3::new(0.0, 0.0, 1.0).unwrap(),
        extra: Default::default(),
    };
    add(
        h,
        2,
        Operation::Known(KnownOperation::Sketch(SketchOpParams {
            sketch: sketch.id,
            plane,
            entities: entities.as_array().unwrap().clone(),
            constraints: constraints.as_array().unwrap().clone(),
            host_face: None,
            extra: Default::default(),
        })),
    )
    .await;
    profile
}

fn stock(profile: SketchRegionRef, distance: f64) -> Operation {
    Operation::Known(KnownOperation::Extrude(ExtrudeParams {
        profile: Some(profile),
        distance: Scalar::new(distance),
        draft_angle_deg: Scalar::new(0.0),
        mode: ExtrudeMode::Blind,
        boolean_mode: BooleanMode::NewBody,
        target_body: None,
        target_face: None,
        two_directions: false,
        mode2: ExtrudeMode::Blind,
        distance2: Scalar::new(0.0),
        target_face2: None,
        extra: Default::default(),
    }))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn source_bottom_contour_stock_is_native_but_incomplete() {
    host::required_worker();
    let source = include_bytes!("../../../fixtures/public-cad/prusa/MINI-fsenzor-box.stp");
    let recipe: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../fixtures/reference-derivatives/prusa/box-native/bottom-stock-recipe.json"
    ))
    .unwrap();
    let diagnostic: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../fixtures/reference-oracles/prusa-sensor.json"
    ))
    .unwrap();
    let expected_sha = diagnostic["parts"]["box"]["sha256"].as_str().unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(source)), expected_sha);
    assert_eq!(recipe["sourceSha256"], expected_sha);
    let dir = tempfile::tempdir().unwrap();
    let h = Harness::new(dir.path()).await;
    api::new_document(&h.state, h.host.clone()).await.unwrap();
    assert!(
        h.state
            .live_worker()
            .unwrap()
            .wait_ready(std::time::Duration::from_secs(15))
            .await
    );
    let profile = stock_profile(&h, &recipe).await;
    add(&h, 3, stock(profile, 13.0)).await;
    let mass = api::query_mass_properties(&h.state, body_id_wire(BodyId(id(3))), None)
        .await
        .unwrap()
        .volume;
    let expected = 13.0 * recipe["outerAreaMm2SameKernel"].as_f64().unwrap();
    assert!(
        (mass - expected).abs() < 1e-4,
        "partial stock volume {mass} vs {expected}"
    );
    eprintln!(
        "incomplete box stock {mass} mm³; source-minus-stock same-kernel {} mm³",
        recipe["sourceMinusStockVolumeMm3SameKernel"]
    );
    h.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn enclosing_stock_is_native_but_not_source_geometry() {
    host::required_worker();
    let source = include_bytes!("../../../fixtures/public-cad/prusa/MINI-fsenzor-box.stp");
    let recipe: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../fixtures/reference-derivatives/prusa/box-native/enclosing-stock-recipe.json"
    ))
    .unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(source)),
        recipe["sourceSha256"].as_str().unwrap()
    );
    let corner = &recipe["corner"];
    let dims = &recipe["dimensions"];
    let x = corner[0].as_f64().unwrap();
    let y = corner[1].as_f64().unwrap();
    let z = corner[2].as_f64().unwrap();
    let width = dims[0].as_f64().unwrap();
    let depth = dims[1].as_f64().unwrap();
    let height = dims[2].as_f64().unwrap();
    let points = [
        [x, y],
        [x + width, y],
        [x + width, y + depth],
        [x, y + depth],
    ];
    let curves: Vec<_> = (0..4)
        .map(|i| serde_json::json!({"kind":"line", "start":points[i], "end":points[(i+1)%4]}))
        .collect();
    let profile_data = serde_json::json!({"curves": curves, "planeOrigin":[0.0,0.0,z]});
    let dir = tempfile::tempdir().unwrap();
    let h = Harness::new(dir.path()).await;
    api::new_document(&h.state, h.host.clone()).await.unwrap();
    assert!(
        h.state
            .live_worker()
            .unwrap()
            .wait_ready(std::time::Duration::from_secs(15))
            .await
    );
    let profile = stock_profile(&h, &profile_data).await;
    add(&h, 3, stock(profile, height)).await;
    let volume = api::query_mass_properties(&h.state, body_id_wire(BodyId(id(3))), None)
        .await
        .unwrap()
        .volume;
    assert!((volume - width * depth * height).abs() < 1e-6);
    eprintln!(
        "enclosing stock {volume} mm³; OCP source-minus-stock {} mm³",
        recipe["sourceMinusStockVolumeMm3SameKernel"]
    );
    h.close().await;
}
