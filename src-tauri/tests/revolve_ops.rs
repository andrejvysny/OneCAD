//! MODEL-HARDEN W3 Revolve wire gate against the REAL C++ OCCT worker, driven
//! through the app's [`DocumentRuntime`] exactly like `wire_contract.rs` /
//! `breadth_ops.rs`.
//!
//! First-ever real-worker Rust integration coverage for Revolve — proves the whole
//! Rust wire → worker dispatch → OCCT → adoption/mesh path for every Revolve shape:
//! * `revolve_new_body_pappus_volume` — 360° NewBody, a 10×10 rect at radius 15 from
//!   a sketch-line axis ⇒ exactly 1 body, Pappus volume 2π·15·100 ≈ 9424.78.
//! * `revolve_partial_angle_half_volume` — the same profile at 180° ⇒ half the
//!   Pappus volume, with the swept solid sitting on ONE side of the axis-normal
//!   (a half-revolution's sign structure).
//! * `revolve_cut_split_children_adopted` — a boolean-mode Revolve Cut whose result
//!   is a two-solid compound mints deterministic split children `body_<opId>:<k>`,
//!   both adopted (D1), parent consumed; exact 8000 / 8800 box arithmetic; ids +
//!   volumes stable across a replay.
//! * `revolve_stale_axis_fails_loud` — a nonexistent axis lineId ⇒ NO silent body,
//!   NO panic: the revolve step's timeline state is `Error` and its projection
//!   `FeatureDto` carries the worker's axis message (`statusMessage`, W0.5 contract).
//! * `revolve_region_binding_explicit_and_fallback` — one revolve bound to an
//!   explicit normative `regionId` (fetched from the solver lane, the exact source
//!   `finish_sketch` reads) and one bound to `""` (first-region fallback) both
//!   publish a body; a THIRD bound to a WRONG regionId is a deterministic OP_FAILED
//!   whose message names the requested + available ids (proves the id reaches the
//!   worker — an id-dropping marshalling regression would hide behind the fallback).
//!
//! REVOLVE-REGION-PARITY additions (the frontend seam the postmortem names, now for
//! Revolve — see `docs/POSTMORTEM_EXTRUDE_COMMIT.md`):
//! * `revolve_commit_after_pure_read_arm_mints_the_profile_record` — the EXACT new
//!   frontend sequence through the PRODUCTION scheduler/driver: AddSketch →
//!   `enter_sketch` → `sketch_upsert` → `cancel_sketch` (SketchController.exit's
//!   teardown half) → `prepare_sketch_regions().drive()` (the PURE-READ arm, which
//!   deliberately authors NO timeline record) → `finish_sketch` (the commit-boundary
//!   guarantee `confirmRevolve` now issues) → typed Revolve commit ⇒ published body.
//! * `revolve_commit_without_the_record_guarantee_fails_loud` — the regression
//!   TRIPWIRE: the identical flow with the `finish_sketch` step REMOVED must fail
//!   with the worker's `Revolve: profile sketch not found in plan`. Delete the
//!   commit-boundary guarantee and this test goes green while the product breaks —
//!   which is exactly how the extrude defect shipped.
//! * `revolve_reedit_binds_the_stored_region_not_the_first` — a TWO-region sketch
//!   whose revolve is bound to the second region by exact id, then re-edited via
//!   `UpdateOperationParams` (the angle-only deep-merge the re-edit sends): the
//!   published volume matches THAT region's Pappus solid, never the first region's.
//!
//! The revolve mesh-volume expectations for the CURVED solids (Pappus / half) are
//! FINE-lod faceting bounds, NOT exactness assertions — BRep-exact volume lives in
//! the worker ctest (`test_wp6_ops` / `test_revolve_split`, `ShapeMetrics`); the
//! Rust-side mesh assertion proves the wire/adoption path, not tessellation
//! accuracy. The split children are PLANAR boxes, so their mesh volume IS exact.
//!
//! Provenance: Pappus numbers cite `worker/tests/test_wp6_ops.cpp:165-201`; the
//! Cut-split geometry + frame mapping + child volumes cite
//! `worker/tests/test_revolve_split.cpp` (reused verbatim).
//!
//! Gated on `ONECAD_WORKER_PATH` (else the dev-tree fallback); a missing binary
//! skips cleanly unless `ONECAD_REQUIRE_WORKER=1` (CI hard-fails).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};
use tokio::sync::{watch, Mutex};

use onecad_core::document::body::split_child_uuid;
use onecad_core::document::record::{
    BooleanMode, ExtrudeMode, ExtrudeParams, KnownOperation, Operation, OperationRecord, PlaneKind,
    RevolveParams, SketchOpParams, SketchPlaneRef,
};
use onecad_core::document::refs::{AxisRef, SketchRegionRef};
use onecad_core::document::variables::Scalar;
use onecad_core::edit::{EditCommand, SketchEditOp};
use onecad_core::ids::{BodyId, ConstraintId, EntityId, RecordId, RegionId, SketchId};
use onecad_core::math::{Vec2, Vec3};
use onecad_core::regen::{
    CancelToken, GeometryEngine, Lod, ModelSnapshot, Outcome, RegenRequest, RegenScheduler,
    SchedulerHandle,
};
use onecad_core::sketch::{Constraint, Sketch, SketchEntity, WorldPlane};

use onecad_lib::document_runtime::{DocumentRuntime, RegenReport};
use onecad_lib::dto::{DocumentChange, DocumentProjection, FeatureStatus, SketchRegionDto};
use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::wire::sketch_wire;
use onecad_lib::worker::{resolve_worker_path, MeshProvider, SolverEngine, WorkerManager};
use onecad_lib::{regen_driver_with_emitter, RegenEmitter};

use onecad_protocol::mesh::{f32_le, u32_le, validate_mesh_blob, MeshHeaderView};

// ─────────────────────────────────────────────────────────────────────────────
// Harness (mirrors wire_contract.rs / breadth_ops.rs)
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

fn body_of(rec: u128) -> BodyId {
    BodyId(Uuid::from_u128(rec))
}

// ─────────────────────────────────────────────────────────────────────────────
// Production scheduler/driver wiring (verbatim from scheduler_commit.rs). The
// REVOLVE-REGION-PARITY tests must cross the same apply → `RegenScheduler::handle`
// → driver → publish chain the real app uses; driving `run_regen` directly would
// skip exactly the seam the extrude postmortem's defect lived in.
// ─────────────────────────────────────────────────────────────────────────────

type Runtime = Arc<Mutex<Option<DocumentRuntime>>>;
/// What the emitter captures per completion: `(outcome_str, document_change, projection)`.
type Emit = (String, Option<DocumentChange>, DocumentProjection);

async fn wire_with(runtime: Runtime) -> (Runtime, SchedulerHandle, UnboundedReceiver<Emit>) {
    let (tx, rx) = unbounded_channel::<Emit>();
    let emit: RegenEmitter = Arc::new(
        move |report: &RegenReport, projection: &DocumentProjection| {
            let _ = tx.send((
                report.outcome_str().to_string(),
                report.document_change(),
                projection.clone(),
            ));
        },
    );
    let autosave_tick = Arc::new(watch::channel(0u64).0);
    let driver = regen_driver_with_emitter(runtime.clone(), emit, autosave_tick);
    let (scheduler, sched) = RegenScheduler::new(driver);
    tokio::spawn(scheduler.run());
    (runtime, sched, rx)
}

/// Awaits the next emitter completion (15 s ceiling — a worker regen is well under).
async fn recv(rx: &mut UnboundedReceiver<Emit>) -> Emit {
    tokio::time::timeout(Duration::from_secs(15), rx.recv())
        .await
        .expect("regen completion emitted within 15s")
        .expect("emitter channel stayed open")
}

/// The `SketchEditOp` batch a drawing session sends for `sketch`'s whole content —
/// exactly what the frontend's incremental draw accumulates into `sketch_upsert`.
fn sketch_edit_ops(sketch: &Sketch) -> Vec<SketchEditOp> {
    sketch
        .entities()
        .iter()
        .cloned()
        .map(|entity| SketchEditOp::AddEntity { entity })
        .chain(
            sketch
                .constraints()
                .iter()
                .cloned()
                .map(|constraint| SketchEditOp::AddConstraint { constraint }),
        )
        .collect()
}

fn sketch_record_count(rt: &DocumentRuntime) -> usize {
    rt.projection()
        .features
        .iter()
        .filter(|f| f.op_type == "Sketch")
        .count()
}

/// A region's fill area and area-weighted centroid `u`, both in sketch (u,v) space —
/// the two Pappus inputs that identify WHICH region a revolve actually consumed.
fn region_area_centroid_u(region: &SketchRegionDto) -> (f64, f64) {
    let t = region.preview_triangles.as_ref().expect("region fill");
    let point = |index: u32| {
        let offset = index as usize * 2;
        [t.positions[offset], t.positions[offset + 1]]
    };
    let mut area = 0.0f64;
    let mut moment_u = 0.0f64;
    for tri in t.indices.chunks_exact(3) {
        let [a, b, c] = [point(tri[0]), point(tri[1]), point(tri[2])];
        let tri_area = ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])).abs() * 0.5;
        area += tri_area;
        moment_u += tri_area * (a[0] + b[0] + c[0]) / 3.0;
    }
    (area, if area > 0.0 { moment_u / area } else { 0.0 })
}

/// Pappus volume of `region` swept `angle_deg` about the constant-`u` line `axis_u`.
/// The XY plane maps `(u,v) → (-v, u, 0)`, so a constant-`u` axis is parallel to the
/// world X-axis and every profile point's radius is exactly `u - axis_u`.
fn pappus_volume(region: &SketchRegionDto, axis_u: f64, angle_deg: f64) -> f64 {
    let (area, centroid_u) = region_area_centroid_u(region);
    (angle_deg / 360.0) * 2.0 * std::f64::consts::PI * (centroid_u - axis_u) * area
}

// Fixed record ids.
const SKETCH_A: u128 = 0xA00; // box A / profile sketch
const EXTRUDE_A: u128 = 0xA01; // box A body producer
const SKETCH_TOOL: u128 = 0xB00; // revolve tool sketch (Cut test)
const REVOLVE: u128 = 0xB01; // the revolve op
const REVOLVE_2: u128 = 0xB02; // a second revolve (region-binding test)
const REVOLVE_3: u128 = 0xB03; // a third revolve — WRONG regionId (region-binding test)

// ─────────────────────────────────────────────────────────────────────────────
// Sketch plane refs. The worker's `parse_plane` (WireSketch.cpp:107-110) uses its
// OWN canonical frame for a NAMED plane kind and ignores the explicit axes — so
// only `kind` is load-bearing on the wire. The axes below are the matching
// canonical frames (documentation), verified against test_revolve_split.cpp:
//   XY: toWorld(u,v) = (-v,  u, 0)   XZ: toWorld(u,v) = (0, u, v)
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

fn xz_plane_ref() -> SketchPlaneRef {
    SketchPlaneRef {
        kind: PlaneKind::Xz,
        origin: Vec3::new_unchecked(0.0, 0.0, 0.0),
        x_axis: Vec3::new_unchecked(0.0, 1.0, 0.0),
        y_axis: Vec3::new_unchecked(0.0, 0.0, 1.0),
        normal: Vec3::new_unchecked(1.0, 0.0, 0.0),
        extra: Default::default(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Sketch + op record builders (rect_sketch verbatim from wire_contract.rs)
// ─────────────────────────────────────────────────────────────────────────────

/// A fully-constrained (dof 0) rectangle at sketch-space `(x0, y0)`, size `w × h`,
/// built the marshaller way (8 points, 4 lines, coincident corners, H/V, a Fixed
/// anchor, H/V dimensions). Identical to `wire_contract::rect_sketch`.
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
        at: Vec2::new_unchecked(x0, y0),
    })
    .unwrap();
    sk.add_constraint(Constraint::HorizontalDistance {
        id: c(10),
        point1: p0s,
        point2: p0e,
        value: Scalar::new(w),
    })
    .unwrap();
    sk.add_constraint(Constraint::VerticalDistance {
        id: c(11),
        point1: p1s,
        point2: p1e,
        value: Scalar::new(h),
    })
    .unwrap();
    sk
}

/// A `rect_sketch` PLUS a separate, fully-fixed revolve axis line at constant
/// sketch-`u` `ax_u`, spanning `v ∈ [ax_v0, ax_v1]`. The axis is a plain (open)
/// line: it participates in NO closed loop, so region detection ignores it (proven
/// in `test_revolve_split.cpp`, where `taxis` is likewise a plain line), yet it is
/// still a resolvable `SketchLine` for the revolve axis. Returns `(sketch, axis
/// line EntityId)` for `AxisRef::SketchLine`. `axis` is `(ax_u, ax_v0, ax_v1)` — a
/// constant-`u` line spanning `v ∈ [ax_v0, ax_v1]`.
fn rect_with_axis_sketch(
    sid: SketchId,
    base: u128,
    x0: f64,
    y0: f64,
    w: f64,
    h: f64,
    axis: (f64, f64, f64),
) -> (Sketch, EntityId) {
    let (ax_u, ax_v0, ax_v1) = axis;
    let mut sk = rect_sketch(sid, base, x0, y0, w, h);
    let e = |n: u128| EntityId(Uuid::from_u128(base + n));
    let c = |n: u128| ConstraintId(Uuid::from_u128(base + 0x40 + n));
    let (a_s, a_e, a_line) = (e(0x20), e(0x21), e(0x22));
    sk.add_entity(SketchEntity::point(
        a_s,
        Vec2::new_unchecked(ax_u, ax_v0),
        false,
        false,
    ))
    .unwrap();
    sk.add_entity(SketchEntity::point(
        a_e,
        Vec2::new_unchecked(ax_u, ax_v1),
        false,
        false,
    ))
    .unwrap();
    sk.add_entity(SketchEntity::line(a_line, a_s, a_e, false))
        .unwrap();
    // Pin both endpoints so the worker's solve places the axis deterministically.
    sk.add_constraint(Constraint::Fixed {
        id: c(0x20),
        point: a_s,
        at: Vec2::new_unchecked(ax_u, ax_v0),
    })
    .unwrap();
    sk.add_constraint(Constraint::Fixed {
        id: c(0x21),
        point: a_e,
        at: Vec2::new_unchecked(ax_u, ax_v1),
    })
    .unwrap();
    (sk, a_line)
}

fn sketch_record(rec: u128, sk: &Sketch, plane: SketchPlaneRef) -> OperationRecord {
    let (_plane, entities, constraints) = sketch_wire(sk);
    let params = SketchOpParams {
        sketch: sk.id,
        plane,
        entities: entities.as_array().cloned().unwrap_or_default(),
        constraints: constraints.as_array().cloned().unwrap_or_default(),
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
    let params = ExtrudeParams {
        profile: Some(SketchRegionRef {
            sketch,
            region: RegionId::new(""), // first-region fallback
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
    OperationRecord::new(
        RecordId(Uuid::from_u128(rec)),
        0,
        "Extrude",
        Operation::Known(KnownOperation::Extrude(params)),
    )
}

/// A Revolve op. `region` is `""` for the first-region fallback or a normative
/// `regionId`; `axis_line` is the axis `SketchLine`'s lineId inside `sketch`.
fn revolve_record(
    rec: u128,
    sketch: SketchId,
    region: &str,
    angle_deg: f64,
    axis_line: EntityId,
    boolean: BooleanMode,
    target: Option<BodyId>,
) -> OperationRecord {
    let params = RevolveParams {
        profile: Some(SketchRegionRef {
            sketch,
            region: RegionId::new(region),
            extra: Default::default(),
        }),
        angle_deg: Scalar::new(angle_deg),
        axis: Some(AxisRef::SketchLine {
            sketch,
            line: axis_line,
            extra: Default::default(),
        }),
        boolean_mode: boolean,
        target_body: target,
        extra: Default::default(),
    };
    OperationRecord::new(
        RecordId(Uuid::from_u128(rec)),
        0,
        "Revolve",
        Operation::Known(KnownOperation::Revolve(params)),
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// MESH1 geometry helpers (from wire_contract.rs)
// ─────────────────────────────────────────────────────────────────────────────

const SEC_POSITIONS: u32 = 1;
const SEC_INDICES: u32 = 3;

fn vertex(blob: &[u8], pbase: usize, i: usize) -> [f64; 3] {
    let o = pbase + i * 12;
    [
        f32_le(blob, o) as f64,
        f32_le(blob, o + 4) as f64,
        f32_le(blob, o + 8) as f64,
    ]
}

/// Signed volume via the divergence theorem — EXACT for a closed, planar-faced
/// polyhedron; for a faceted CURVED solid it is the inscribed-mesh volume (an
/// under-estimate of the true BRep volume).
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

fn bbox_dims(view: &MeshHeaderView) -> [f64; 3] {
    [
        f64::from(view.bbox_max[0] - view.bbox_min[0]),
        f64::from(view.bbox_max[1] - view.bbox_min[1]),
        f64::from(view.bbox_max[2] - view.bbox_min[2]),
    ]
}

async fn body_mesh(rt: &mut DocumentRuntime, body: BodyId, lod: Lod) -> Arc<Vec<u8>> {
    rt.get_mesh(body, lod, None).await.expect("fetch body mesh")
}

async fn mesh_vol(rt: &mut DocumentRuntime, body: BodyId, lod: Lod) -> f64 {
    let mesh = body_mesh(rt, body, lod).await;
    let view = validate_mesh_blob(&mesh).expect("child MESH1 validates");
    mesh_volume(&view, &mesh)
}

// ─────────────────────────────────────────────────────────────────────────────
// 1. NewBody Pappus volume — 10×10 rect at radius 15, 360° ⇒ 2π·15·100 ≈ 9424.78.
//    (Provenance: worker/tests/test_wp6_ops.cpp:165-199.)
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revolve_new_body_pappus_volume() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    // XY plane toWorld(u,v)=(-v,u,0): rect u∈[10,20] v∈[0,10] → world x∈[-10,0],
    // y∈[10,20], z=0 (area 100, centroid world-y = 15 ⇒ radius 15 from the u=0
    // axis, which maps to the world X-axis). 360° ⇒ V = 2π·15·100 = 9424.78.
    let sid = SketchId(Uuid::from_u128(0xA));
    let (sk, axis) = rect_with_axis_sketch(sid, 0x1000, 10.0, 0.0, 10.0, 10.0, (0.0, -5.0, 15.0));
    add_op(&mut rt, sketch_record(SKETCH_A, &sk, xy_plane_ref()));
    add_op(
        &mut rt,
        revolve_record(REVOLVE, sid, "", 360.0, axis, BooleanMode::NewBody, None),
    );

    let rep = regen_all(&mut rt).await;
    let snap = published(&rep, "pappus revolve");
    assert_eq!(
        snap.bodies.len(),
        1,
        "360° NewBody revolve = exactly 1 body"
    );

    // FINE lod (0.5% linear / 0.2 rad angular, Tessellate.cpp:44). This asserts the
    // wire/adoption path, NOT tessellation accuracy: a revolved (curved) solid's
    // faceted mesh UNDER-estimates the true volume (inscribed facets). BRep-exact
    // volume is ctest's job (test_wp6_ops, ShapeMetrics, ±30 on 9424.78). The bound
    // below carries ≥2× headroom over the observed fine-mesh faceting deficit:
    // observed vol 9409.161 ⇒ deficit 15.617 (0.17%), so 40.0 is ≈2.6× — tight
    // enough to still fail a real regression (a 1% volume error = ~94 > 40).
    const PAPPUS: f64 = 9424.778; // 2π·15·100
    let vol = mesh_vol(&mut rt, body_of(REVOLVE), Lod::Fine).await;
    eprintln!(
        "pappus 360°: fine-mesh volume {vol:.3} vs BRep {PAPPUS:.3} (faceting deficit {:.3})",
        PAPPUS - vol
    );
    assert!(
        (vol - PAPPUS).abs() < 40.0,
        "fine-mesh revolve volume within faceting tolerance of Pappus 9424.78, got {vol}"
    );

    wm.shutdown().await;
    eprintln!("Revolve NewBody PASS: 1 body, fine-mesh volume {vol:.3} ≈ Pappus 9424.78");
}

// ─────────────────────────────────────────────────────────────────────────────
// 2. Partial 180° ⇒ half the Pappus volume; the swept solid is ONE-SIDED along the
//    axis-normal it sweeps into (a half-revolution's sign structure).
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revolve_partial_angle_half_volume() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    // Same profile as the Pappus test; 180° ⇒ V = π·15·100 = 4712.39. The profile
    // sits in the world z=0 plane at world-y ∈ [10,20], revolved about the world
    // X-axis: at angle θ a profile point (y₀, z=0) maps to (y₀·cosθ, y₀·sinθ). Over
    // θ∈[0,180°] cosθ spans [-1,1] ⇒ Y is FULL/symmetric [±20]; sinθ ≥ 0 ⇒ Z is
    // ONE-SIDED (only [0,20] or, if the axis direction flips the sweep, [-20,0]).
    let sid = SketchId(Uuid::from_u128(0xA));
    let (sk, axis) = rect_with_axis_sketch(sid, 0x1000, 10.0, 0.0, 10.0, 10.0, (0.0, -5.0, 15.0));
    add_op(&mut rt, sketch_record(SKETCH_A, &sk, xy_plane_ref()));
    add_op(
        &mut rt,
        revolve_record(REVOLVE, sid, "", 180.0, axis, BooleanMode::NewBody, None),
    );

    let rep = regen_all(&mut rt).await;
    let snap = published(&rep, "half revolve");
    assert_eq!(
        snap.bodies.len(),
        1,
        "180° NewBody revolve = exactly 1 body"
    );

    let mesh = body_mesh(&mut rt, body_of(REVOLVE), Lod::Fine).await;
    let view = validate_mesh_blob(&mesh).expect("half-revolve MESH1 validates");
    let vol = mesh_volume(&view, &mesh);
    let dims = bbox_dims(&view);
    let (ymin, ymax) = (f64::from(view.bbox_min[1]), f64::from(view.bbox_max[1]));
    let (zmin, zmax) = (f64::from(view.bbox_min[2]), f64::from(view.bbox_max[2]));

    // Half the Pappus volume, same fine-mesh faceting regime. Observed vol 4704.823
    // ⇒ deficit 7.566 (0.16%), so 20.0 is ≈2.6× headroom (same regime as the 360°).
    const HALF_PAPPUS: f64 = 4712.389; // π·15·100
    eprintln!(
        "half 180°: fine-mesh volume {vol:.3} vs BRep {HALF_PAPPUS:.3} (deficit {:.3}); \
         bbox y[{ymin:.2},{ymax:.2}] z[{zmin:.2},{zmax:.2}] dims {dims:?}",
        HALF_PAPPUS - vol
    );
    assert!(
        (vol - HALF_PAPPUS).abs() < 20.0,
        "fine-mesh half-revolve volume within faceting tolerance of π·15·100, got {vol}"
    );

    // Y is (nearly) symmetric about the axis: [-20, +20].
    assert!(
        (ymin + 20.0).abs() < 2.0 && (ymax - 20.0).abs() < 2.0,
        "the profile-containing axis-normal (world Y) spans the full ±20, got [{ymin},{ymax}]"
    );
    // Z is ONE-SIDED: one bound sits on the profile plane (≈0), the other at ≈±20 —
    // the swept solid never crosses to the opposite half of the axis-normal.
    let z_one_sided = (zmin.abs() < 1.5 && (zmax - 20.0).abs() < 2.0)
        || (zmax.abs() < 1.5 && (zmin + 20.0).abs() < 2.0);
    assert!(
        z_one_sided,
        "a 180° sweep is one-sided along the swept axis-normal (world Z): one bound ≈0, \
         the other ≈±20, got z[{zmin},{zmax}]"
    );

    wm.shutdown().await;
    eprintln!("Revolve partial PASS: 180° volume {vol:.3} ≈ half-Pappus; bbox one-sided in Z");
}

// ─────────────────────────────────────────────────────────────────────────────
// 3. Cut-split children adopted — a boolean-mode Revolve Cut that bisects a box
//    mints deterministic split children body_<opId>:0/:1 (D1), both adopted, parent
//    consumed. Exact box arithmetic 8000 / 8800; ids + volumes stable on replay.
//    (Provenance: worker/tests/test_revolve_split.cpp — geometry reused verbatim.)
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revolve_cut_split_children_adopted() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    // Box A (XY, rect u∈[0,40] v∈[0,20], Blind +Z 25): world x∈[-20,0], y∈[0,40],
    // z∈[0,25], vol 20000.
    let sa = SketchId(Uuid::from_u128(0xA));
    add_op(
        &mut rt,
        sketch_record(
            SKETCH_A,
            &rect_sketch(sa, 0x1000, 0.0, 0.0, 40.0, 20.0),
            xy_plane_ref(),
        ),
    );
    add_op(&mut rt, extrude_record(EXTRUDE_A, sa, 25.0));

    // Tool (XZ sketch, 360° revolve): profile rect u∈[-9,60] v∈[10,14]; axis line at
    // u=-10 (outside the profile footprint, -10 < -9). r(x,y)=√(x²+(y+10)²); the
    // profile spans r∈[1,70] ⇒ a 360° washer (z-slab z∈[10,14]) that fully covers
    // box A's cross-section (box r∈[10,53.85] ⊂ [1,70]) ⇒ Cut splits A into two exact
    // box children: bottom z∈[0,10] = 8000 (:0) and top z∈[14,25] = 8800 (:1).
    let st = SketchId(Uuid::from_u128(0xB));
    let (tool_sk, axis) =
        rect_with_axis_sketch(st, 0x3000, -9.0, 10.0, 69.0, 4.0, (-10.0, 0.0, 20.0));
    add_op(
        &mut rt,
        sketch_record(SKETCH_TOOL, &tool_sk, xz_plane_ref()),
    );
    add_op(
        &mut rt,
        revolve_record(
            REVOLVE,
            st,
            "",
            360.0,
            axis,
            BooleanMode::Cut,
            Some(body_of(EXTRUDE_A)),
        ),
    );

    // Deterministic split-child ids (D1): body_<revolveOpId>:0/:1 adopted as
    // BodyId(split_child_uuid(revolveUuid, k)).
    let child0 = BodyId(split_child_uuid(Uuid::from_u128(REVOLVE), 0));
    let child1 = BodyId(split_child_uuid(Uuid::from_u128(REVOLVE), 1));

    let rep = regen_all(&mut rt).await;
    let snap = published(&rep, "revolve cut split");
    let head: std::collections::HashSet<BodyId> = snap.bodies.iter().map(|b| b.body).collect();
    assert_eq!(
        head.len(),
        2,
        "the Cut consumed box A into exactly two split children, got {head:?}"
    );
    assert!(
        !head.contains(&body_of(EXTRUDE_A)),
        "parent body (box A) removed from head"
    );
    assert!(
        head.contains(&child0) && head.contains(&child1),
        "head is exactly {{body_<op>:0, body_<op>:1}}, got {head:?}"
    );

    // Planar box children ⇒ mesh volume is EXACT (divergence theorem); tight bound.
    // Volume-ascending ordering: :0 = bottom slab 8000, :1 = top slab 8800.
    let v0 = mesh_vol(&mut rt, child0, Lod::Coarse).await;
    let v1 = mesh_vol(&mut rt, child1, Lod::Coarse).await;
    assert!(
        (v0 - 8000.0).abs() < 1.0,
        "child :0 = bottom slab 40·20·10 = 8000, got {v0}"
    );
    assert!(
        (v1 - 8800.0).abs() < 1.0,
        "child :1 = top slab 40·20·11 = 8800, got {v1}"
    );

    // Ids + volumes stable across a second from-0 regen (pure fn of opId + ordinal).
    let rep2 = regen_all(&mut rt).await;
    let snap2 = published(&rep2, "revolve cut split replay");
    let head2: std::collections::HashSet<BodyId> = snap2.bodies.iter().map(|b| b.body).collect();
    assert_eq!(head, head2, "split child ids identical across a replay");
    let v0b = mesh_vol(&mut rt, child0, Lod::Coarse).await;
    let v1b = mesh_vol(&mut rt, child1, Lod::Coarse).await;
    assert!(
        (v0b - 8000.0).abs() < 1.0 && (v1b - 8800.0).abs() < 1.0,
        "replay child volumes stable, got {v0b} / {v1b}"
    );

    wm.shutdown().await;
    eprintln!(
        "Revolve Cut-split PASS: parent consumed, children :0={v0} :1={v1}, ids + volumes stable"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 4. Stale axis fails loud — a nonexistent axis lineId ⇒ NO silent body, NO panic:
//    the revolve step is Error and its projection FeatureDto carries the worker's
//    axis message (W0.5 statusMessage contract).
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revolve_stale_axis_fails_loud() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    // A valid profile (so the profile builds and the axis is the ONLY failure point:
    // execute_revolve resolves the profile BEFORE the axis, RevolveOp.cpp:154-175),
    // then a revolve whose axis references a lineId that does not exist in the sketch.
    let sid = SketchId(Uuid::from_u128(0xA));
    add_op(
        &mut rt,
        sketch_record(
            SKETCH_A,
            &rect_sketch(sid, 0x1000, 10.0, 0.0, 10.0, 10.0),
            xy_plane_ref(),
        ),
    );
    let bogus_axis = EntityId(Uuid::from_u128(0xDEAD_BEEF));
    add_op(
        &mut rt,
        revolve_record(
            REVOLVE,
            sid,
            "",
            360.0,
            bogus_axis,
            BooleanMode::NewBody,
            None,
        ),
    );

    // A per-op failure is a Published snapshot with the failed step marked Error
    // (executor.rs:668; PlanExecutor stoppedReason=opFailed, last_valid_step=m-1) —
    // NOT an EngineFailed. No revolve body is created (a sketch mints none either).
    let rep = regen_all(&mut rt).await;
    let snap = published(&rep, "stale axis revolve");
    assert!(
        snap.bodies.is_empty(),
        "the failed revolve minted NO body (no silent body), got {:?}",
        snap.bodies.iter().map(|b| b.body).collect::<Vec<_>>()
    );

    // The projection surfaces the failure: the revolve FeatureDto is Error and its
    // statusMessage carries the worker's axis message
    // (RevolveOp.cpp:86 "Revolve: axis line '<id>' not found in sketch").
    let proj = rt.projection();
    let rev_id = RecordId(Uuid::from_u128(REVOLVE)).to_string();
    let feat = proj
        .features
        .iter()
        .find(|f| f.id == rev_id)
        .expect("the revolve feature is projected");
    assert_eq!(
        feat.status,
        FeatureStatus::Error,
        "the stale-axis revolve step is Error, got {:?}",
        feat.status
    );
    let msg = feat
        .status_message
        .as_deref()
        .expect("an errored feature carries a statusMessage (W0.5)");
    eprintln!("stale-axis: revolve statusMessage = {msg:?}");
    assert!(
        msg.contains("not found in sketch"),
        "statusMessage carries the worker's axis-not-found message, got {msg:?}"
    );

    wm.shutdown().await;
    eprintln!("Revolve stale-axis PASS: 0 bodies, revolve step Error, statusMessage surfaced");
}

// ─────────────────────────────────────────────────────────────────────────────
// 5. Region binding — an explicit normative regionId AND the "" first-region
//    fallback both publish a body. The explicit id is fetched from the solver lane
//    (SketchUpsert + SketchRegions), the exact source finish_sketch reads; a wrong
//    non-empty id would fail loudly (strict M4a rule), so a publish proves the bind.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revolve_region_binding_explicit_and_fallback() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary");
        return;
    };
    let wm = spawn_worker(bin).await;

    let sid = SketchId(Uuid::from_u128(0xA));
    // The plan sketch (rect + axis). The rect's outer-loop member ids are the sole
    // input to the normative FNV regionId (derive_region_id); the dangling axis line
    // is not part of any loop, so a PLAIN rect (same rect line ids) yields the SAME
    // regionId while sidestepping any solver-lane region-detection quirk on the open
    // line. Fetch it via the solver lane — exactly what finish_sketch does.
    let (plan_sk, axis) =
        rect_with_axis_sketch(sid, 0x1000, 10.0, 0.0, 10.0, 10.0, (0.0, -5.0, 15.0));
    let plain_rect = rect_sketch(sid, 0x1000, 10.0, 0.0, 10.0, 10.0);
    let _ = SolverEngine::sketch_upsert(&wm, &plain_rect)
        .await
        .expect("upsert sketch to solver lane");
    let regions = SolverEngine::sketch_regions(&wm, &sid.to_string())
        .await
        .expect("solver-lane sketch regions");
    assert!(!regions.is_empty(), "the rect yields ≥1 closed region");
    let region_id = regions[0].region_id.clone();
    assert!(
        region_id.starts_with("r_"),
        "a normative FNV regionId (SCHEMA §7.4), got {region_id:?}"
    );

    let mut rt = runtime_over(&wm);
    add_op(&mut rt, sketch_record(SKETCH_A, &plan_sk, xy_plane_ref()));
    // Revolve 1: explicit, valid regionId. Revolve 2: "" first-region fallback.
    add_op(
        &mut rt,
        revolve_record(
            REVOLVE,
            sid,
            &region_id,
            360.0,
            axis,
            BooleanMode::NewBody,
            None,
        ),
    );
    add_op(
        &mut rt,
        revolve_record(REVOLVE_2, sid, "", 360.0, axis, BooleanMode::NewBody, None),
    );

    let rep = regen_all(&mut rt).await;
    let snap = published(&rep, "revolve region binding");
    let head: std::collections::HashSet<BodyId> = snap.bodies.iter().map(|b| b.body).collect();
    assert_eq!(
        head.len(),
        2,
        "the explicit-region revolve AND the fallback revolve both publish, got {head:?}"
    );
    assert!(
        head.contains(&body_of(REVOLVE)) && head.contains(&body_of(REVOLVE_2)),
        "both revolve bodies present, got {head:?}"
    );

    // Finding 7: a WRONG (non-empty, non-matching) regionId is a deterministic HARD
    // FAILURE whose message names BOTH the requested id and the available ids — proof
    // the regionId actually reaches the worker. Without this case the explicit-region
    // test is indistinguishable from the "" fallback under an id-DROPPING marshalling
    // regression (both would still publish a first-region body).
    let bogus = "r_deadbeefdeadbeef";
    add_op(
        &mut rt,
        revolve_record(
            REVOLVE_3,
            sid,
            bogus,
            360.0,
            axis,
            BooleanMode::NewBody,
            None,
        ),
    );
    let rep = regen_all(&mut rt).await;
    // A per-op failure still Publishes the valid prefix (the two good revolves); the
    // wrong-region step is Error with NO body of its own (mirrors the stale-axis case).
    let snap = published(&rep, "wrong-region revolve");
    let head: std::collections::HashSet<BodyId> = snap.bodies.iter().map(|b| b.body).collect();
    assert!(
        !head.contains(&body_of(REVOLVE_3)),
        "the wrong-region revolve minted NO body, got {head:?}"
    );
    let proj = rt.projection();
    let rev3 = RecordId(Uuid::from_u128(REVOLVE_3)).to_string();
    let feat = proj
        .features
        .iter()
        .find(|f| f.id == rev3)
        .expect("the wrong-region revolve feature is projected");
    assert_eq!(
        feat.status,
        FeatureStatus::Error,
        "a non-matching regionId is a deterministic OP_FAILED, got {:?}",
        feat.status
    );
    let msg = feat
        .status_message
        .as_deref()
        .expect("an errored feature carries a statusMessage (W0.5)");
    assert!(
        msg.contains(bogus),
        "the message names the REQUESTED regionId (proves it reached the worker), got {msg:?}"
    );
    assert!(
        msg.contains("available") && msg.contains(&region_id),
        "the message names the AVAILABLE region ids, got {msg:?}"
    );

    wm.shutdown().await;
    eprintln!(
        "Revolve region-binding PASS: explicit regionId {region_id} + \"\" fallback both published; \
         wrong regionId '{bogus}' ⇒ OP_FAILED naming requested + available — {msg}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 6. REVOLVE-REGION-PARITY: the interactive frontend sequence, end to end, through
//    the PRODUCTION scheduler. Arming a revolve is now a PURE region read
//    (`prepare_sketch_regions` — the backend of `getSketchRegions`), which by
//    design authors no timeline record; the record is guaranteed at the COMMIT
//    boundary instead (`confirmRevolve`'s `finishSketch`). This test walks that
//    exact ladder and proves the revolve commits.
//    (Provenance: docs/POSTMORTEM_EXTRUDE_COMMIT.md; shape mirrors
//    scheduler_commit.rs::interactive_sketch_flow_mints_the_timeline_record_and_extrude_commits.)
// ─────────────────────────────────────────────────────────────────────────────

/// The drawn content shared by the two interactive-flow tests: the Pappus fixture
/// (10×10 rect at `u ∈ [10,20]`, `v ∈ [0,10]`) plus a dangling axis line at `u = 0`.
/// Returns `(sketch, axis line id)`.
fn interactive_profile(sid: SketchId) -> (Sketch, EntityId) {
    rect_with_axis_sketch(sid, 0x1000, 10.0, 0.0, 10.0, 10.0, (0.0, -5.0, 15.0))
}

/// Runs AddSketch → enter → upsert → cancel (the `SketchController.exit` teardown
/// half) → `prepare_sketch_regions().drive()` (the pure-read arm), asserting that
/// NOTHING in that ladder minted a `Sketch` timeline record. Returns the armed
/// region's normative id.
async fn interactive_arm(runtime: &Runtime, sid: SketchId, drawn: &Sketch) -> String {
    {
        let mut guard = runtime.lock().await;
        let rt = guard.as_mut().expect("document open");
        rt.apply(EditCommand::AddSketch {
            sketch: Sketch::on_world_plane(sid, "Sketch 1", WorldPlane::XY),
        })
        .expect("AddSketch");
        rt.enter_sketch(sid).await.expect("enter_sketch");
        rt.sketch_upsert(sid, sketch_edit_ops(drawn))
            .await
            .expect("sketch_upsert");
        // SketchController.exit()'s FIRST half: worker-gesture teardown + squash.
        rt.cancel_sketch(sid).await.expect("cancel_sketch");
        assert_eq!(
            sketch_record_count(rt),
            0,
            "AddSketch + upsert + cancel author NO Sketch timeline record (this is \
             precisely why the commit boundary must guarantee one)"
        );
    }

    // The arm: a PURE read driven with the runtime lock RELEASED (production shape).
    let prepared = {
        let guard = runtime.lock().await;
        guard
            .as_ref()
            .expect("document open")
            .prepare_sketch_regions(sid)
            .expect("prepare_sketch_regions")
    };
    let regions = prepared.drive().await.expect("regions").regions;
    assert_eq!(
        regions.len(),
        1,
        "the rect derives exactly one closed region (the dangling axis line closes none), got {:?}",
        regions.iter().map(|r| &r.region_id).collect::<Vec<_>>()
    );
    {
        let guard = runtime.lock().await;
        assert_eq!(
            sketch_record_count(guard.as_ref().expect("document open")),
            0,
            "the PURE-READ arm must not author a Sketch record either"
        );
    }
    regions[0].region_id.clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revolve_commit_after_pure_read_arm_mints_the_profile_record() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary");
        return;
    };
    let wm = spawn_worker(bin).await;
    let (runtime, sched, mut rx) = wire_with(Arc::new(Mutex::new(Some(runtime_over(&wm))))).await;
    let sid = SketchId(Uuid::from_u128(0xF0A));
    let (drawn, axis) = interactive_profile(sid);

    let region_id = interactive_arm(&runtime, sid, &drawn).await;

    // The COMMIT-BOUNDARY GUARANTEE (`confirmRevolve` → `client.finishSketch`): the
    // only step in the whole interactive ladder that mints the `Sketch` record.
    let finish_outcome = {
        let mut guard = runtime.lock().await;
        let rt = guard.as_mut().expect("document open");
        let (_dto, outcome) = rt
            .finish_sketch_with_outcome(sid)
            .await
            .expect("finish_sketch");
        assert_eq!(
            sketch_record_count(rt),
            1,
            "the commit-boundary finish minted exactly ONE Sketch timeline record"
        );
        outcome.expect("the first finish appends the record ⇒ Some(outcome)")
    };
    sched.handle(&finish_outcome);
    let (outcome_str, _, _) = recv(&mut rx).await;
    assert_eq!(outcome_str, "published", "the sketch-only regen publishes");

    // The commit: the exact frontend shape — a TYPED profile (sketchId + the
    // normative regionId read at arm time) plus the picked axis line.
    let revolve_outcome = {
        let mut guard = runtime.lock().await;
        let rt = guard.as_mut().expect("document open");
        rt.apply(EditCommand::AddOperation {
            record: revolve_record(
                REVOLVE,
                sid,
                &region_id,
                360.0,
                axis,
                BooleanMode::NewBody,
                None,
            ),
            at_cursor: false, // the frontend commit shape
        })
        .expect("revolve commit")
    };
    sched.handle(&revolve_outcome);
    let (outcome_str, change, proj) = recv(&mut rx).await;
    assert_eq!(outcome_str, "published", "the revolve commit published");
    let change = change.expect("a published regen carries a document_change");
    assert_eq!(
        change.changed_bodies.len(),
        1,
        "the revolve off the interactively drawn sketch produced its body, got {:?}",
        change.changed_bodies
    );
    let rev_id = RecordId(Uuid::from_u128(REVOLVE)).to_string();
    let feat = proj
        .features
        .iter()
        .find(|f| f.id == rev_id)
        .expect("the revolve feature is projected");
    assert_eq!(
        feat.status,
        FeatureStatus::Ok,
        "the revolve step succeeded, statusMessage={:?}",
        feat.status_message
    );

    // The body is the RIGHT one: the Pappus solid of the armed region (same fixture
    // and same fine-mesh faceting regime as `revolve_new_body_pappus_volume`).
    let vol = {
        let mut guard = runtime.lock().await;
        let rt = guard.as_mut().expect("document open");
        mesh_vol(rt, body_of(REVOLVE), Lod::Fine).await
    };
    const PAPPUS: f64 = 9424.778; // 2π·15·100
    assert!(
        (vol - PAPPUS).abs() < 40.0,
        "the committed body is the armed region's Pappus solid, got {vol}"
    );

    sched.shutdown();
    wm.shutdown().await;
    eprintln!(
        "REVOLVE-INTERACTIVE-FLOW PASS: pure-read arm authored no record; the commit-boundary \
         finish minted it; the typed Revolve published 1 body (volume {vol:.3} ≈ Pappus)"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 7. The regression TRIPWIRE for the test above: the SAME ladder with the
//    commit-boundary `finish_sketch` REMOVED must fail loudly. Delete the
//    guarantee from `confirmRevolve` and this test goes green while every real
//    revolve silently does nothing — the exact failure mode the extrude
//    postmortem documents.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revolve_commit_without_the_record_guarantee_fails_loud() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary");
        return;
    };
    let wm = spawn_worker(bin).await;
    let (runtime, sched, mut rx) = wire_with(Arc::new(Mutex::new(Some(runtime_over(&wm))))).await;
    let sid = SketchId(Uuid::from_u128(0xF0A));
    let (drawn, axis) = interactive_profile(sid);

    let region_id = interactive_arm(&runtime, sid, &drawn).await;
    // ── NO finish_sketch here — that is the whole point of this test. ──

    let revolve_outcome = {
        let mut guard = runtime.lock().await;
        let rt = guard.as_mut().expect("document open");
        rt.apply(EditCommand::AddOperation {
            record: revolve_record(
                REVOLVE,
                sid,
                &region_id,
                360.0,
                axis,
                BooleanMode::NewBody,
                None,
            ),
            at_cursor: false,
        })
        .expect("revolve commit")
    };
    sched.handle(&revolve_outcome);
    let (outcome_str, change, proj) = recv(&mut rx).await;

    // A per-op failure still publishes the (empty) valid prefix — the failure is
    // carried by the step, not by the regen outcome.
    assert_eq!(outcome_str, "published");
    assert!(
        change.is_none_or(|c| c.changed_bodies.is_empty()),
        "the recordless revolve minted NO body"
    );
    let rev_id = RecordId(Uuid::from_u128(REVOLVE)).to_string();
    let feat = proj
        .features
        .iter()
        .find(|f| f.id == rev_id)
        .expect("the revolve feature is projected");
    assert_eq!(
        feat.status,
        FeatureStatus::Error,
        "a revolve whose profile sketch has no timeline record is a hard step failure"
    );
    let msg = feat
        .status_message
        .as_deref()
        .expect("an errored feature carries a statusMessage (W0.5)");
    assert!(
        msg.contains("profile sketch not found in plan"),
        "the tripwire message is the postmortem's, got {msg:?}"
    );

    sched.shutdown();
    wm.shutdown().await;
    eprintln!(
        "REVOLVE-TRIPWIRE PASS: without the commit-boundary finish the revolve fails with {msg:?} \
         (0 bodies) — this is what the guarantee prevents"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 8. Re-edit binds the STORED region, never the first. A two-region sketch whose
//    revolve is bound to region index 1 by exact id survives an angle re-edit
//    (`UpdateOperationParams`, the deep-merge the frontend sends): the published
//    volume tracks THAT region's Pappus solid at the new angle. Before the FE fix
//    a re-edit re-armed `regions[0]` and would have re-authored the wrong profile.
// ─────────────────────────────────────────────────────────────────────────────

/// Two disjoint rects (`u ∈ [0,40] v ∈ [0,20]` and `u ∈ [60,70] v ∈ [0,10]`, areas
/// 800 vs 100) plus a dangling axis line at `u = -10` — left of BOTH footprints, so
/// it splits neither profile. Returns `(sketch, axis line id)`.
fn two_region_with_axis_sketch(sid: SketchId) -> (Sketch, EntityId) {
    let (mut sk, axis) =
        rect_with_axis_sketch(sid, 0x5000, 0.0, 0.0, 40.0, 20.0, (-10.0, -5.0, 25.0));
    let second = rect_sketch(sid, 0x6000, 60.0, 0.0, 10.0, 10.0);
    for entity in second.entities().iter().cloned() {
        sk.add_entity(entity).unwrap();
    }
    for constraint in second.constraints().iter().cloned() {
        sk.add_constraint(constraint).unwrap();
    }
    (sk, axis)
}

const AXIS_U: f64 = -10.0;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revolve_reedit_binds_the_stored_region_not_the_first() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let sid = SketchId(Uuid::from_u128(0xE0A));
    let (sk, axis) = two_region_with_axis_sketch(sid);

    rt.apply(EditCommand::AddSketch { sketch: sk.clone() })
        .expect("AddSketch");
    add_op(&mut rt, sketch_record(SKETCH_A, &sk, xy_plane_ref()));

    // The arm's region read (the exact source the frontend binds from).
    let regions = rt
        .prepare_sketch_regions(sid)
        .expect("prepare_sketch_regions")
        .drive()
        .await
        .expect("regions")
        .regions;
    assert_eq!(regions.len(), 2, "both disjoint cells are selectable");
    let bound = &regions[1]; // NOT regions[0] — the old silent fallback
    let other = &regions[0];

    let want_360 = pappus_volume(bound, AXIS_U, 360.0);
    let wrong_360 = pappus_volume(other, AXIS_U, 360.0);
    assert!(
        (want_360 - wrong_360).abs() > want_360.max(wrong_360) * 0.5,
        "the fixture regions must sweep VERY different volumes for this test to \
         discriminate ({want_360} vs {wrong_360})"
    );

    add_op(
        &mut rt,
        revolve_record(
            REVOLVE,
            sid,
            &bound.region_id,
            360.0,
            axis,
            BooleanMode::NewBody,
            None,
        ),
    );
    let report = regen_all(&mut rt).await;
    let snap = published(&report, "two-region revolve");
    assert_eq!(snap.bodies.len(), 1, "one revolve body");
    let vol = mesh_vol(&mut rt, body_of(REVOLVE), Lod::Fine).await;
    eprintln!(
        "bound region {} 360°: mesh {vol:.3} vs Pappus {want_360:.3} (the OTHER region \
         would be {wrong_360:.3})",
        bound.region_id
    );
    // Faceting deficit on a revolved solid is ~0.2%; 1.5% is ≈7× headroom yet far
    // tighter than the ≥50% gap to the wrong region's volume.
    assert!(
        (vol - want_360).abs() < want_360 * 0.015,
        "the bound region's Pappus solid ({want_360}), got {vol}"
    );
    assert!(
        (vol - wrong_360).abs() > wrong_360 * 0.1,
        "and emphatically NOT the first region's solid ({wrong_360}), got {vol}"
    );

    // The re-edit: an angle-only `UpdateOperationParams` carrying the SAME typed
    // profile + axis (the frontend's deep-merge shape). The binding must survive.
    let reedit = revolve_record(
        REVOLVE,
        sid,
        &bound.region_id,
        180.0,
        axis,
        BooleanMode::NewBody,
        None,
    );
    rt.apply(EditCommand::UpdateOperationParams {
        record: RecordId(Uuid::from_u128(REVOLVE)),
        op: reedit.op,
    })
    .expect("angle re-edit");
    let report = regen_all(&mut rt).await;
    let snap = published(&report, "two-region revolve re-edit");
    assert_eq!(
        snap.bodies.len(),
        1,
        "still one revolve body after the re-edit"
    );

    let want_180 = pappus_volume(bound, AXIS_U, 180.0);
    let wrong_180 = pappus_volume(other, AXIS_U, 180.0);
    let vol2 = mesh_vol(&mut rt, body_of(REVOLVE), Lod::Fine).await;
    eprintln!(
        "after the 180° re-edit: mesh {vol2:.3} vs Pappus {want_180:.3} (the OTHER region \
         would be {wrong_180:.3})"
    );
    assert!(
        (vol2 - want_180).abs() < want_180 * 0.015,
        "the re-edited angle applied to the SAME bound region ({want_180}), got {vol2}"
    );
    assert!(
        (vol2 - wrong_180).abs() > wrong_180 * 0.1,
        "the re-edit did not silently re-bind to the first region ({wrong_180}), got {vol2}"
    );

    wm.shutdown().await;
    eprintln!(
        "REVOLVE-REEDIT-BINDING PASS: region {} held across a 360°→180° \
         UpdateOperationParams ({vol:.3} → {vol2:.3})",
        bound.region_id
    );
}
