//! Real-worker harness for `boolean_multi_tool.rs` (mirrors `preview_boolean.rs`).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use onecad_core::document::record::{
    BooleanMode, BooleanOp, BooleanParams, ExtrudeMode, ExtrudeParams, KnownOperation, Operation,
    OperationRecord, PlaneKind, SketchOpParams, SketchPlaneRef,
};
use onecad_core::document::refs::SketchRegionRef;
use onecad_core::document::variables::Scalar;
use onecad_core::edit::EditCommand;
use onecad_core::ids::{BodyId, ConstraintId, EntityId, RecordId, RegionId, SketchId};
use onecad_core::io::container::SaveMeta;
use onecad_core::math::{Vec2, Vec3};
use onecad_core::regen::{CancelToken, GeometryEngine, Lod, ModelSnapshot, Outcome, RegenRequest};
use onecad_core::sketch::{Constraint, CurvePosition, Sketch, SketchEntity, WorldPlane};

use onecad_app::document_runtime::{DocumentRuntime, RegenReport};
use onecad_app::worker::manager::SupervisorConfig;
use onecad_app::worker::wire::sketch_wire;
use onecad_app::worker::{resolve_worker_path, MeshProvider, SolverEngine, WorkerManager};

use onecad_protocol::mesh::{f32_le, u32_le, validate_mesh_blob};

pub fn real_worker() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("ONECAD_WORKER_PATH") {
        let path = PathBuf::from(&p);
        assert!(
            path.is_file(),
            "ONECAD_WORKER_PATH={p:?} has no worker binary"
        );
        return Some(path);
    }
    if let Some(path) = resolve_worker_path() {
        return Some(path);
    }
    assert!(
        std::env::var("ONECAD_REQUIRE_WORKER").as_deref() != Ok("1"),
        "ONECAD_REQUIRE_WORKER=1 but no worker binary resolved"
    );
    None
}

pub async fn spawn_worker(bin: PathBuf) -> WorkerManager {
    let wm = WorkerManager::spawn(SupervisorConfig::production(bin));
    assert!(wm.wait_ready(Duration::from_secs(10)).await, "worker ready");
    wm
}

pub fn runtime_over(wm: &WorkerManager) -> DocumentRuntime {
    let engine: Arc<dyn GeometryEngine> = Arc::new(wm.clone());
    let meshes: Arc<dyn MeshProvider> = Arc::new(wm.clone());
    let solver: Arc<dyn SolverEngine> = Arc::new(wm.clone());
    DocumentRuntime::new_blank(engine, meshes, solver)
}

pub fn open_over(wm: &WorkerManager, path: &std::path::Path) -> DocumentRuntime {
    let engine: Arc<dyn GeometryEngine> = Arc::new(wm.clone());
    let meshes: Arc<dyn MeshProvider> = Arc::new(wm.clone());
    let solver: Arc<dyn SolverEngine> = Arc::new(wm.clone());
    DocumentRuntime::open(path, engine, meshes, solver).expect("reopen")
}

pub fn save_meta() -> SaveMeta {
    SaveMeta {
        app_version: "0.1.0-test".into(),
        occt_fingerprint: Some("occt-8.0.1".into()),
        created: "2026-10-21T00:00:00Z".into(),
        modified: "2026-10-21T00:00:00Z".into(),
    }
}

pub fn add_op(rt: &mut DocumentRuntime, record: OperationRecord) {
    rt.apply(EditCommand::AddOperation {
        record,
        at_cursor: true,
    })
    .expect("AddOperation");
}

pub async fn regen_all(rt: &mut DocumentRuntime) -> RegenReport {
    rt.run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await
}

pub fn published<'a>(report: &'a RegenReport, what: &str) -> &'a Arc<ModelSnapshot> {
    match &report.outcome {
        Outcome::Published(s) => s,
        other => panic!("{what}: expected Published, got {other:?}"),
    }
}

pub fn rid(n: u128) -> RecordId {
    RecordId(Uuid::from_u128(n))
}

/// A NewBody extrude's body id is its record id (`body_<opId>`, adopted by Rust).
pub fn body_of(rec: u128) -> BodyId {
    BodyId(Uuid::from_u128(rec))
}

/// Sorted `(body, signature)` of the published head.
pub fn signatures(snapshot: &ModelSnapshot) -> Vec<(String, String)> {
    let mut bodies: Vec<(String, String)> = snapshot
        .bodies
        .iter()
        .map(|b| (b.body.to_string(), b.signature.0.clone()))
        .collect();
    bodies.sort();
    bodies
}

pub fn head(rt: &DocumentRuntime) -> Arc<ModelSnapshot> {
    rt.subscribe_snapshots()
        .borrow()
        .clone()
        .expect("a published head snapshot")
}

fn xy_plane_ref() -> SketchPlaneRef {
    SketchPlaneRef {
        kind: PlaneKind::Xy,
        origin: Vec3::new_unchecked(0.0, 0.0, 0.0),
        x_axis: Vec3::new_unchecked(0.0, 1.0, 0.0),
        y_axis: Vec3::new_unchecked(-1.0, 0.0, 0.0),
        normal: Vec3::new_unchecked(0.0, 0.0, 1.0),
        extra: Default::default(),
    }
}

/// A rectangle `(x0, y0, w, h)` of four lines with coincident corners, a fixed
/// anchor and its two dimensions.
fn rect_sketch(sid: SketchId, base: u128, [x0, y0, w, h]: [f64; 4]) -> Sketch {
    let e = |n: u128| EntityId(Uuid::from_u128(base + n));
    let c = |n: u128| ConstraintId(Uuid::from_u128(base + 0x40 + n));
    let corners = [(x0, y0), (x0 + w, y0), (x0 + w, y0 + h), (x0, y0 + h)];
    let mut sk = Sketch::on_world_plane(sid, "Rect", WorldPlane::XY);
    for i in 0..4u128 {
        let (a, b) = (corners[i as usize], corners[(i as usize + 1) % 4]);
        for (n, (x, y)) in [(2 * i, a), (2 * i + 1, b)] {
            let point = SketchEntity::point(e(n), Vec2::new_unchecked(x, y), false, false);
            sk.add_entity(point).unwrap();
        }
        let line = SketchEntity::line(e(0x10 + i), e(2 * i), e(2 * i + 1), false);
        sk.add_entity(line).unwrap();
    }
    for i in 0..4u128 {
        sk.add_constraint(Constraint::Coincident {
            id: c(i),
            point1: e(2 * i + 1),
            point2: e((2 * i + 2) % 8),
            point1_position: CurvePosition::Arbitrary,
            point2_position: CurvePosition::Arbitrary,
        })
        .unwrap();
    }
    let fixed = Constraint::Fixed {
        id: c(9),
        point: e(0),
        point_position: CurvePosition::Arbitrary,
        at: Vec2::new_unchecked(x0, y0),
    };
    sk.add_constraint(fixed).unwrap();
    sk
}

fn sketch_id(rec: u128) -> SketchId {
    SketchId(Uuid::from_u128(rec | (0x5C << 96)))
}

pub fn sketch_record(rec: u128, rect: [f64; 4]) -> OperationRecord {
    let sk = rect_sketch(sketch_id(rec), (0xE << 96) | (rec << 8), rect);
    let (_plane, entities, constraints) = sketch_wire(&sk);
    let params = SketchOpParams {
        sketch: sk.id,
        plane: xy_plane_ref(),
        entities: entities.as_array().cloned().unwrap_or_default(),
        constraints: constraints.as_array().cloned().unwrap_or_default(),
        host_face: None,
        extra: Default::default(),
    };
    let op = Operation::Known(KnownOperation::Sketch(params));
    OperationRecord::new(rid(rec), 0, "Sketch", op)
}

pub fn extrude_record(rec: u128, sketch_rec: u128, dist: f64) -> OperationRecord {
    let params = ExtrudeParams {
        profile: Some(SketchRegionRef {
            sketch: sketch_id(sketch_rec),
            region: RegionId::new(""), // empty ⇒ V1 first-region fallback
            region_identity_version: None,
            region_anchor: None,
            extra: Default::default(),
        }),
        distance: Scalar::new(dist),
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
    };
    let op = Operation::Known(KnownOperation::Extrude(params));
    OperationRecord::new(rid(rec), 0, "Extrude", op)
}

/// One sketch (record `rec`) + its NewBody extrude (record `rec + 1`).
pub fn add_box(rt: &mut DocumentRuntime, rec: u128, rect: [f64; 4], height: f64) -> BodyId {
    add_op(rt, sketch_record(rec, rect));
    add_op(rt, extrude_record(rec + 1, rec, height));
    body_of(rec + 1)
}

pub fn boolean_op(op: BooleanOp, target: BodyId, tools: &[BodyId], keep: bool) -> Operation {
    let params = BooleanParams::new(op, target, tools.to_vec(), keep);
    Operation::Known(KnownOperation::Boolean(params))
}

pub fn boolean_record(rec: u128, op: Operation) -> OperationRecord {
    OperationRecord::new(rid(rec), 0, "Boolean", op)
}

/// Divergence-theorem volume of the body's MESH1 — exact for planar polyhedra.
pub async fn volume(rt: &mut DocumentRuntime, body: BodyId) -> f64 {
    let blob = rt.get_mesh(body, Lod::Coarse, None).await.expect("mesh");
    blob_volume(&blob)
}

pub fn blob_volume(blob: &[u8]) -> f64 {
    let view = validate_mesh_blob(blob).expect("MESH1 valid");
    let pos = view.section(1).expect("POSITIONS").offset as usize;
    let idx = view.section(3).expect("INDICES").offset as usize;
    let vertex = |i: usize| {
        let o = pos + i * 12;
        [0, 4, 8].map(|d| f64::from(f32_le(blob, o + d)))
    };
    let mut vol6 = 0.0f64;
    for t in 0..view.triangle_count as usize {
        let o = idx + t * 12;
        let [a, b, c] = [0, 4, 8].map(|d| vertex(u32_le(blob, o + d) as usize));
        vol6 += a[0] * (b[1] * c[2] - b[2] * c[1])
            + a[1] * (b[2] * c[0] - b[0] * c[2])
            + a[2] * (b[0] * c[1] - b[1] * c[0]);
    }
    (vol6 / 6.0).abs()
}

pub fn assert_volume(actual: f64, expected: f64, what: &str) {
    assert!(
        (actual - expected).abs() < 1.0,
        "{what}: volume {actual}, expected {expected}"
    );
}
