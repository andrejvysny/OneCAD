//! Source-derived native cover history and separate incomplete top-stock diagnostic.
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

#[path = "prusa_native_cover_support/construction.rs"]
mod construction;
#[path = "prusa_native_cover_support/features.rs"]
mod features;
#[path = "prusa_native_cover_support/lifecycle.rs"]
mod lifecycle;
#[path = "prusa_native_cover_support/mesh_topology.rs"]
mod mesh_topology;
#[path = "prusa_native_cover_support/validation.rs"]
mod validation;
use construction::{
    apply_prism, bevel_profile, create_profile, cut_polygon, frame, profile_from_points,
    remaining_perimeter_and_tab,
};
use features::{fastener_site, hinge_cut, underside_recess};
use lifecycle::{exercise_history, reopen_and_edit, run_fresh_process};
use validation::{save_native_capture, validate_cover};

fn id(n: u128) -> Uuid {
    Uuid::from_u128(0x531100 + n)
}
fn point(value: &serde_json::Value) -> Vec2 {
    Vec2::new(value[0].as_f64().unwrap(), value[1].as_f64().unwrap()).unwrap()
}

fn feature_label(n: u128) -> &'static str {
    match n {
        2 => "Cover base outline",
        3 => "Cover base extrusion",
        11 | 21 | 31 | 41 => "Perimeter bevel profile",
        12 | 22 | 32 | 42 => "Perimeter bevel cut",
        51 => "Mounting tab profile",
        52 => "Mounting tab extrusion",
        61 => "Underside recess profile",
        62 => "Underside recess cut",
        71 => "Hinge profile",
        72 => "Hinge revolve cut",
        101 | 201 => "Fastener counterbore profile",
        102 | 202 => "Fastener counterbore cut",
        111 | 211 => "Fastener clipped-circle profile",
        112 | 212 => "Fastener clipped-circle cut",
        121 | 221 => "Fastener square profile",
        122 | 222 => "Fastener square cut",
        131 | 231 => "Fastener through-hole profile",
        132 | 232 => "Fastener through-hole cut",
        _ => "Cover feature",
    }
}

async fn add(h: &Harness, n: u128, op: Operation) {
    let response = api::apply_operation(
        &h.state,
        h.host.clone(),
        vec![EditCommand::AddOperation {
            record: OperationRecord::new(RecordId(id(n)), 0, feature_label(n), op),
            at_cursor: true,
        }],
        None,
    )
    .await
    .unwrap();
    h.settled(&response.projection).await;
}

fn stock(profile: SketchRegionRef) -> Operation {
    Operation::Known(KnownOperation::Extrude(ExtrudeParams {
        profile: Some(profile),
        distance: Scalar::new(4.0),
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
async fn source_top_contour_stock_is_native_but_incomplete() {
    host::required_worker();
    let source = include_bytes!("../../../fixtures/public-cad/prusa/MINI-fsenzor-cover.stp");
    let data: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../fixtures/reference-derivatives/prusa/cover-native/stock-recipe.json"
    ))
    .unwrap();
    let diagnostic: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../fixtures/reference-oracles/prusa-sensor.json"
    ))
    .unwrap();
    let expected_sha = diagnostic["parts"]["cover"]["sha256"].as_str().unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(source)), expected_sha);
    assert_eq!(data["sourceSha256"], expected_sha);
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
    let profile = create_profile(&h, &data, 1).await;
    add(&h, 3, stock(profile)).await;
    let mass = api::query_mass_properties(&h.state, body_id_wire(BodyId(id(3))), None)
        .await
        .unwrap();
    let expected = 4.0 * data["outerAreaMm2SameKernel"].as_f64().unwrap();
    assert!(
        (mass.volume - expected).abs() < 1e-4,
        "stock volume {} vs {expected}",
        mass.volume
    );
    eprintln!(
        "incomplete native top-contour stock: {} mm³; source cover diagnostic {} mm³",
        mass.volume, diagnostic["parts"]["cover"]["volumeMm3"]
    );
    h.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn source_derived_cover_native_history() {
    host::required_worker();
    if let Some(path) = std::env::var_os("ONECAD_COVER_REOPEN") {
        reopen_and_edit(std::path::Path::new(&path)).await;
        return;
    }
    let data: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../fixtures/reference-derivatives/prusa/cover-native/lower-stock-recipe.json"
    ))
    .unwrap();
    let extraction: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../fixtures/reference-derivatives/prusa/cover-native/geometry-extraction.json"
    ))
    .unwrap();
    let source = include_bytes!("../../../fixtures/public-cad/prusa/MINI-fsenzor-cover.stp");
    assert_eq!(
        format!("{:x}", Sha256::digest(source)),
        data["sourceSha256"].as_str().unwrap()
    );
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
    lower_stock_and_first_bevel(&h, &data, &extraction).await;
    remaining_perimeter_and_tab(&h, &extraction).await;
    underside_recess(&h).await;
    hinge_cut(&h, &extraction).await;
    fastener_site(&h, &extraction, 41, 50, 100).await;
    fastener_site(&h, &extraction, 28, 49, 200).await;
    validate_cover(&h).await;
    let volume = exercise_history(&h).await;
    let path = save_native_capture(&h, dir.path()).await;
    h.close().await;
    run_fresh_process(&path, volume);
}

async fn lower_stock_and_first_bevel(
    h: &Harness,
    data: &serde_json::Value,
    extraction: &serde_json::Value,
) {
    let profile = create_profile(h, data, 1).await;
    add(h, 3, stock(profile)).await;
    let before = api::query_mass_properties(&h.state, body_id_wire(BodyId(id(3))), None)
        .await
        .unwrap()
        .volume;
    let expected = 4.0 * data["outerAreaMm2SameKernel"].as_f64().unwrap();
    assert!(
        (before - expected).abs() < 1e-4,
        "lower stock volume {before} vs {expected}"
    );
    let face = &extraction["faces"][3];
    let profile = bevel_profile(h, face).await;
    let mut cut = match stock(profile) {
        Operation::Known(KnownOperation::Extrude(params)) => params,
        _ => unreachable!(),
    };
    cut.distance = Scalar::new(6.0);
    cut.boolean_mode = BooleanMode::Cut;
    cut.target_body = Some(BodyId(id(3)));
    add(h, 12, Operation::Known(KnownOperation::Extrude(cut))).await;
    let after = api::query_mass_properties(&h.state, body_id_wire(BodyId(id(3))), None)
        .await
        .unwrap()
        .volume;
    let edge = &face["wires"][0][0];
    let width = (edge["start"][0].as_f64().unwrap() - edge["end"][0].as_f64().unwrap()).abs();
    let height = (edge["start"][1].as_f64().unwrap() - edge["end"][1].as_f64().unwrap()).abs();
    let removed = width * height * 3.0;
    assert!(
        (before - after - removed).abs() < 1e-4,
        "first bevel removes {} vs {removed}",
        before - after
    );
    eprintln!(
        "incomplete lower stock volume {before}; first bevel volume {after}; source-minus-stock same-kernel {} mm³",
        data["sourceMinusStockVolumeMm3SameKernel"]
    );
}
