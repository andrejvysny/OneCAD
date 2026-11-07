//! Numeric source-intent pipe feature history. This is not interactive sketch UI
//! qualification, a source-patch equivalence claim, or a finished enclosure.
#[path = "framework_open_arc_support/mod.rs"]
mod arc;
#[allow(dead_code)]
#[path = "native_plate_support/host.rs"]
mod host;
use arc::Geometry;

use host::Harness;
use onecad_app::{
    api,
    dto::DocumentProjection,
    worker::{
        wire::{body_id_wire, sketch_wire},
        SolverEngine,
    },
};
use onecad_core::{
    document::{
        record::{
            BooleanMode, KnownOperation, Operation, OperationRecord, PlaneKind, SketchOpParams,
            SketchPlaneRef, SweepParams, SweepPathRange,
        },
        refs::SketchRegionRef,
    },
    edit::EditCommand,
    ids::{BodyId, EntityId, RecordId, RegionId, SketchId},
    math::{Vec2, Vec3},
    sketch::{Sketch, SketchEntity, WorldPlane},
};
use onecad_protocol::mesh::{MeshLimits, MeshView};
use sha2::{Digest, Sha256};
use std::{path::Path, time::Duration};
use uuid::Uuid;

// Declared before native measurement. Numeric tube model only; not source tolerance.
const VOLUME_TOLERANCE_MM3: f64 = 1e-6;
const CENTROID_TOLERANCE_MM: f64 = 1e-6;
// Checked Fine MESH1 is f32 tessellation, not an analytic source oracle.
const MESH_BOUNDS_TOLERANCE_MM: f64 = 0.05;
const RADIUS: f64 = 0.3;
const EDIT_RADIUS: f64 = 0.29;
const REOPEN_RADIUS: f64 = 0.28;
const SOURCE_SHA: &str = "6128186260b22ace271e83f096e2c16707b60866b3aa1b3a3f2ea0e38c3c7c09";
fn id(n: u128) -> Uuid {
    Uuid::from_u128(0xfa90_0000 + n)
}
fn rec(n: u128) -> RecordId {
    RecordId(id(n))
}
fn sk(n: u128) -> SketchId {
    SketchId(id(100 + n))
}
fn body() -> BodyId {
    BodyId(rec(3).0)
}
fn vector(a: [f64; 3]) -> Vec3 {
    Vec3::new(a[0], a[1], a[2]).unwrap()
}
fn frame(origin: [f64; 3], x: [f64; 3], y: [f64; 3], normal: [f64; 3]) -> SketchPlaneRef {
    SketchPlaneRef {
        kind: PlaneKind::Custom,
        origin: vector(origin),
        x_axis: vector(x),
        y_axis: vector(y),
        normal: vector(normal),
        extra: Default::default(),
    }
}

fn geometry(face: u64) -> Geometry {
    let source =
        include_bytes!("../../../fixtures/public-cad/framework/ExpansionCard_ThreadedInsert.stp");
    assert_eq!(format!("{:x}", Sha256::digest(source)), SOURCE_SHA);
    let data: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../fixtures/reference-histories/framework-open-arc-pipes.json"
    ))
    .unwrap();
    assert_eq!(data["sourceSha256"], SOURCE_SHA);
    let row = data["pipes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["sourcePatchFace"] == face)
        .unwrap();
    let arr = |v: &serde_json::Value| {
        [
            v[0].as_f64().unwrap(),
            v[1].as_f64().unwrap(),
            v[2].as_f64().unwrap(),
        ]
    };
    let profile = &row["profile"];
    let expected_volume = row["analyticVolumeMm3"].as_f64().unwrap();
    let geometry = Geometry {
        center: arr(&row["centerMm"]),
        major: arr(&row["majorUnit"]),
        minor: arr(&row["minorUnit"]),
        normal: arr(&row["normalUnit"]),
        profile_center: arr(&profile["centerMm"]),
        profile_x: arr(&profile["xAxisUnit"]),
        profile_y: arr(&profile["yAxisUnit"]),
        profile_normal: arr(&profile["normalUnit"]),
        a: row["semiaxesMm"][0].as_f64().unwrap(),
        b: row["semiaxesMm"][1].as_f64().unwrap(),
        start: row["pathRangeRadians"][0].as_f64().unwrap(),
        end: row["pathRangeRadians"][1].as_f64().unwrap(),
    };
    assert!((arc::expected(&geometry, RADIUS, geometry.end).0 - expected_volume).abs() < 1e-10);
    geometry
}
fn sketch(n: u128, ellipse: bool, radius: f64, g: &Geometry) -> Sketch {
    let mut s = Sketch::on_world_plane(sk(n), "Region oracle", WorldPlane::XY);
    let center = EntityId(id(200 + n));
    let curve = EntityId(id(300 + n));
    s.add_entity(SketchEntity::point(
        center,
        Vec2::new(0., 0.).unwrap(),
        false,
        false,
    ))
    .unwrap();
    let entity = if ellipse {
        SketchEntity::ellipse(curve, center, g.a, g.b, 0., false).unwrap()
    } else {
        SketchEntity::circle(curve, center, radius, false).unwrap()
    };
    s.add_entity(entity).unwrap();
    s
}
fn sketch_operation(s: &Sketch, plane: SketchPlaneRef) -> Operation {
    let (_, entities, constraints) = sketch_wire(s);
    Operation::Known(KnownOperation::Sketch(SketchOpParams {
        sketch: s.id,
        plane,
        entities: entities.as_array().unwrap().clone(),
        constraints: constraints.as_array().unwrap().clone(),
        host_face: None,
        extra: Default::default(),
    }))
}
async fn profile_region(h: &Harness, s: &Sketch) -> SketchRegionRef {
    let worker = h.state.live_worker().unwrap();
    worker.sketch_upsert(s).await.unwrap();
    let result = worker.sketch_regions(&s.id.to_string()).await.unwrap();
    assert_eq!(result.regions.len(), 1);
    SketchRegionRef {
        sketch: s.id,
        region: RegionId::new(&result.regions[0].region_id),
        region_identity_version: Some(result.region_identity_version),
        region_anchor: Some([0., 0.]),
        extra: Default::default(),
    }
}
async fn apply(h: &Harness, command: EditCommand) -> DocumentProjection {
    let response = api::apply_operation(&h.state, h.host.clone(), vec![command], None)
        .await
        .unwrap();
    h.settled(&response.projection).await
}
fn section_frame(g: &Geometry) -> SketchPlaneRef {
    frame(g.profile_center, g.profile_x, g.profile_y, g.profile_normal)
}
async fn add_features(h: &Harness, g: &Geometry) {
    let path = sketch(1, true, RADIUS, g);
    let section = sketch(2, false, RADIUS, g);
    let profile = profile_region(h, &section).await;
    let path_op = sketch_operation(&path, frame(g.center, g.major, g.minor, g.normal));
    let section_op = sketch_operation(&section, section_frame(g));
    for (n, op) in [(1, path_op), (2, section_op)] {
        apply(
            h,
            EditCommand::AddOperation {
                record: OperationRecord::new(rec(n), 0, "Analytic sketch", op),
                at_cursor: true,
            },
        )
        .await;
    }
    let op = Operation::Known(KnownOperation::Sweep(SweepParams {
        profile: Some(profile),
        path_sketch: Some(sk(1)),
        path_edge: None,
        path_range: Some(SweepPathRange {
            start_radians: g.start,
            end_radians: g.end,
        }),
        boolean_mode: BooleanMode::NewBody,
        extra: Default::default(),
    }));
    apply(
        h,
        EditCommand::AddOperation {
            record: OperationRecord::new(rec(3), 2, "Offset ellipse pipe", op),
            at_cursor: true,
        },
    )
    .await;
}
async fn check(h: &Harness, g: &Geometry, r: f64, end: f64) {
    let topology = api::query_body_topology(&h.state, body_id_wire(body()))
        .await
        .unwrap();
    assert_eq!(topology.solid_count, 1);
    let mass = api::query_mass_properties(&h.state, body_id_wire(body()), None)
        .await
        .unwrap();
    let (expected, centroid) = arc::expected(g, r, end);
    assert!(
        (mass.volume - expected).abs() < VOLUME_TOLERANCE_MM3,
        "BRep {} vs analytic {expected}",
        mass.volume
    );
    for (i, expected) in centroid.iter().enumerate() {
        assert!((mass.centroid[i] - expected).abs() < CENTROID_TOLERANCE_MM);
    }
    let bytes = api::get_mesh(&h.state, body().to_string(), "fine".into(), None, None)
        .await
        .unwrap();
    let mesh = MeshView::decode(&bytes, MeshLimits::default()).unwrap();
    assert!(mesh.header().triangle_count > 0);
    let mut measured_low = [f64::INFINITY; 3];
    let mut measured_high = [f64::NEG_INFINITY; 3];
    for tri in 0..mesh.header().triangle_count as usize {
        for vertex in mesh.triangle(tri).unwrap() {
            let p = mesh.position(vertex as usize).unwrap();
            for i in 0..3 {
                measured_low[i] = measured_low[i].min(f64::from(p[i]));
                measured_high[i] = measured_high[i].max(f64::from(p[i]));
            }
        }
    }
    eprintln!(
        "mesh bbox header {:?} {:?}, vertex {:?} {:?}, center {:?}",
        mesh.header().bbox_min,
        mesh.header().bbox_max,
        measured_low,
        measured_high,
        g.center
    );
    let (expected_low, expected_high) = arc::bounds(g, r, end);
    for i in 0..3 {
        let low = measured_low[i];
        let high = measured_high[i];
        assert!(f64::from(mesh.header().bbox_min[i]) <= low);
        assert!(f64::from(mesh.header().bbox_max[i]) >= high);
        assert!(
            (low - expected_low[i]).abs() < MESH_BOUNDS_TOLERANCE_MM,
            "axis {i} low {low}, expected {}",
            expected_low[i]
        );
        assert!(
            (high - expected_high[i]).abs() < MESH_BOUNDS_TOLERANCE_MM,
            "axis {i} high {high}, expected {}",
            expected_high[i]
        );
    }
    eprintln!(
        "ellipse pipe radius {r}: BRep {}, analytic {expected}, center {:?}",
        mass.volume, mass.centroid
    );
}
async fn edit(h: &Harness, g: &Geometry, r: f64, end: f64) {
    let op = sketch_operation(&sketch(2, false, r, g), section_frame(g));
    apply(h, EditCommand::UpdateOperationParams { record: rec(2), op }).await;
    check(h, g, r, end).await;
}
async fn edit_range(h: &Harness, g: &Geometry, end: f64) {
    let mut record = api::get_operation_record(&h.state, rec(3)).await.unwrap();
    let Operation::Known(KnownOperation::Sweep(ref mut params)) = record.op else {
        panic!("stored Sweep")
    };
    params.path_range = Some(SweepPathRange {
        start_radians: g.start,
        end_radians: end,
    });
    apply(
        h,
        EditCommand::UpdateOperationParams {
            record: rec(3),
            op: record.op,
        },
    )
    .await;
}
async fn history(h: &Harness, g: &Geometry, radius: f64, end: f64, redo: bool) {
    let projection = if redo {
        api::redo(&h.state, h.host.clone()).await.unwrap()
    } else {
        api::undo(&h.state, h.host.clone()).await.unwrap()
    };
    h.settled(&projection).await;
    check(h, g, radius, end).await;
}
async fn author(root: &Path, face: u64) {
    let h = Harness::new(&root.join(format!("author-{face}"))).await;
    api::new_document(&h.state, h.host.clone()).await.unwrap();
    assert!(
        h.state
            .live_worker()
            .unwrap()
            .wait_ready(Duration::from_secs(15))
            .await
    );
    let g = geometry(face);
    add_features(&h, &g).await;
    check(&h, &g, RADIUS, g.end).await;
    edit(&h, &g, EDIT_RADIUS, g.end).await;
    history(&h, &g, RADIUS, g.end, false).await;
    history(&h, &g, EDIT_RADIUS, g.end, true).await;
    let shorter = g.end - 0.01;
    edit_range(&h, &g, shorter).await;
    check(&h, &g, EDIT_RADIUS, shorter).await;
    history(&h, &g, EDIT_RADIUS, g.end, false).await;
    history(&h, &g, EDIT_RADIUS, shorter, true).await;
    let path = root.join(format!("open-pipe-{face}.onecad"));
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
    let saved = onecad_core::io::container::ContainerReader::open(&path).unwrap();
    assert!(saved.import_blobs().is_empty());
    assert!(saved.document().sketches.is_empty());
    let records = saved.document().timeline.records();
    let kinds: Vec<_> = records.iter().map(|r| r.op.op_type()).collect();
    assert_eq!(kinds, ["Sketch", "Sketch", "Sweep"]);
    let Operation::Known(KnownOperation::Sweep(params)) = &records[2].op else {
        panic!("stored Sweep")
    };
    let range = params.path_range.unwrap();
    assert_eq!(range.start_radians, g.start);
    assert_eq!(range.end_radians, shorter);
    h.close().await;
}
async fn reopen(root: &Path, face: u64) {
    let h = Harness::new(&root.join(format!("reopen-{face}"))).await;
    api::open_document(
        &h.state,
        h.host.clone(),
        root.join(format!("open-pipe-{face}.onecad"))
            .to_string_lossy()
            .into_owned(),
        None,
    )
    .await
    .unwrap();
    h.settled(&h.projection().await).await;
    let g = geometry(face);
    let end = g.end - 0.01;
    let record = api::get_operation_record(&h.state, rec(3)).await.unwrap();
    let Operation::Known(KnownOperation::Sweep(params)) = record.op else {
        panic!("stored Sweep")
    };
    let range = params.path_range.unwrap();
    assert_eq!((range.start_radians, range.end_radians), (g.start, end));
    check(&h, &g, EDIT_RADIUS, end).await;
    edit(&h, &g, REOPEN_RADIUS, end).await;
    h.close().await;
}
#[test]
fn framework_open_arc_pipe_native_history() {
    host::required_worker();
    if let Ok(phase) = std::env::var("ONECAD_NATIVE_PLATE_TEST_PHASE") {
        if let Some((kind, face)) = phase
            .strip_prefix("framework-open-arc-")
            .and_then(|s| s.split_once('-'))
        {
            let face: u64 = face.parse().unwrap();
            assert!([110, 112].contains(&face));
            let root = std::path::PathBuf::from(
                std::env::var_os("ONECAD_NATIVE_PLATE_TEST_ROOT").unwrap(),
            );
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    match kind {
                        "author" => author(&root, face).await,
                        "reopen" => reopen(&root, face).await,
                        _ => panic!("unknown phase"),
                    }
                });
            return;
        }
    }
    let root = tempfile::tempdir().unwrap();
    for face in [110, 112] {
        for kind in ["author", "reopen"] {
            host::run_isolated_phase(
                root.path(),
                &format!("framework-open-arc-{kind}-{face}"),
                "framework_open_arc_pipe_native_history",
            );
        }
    }
}
