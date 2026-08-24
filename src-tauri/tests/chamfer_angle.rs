//! Distance-angle **Chamfer** (SCHEMA §7.3 `angleDeg`, DEGREES) against the REAL
//! C++ OCCT worker, driven through the app's [`DocumentRuntime`] exactly like
//! `preview_edge_shell.rs`.
//!
//! The C++ lane already pins the kernel arithmetic (`worker/tests/test_wp6_ops.cpp`
//! `test_chamfer_angle_distance` / `test_chamfer_angle_45_is_equal_leg` /
//! `test_chamfer_angle_refusals`) and the frontend lane pins the FSM and the
//! marshalling. Nothing in the RUST lane replayed the geometry, so `angleDeg`
//! could have been dropped, degree/radian-confused, or reordered anywhere between
//! `ChamferParams` and the OCW1 frame and every gate would still have been green.
//! This file closes that: the number the user types comes back as a measured solid
//! volume through `AddOperation` → regen → MESH1.
//!
//! * `chamfer_angle_deg_commits_and_matches_analytic_volume` — the committed body
//!   is the analytic distance-angle solid, not the equal-leg reading of `radius`.
//! * `chamfer_angle_45_equals_equal_leg` — on a 90° dihedral, `angleDeg: 45` IS the
//!   equal-leg chamfer of the same distance. That identity is what pins the angle's
//!   ZERO (measured off the reference face, not off the edge or the other face).
//! * `preview_equals_commit_for_angle_mode` — the `PreviewOp` candidate the radius
//!   drag shows is the solid that later commits, in the angle mode too.
//! * `chamfer_angle_and_distance2_refused_by_core_before_the_worker` — the mutually
//!   exclusive pair is refused BY NAME by the single writer, with no regen issued.
//! * `chamfer_angle_out_of_range_is_recoverable_op_failed` — 180° is refused twice
//!   over: by the core session on the authoring path, and by the worker itself as a
//!   RECOVERABLE `OP_FAILED` on the `PreviewOp` path (which does not pass through
//!   the session), leaving the box intact and the worker alive both times.
//!
//! Gated on `ONECAD_WORKER_PATH` (else the dev-tree fallback); a missing binary
//! skips cleanly (CI sets `ONECAD_REQUIRE_WORKER=1` to make that a hard failure).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use onecad_core::document::record::{
    BooleanMode, ChamferParams, ExtrudeMode, ExtrudeParams, KnownOperation, Operation,
    OperationRecord, PlaneKind, SketchOpParams, SketchPlaneRef,
};
use onecad_core::document::refs::{
    AnchorIntent, ElementKind, ElementRef, PrimaryRef, SketchRegionRef,
};
use onecad_core::document::variables::Scalar;
use onecad_core::edit::EditCommand;
use onecad_core::ids::{
    BodyId, ConstraintId, ElementId, EntityId, RecordId, RegionId, SketchId, SnapshotId, TopoKey,
};
use onecad_core::math::{Vec2, Vec3};
use onecad_core::regen::{
    CancelToken, EngineError, GeometryEngine, Lod, ModelSnapshot, OpFailureCode, Outcome,
    RegenRequest,
};
use onecad_core::sketch::{Constraint, CurvePosition, Sketch, SketchEntity, WorldPlane};

use onecad_lib::document_runtime::{DocumentRuntime, RegenReport};
use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::wire::sketch_wire;
use onecad_lib::worker::{
    resolve_worker_path, MeshProvider, PreviewEngine, SolverEngine, WorkerManager,
};

use onecad_protocol::mesh::{f32_le, u32_le, validate_mesh_blob, MeshHeaderView};

// ─────────────────────────────────────────────────────────────────────────────
// Harness (mirrors preview_edge_shell.rs / wire_contract.rs)
// ─────────────────────────────────────────────────────────────────────────────

fn real_worker() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("ONECAD_WORKER_PATH") {
        let path = PathBuf::from(&p);
        assert!(
            path.is_file(),
            "ONECAD_WORKER_PATH={p:?} is set but no worker binary exists there \
             (misconfiguration — refusing to skip as green)"
        );
        return Some(path);
    }
    if let Some(path) = resolve_worker_path() {
        return Some(path);
    }
    assert!(
        std::env::var("ONECAD_REQUIRE_WORKER").as_deref() != Ok("1"),
        "ONECAD_REQUIRE_WORKER=1 but no worker binary resolved (CI must hard-fail here)"
    );
    None
}

async fn spawn_worker(bin: PathBuf) -> WorkerManager {
    let wm = WorkerManager::spawn(SupervisorConfig::production(bin));
    assert!(
        wm.wait_ready(Duration::from_secs(10)).await,
        "real worker must connect + handshake + OpenSession"
    );
    wm
}

fn runtime_over(wm: &WorkerManager) -> DocumentRuntime {
    let engine: Arc<dyn GeometryEngine> = Arc::new(wm.clone());
    let meshes: Arc<dyn MeshProvider> = Arc::new(wm.clone());
    let solver: Arc<dyn SolverEngine> = Arc::new(wm.clone());
    DocumentRuntime::new_blank(engine, meshes, solver)
}

fn add_op(rt: &mut DocumentRuntime, record: OperationRecord) {
    rt.apply(EditCommand::AddOperation {
        record,
        at_cursor: true,
    })
    .expect("AddOperation");
}

async fn regen_all(rt: &mut DocumentRuntime) -> RegenReport {
    rt.run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await
}

fn published<'a>(report: &'a RegenReport, what: &str) -> &'a Arc<ModelSnapshot> {
    match &report.outcome {
        Outcome::Published(s) => s,
        other => panic!("{what}: expected Published, got {other:?}"),
    }
}

const SKETCH_A: u128 = 0xA00;
const EXTRUDE_A: u128 = 0xA01;
const OP_TAIL: u128 = 0xF00;

/// The 10 mm cube the C++ lane measures on (`BRepPrimAPI_MakeBox(10, 10, 10)`),
/// rebuilt here as sketch + extrude so the whole Rust authoring path runs.
const BOX: f64 = 10.0;
/// Volume of that cube: the un-chamfered baseline every refusal case falls back to.
const BOX_VOLUME: f64 = 1000.0;

fn body_of(rec: u128) -> BodyId {
    BodyId(Uuid::from_u128(rec))
}

// ─────────────────────────────────────────────────────────────────────────────
// Sketch + op record builders (verbatim shapes from preview_edge_shell.rs)
// ─────────────────────────────────────────────────────────────────────────────

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

fn rect_sketch(sid: SketchId, base: u128, x0: f64, y0: f64, w: f64, h: f64) -> Sketch {
    let e = |n: u128| EntityId(Uuid::from_u128(base + n));
    let c = |n: u128| ConstraintId(Uuid::from_u128(base + 0x40 + n));
    let (p0s, p0e) = (e(0), e(1));
    let (p1s, p1e) = (e(2), e(3));
    let (p2s, p2e) = (e(4), e(5));
    let (p3s, p3e) = (e(6), e(7));
    let (l0, l1, l2, l3) = (e(0x10), e(0x11), e(0x12), e(0x13));

    let mut sk = Sketch::on_world_plane(sid, "Rect", WorldPlane::XY);
    let pt = |sk: &mut Sketch, id: EntityId, x: f64, y: f64| {
        sk.add_entity(SketchEntity::point(
            id,
            Vec2::new_unchecked(x, y),
            false,
            false,
        ))
        .unwrap();
    };
    pt(&mut sk, p0s, x0, y0);
    pt(&mut sk, p0e, x0 + w, y0);
    pt(&mut sk, p1s, x0 + w, y0);
    pt(&mut sk, p1e, x0 + w, y0 + h);
    pt(&mut sk, p2s, x0 + w, y0 + h);
    pt(&mut sk, p2e, x0, y0 + h);
    pt(&mut sk, p3s, x0, y0 + h);
    pt(&mut sk, p3e, x0, y0);
    sk.add_entity(SketchEntity::line(l0, p0s, p0e, false))
        .unwrap();
    sk.add_entity(SketchEntity::line(l1, p1s, p1e, false))
        .unwrap();
    sk.add_entity(SketchEntity::line(l2, p2s, p2e, false))
        .unwrap();
    sk.add_entity(SketchEntity::line(l3, p3s, p3e, false))
        .unwrap();

    let coincident = |sk: &mut Sketch, id, a, b| {
        sk.add_constraint(Constraint::Coincident {
            id,
            point1: a,
            point2: b,
            point1_position: CurvePosition::Arbitrary,
            point2_position: CurvePosition::Arbitrary,
        })
        .unwrap();
    };
    coincident(&mut sk, c(1), p0e, p1s);
    coincident(&mut sk, c(2), p1e, p2s);
    coincident(&mut sk, c(3), p2e, p3s);
    coincident(&mut sk, c(4), p3e, p0s);
    sk.add_constraint(Constraint::Horizontal { id: c(5), line: l0 })
        .unwrap();
    sk.add_constraint(Constraint::Horizontal { id: c(6), line: l2 })
        .unwrap();
    sk.add_constraint(Constraint::Vertical { id: c(7), line: l1 })
        .unwrap();
    sk.add_constraint(Constraint::Vertical { id: c(8), line: l3 })
        .unwrap();
    sk.add_constraint(Constraint::Fixed {
        id: c(9),
        point: p0s,
        point_position: CurvePosition::Arbitrary,
        at: Vec2::new_unchecked(x0, y0),
    })
    .unwrap();
    sk.add_constraint(Constraint::HorizontalDistance {
        id: c(10),
        point1: p0s,
        point1_position: CurvePosition::Arbitrary,
        point2: p0e,
        point2_position: CurvePosition::Arbitrary,
        value: Scalar::new(w),
    })
    .unwrap();
    sk.add_constraint(Constraint::VerticalDistance {
        id: c(11),
        point1: p1s,
        point1_position: CurvePosition::Arbitrary,
        point2: p1e,
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
    OperationRecord::new(
        RecordId(Uuid::from_u128(rec)),
        0,
        "Sketch",
        Operation::Known(KnownOperation::Sketch(params)),
    )
}

fn extrude_record(rec: u128, sketch: SketchId, dist: f64) -> OperationRecord {
    OperationRecord::new(
        RecordId(Uuid::from_u128(rec)),
        0,
        "Extrude",
        Operation::Known(KnownOperation::Extrude(ExtrudeParams {
            profile: Some(SketchRegionRef {
                sketch,
                region: RegionId::new(""), // empty ⇒ V1 first-region fallback
                region_identity_version: None,
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
        })),
    )
}

/// A typed element ref carrying the operated body + a world-point anchor — the
/// shape a real UI edge pick lowers to.
fn anchored_ref(body: BodyId, element: &ElementId, kind: ElementKind, at: Vec3) -> ElementRef {
    ElementRef {
        primary: Some(PrimaryRef {
            body,
            element: element.clone(),
            kind,
            extra: Default::default(),
        }),
        intent: None,
        anchor: Some(AnchorIntent {
            world_point: at,
            surface_uv: None,
            local_frame: None,
            adjacency_hint: None,
            extra: Default::default(),
        }),
        extra: Default::default(),
    }
}

/// A Chamfer over one promoted edge. `distance2` / `angle_deg` are passed through
/// verbatim (including the illegal combinations the refusal cases need) — the
/// mode is chosen by PRESENCE, never here.
fn chamfer_record(
    rec: u128,
    body: BodyId,
    edge: &ElementId,
    at: Vec3,
    radius: f64,
    distance2: Option<f64>,
    angle_deg: Option<f64>,
) -> OperationRecord {
    OperationRecord::new(
        RecordId(Uuid::from_u128(rec)),
        0,
        "Chamfer",
        Operation::Known(KnownOperation::Chamfer(ChamferParams {
            radius: Scalar::new(radius),
            distance2: distance2.map(Scalar::new),
            angle_deg: angle_deg.map(Scalar::new),
            edge_ids: vec![edge.clone()],
            edges: vec![anchored_ref(body, edge, ElementKind::Edge, at)],
            chain_tangent_edges: false,
            tangent_closure_version: None,
            extra: Default::default(),
        })),
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// MESH1 geometry helpers (exact for planar-faced polyhedra — a chamfered box is)
// ─────────────────────────────────────────────────────────────────────────────

const SEC_POSITIONS: u32 = 1;
const SEC_INDICES: u32 = 3;
const SEC_EDGE_RANGES: u32 = 7;
const SEC_EDGE_POSITIONS: u32 = 8;
const SEC_EDGE_ID_OFFS: u32 = 9;
const SEC_EDGE_ID_CHARS: u32 = 10;

fn vertex(blob: &[u8], pbase: usize, i: usize) -> [f64; 3] {
    let o = pbase + i * 12;
    [
        f32_le(blob, o) as f64,
        f32_le(blob, o + 4) as f64,
        f32_le(blob, o + 8) as f64,
    ]
}

/// Signed volume via the divergence theorem — EXACT for a closed polyhedron, which
/// every solid in this file is (a chamfered box has only planar faces).
fn mesh_volume(view: &MeshHeaderView, blob: &[u8]) -> f64 {
    let pos = view.section(SEC_POSITIONS).expect("POSITIONS");
    let idx = view.section(SEC_INDICES).expect("INDICES");
    let (pbase, ibase) = (pos.offset as usize, idx.offset as usize);
    let mut vol6 = 0.0f64;
    for t in 0..view.triangle_count as usize {
        let o = ibase + t * 12;
        let a = vertex(blob, pbase, u32_le(blob, o) as usize);
        let b = vertex(blob, pbase, u32_le(blob, o + 4) as usize);
        let c = vertex(blob, pbase, u32_le(blob, o + 8) as usize);
        vol6 += a[0] * (b[1] * c[2] - b[2] * c[1])
            + a[1] * (b[2] * c[0] - b[0] * c[2])
            + a[2] * (b[0] * c[1] - b[1] * c[0]);
    }
    (vol6 / 6.0).abs()
}

fn id_table(
    view: &MeshHeaderView,
    blob: &[u8],
    offs_ty: u32,
    chars_ty: u32,
    count: usize,
) -> Vec<String> {
    let offs = view.section(offs_ty).expect("id-offs");
    let chars = view.section(chars_ty).expect("id-chars");
    let (obase, cbase) = (offs.offset as usize, chars.offset as usize);
    (0..count)
        .map(|i| {
            let lo = u32_le(blob, obase + i * 4) as usize;
            let hi = u32_le(blob, obase + (i + 1) * 4) as usize;
            String::from_utf8_lossy(&blob[cbase + lo..cbase + hi]).into_owned()
        })
        .collect()
}

fn edge_polyline_stats(blob: &[u8], base: usize, first: usize, count: usize) -> (Vec3, [f64; 3]) {
    let (mut lo, mut hi, mut sum) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3], [0.0; 3]);
    for point in first..first + count {
        let offset = base + point * 12;
        let value = [
            f32_le(blob, offset) as f64,
            f32_le(blob, offset + 4) as f64,
            f32_le(blob, offset + 8) as f64,
        ];
        for axis in 0..3 {
            lo[axis] = lo[axis].min(value[axis]);
            hi[axis] = hi[axis].max(value[axis]);
            sum[axis] += value[axis];
        }
    }
    let n = count as f64;
    (
        Vec3::new_unchecked(sum[0] / n, sum[1] / n, sum[2] / n),
        [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]],
    )
}

/// The box's VERTICAL edge nearest the `(bbox_min.x, bbox_min.y)` corner: the
/// analogue of the C++ lane's `edge_by_center(box, 0, 0, 5)`. Returns its real
/// snapshot-scoped `TopoKey` and polyline centroid.
///
/// The four vertical edges of a cube are congruent, so which one wins does not
/// change the removed volume — but the pick must be a Z-running edge, and that is
/// asserted rather than assumed.
fn vertical_edge_pick(view: &MeshHeaderView, blob: &[u8]) -> (String, Vec3) {
    assert!(
        view.has_edges(),
        "MESH1 must carry edges for the chamfer pick"
    );
    let ranges = view.section(SEC_EDGE_RANGES).expect("EDGE_RANGES");
    let positions = view.section(SEC_EDGE_POSITIONS).expect("EDGE_POSITIONS");
    let keys = id_table(
        view,
        blob,
        SEC_EDGE_ID_OFFS,
        SEC_EDGE_ID_CHARS,
        view.edge_count as usize,
    );
    let target = (
        f64::from(view.bbox_min[0]),
        f64::from(view.bbox_min[1]),
        f64::from(view.bbox_min[2] + view.bbox_max[2]) / 2.0,
    );
    let (rbase, pbase) = (ranges.offset as usize, positions.offset as usize);
    let mut vertical = 0usize;
    let mut best: Option<(usize, f64, Vec3, [f64; 3])> = None;
    for edge in 0..view.edge_count as usize {
        let first = u32_le(blob, rbase + edge * 8) as usize;
        let count = u32_le(blob, rbase + edge * 8 + 4) as usize;
        if count == 0 {
            continue;
        }
        let (centroid, span) = edge_polyline_stats(blob, pbase, first, count);
        if span[2] <= span[0] || span[2] <= span[1] {
            continue; // not a Z-running edge
        }
        vertical += 1;
        let distance = (centroid.x - target.0).powi(2)
            + (centroid.y - target.1).powi(2)
            + (centroid.z - target.2).powi(2);
        if best.as_ref().is_none_or(|(_, d, _, _)| distance < *d) {
            best = Some((edge, distance, centroid, span));
        }
    }
    assert_eq!(vertical, 4, "a box has exactly four vertical edges");
    let (edge, _, centroid, span) = best.expect("at least one vertical mesh edge");
    assert!(
        (span[2] - BOX).abs() < 1e-3,
        "the picked vertical edge spans the full box height, got {}",
        span[2]
    );
    (keys[edge].clone(), centroid)
}

async fn body_mesh(rt: &mut DocumentRuntime, body: BodyId) -> Arc<Vec<u8>> {
    rt.get_mesh(body, Lod::Coarse, None)
        .await
        .expect("fetch body mesh")
}

/// Measured `(volume, face_count)` of the live body at the current head.
async fn measure(rt: &mut DocumentRuntime, body: BodyId) -> (f64, u32) {
    let blob = body_mesh(rt, body).await;
    let view = validate_mesh_blob(&blob).expect("MESH1 validates");
    (mesh_volume(&view, &blob), view.face_count)
}

// ─────────────────────────────────────────────────────────────────────────────
// Stock: the 10 mm cube with ONE vertical edge already promoted to an ElementId
// ─────────────────────────────────────────────────────────────────────────────

struct Stock {
    wm: WorkerManager,
    rt: DocumentRuntime,
    body: BodyId,
    edge: ElementId,
    anchor: Vec3,
}

/// Builds the 10 mm cube on a fresh worker, verifies it is the sharp 6-face solid
/// of volume 1000, and promotes one vertical edge through `AcquireElementIds` —
/// the real viewport authoring order (box → TopoKey pick → promote → edge op).
async fn stock_box(bin: PathBuf) -> Stock {
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let sa = SketchId(Uuid::from_u128(0xA));
    add_op(
        &mut rt,
        sketch_record(SKETCH_A, &rect_sketch(sa, 0x1000, 0.0, 0.0, BOX, BOX)),
    );
    add_op(&mut rt, extrude_record(EXTRUDE_A, sa, BOX));
    let report = regen_all(&mut rt).await;
    let snapshot = SnapshotId(report.snapshot_id);
    assert_eq!(
        published(&report, "stock box")
            .repair_summary
            .needs_repair_count,
        0
    );
    let body = body_of(EXTRUDE_A);

    let blob = body_mesh(&mut rt, body).await;
    let view = validate_mesh_blob(&blob).expect("stock box MESH1 validates");
    assert_eq!(view.face_count, 6, "the stock is a sharp six-face box");
    let volume = mesh_volume(&view, &blob);
    // 10×10×10 — the same solid `test_chamfer_angle_distance` measures on
    // (worker/tests/test_wp6_ops.cpp: `BRepPrimAPI_MakeBox(10.0, 10.0, 10.0)`).
    assert!(
        (volume - BOX_VOLUME).abs() < 1e-6,
        "10×10×10 = {BOX_VOLUME}, got {volume}"
    );

    let (topo_key, anchor) = vertical_edge_pick(&view, &blob);
    let promoted = rt
        .promote_selection(
            snapshot,
            body,
            vec![(
                TopoKey::new(&topo_key),
                Some(AnchorIntent {
                    world_point: anchor,
                    surface_uv: None,
                    local_frame: None,
                    adjacency_hint: None,
                    extra: Default::default(),
                }),
            )],
        )
        .await
        .unwrap_or_else(|error| panic!("promote the vertical edge pick: {error}"));
    assert_eq!(promoted.len(), 1, "one promoted edge");
    assert_eq!(promoted[0].topo_key, topo_key);
    assert_eq!(promoted[0].kind, "edge");

    Stock {
        wm,
        rt,
        body,
        edge: ElementId::new(&promoted[0].element_id),
        anchor,
    }
}

/// Commits ONE chamfer of `(radius, angle_deg)` onto a fresh stock cube and
/// returns the measured `(volume, face_count)` of the committed body.
async fn commit_chamfer(bin: PathBuf, radius: f64, angle_deg: Option<f64>) -> (f64, u32) {
    let mut stock = stock_box(bin).await;
    add_op(
        &mut stock.rt,
        chamfer_record(
            OP_TAIL,
            stock.body,
            &stock.edge,
            stock.anchor,
            radius,
            None,
            angle_deg,
        ),
    );
    let report = regen_all(&mut stock.rt).await;
    let snapshot = published(&report, "chamfer commit");
    assert_eq!(
        snapshot.repair_summary.needs_repair_count, 0,
        "a freshly promoted edge must commit without repair"
    );
    assert!(
        report.failed_steps.is_empty(),
        "the chamfer step must not fail: {:?}",
        report.failed_steps
    );
    let measured = measure(&mut stock.rt, stock.body).await;
    stock.wm.shutdown().await;
    measured
}

/// Removed material of a distance-angle chamfer swept the full box height:
/// `d · (d · tan A) / 2 · L`. Provenance: `worker/tests/test_wp6_ops.cpp`
/// `test_chamfer_angle_distance`, which checks the kernel volume against exactly
/// this expression at 1e-6 — `radius` is the leg ON the reference face and
/// `angleDeg` is measured off THAT face, so the far leg is `radius · tan(angle)`.
fn removed_by_angle(radius: f64, angle_deg: f64) -> f64 {
    0.5 * radius * (radius * (angle_deg * std::f64::consts::PI / 180.0).tan()) * BOX
}

// ─────────────────────────────────────────────────────────────────────────────
// 1. The committed body is the analytic distance-angle solid
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chamfer_angle_deg_commits_and_matches_analytic_volume() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let (volume, faces) = commit_chamfer(bin, 1.0, Some(60.0)).await;

    // d = 1, A = 60° ⇒ far leg = tan 60° = 1.7320508…, removed = 8.660254037844384,
    // so the solid is 991.3397459621556. Same figure the C++ lane measures
    // (worker/tests/test_wp6_ops.cpp `test_chamfer_angle_distance`, 1e-6 on the
    // exact OCCT volume); here it is read back off MESH1, which is exact for this
    // all-planar solid up to f32 vertex rounding.
    let expected = BOX_VOLUME - removed_by_angle(1.0, 60.0);
    assert!(
        (volume - expected).abs() < 1e-3,
        "distance-angle chamfer volume = {expected} (≈991.3397), got {volume}"
    );
    // 6 + 1 flat chamfer face (same count the C++ lane pins).
    assert_eq!(faces, 7, "chamfer adds exactly one flat face");
    // …and NOT the equal-leg reading of `radius`, which would be 1000 − 1·1/2·10 =
    // 995. Without this, an `angleDeg` silently dropped anywhere between
    // `ChamferParams` and the OCW1 frame would still land inside a loose tolerance.
    assert!(
        (volume - 995.0).abs() > 1.0,
        "angleDeg must actually reach the kernel (equal-leg would be 995), got {volume}"
    );
    eprintln!("chamfer angleDeg=60 committed volume {volume} (expected {expected})");
}

// ─────────────────────────────────────────────────────────────────────────────
// 2. On a 90° dihedral, angleDeg 45 IS the equal-leg chamfer — the angle's zero
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chamfer_angle_45_equals_equal_leg() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    // Two independent workers so neither run can observe the other's session.
    let (at45, at45_faces) = commit_chamfer(bin.clone(), 1.0, Some(45.0)).await;
    let (equal_leg, equal_faces) = commit_chamfer(bin.clone(), 1.0, None).await;

    // The identity that pins the angle's ZERO: measured off the reference FACE,
    // not off the edge (which would make 45 a 45° roll of a different solid) and
    // not off the other face (which would swap the legs). Provenance:
    // `test_chamfer_angle_45_is_equal_leg` (worker/tests/test_wp6_ops.cpp), which
    // checks the same equality at 1e-9 on the exact kernel volume.
    assert!(
        (at45 - equal_leg).abs() < 1e-9,
        "angleDeg 45 must be the equal-leg chamfer of the same d: {at45} vs {equal_leg}"
    );
    assert_eq!(at45_faces, equal_faces, "identical face count");
    // Both are the analytic equal-leg solid: 1000 − 1·1/2·10 = 995.
    assert!(
        (equal_leg - 995.0).abs() < 1e-3,
        "equal-leg d=1 on a 10 mm edge = 995, got {equal_leg}"
    );

    // …and 60° is a DIFFERENT solid, which is what makes the equality above an
    // assertion about the angle rather than about a dead code path.
    let (at60, _) = commit_chamfer(bin, 1.0, Some(60.0)).await;
    assert!(
        (at60 - equal_leg).abs() > 1.0,
        "a different angle must be a different solid: {at60} vs {equal_leg}"
    );
    eprintln!("angleDeg=45 {at45} == equal-leg {equal_leg}; angleDeg=60 {at60}");
}

// ─────────────────────────────────────────────────────────────────────────────
// 3. The previewed candidate IS the committed solid, in the angle mode too
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn preview_equals_commit_for_angle_mode() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let mut stock = stock_box(bin).await;
    let head_before = stock.rt.projection().revision;
    let candidate = chamfer_record(
        OP_TAIL,
        stock.body,
        &stock.edge,
        stock.anchor,
        1.0,
        None,
        Some(60.0),
    );

    let preview = stock
        .wm
        .preview_op(
            candidate.op.clone(),
            candidate.record_id.to_string(),
            None,
            None,
            Lod::Coarse,
        )
        .await
        .expect("distance-angle Chamfer PreviewOp reaches the worker");
    assert!(
        preview.needs_repair.is_empty(),
        "a freshly promoted edge must resolve in preview, got {:?}",
        preview.needs_repair
    );
    assert_eq!(
        preview.changed_bodies,
        vec![stock.body.to_string()],
        "Chamfer MODIFIES the target body (id preserved)"
    );
    assert_eq!(preview.bodies.len(), 1, "one candidate body");
    let blob = &preview.bodies[0].mesh;
    let view = validate_mesh_blob(blob).expect("preview MESH1 validates");
    let preview_volume = mesh_volume(&view, blob);
    let expected = BOX_VOLUME - removed_by_angle(1.0, 60.0); // 991.3397459621556
    assert!(
        (preview_volume - expected).abs() < 1e-3,
        "the PREVIEW is already the analytic distance-angle solid {expected}, got {preview_volume}"
    );

    // No trace: same revision, and the real body is still the sharp box.
    assert_eq!(
        stock.rt.projection().revision,
        head_before,
        "a preview must not bump the document revision"
    );
    let (untouched, untouched_faces) = measure(&mut stock.rt, stock.body).await;
    assert_eq!(untouched_faces, 6, "the real body is still sharp");
    assert!((untouched - BOX_VOLUME).abs() < 1e-6);

    // The SAME record now commits — and lands exactly where the preview promised.
    add_op(&mut stock.rt, candidate);
    let report = regen_all(&mut stock.rt).await;
    assert_eq!(
        published(&report, "angle chamfer commit")
            .repair_summary
            .needs_repair_count,
        0
    );
    let (committed, committed_faces) = measure(&mut stock.rt, stock.body).await;
    assert_eq!(
        committed_faces, view.face_count,
        "preview and commit have the same face count"
    );
    // Both sides run the SAME executor over the same base at the same LOD, so the
    // tessellations are identical — this is an equality, not an approximation.
    assert!(
        (committed - preview_volume).abs() < 1e-9,
        "preview {preview_volume} and commit {committed} must agree — that is the point"
    );
    eprintln!("angle-mode preview {preview_volume} == commit {committed}");
    stock.wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// 4. distance2 + angleDeg is refused BY NAME by the single writer, before regen
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chamfer_angle_and_distance2_refused_by_core_before_the_worker() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let mut stock = stock_box(bin).await;
    let revision_before = stock.rt.projection().revision;
    let ops_before = stock.rt.projection().total_ops;

    // The two fields describe the same second leg twice. Resolving that by
    // precedence would silently discard one of the numbers the user typed, so the
    // refusal must NAME both — see `ChamferParams::validate` (record.rs) and the
    // worker's independent mirror of the same rule (FilletChamferOp.cpp:275).
    let error = stock
        .rt
        .apply(EditCommand::AddOperation {
            record: chamfer_record(
                OP_TAIL,
                stock.body,
                &stock.edge,
                stock.anchor,
                1.0,
                Some(2.5),
                Some(45.0),
            ),
            at_cursor: true,
        })
        .expect_err("distance2 + angleDeg is not a document the session may write")
        .to_string();
    assert!(
        error.contains("distance2") && error.contains("angleDeg"),
        "the refusal must name BOTH fields so the caller knows which to clear: {error}"
    );

    // Nothing was written and no regen was issued: same revision, same op count.
    assert_eq!(
        stock.rt.projection().revision,
        revision_before,
        "a rejected edit must not bump the document revision"
    );
    assert_eq!(
        stock.rt.projection().total_ops,
        ops_before,
        "a rejected edit must not land on the timeline"
    );
    // …and the worker head is exactly the box it was before.
    let (volume, faces) = measure(&mut stock.rt, stock.body).await;
    assert_eq!(faces, 6, "the worker head is still the sharp box");
    assert!(
        (volume - BOX_VOLUME).abs() < 1e-6,
        "the worker head is still {BOX_VOLUME}, got {volume}"
    );
    eprintln!("both-modes refusal: {error}");
    stock.wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// 5. An out-of-range angle is refused twice over, and neither refusal is fatal
// ─────────────────────────────────────────────────────────────────────────────

/// 180° is the degenerate chamfer face folded back onto the reference face. The
/// core session refuses it on the authoring path (`ChamferParams::validate`), so
/// it never reaches the worker THAT way — but the worker is an independent trust
/// boundary and `PreviewOp` does not pass through the session, so the same value
/// is pushed down that lane too. Both refusals must leave the box intact, and the
/// worker one must be a RECOVERABLE `OP_FAILED` rather than a torn-down session.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chamfer_angle_out_of_range_is_recoverable_op_failed() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let mut stock = stock_box(bin).await;
    let revision_before = stock.rt.projection().revision;
    let flat = chamfer_record(
        OP_TAIL,
        stock.body,
        &stock.edge,
        stock.anchor,
        1.0,
        None,
        Some(180.0),
    );

    // (a) The authoring path. `ChamferParams::validate` bounds `angleDeg` to the
    // OPEN interval (0, 180), so the edit is rejected by name before a plan is
    // compiled.
    let error = stock
        .rt
        .apply(EditCommand::AddOperation {
            record: flat.clone(),
            at_cursor: true,
        })
        .expect_err("angleDeg 180 is not a document the session may write")
        .to_string();
    assert!(
        error.contains("angleDeg"),
        "the refusal must name the offending field: {error}"
    );
    assert_eq!(
        stock.rt.projection().revision,
        revision_before,
        "a rejected edit must not bump the document revision"
    );

    // (b) The preview path, which bypasses the session entirely. The worker's own
    // static bound fires (FilletChamferOp.cpp:309, message pinned by
    // `test_chamfer_angle_refusals`) as a RECOVERABLE OP_FAILED.
    let failure = stock
        .wm
        .preview_op(
            flat.op.clone(),
            flat.record_id.to_string(),
            None,
            None,
            Lod::Coarse,
        )
        .await
        .expect_err("the worker must refuse angleDeg 180");
    match &failure {
        EngineError::OpFailed {
            code,
            recoverable,
            message,
            ..
        } => {
            assert_eq!(*code, OpFailureCode::OpFailed, "a plain OP_FAILED");
            assert!(*recoverable, "an out-of-range param is recoverable");
            assert!(
                message.contains("angleDeg"),
                "the worker refusal names the field: {message}"
            );
        }
        other => panic!("expected a recoverable OpFailed, got {other:?}"),
    }

    // Recoverable means the session survived: the document still holds the
    // un-chamfered box, and the very next legal candidate previews normally.
    let (volume, faces) = measure(&mut stock.rt, stock.body).await;
    assert_eq!(faces, 6, "the refused angle left the box sharp");
    assert!(
        (volume - BOX_VOLUME).abs() < 1e-6,
        "the refused angle left {BOX_VOLUME} intact, got {volume}"
    );
    let ok = stock
        .wm
        .preview_op(
            chamfer_record(
                OP_TAIL,
                stock.body,
                &stock.edge,
                stock.anchor,
                1.0,
                None,
                Some(60.0),
            )
            .op,
            "op_after_refusal".into(),
            None,
            None,
            Lod::Coarse,
        )
        .await
        .expect("the worker still serves a legal candidate after the refusal");
    let blob = &ok.bodies[0].mesh;
    let view = validate_mesh_blob(blob).expect("post-refusal preview MESH1");
    let recovered = mesh_volume(&view, blob);
    let expected = BOX_VOLUME - removed_by_angle(1.0, 60.0); // 991.3397459621556
    assert!(
        (recovered - expected).abs() < 1e-3,
        "the session was untouched by the refusal: expected {expected}, got {recovered}"
    );
    eprintln!("angleDeg 180 refused: core={error}; worker={failure}");
    stock.wm.shutdown().await;
}
