//! Revolve drag-time PREVIEW gate against the REAL C++ OCCT worker, driven through
//! the app's [`DocumentRuntime`] exactly like `wire_contract.rs` / `revolve_ops.rs`.
//!
//! Until now Revolve had no kernel preview at all: the drag showed only the
//! frontend's L1 Rodrigues lathe, a solid shell synthesized in JavaScript. That shell
//! is a *lie* for a boolean-mode revolve — a Cut dragged as if it were adding
//! material and the subtraction only appeared after the commit. `PreviewOp` (SCHEMA
//! §7.6) runs the candidate against a throwaway copy of the head, so the drag mesh IS
//! what a commit would produce. These two tests pin exactly that for Revolve:
//!
//! * `revolve_preview_matches_the_commit` — a 10×10 rect at radius 15 previewed at
//!   90° NewBody: the preview volume matches the analytic quarter-Pappus solid within
//!   1%, the HEAD is byte-identical afterwards (same snapshot id + generation, same
//!   per-body signatures, same document revision), and committing the SAME op lands
//!   on the volume the preview promised (|committed − preview| < 1).
//! * `revolve_cut_preview_subtracts` — the Extrude-Cut-class proof for Revolve: a
//!   360° Cut against an existing box reports a `modified` bodyEvent for the TARGET
//!   body, previews a volume strictly BELOW the base (the subtraction the L1 lathe
//!   could never show), leaves the real body untouched, and again agrees with the
//!   commit.
//!
//! Both volume comparisons are taken at `Lod::Fine` on BOTH sides, so the preview and
//! the commit are the same tessellation regime and the < 1.0 agreement bound is a
//! real equality assertion, not a faceting-tolerance one. The analytic bound is 1%
//! because a revolved (curved) solid's faceted mesh under-estimates the true BRep
//! volume by ~0.2% at Fine — BRep-exact volume is ctest's job (`test_wp6_ops`).
//!
//! Provenance: the Pappus fixture (10×10 rect at radius 15 about a sketch-line axis)
//! is `worker/tests/test_wp6_ops.cpp:165-201`, reused verbatim from `revolve_ops.rs`;
//! the Cut geometry adapts `worker/tests/test_revolve_split.cpp`'s frame mapping with
//! a NARROWED washer so the cut leaves ONE connected solid (a partial bite) instead
//! of the split compound that test deliberately produces.
//!
//! Gated on `ONECAD_WORKER_PATH` (else the dev-tree fallback); a missing binary skips
//! cleanly unless `ONECAD_REQUIRE_WORKER=1` (CI hard-fails).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use onecad_core::document::record::{
    BooleanMode, ExtrudeMode, ExtrudeParams, KnownOperation, Operation, OperationRecord, PlaneKind,
    RevolveParams, SketchOpParams, SketchPlaneRef,
};
use onecad_core::document::refs::{AxisRef, SketchRegionRef};
use onecad_core::document::variables::Scalar;
use onecad_core::edit::EditCommand;
use onecad_core::ids::{BodyId, ConstraintId, EntityId, RecordId, RegionId, SketchId};
use onecad_core::math::{Vec2, Vec3};
use onecad_core::regen::{CancelToken, GeometryEngine, Lod, ModelSnapshot, Outcome, RegenRequest};
use onecad_core::sketch::{Constraint, Sketch, SketchEntity, WorldPlane};

use onecad_lib::document_runtime::{DocumentRuntime, RegenReport};
use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::wire::sketch_wire;
use onecad_lib::worker::{
    resolve_worker_path, MeshProvider, PreviewEngine, SolverEngine, WorkerManager,
};

use onecad_protocol::mesh::{f32_le, u32_le, validate_mesh_blob, MeshHeaderView};

// ─────────────────────────────────────────────────────────────────────────────
// Harness (mirrors revolve_ops.rs / wire_contract.rs)
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

/// The whole observable head state a preview MUST NOT touch: the published
/// snapshot's id + generation, every body's worker-computed topology signature, and
/// the document revision. Compared verbatim before and after `PreviewOp`.
fn head_fingerprint(rt: &DocumentRuntime) -> (u64, u64, u64, Vec<(String, String)>) {
    let snapshot = rt
        .subscribe_snapshots()
        .borrow()
        .clone()
        .expect("a published head snapshot");
    let mut bodies: Vec<(String, String)> = snapshot
        .bodies
        .iter()
        .map(|b| (b.body.to_string(), b.signature.0.clone()))
        .collect();
    bodies.sort();
    (
        snapshot.id.0,
        snapshot.generation,
        rt.projection().revision,
        bodies,
    )
}

// Fixed record ids.
const SKETCH_A: u128 = 0xA00; // profile / base-box sketch
const EXTRUDE_A: u128 = 0xA01; // base box body producer
const SKETCH_TOOL: u128 = 0xB00; // revolve tool sketch (Cut test)
const REVOLVE: u128 = 0xB01; // the previewed + committed revolve

// ─────────────────────────────────────────────────────────────────────────────
// Sketch plane refs. The worker's `parse_plane` uses its OWN canonical frame for a
// NAMED plane kind and ignores the explicit axes, so only `kind` is load-bearing on
// the wire; the axes below document the matching canonical frame:
//   XY: toWorld(u,v) = (-v, u, 0)
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

// ─────────────────────────────────────────────────────────────────────────────
// Sketch + op record builders (verbatim from revolve_ops.rs)
// ─────────────────────────────────────────────────────────────────────────────

/// A fully-constrained (dof 0) rectangle at sketch-space `(x0, y0)`, size `w × h`,
/// built the marshaller way (8 points, 4 lines, coincident corners, H/V, a Fixed
/// anchor, H/V dimensions).
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
/// sketch-`u` `ax_u`, spanning `v ∈ [ax_v0, ax_v1]`. The axis is a plain (open) line:
/// it joins no closed loop, so region detection ignores it, yet it is still a
/// resolvable `SketchLine` for `AxisRef::SketchLine`. Returns `(sketch, axis lineId)`.
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

/// The Revolve op the frontend's preview lane builds and its commit materializes —
/// ONE record, used first as a `PreviewOp` candidate and then committed verbatim.
fn revolve_record(
    rec: u128,
    sketch: SketchId,
    angle_deg: f64,
    axis_line: EntityId,
    boolean: BooleanMode,
    target: Option<BodyId>,
) -> OperationRecord {
    let params = RevolveParams {
        profile: Some(SketchRegionRef {
            sketch,
            region: RegionId::new(""), // first-region fallback (V1)
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
// MESH1 geometry helpers (from revolve_ops.rs)
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
/// polyhedron; for a faceted CURVED solid it is the inscribed-mesh volume.
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

fn blob_volume(blob: &[u8], what: &str) -> f64 {
    let view = validate_mesh_blob(blob).unwrap_or_else(|e| panic!("{what}: MESH1 invalid: {e:?}"));
    mesh_volume(&view, blob)
}

async fn mesh_vol(rt: &mut DocumentRuntime, body: BodyId, lod: Lod) -> f64 {
    let mesh = rt.get_mesh(body, lod, None).await.expect("fetch body mesh");
    blob_volume(&mesh, "body mesh")
}

// ─────────────────────────────────────────────────────────────────────────────
// 1. NewBody: preview == analytic == commit, and the head is untouched.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revolve_preview_matches_the_commit() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    // XY toWorld(u,v)=(-v,u,0): rect u∈[10,20] v∈[0,10] ⇒ area 100, centroid world-y
    // 15 ⇒ radius 15 from the u=0 axis line (which maps to the world X-axis).
    // 90° ⇒ V = (1/4)·2π·15·100 = 2356.194.
    let sid = SketchId(Uuid::from_u128(0xA));
    let (sk, axis) = rect_with_axis_sketch(sid, 0x1000, 10.0, 0.0, 10.0, 10.0, (0.0, -5.0, 15.0));
    add_op(&mut rt, sketch_record(SKETCH_A, &sk, xy_plane_ref()));
    rt.apply(EditCommand::AddSketch { sketch: sk.clone() })
        .expect("AddSketch");
    let base = regen_all(&mut rt).await;
    assert!(
        published(&base, "profile sketch").bodies.is_empty(),
        "the head is a bare sketch — no body yet"
    );

    // The sketch must be in the worker's SketchStore for the preview to seed its
    // profile — exactly the state a real drag is in when it previews.
    rt.enter_sketch(sid).await.expect("enter_sketch");

    let before = head_fingerprint(&rt);

    // PREVIEW the candidate the drag would send at 90°.
    let candidate = revolve_record(REVOLVE, sid, 90.0, axis, BooleanMode::NewBody, None);
    let preview = wm
        .preview_op(
            candidate.op.clone(),
            candidate.record_id.to_string(),
            Some(sid.0.to_string()),
            None,
            Lod::Fine,
        )
        .await
        .expect("PreviewOp reaches the worker");

    assert_eq!(
        preview.needs_repair.len(),
        0,
        "a well-formed revolve candidate resolves cleanly: {:?}",
        preview.needs_repair
    );
    assert_eq!(
        preview.bodies.len(),
        1,
        "a NewBody revolve previews exactly one body"
    );
    let created: Vec<&str> = preview
        .body_events
        .iter()
        .filter(|e| e.kind == "created")
        .map(|e| e.body_id.as_str())
        .collect();
    assert_eq!(
        created,
        vec![body_of(REVOLVE).to_string().as_str()],
        "the NewBody event is the deterministic body_<opId> (D1), events={:?}",
        preview.body_events
    );

    const QUARTER_PAPPUS: f64 = 2356.194; // (90/360)·2π·15·100
    let pv = blob_volume(&preview.bodies[0].mesh, "preview");
    eprintln!(
        "revolve 90° preview volume = {pv:.3} vs analytic {QUARTER_PAPPUS:.3} \
         (faceting deficit {:.3})",
        QUARTER_PAPPUS - pv
    );
    assert!(
        (pv - QUARTER_PAPPUS).abs() < QUARTER_PAPPUS * 0.01,
        "the preview must already BE the quarter-Pappus solid (±1%), got {pv}"
    );

    // The head is exactly where it was: same snapshot, same signatures, same revision.
    let after = head_fingerprint(&rt);
    assert_eq!(
        before, after,
        "a preview must leave the head fingerprint (snapshot id/generation, body \
         signatures, document revision) byte-identical"
    );
    assert_eq!(
        preview.snapshot_id, before.0,
        "the preview ran against the HEAD snapshot and minted none of its own"
    );

    // Committing the SAME op must land on the volume the preview promised.
    add_op(&mut rt, candidate);
    let snap = regen_all(&mut rt).await;
    assert_eq!(
        published(&snap, "commit after preview").bodies.len(),
        1,
        "the commit publishes exactly the previewed body"
    );
    let committed = mesh_vol(&mut rt, body_of(REVOLVE), Lod::Fine).await;
    eprintln!("revolve preview {pv:.3} vs commit {committed:.3}");
    assert!(
        (committed - pv).abs() < 1.0,
        "preview {pv} and commit {committed} must agree — that is the entire point"
    );

    wm.shutdown().await;
    eprintln!("Revolve preview/commit agreement PASS: {pv:.3} == {committed:.3}");
}

// ─────────────────────────────────────────────────────────────────────────────
// 2. Cut: the preview SUBTRACTS. This is the class of defect the whole lane exists
//    for — the L1 lathe drew a solid shell and a Cut drag looked like an addition.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revolve_cut_preview_subtracts() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    // Box A (XY, rect u∈[0,40] v∈[0,20], Blind +Z 25): world x∈[-20,0], y∈[0,40],
    // z∈[0,25] ⇒ volume 20000.
    let sa = SketchId(Uuid::from_u128(0xA));
    let rect_a = rect_sketch(sa, 0x1000, 0.0, 0.0, 40.0, 20.0);
    add_op(&mut rt, sketch_record(SKETCH_A, &rect_a, xy_plane_ref()));
    rt.apply(EditCommand::AddSketch {
        sketch: rect_a.clone(),
    })
    .expect("AddSketch A");
    add_op(&mut rt, extrude_record(EXTRUDE_A, sa, 25.0));

    // Tool: an XY sketch too — `rect_sketch` builds its document Sketch on
    // `WorldPlane::XY`, and `enter_sketch` pushes THAT into the worker's SketchStore,
    // which is the store `PreviewOp` seeds its profile + axis from. Authoring the
    // timeline record on a DIFFERENT plane would make the preview and the commit
    // revolve two different solids (observed: the preview removed 1600 where the
    // commit removed 1408). Record plane == document plane, always.
    //
    // XY toWorld(u,v) = (-v, u, 0): profile rect u∈[0,25] v∈[0,20] ⇒ world
    // y∈[0,25], x∈[-20,0], z=0; the axis line at u=-5 maps to the world-X-parallel
    // line through (y=-5, z=0). Revolving 360° sweeps an annular TUBE along
    // x∈[-20,0] with r=√((y+5)²+z²) ∈ [5,30].
    //
    // Box A's (y,z) section spans r∈[5,51.5], so the tube bites the low-r part and
    // leaves the high-y/high-z corner connected ⇒ ONE solid out (a `modified` event),
    // not the split compound `test_revolve_split.cpp` deliberately produces.
    // Analytic removed volume: 20·∫₀²⁵(√(900−z²) − 5)dz = 20·525.59 = 10511.8.
    let st = SketchId(Uuid::from_u128(0xB));
    let (tool_sk, axis) =
        rect_with_axis_sketch(st, 0x3000, 0.0, 0.0, 25.0, 20.0, (-5.0, -10.0, 30.0));
    add_op(
        &mut rt,
        sketch_record(SKETCH_TOOL, &tool_sk, xy_plane_ref()),
    );
    rt.apply(EditCommand::AddSketch {
        sketch: tool_sk.clone(),
    })
    .expect("AddSketch tool");

    let base = regen_all(&mut rt).await;
    assert_eq!(
        published(&base, "base box").bodies.len(),
        1,
        "the head is the single base box"
    );
    // The tool sketch must be in the worker's SketchStore for the preview to seed
    // from it — exactly the state a real drag is in when it previews.
    rt.enter_sketch(st).await.expect("enter_sketch tool");

    let base_vol = mesh_vol(&mut rt, body_of(EXTRUDE_A), Lod::Fine).await;
    assert!(
        (base_vol - 20000.0).abs() < 1.0,
        "base box is 20000, got {base_vol}"
    );
    let before = head_fingerprint(&rt);

    // PREVIEW the Cut candidate.
    let candidate = revolve_record(
        REVOLVE,
        st,
        360.0,
        axis,
        BooleanMode::Cut,
        Some(body_of(EXTRUDE_A)),
    );
    let preview = wm
        .preview_op(
            candidate.op.clone(),
            candidate.record_id.to_string(),
            Some(st.0.to_string()),
            None,
            Lod::Fine,
        )
        .await
        .expect("PreviewOp reaches the worker");

    assert_eq!(
        preview.needs_repair.len(),
        0,
        "the Cut target resolves cleanly: {:?}",
        preview.needs_repair
    );
    let target = body_of(EXTRUDE_A).to_string();
    let modified: Vec<&str> = preview
        .body_events
        .iter()
        .filter(|e| e.kind == "modified")
        .map(|e| e.body_id.as_str())
        .collect();
    assert_eq!(
        modified,
        vec![target.as_str()],
        "a Cut MODIFIES the target body (never creates one), events={:?}",
        preview.body_events
    );
    assert_eq!(
        preview.changed_bodies,
        vec![target.clone()],
        "changedBodies is exactly the cut target"
    );
    assert!(
        preview.deleted_bodies.is_empty(),
        "a partial bite deletes nothing, got {:?}",
        preview.deleted_bodies
    );
    assert_eq!(
        preview.bodies.len(),
        1,
        "the partial cut leaves ONE connected solid (no split compound)"
    );

    // THE assertion: the preview already shows the SUBTRACTION. Pre-lane the drag
    // showed the un-subtracted L1 lathe shell instead — a Cut looked like an Add.
    let pv = blob_volume(&preview.bodies[0].mesh, "preview");
    let removed = base_vol - pv;
    const ANALYTIC_REMOVED: f64 = 10511.8; // 20·∫₀²⁵(√(900−z²) − 5)dz
    eprintln!(
        "revolve Cut preview volume = {pv:.3} (base {base_vol:.3}, removed {removed:.3} \
         vs analytic {ANALYTIC_REMOVED:.1})"
    );
    assert!(
        pv < base_vol - 1.0,
        "the Cut preview must remove material (base {base_vol}), got {pv}"
    );
    // …and it must remove the RIGHT material: the previewed solid is the analytic
    // one within the faceting band, which is what catches a preview that revolved a
    // different profile/axis than the commit would.
    assert!(
        (removed - ANALYTIC_REMOVED).abs() < ANALYTIC_REMOVED * 0.02,
        "the Cut preview must remove the analytic {ANALYTIC_REMOVED} (±2%), got {removed}"
    );

    // And the real body is untouched: same head fingerprint, same geometry.
    let after = head_fingerprint(&rt);
    assert_eq!(
        before, after,
        "a preview must leave the head fingerprint byte-identical"
    );
    assert_eq!(
        preview.snapshot_id, before.0,
        "the preview ran against the HEAD snapshot and minted none of its own"
    );
    let untouched = mesh_vol(&mut rt, body_of(EXTRUDE_A), Lod::Fine).await;
    assert!(
        (untouched - base_vol).abs() < 1.0,
        "the real body is untouched by the preview ({base_vol}), got {untouched}"
    );

    // Committing the SAME op must land on the volume the preview promised.
    add_op(&mut rt, candidate);
    published(&regen_all(&mut rt).await, "commit after preview");
    let committed = mesh_vol(&mut rt, body_of(EXTRUDE_A), Lod::Fine).await;
    eprintln!("revolve Cut preview {pv:.3} vs commit {committed:.3}");
    assert!(
        (committed - pv).abs() < 1.0,
        "preview {pv} and commit {committed} must agree — the Extrude-Cut-class proof"
    );

    wm.shutdown().await;
    eprintln!(
        "Revolve Cut preview/commit agreement PASS: {pv:.3} == {committed:.3} < base {base_vol:.3}"
    );
}
