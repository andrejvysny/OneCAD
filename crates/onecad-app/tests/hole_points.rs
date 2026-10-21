//! ONECAD-361 ordered multi-point `Hole` (SCHEMA §7.3 `Hole.points`,
//! `resultPolicyVersion: 3`) against the REAL C++ OCCT worker, driven through
//! the app's [`DocumentRuntime`] exactly like `hole_ops.rs`.
//!
//! * `lifecycle` — exact volumes for many through holes and a blind counterbore
//!   set in ONE step, preview ≡ commit, a single bad point refusing the whole
//!   feature with `reasonCode`/`pointIndex`, an expression-driven overlap the
//!   worker refuses, Undo/Redo across 1 → 3 → 2 placements, and save → fresh
//!   worker → reopen with a byte-stable `document.json`.
//! * `identity` — a Chamfer on one hole's rim, promoted through the real
//!   `AcquireElementIds` path, stays on THAT hole across sibling moves and
//!   removals, follows its own hole, and goes NeedsRepair when its hole is
//!   removed — never a sibling; the same holds after save → fresh worker →
//!   reopen.
//!
//! Gated on `ONECAD_WORKER_PATH` (else the dev-tree fallback); a missing binary
//! skips cleanly unless `ONECAD_REQUIRE_WORKER=1`.

mod hole_points {
    mod identity;
    mod lifecycle;
}

use std::f64::consts::PI;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use onecad_core::document::record::{
    BooleanMode, ExtrudeMode, ExtrudeParams, HoleParams, HolePlacement, HoleType, KnownOperation,
    Operation, OperationRecord, PlaneKind, SketchOpParams, SketchPlaneRef,
};
use onecad_core::document::refs::{
    AnchorIntent, ElementKind, ElementRef, PrimaryRef, SketchRegionRef,
};
use onecad_core::document::variables::Scalar;
use onecad_core::edit::EditCommand;
use onecad_core::ids::{
    BodyId, ConstraintId, ElementId, EntityId, RecordId, RegionId, SketchId, TopoKey,
};
use onecad_core::io::container::SaveMeta;
use onecad_core::math::{Vec2, Vec3};
use onecad_core::regen::{CancelToken, GeometryEngine, ModelSnapshot, Outcome, RegenRequest};
use onecad_core::sketch::{Constraint, CurvePosition, Sketch, SketchEntity, WorldPlane};

use onecad_app::document_runtime::{DocumentRuntime, RegenReport};
use onecad_app::dto::FeatureDto;
use onecad_app::worker::manager::SupervisorConfig;
use onecad_app::worker::wire::sketch_wire;
use onecad_app::worker::{
    resolve_worker_path, ElementQuery, MeshProvider, SolverEngine, WorkerManager,
};

// Plate A: sketch u ∈ [0,60], v ∈ [0,20], 10 mm thick. With the XY sketch
// basis {x:(0,1,0), y:(−1,0,0)} that is world x ∈ [−20,0], y ∈ [0,60], top z=10.
pub(crate) const SKETCH_A: u128 = 0xA00;
pub(crate) const EXTRUDE_A: u128 = 0xA01;
pub(crate) const SKETCH_B: u128 = 0xB00;
pub(crate) const EXTRUDE_B: u128 = 0xB01;
pub(crate) const HOLE_A: u128 = 0xF01;
pub(crate) const HOLE_B: u128 = 0xF02;
pub(crate) const CHAMFER: u128 = 0xC01;
pub(crate) const PLATE_VOLUME: f64 = 60.0 * 20.0 * 10.0;
pub(crate) const TOP: f64 = 10.0;

pub(crate) fn real_worker() -> Option<PathBuf> {
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

pub(crate) async fn spawn_worker(bin: PathBuf) -> WorkerManager {
    let wm = WorkerManager::spawn(SupervisorConfig::production(bin));
    assert!(wm.wait_ready(Duration::from_secs(10)).await, "worker ready");
    wm
}

pub(crate) fn runtime_over(wm: &WorkerManager) -> DocumentRuntime {
    let engine: Arc<dyn GeometryEngine> = Arc::new(wm.clone());
    let meshes: Arc<dyn MeshProvider> = Arc::new(wm.clone());
    let solver: Arc<dyn SolverEngine> = Arc::new(wm.clone());
    DocumentRuntime::new_blank(engine, meshes, solver)
}

pub(crate) fn open_over(wm: &WorkerManager, path: &Path) -> DocumentRuntime {
    let engine: Arc<dyn GeometryEngine> = Arc::new(wm.clone());
    let meshes: Arc<dyn MeshProvider> = Arc::new(wm.clone());
    let solver: Arc<dyn SolverEngine> = Arc::new(wm.clone());
    DocumentRuntime::open(path, engine, meshes, solver).expect("reopen the saved container")
}

pub(crate) fn add_op(rt: &mut DocumentRuntime, record: OperationRecord) {
    rt.apply(EditCommand::AddOperation {
        record,
        at_cursor: true,
    })
    .expect("AddOperation");
}

/// A clean replay (no `editedFrom` claim).
pub(crate) async fn clean_regen(rt: &mut DocumentRuntime) -> RegenReport {
    rt.run_regen(RegenRequest::RevertToEnd { from: 0 }, CancelToken::new())
        .await
}

/// The edit lane: a content edit landed at step `from` (SCHEMA §7.2 `editedFrom`).
pub(crate) async fn regen_from(rt: &mut DocumentRuntime, from: usize) -> RegenReport {
    rt.run_regen(RegenRequest::ToEnd { from }, CancelToken::new())
        .await
}

pub(crate) fn published<'a>(report: &'a RegenReport, what: &str) -> &'a Arc<ModelSnapshot> {
    match &report.outcome {
        Outcome::Published(s) => s,
        other => panic!("{what}: expected Published, got {other:?}"),
    }
}

pub(crate) fn save_meta() -> SaveMeta {
    SaveMeta {
        app_version: "0.1.0-test".into(),
        occt_fingerprint: Some("occt-7.9.3".into()),
        created: "2026-10-21T00:00:00Z".into(),
        modified: "2026-10-21T00:00:00Z".into(),
    }
}

pub(crate) fn document_json(path: &Path) -> Vec<u8> {
    use std::io::Read;
    let file = std::fs::File::open(path).expect("open container");
    let mut zip = zip::ZipArchive::new(file).expect("read container zip");
    let mut entry = zip.by_name("document.json").expect("document.json entry");
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes).expect("read document.json");
    bytes
}

/// Exact kernel volume (`QueryMassProperties`, SCHEMA §7.5).
pub(crate) async fn exact_volume(wm: &WorkerManager, body: BodyId) -> f64 {
    ElementQuery::query_mass_properties(wm, body, "b".into())
        .await
        .expect("QueryMassProperties")
        .volume
}

pub(crate) fn assert_volume(got: f64, want: f64, what: &str) {
    assert!(
        (got - want).abs() < want * 1e-9,
        "{what}: want {want}, got {got}"
    );
}

/// `π·r²·h` for a Ø`d` cylinder.
pub(crate) fn cylinder(d: f64, h: f64) -> f64 {
    PI * (d / 2.0).powi(2) * h
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

/// A fully constrained `w × h` rectangle at sketch `(u0, 0)` (hole_ops.rs shape).
fn rect_sketch(sid: SketchId, base: u128, u0: f64, w: f64, h: f64) -> Sketch {
    let e = |n: u128| EntityId(Uuid::from_u128(base + n));
    let c = |n: u128| ConstraintId(Uuid::from_u128(base + 0x40 + n));
    let corners = [(u0, 0.0), (u0 + w, 0.0), (u0 + w, h), (u0, h)];
    let mut sk = Sketch::on_world_plane(sid, "Rect", WorldPlane::XY);
    for k in 0..4u128 {
        let (a, b) = (corners[k as usize], corners[(k as usize + 1) % 4]);
        for (n, (x, y)) in [(2 * k, a), (2 * k + 1, b)] {
            sk.add_entity(SketchEntity::point(
                e(n),
                Vec2::new_unchecked(x, y),
                false,
                false,
            ))
            .unwrap();
        }
        sk.add_entity(SketchEntity::line(
            e(0x10 + k),
            e(2 * k),
            e(2 * k + 1),
            false,
        ))
        .unwrap();
    }
    for k in 0..4u128 {
        sk.add_constraint(Constraint::Coincident {
            id: c(1 + k),
            point1: e(2 * k + 1),
            point2: e((2 * k + 2) % 8),
            point1_position: CurvePosition::Arbitrary,
            point2_position: CurvePosition::Arbitrary,
        })
        .unwrap();
    }
    for (n, line, horizontal) in [
        (5, 0x10, true),
        (6, 0x12, true),
        (7, 0x11, false),
        (8, 0x13, false),
    ] {
        let constraint = if horizontal {
            Constraint::Horizontal {
                id: c(n),
                line: e(line),
            }
        } else {
            Constraint::Vertical {
                id: c(n),
                line: e(line),
            }
        };
        sk.add_constraint(constraint).unwrap();
    }
    sk.add_constraint(Constraint::Fixed {
        id: c(9),
        point: e(0),
        point_position: CurvePosition::Arbitrary,
        at: Vec2::new_unchecked(u0, 0.0),
    })
    .unwrap();
    sk.add_constraint(Constraint::HorizontalDistance {
        id: c(10),
        point1: e(0),
        point1_position: CurvePosition::Arbitrary,
        point2: e(1),
        point2_position: CurvePosition::Arbitrary,
        value: Scalar::new(w),
    })
    .unwrap();
    sk.add_constraint(Constraint::VerticalDistance {
        id: c(11),
        point1: e(2),
        point1_position: CurvePosition::Arbitrary,
        point2: e(3),
        point2_position: CurvePosition::Arbitrary,
        value: Scalar::new(h),
    })
    .unwrap();
    sk
}

fn sketch_record(rec: u128, sk: &Sketch) -> OperationRecord {
    let (_plane, entities, constraints) = sketch_wire(sk);
    let params = SketchOpParams {
        sketch: sk.id,
        plane: xy_plane_ref(),
        entities: entities.as_array().cloned().unwrap_or_default(),
        constraints: constraints.as_array().cloned().unwrap_or_default(),
        host_face: None,
        extra: Default::default(),
    };
    let op = Operation::Known(KnownOperation::Sketch(params));
    OperationRecord::new(RecordId(Uuid::from_u128(rec)), 0, "Sketch", op)
}

fn extrude_record(rec: u128, sketch: SketchId, dist: f64) -> OperationRecord {
    let op = Operation::Known(KnownOperation::Extrude(ExtrudeParams {
        profile: Some(SketchRegionRef {
            sketch,
            region: RegionId::new(""),
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
    }));
    OperationRecord::new(RecordId(Uuid::from_u128(rec)), 0, "Extrude", op)
}

/// A 60×20×10 plate at sketch offset `u0` (world y ∈ [u0, u0+60]).
pub(crate) async fn build_plate(
    rt: &mut DocumentRuntime,
    sketch: u128,
    extrude: u128,
    u0: f64,
) -> BodyId {
    let sid = SketchId(Uuid::from_u128(sketch + 0x10_0000));
    add_op(
        rt,
        sketch_record(sketch, &rect_sketch(sid, sketch + 0x1000, u0, 60.0, 20.0)),
    );
    add_op(rt, extrude_record(extrude, sid, TOP));
    published(&clean_regen(rt).await, "plate");
    BodyId(Uuid::from_u128(extrude))
}

pub(crate) fn anchor_at(at: Vec3) -> AnchorIntent {
    AnchorIntent {
        world_point: at,
        surface_uv: None,
        local_frame: None,
        adjacency_hint: None,
        extra: Default::default(),
    }
}

pub(crate) fn anchored_ref(
    body: BodyId,
    element: &ElementId,
    kind: ElementKind,
    at: Vec3,
) -> ElementRef {
    ElementRef {
        primary: Some(PrimaryRef {
            body,
            element: element.clone(),
            kind,
            extra: Default::default(),
        }),
        intent: None,
        anchor: Some(anchor_at(at)),
        extra: Default::default(),
    }
}

pub(crate) fn top(x: f64, y: f64) -> Vec3 {
    Vec3::new_unchecked(x, y, TOP)
}

/// Simple through-all Ø`d` holes at `points` (world, on the top face), written
/// through the canonical writer: one point stays legacy V2, more become V3.
pub(crate) fn holes(body: BodyId, face_at: Vec3, points: &[Vec3], d: f64) -> HoleParams {
    let mut params = HoleParams {
        target_body: body,
        face: anchored_ref(
            body,
            &ElementId::new(format!("el_top_{body}")),
            ElementKind::Face,
            face_at,
        ),
        point: points[0],
        points: None,
        hole_type: HoleType::Simple,
        diameter: Scalar::new(d),
        depth: None,
        cb_diameter: None,
        cb_depth: None,
        cs_diameter: None,
        cs_angle_deg: None,
        thread: None,
        result_policy_version: Some(2),
        extra: Default::default(),
    };
    let list = points
        .iter()
        .map(|at| HolePlacement { id: None, at: *at })
        .collect();
    params.set_placements(list).expect("canonical writer");
    params
}

pub(crate) fn hole_record(rec: u128, params: HoleParams) -> OperationRecord {
    let op = Operation::Known(KnownOperation::Hole(params));
    OperationRecord::new(RecordId(Uuid::from_u128(rec)), 0, "Hole", op)
}

pub(crate) fn edit_hole(rt: &mut DocumentRuntime, rec: u128, params: HoleParams) {
    rt.apply(EditCommand::UpdateOperationParams {
        record: RecordId(Uuid::from_u128(rec)),
        op: Operation::Known(KnownOperation::Hole(params)),
    })
    .expect("UpdateOperationParams(Hole)");
}

pub(crate) fn stored_hole(rt: &DocumentRuntime, rec: u128) -> HoleParams {
    match rt
        .operation_record(RecordId(Uuid::from_u128(rec)))
        .map(|r| r.op)
    {
        Some(Operation::Known(KnownOperation::Hole(p))) => p,
        other => panic!("expected a stored Hole, got {other:?}"),
    }
}

pub(crate) fn feature_row(rt: &DocumentRuntime, rec: u128) -> FeatureDto {
    let id = RecordId(Uuid::from_u128(rec)).to_string();
    rt.projection()
        .features
        .into_iter()
        .find(|f| f.id == id)
        .unwrap_or_else(|| panic!("no feature row for {id}"))
}

/// Every circular edge of `body` at the head: `(centre, radius, topoKey)`.
pub(crate) async fn circles(wm: &WorkerManager, body: BodyId) -> Vec<(Vec3, f64, String)> {
    let mut out = Vec::new();
    for k in 1.. {
        let key = format!("e:{k}");
        let Ok(Some(dto)) = ElementQuery::classify_element_by_topo_key(wm, body, &key).await else {
            break;
        };
        let Some(frame) = dto.frame.filter(|_| dto.curve_type == "circle") else {
            continue;
        };
        let [x, y, z] = frame.origin;
        out.push((
            Vec3::new_unchecked(x, y, z),
            frame.radius.unwrap_or(0.0),
            key,
        ));
    }
    out
}

/// Promotes the plate's top face at the head, the way a viewport face pick
/// does, so a Hole's face ref carries real descriptor evidence (an anchor-only
/// ref is refused by design on any post-edit replay).
pub(crate) async fn promote_top_face(
    rt: &mut DocumentRuntime,
    wm: &WorkerManager,
    body: BodyId,
) -> ElementId {
    let snapshot = rt.head_snapshot_id().expect("a published head");
    let mut key = None;
    for k in 1.. {
        let candidate = format!("f:{k}");
        let Ok(Some(dto)) = ElementQuery::classify_element_by_topo_key(wm, body, &candidate).await
        else {
            break;
        };
        let frame = dto.frame.filter(|_| dto.surface_type == "plane");
        if frame
            .is_some_and(|f| f.normal == Some([0.0, 0.0, 1.0]) && (f.origin[2] - TOP).abs() < 1e-9)
        {
            key = Some(candidate);
            break;
        }
    }
    let key = key.expect("top face");
    let promoted = rt
        .promote_selection(snapshot, body, vec![(TopoKey::new(key.as_str()), None)])
        .await
        .expect("promote the top face");
    ElementId::new(&promoted[0].element_id)
}

pub(crate) fn has_circle(circles: &[(Vec3, f64, String)], at: Vec3, r: f64) -> bool {
    circles
        .iter()
        .any(|(c, radius, _)| c.approx_eq(&at, 1e-6) && (radius - r).abs() < 1e-6)
}
