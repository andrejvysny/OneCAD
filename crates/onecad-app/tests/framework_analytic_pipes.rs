//! Numeric source-intent pipe feature history. This is not interactive sketch UI
//! qualification, a source-patch equivalence claim, or a finished enclosure.
#[allow(dead_code)]
#[path = "native_plate_support/host.rs"]
mod host;

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
            SketchPlaneRef, SweepParams,
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
use std::{f64::consts::PI, path::Path, time::Duration};
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
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|i| a[i] * b[i]).sum()
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn unit(a: [f64; 3]) -> [f64; 3] {
    let n = dot(a, a).sqrt();
    assert!(n > 0.0);
    a.map(|x| x / n)
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

struct Geometry {
    center: [f64; 3],
    major: [f64; 3],
    minor: [f64; 3],
    normal: [f64; 3],
    a: f64,
    b: f64,
}
fn geometry() -> Geometry {
    let source =
        include_bytes!("../../../fixtures/public-cad/framework/ExpansionCard_ThreadedInsert.stp");
    assert_eq!(format!("{:x}", Sha256::digest(source)), SOURCE_SHA);
    let data: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../fixtures/reference-histories/framework-enclosure-profiles.json"
    ))
    .unwrap();
    assert_eq!(data["sourceSha256"], SOURCE_SHA);
    let plane = data["clipCutPlanes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["faceIndex"] == 111)
        .unwrap();
    let arr = |v: &serde_json::Value| {
        [
            v[0].as_f64().unwrap(),
            v[1].as_f64().unwrap(),
            v[2].as_f64().unwrap(),
        ]
    };
    let transform = |p: [f64; 3]| [p[0], -p[2], p[1]];
    let origin = transform(arr(&plane["origin"]));
    let normal = unit(transform(arr(&plane["normal"])));
    // Analytic source cylinder #109: its axis and radius, captured from pinned STEP.
    let cylinder_origin = [13.5, 31.06781301227, 3.920976627276];
    let axis = [0., -1., 0.];
    let t = (-0.3
        + dot(
            normal,
            std::array::from_fn(|i| origin[i] - cylinder_origin[i]),
        ))
        / dot(normal, axis);
    let center = std::array::from_fn(|i| cylinder_origin[i] + t * axis[i]);
    let minor = unit(cross(axis, normal));
    let major = unit(cross(minor, normal));
    Geometry {
        center,
        major,
        minor,
        normal,
        a: 0.6 / dot(normal, axis).abs(),
        b: 0.6,
    }
}
fn expected_volume(g: &Geometry, r: f64) -> f64 {
    fn perimeter(g: &Geometry, n: usize) -> f64 {
        let h = 2. * PI / n as f64;
        let speed = |t: f64| (g.a * t.sin()).hypot(g.b * t.cos());
        h / 3.
            * (speed(0.)
                + speed(2. * PI)
                + (1..n)
                    .map(|i| {
                        if i % 2 == 0 {
                            2. * speed(i as f64 * h)
                        } else {
                            4. * speed(i as f64 * h)
                        }
                    })
                    .sum::<f64>())
    }
    let l = perimeter(g, 65536);
    assert!((l - perimeter(g, 32768)).abs() < 1e-10);
    PI * r * r * l
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
    let origin = std::array::from_fn(|i| g.center[i] + g.a * g.major[i]);
    frame(origin, g.major, cross(g.minor, g.major), g.minor)
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
async fn check(h: &Harness, g: &Geometry, r: f64) {
    let topology = api::query_body_topology(&h.state, body_id_wire(body()))
        .await
        .unwrap();
    assert_eq!(topology.solid_count, 1);
    let mass = api::query_mass_properties(&h.state, body_id_wire(body()), None)
        .await
        .unwrap();
    let expected = expected_volume(g, r);
    assert!(
        (mass.volume - expected).abs() < VOLUME_TOLERANCE_MM3,
        "BRep {} vs analytic {expected}",
        mass.volume
    );
    for i in 0..3 {
        assert!((mass.centroid[i] - g.center[i]).abs() < CENTROID_TOLERANCE_MM);
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
    for i in 0..3 {
        let extent = (g.a * g.major[i]).hypot(g.b * g.minor[i]) + r;
        let low = measured_low[i];
        let high = measured_high[i];
        assert!(f64::from(mesh.header().bbox_min[i]) <= low);
        assert!(f64::from(mesh.header().bbox_max[i]) >= high);
        assert!(
            (low - (g.center[i] - extent)).abs() < MESH_BOUNDS_TOLERANCE_MM,
            "axis {i} low {low}, expected {}",
            g.center[i] - extent
        );
        assert!(
            (high - (g.center[i] + extent)).abs() < MESH_BOUNDS_TOLERANCE_MM,
            "axis {i} high {high}, expected {}",
            g.center[i] + extent
        );
    }
    eprintln!(
        "ellipse pipe radius {r}: BRep {}, analytic {expected}, center {:?}",
        mass.volume, mass.centroid
    );
}
async fn edit(h: &Harness, g: &Geometry, r: f64) {
    let op = sketch_operation(&sketch(2, false, r, g), section_frame(g));
    apply(h, EditCommand::UpdateOperationParams { record: rec(2), op }).await;
    check(h, g, r).await;
}
async fn author(root: &Path) {
    let h = Harness::new(&root.join("author")).await;
    api::new_document(&h.state, h.host.clone()).await.unwrap();
    assert!(
        h.state
            .live_worker()
            .unwrap()
            .wait_ready(Duration::from_secs(15))
            .await
    );
    let g = geometry();
    add_features(&h, &g).await;
    check(&h, &g, RADIUS).await;
    edit(&h, &g, EDIT_RADIUS).await;
    let p = api::undo(&h.state, h.host.clone()).await.unwrap();
    h.settled(&p).await;
    check(&h, &g, RADIUS).await;
    let p = api::redo(&h.state, h.host.clone()).await.unwrap();
    h.settled(&p).await;
    check(&h, &g, EDIT_RADIUS).await;
    assert!(
        api::save_document(
            &h.state,
            h.host.clone(),
            Some(
                root.join("ellipse-pipe.onecad")
                    .to_string_lossy()
                    .into_owned()
            ),
            None
        )
        .await
        .unwrap()
        .clean
    );
    let saved =
        onecad_core::io::container::ContainerReader::open(&root.join("ellipse-pipe.onecad"))
            .unwrap();
    assert!(saved.import_blobs().is_empty());
    assert!(
        saved.document().sketches.is_empty(),
        "numeric operation records do not claim interactive sketch authority"
    );
    let kinds: Vec<_> = saved
        .document()
        .timeline
        .records()
        .iter()
        .map(|record| record.op.op_type())
        .collect();
    assert_eq!(kinds, ["Sketch", "Sketch", "Sweep"]);
    h.close().await;
}
async fn reopen(root: &Path) {
    let h = Harness::new(&root.join("reopen")).await;
    api::open_document(
        &h.state,
        h.host.clone(),
        root.join("ellipse-pipe.onecad")
            .to_string_lossy()
            .into_owned(),
        None,
    )
    .await
    .unwrap();
    h.settled(&h.projection().await).await;
    let g = geometry();
    check(&h, &g, EDIT_RADIUS).await;
    edit(&h, &g, REOPEN_RADIUS).await;
    h.close().await;
}
#[test]
fn framework_offset_ellipse_pipe_native_history() {
    host::required_worker();
    match std::env::var("ONECAD_NATIVE_PLATE_TEST_PHASE")
        .ok()
        .as_deref()
    {
        Some(phase @ ("framework-pipe-author" | "framework-pipe-reopen")) => {
            let root = std::path::PathBuf::from(
                std::env::var_os("ONECAD_NATIVE_PLATE_TEST_ROOT").unwrap(),
            );
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    if phase == "framework-pipe-author" {
                        author(&root).await
                    } else {
                        reopen(&root).await
                    }
                });
        }
        _ => {
            let root = tempfile::tempdir().unwrap();
            for phase in ["framework-pipe-author", "framework-pipe-reopen"] {
                host::run_isolated_phase(
                    root.path(),
                    phase,
                    "framework_offset_ellipse_pipe_native_history",
                );
            }
        }
    }
}
