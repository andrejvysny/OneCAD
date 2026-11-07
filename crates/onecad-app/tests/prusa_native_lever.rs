//! Source-derived native lever history; same-kernel diagnostics are not an independent oracle.
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
use onecad_core::io::container::ContainerReader;
use onecad_core::{
    document::{
        record::{
            BooleanMode, ExtrudeMode, ExtrudeParams, KnownOperation, Operation, OperationRecord,
            PlaneKind, RevolveParams, SketchOpParams, SketchPlaneRef,
        },
        refs::{AxisRef, SketchRegionRef},
        variables::Scalar,
    },
    edit::EditCommand,
    ids::{BodyId, EntityId, RecordId, RegionId, SketchId},
    math::{Vec2, Vec3},
    sketch::{Sketch, SketchEntity, WorldPlane},
};
use onecad_protocol::mesh::{MeshLimits, MeshView};
use sha2::{Digest, Sha256};
use uuid::Uuid;

fn source_diagnostic() -> serde_json::Value {
    let data: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../fixtures/reference-oracles/prusa-sensor.json"
    ))
    .unwrap();
    data["parts"]["lever"].clone()
}
fn id(n: u128) -> Uuid {
    Uuid::from_u128(0x531000 + n)
}
fn point(v: &serde_json::Value) -> Vec2 {
    Vec2::new(v[0].as_f64().unwrap(), v[1].as_f64().unwrap()).unwrap()
}

#[path = "prusa_native_lever_support/construction.rs"]
mod construction;
#[path = "prusa_native_lever_support/lifecycle.rs"]
mod lifecycle;
use construction::{add, apply_remaining_features, outer_sketch};
use lifecycle::{assert_full_geometry, exercise_history, reopen_and_edit};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_lever_history_and_fresh_process_reopen() {
    assert_eq!(std::env::var("ONECAD_REQUIRE_WORKER").as_deref(), Ok("1"));
    if let Some(path) = std::env::var_os("ONECAD_LEVER_REOPEN") {
        reopen_and_edit(std::path::Path::new(&path)).await;
        return;
    }
    let source = include_bytes!("../../../fixtures/public-cad/prusa/MINI-fsenzor-lever.stp");
    let diagnostic = source_diagnostic();
    let source_sha = diagnostic["sha256"].as_str().unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(source)), source_sha);
    let data: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../fixtures/reference-derivatives/prusa/lever-native/recipe.json"
    ))
    .unwrap();
    assert_eq!(data["sourceSha256"], source_sha);
    let dir = tempfile::tempdir().unwrap();
    let h = Harness::new(dir.path()).await;
    api::new_document(&h.state, h.host.clone()).await.unwrap();
    let worker = h.state.live_worker().unwrap();
    assert!(worker.wait_ready(std::time::Duration::from_secs(15)).await);
    create_stock(&h, &data).await;
    apply_remaining_features(&h, &data).await;
    compare_source_volume(&h, &diagnostic).await;
    assert_full_geometry(&h).await;
    exercise_history(&h).await;
    let path = dir.path().join("native-lever.onecad");
    save_and_validate(&h, &path).await;
    capture_artifact(&path);
    h.close().await;
    run_reopen(&path);
}

async fn create_stock(h: &Harness, data: &serde_json::Value) {
    let worker = h.state.live_worker().unwrap();
    let sketch = outer_sketch(data);
    worker.sketch_upsert(&sketch).await.unwrap();
    let regions = worker.sketch_regions(&sketch.id.to_string()).await.unwrap();
    assert_eq!(
        regions.regions.len(),
        1,
        "outer contour must form exactly one region"
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
        origin: Vec3::new(0.0, 4.00000005960464, 0.0).unwrap(),
        x_axis: Vec3::new(1.0, 0.0, 0.0).unwrap(),
        y_axis: Vec3::new(0.0, 0.0, 1.0).unwrap(),
        normal: Vec3::new(0.0, -1.0, 0.0).unwrap(),
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
    add(h, 3, stock_extrude(profile)).await;
    let mass = api::query_mass_properties(&h.state, body_id_wire(BodyId(id(3))), None)
        .await
        .unwrap();
    let expected = 8.0 * data["outerAreaMm2SameKernel"].as_f64().unwrap();
    assert!(
        (mass.volume - expected).abs() < 1e-4,
        "partial outer volume {} vs {expected}",
        mass.volume
    );
}

fn stock_extrude(profile: SketchRegionRef) -> Operation {
    Operation::Known(KnownOperation::Extrude(ExtrudeParams {
        profile: Some(profile),
        distance: Scalar::new(8.0),
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

async fn save_and_validate(h: &Harness, path: &std::path::Path) {
    assert!(
        api::save_document(
            &h.state,
            h.host.clone(),
            Some(path.to_string_lossy().into_owned()),
            None
        )
        .await
        .unwrap()
        .clean
    );
    let saved = ContainerReader::open(path).unwrap();
    assert!(saved.import_blobs().is_empty());
    assert!(saved.library_archives().is_empty());
    assert_eq!(saved.document().timeline.records().len(), 8);
    assert!(saved.document().timeline.records().iter().all(|r| matches!(
        r.op,
        Operation::Known(
            KnownOperation::Sketch(_) | KnownOperation::Extrude(_) | KnownOperation::Revolve(_)
        )
    )));
}

fn run_reopen(path: &std::path::Path) {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "native_lever_history_and_fresh_process_reopen",
            "--nocapture",
        ])
        .env("ONECAD_LEVER_REOPEN", path)
        .output()
        .unwrap();
    eprintln!(
        "fresh process stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    eprintln!(
        "fresh process stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.status.success(),
        "fresh process reopen failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn compare_source_volume(h: &Harness, diagnostic: &serde_json::Value) {
    let actual = api::query_mass_properties(&h.state, body_id_wire(BodyId(id(3))), None)
        .await
        .unwrap()
        .volume;
    let expected = diagnostic["volumeMm3"].as_f64().unwrap();
    eprintln!("native BRep volume {actual}; source diagnostic {expected}");
    assert!((actual - expected).abs() <= expected * 0.02);
}

fn capture_artifact(path: &std::path::Path) {
    use std::io::Write;
    let Some(destination) = std::env::var_os("ONECAD_LEVER_CAPTURE") else {
        return;
    };
    let bytes = std::fs::read(path).unwrap();
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&destination)
        .unwrap();
    file.write_all(&bytes).unwrap();
    file.sync_all().unwrap();
    eprintln!(
        "native capture {} SHA-256 {:x}",
        std::path::Path::new(&destination).display(),
        Sha256::digest(&bytes)
    );
}
