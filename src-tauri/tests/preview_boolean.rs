//! Standalone Boolean drag-time PREVIEW gate against the REAL C++ OCCT worker,
//! driven through the app's [`DocumentRuntime`] exactly like
//! `wire_contract.rs::preview_matches_the_commit_and_leaves_no_trace` and
//! `preview_revolve.rs`.
//!
//! Until now an armed standalone Boolean showed only a translucent highlight of the
//! target + tool bodies — never the actual fused/cut result. `PreviewOp` (SCHEMA
//! §7.6) runs the candidate `Boolean` against a throwaway copy of the head, so the
//! candidate mesh IS what a commit would produce, and both source bodies can be
//! hidden (`changedBodies` ∪ `deletedBodies`) while it is shown. These two tests pin
//! exactly that:
//!
//! * `boolean_union_preview_matches_the_commit` — two overlapping boxes (worldY
//!   ranges `[0,40]` and `[20,60]`, both worldX `[-20,0]` × Z `[0,25]`, overlap
//!   `20×20×25=10000`) previewed as `Union`: `changedBodies == [target]`,
//!   `deletedBodies == [tool]` (a Union CONSUMES its tool, same as a commit), volume
//!   `== A + B − overlap == 30000` (±1), the HEAD is byte-identical afterwards (same
//!   snapshot id/generation, same per-body signatures, same document revision), and
//!   committing the SAME op lands on the volume the preview promised.
//! * `boolean_cut_preview_matches_the_commit` — the same two boxes previewed as
//!   `Cut`: `changedBodies == [target]`, `deletedBodies == [tool]`, volume
//!   `== A − overlap == 10000` (±1), same head-untouched + preview==commit proof.
//!
//! Geometry provenance: identical to `wire_contract.rs::run_boolean` (`standalone_
//! boolean_cut`/`_union`), reused verbatim so the volumes are already independently
//! ctest/wire-proven — this file adds the PREVIEW half those tests never covered.
//!
//! Gated on `ONECAD_WORKER_PATH` (else the dev-tree fallback); a missing binary skips
//! cleanly unless `ONECAD_REQUIRE_WORKER=1` (CI hard-fails). Worker ctest already
//! covers Boolean's `deletedBodies` semantics directly (`worker/tests`) — this file
//! is Rust-side wire plumbing only, not a re-derivation of the kernel behavior.

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
use onecad_core::math::{Vec2, Vec3};
use onecad_core::regen::{CancelToken, GeometryEngine, Lod, ModelSnapshot, Outcome, RegenRequest};
use onecad_core::sketch::{Constraint, CurvePosition, Sketch, SketchEntity, WorldPlane};

use onecad_lib::document_runtime::{DocumentRuntime, RegenReport};
use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::wire::sketch_wire;
use onecad_lib::worker::{
    resolve_worker_path, MeshProvider, PreviewEngine, SolverEngine, WorkerManager,
};

use onecad_protocol::mesh::{f32_le, u32_le, validate_mesh_blob, MeshHeaderView};

// ─────────────────────────────────────────────────────────────────────────────
// Harness (mirrors preview_revolve.rs / wire_contract.rs)
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

// Fixed record ids (mirrors wire_contract.rs::run_boolean).
const SKETCH_A: u128 = 0xA00;
const EXTRUDE_A: u128 = 0xA01;
const SKETCH_B: u128 = 0xB00;
const EXTRUDE_B: u128 = 0xB01;
const OP_TAIL: u128 = 0xC00; // the previewed + committed standalone Boolean

// ─────────────────────────────────────────────────────────────────────────────
// Sketch + op record builders (verbatim from wire_contract.rs)
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

/// A fully-constrained (dof 0) rectangle at sketch-space `(x0, y0)`, size `w × h`
/// (8 synthesized points, 4 lines, coincident corners, H/V, a Fixed anchor, and H/V
/// dimension constraints — the marshaller shape).
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
    let params = ExtrudeParams {
        profile: Some(SketchRegionRef {
            sketch,
            region: RegionId::new(""), // empty ⇒ V1 first-region fallback
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

/// The standalone Boolean op the frontend's preview lane builds
/// (`ipc/previewOps.ts booleanOp`) and `commitBoolean` materializes verbatim — ONE
/// record, used first as a `PreviewOp` candidate and then committed unchanged.
fn boolean_record(rec: u128, op: BooleanOp, target: BodyId, tool: BodyId) -> OperationRecord {
    OperationRecord::new(
        RecordId(Uuid::from_u128(rec)),
        0,
        "Boolean",
        Operation::Known(KnownOperation::Boolean(BooleanParams {
            operation: op,
            target_body: target,
            tool_body: tool,
            extra: Default::default(),
        })),
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// MESH1 geometry helpers (from wire_contract.rs / preview_revolve.rs)
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
/// polyhedron (a box, a box minus/plus a box), so box arithmetic is testable to f32
/// precision.
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

/// Build the two overlapping boxes (A NewBody, B NewBody) and regen to a published
/// head. Returns `(rt, wm)` with the head containing exactly A and B.
///
/// A = worldX[-20,0] × worldY[0,40] × Z[0,25] = 20000.
/// B = worldX[-20,0] × worldY[20,60] × Z[0,25] = 20000.
/// A∩B = worldX[-20,0] × worldY[20,40] × Z[0,25] = 10000.
async fn two_overlapping_boxes() -> (DocumentRuntime, WorkerManager) {
    let bin = real_worker().expect("worker checked by caller");
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let sa = SketchId(Uuid::from_u128(0xA));
    add_op(
        &mut rt,
        sketch_record(SKETCH_A, &rect_sketch(sa, 0x1000, 0.0, 0.0, 40.0, 20.0)),
    );
    add_op(&mut rt, extrude_record(EXTRUDE_A, sa, 25.0));

    let sb = SketchId(Uuid::from_u128(0xB));
    add_op(
        &mut rt,
        sketch_record(SKETCH_B, &rect_sketch(sb, 0x2000, 20.0, 0.0, 40.0, 20.0)),
    );
    add_op(&mut rt, extrude_record(EXTRUDE_B, sb, 25.0));

    let base = regen_all(&mut rt).await;
    assert_eq!(
        published(&base, "two boxes").bodies.len(),
        2,
        "the head is exactly the two disjoint-then-overlapping boxes"
    );
    let vol_a = mesh_vol(&mut rt, body_of(EXTRUDE_A), Lod::Coarse).await;
    let vol_b = mesh_vol(&mut rt, body_of(EXTRUDE_B), Lod::Coarse).await;
    assert!((vol_a - 20000.0).abs() < 1.0, "box A is 20000, got {vol_a}");
    assert!((vol_b - 20000.0).abs() < 1.0, "box B is 20000, got {vol_b}");

    (rt, wm)
}

/// Shared body: preview a standalone `op` (target=A, tool=B), assert the
/// changed/deleted body sets + volume + untouched head, then commit the SAME op and
/// assert the committed volume agrees. Returns the preview volume.
async fn run_preview_then_commit(op: BooleanOp, expected_vol: f64, what: &str) -> f64 {
    let (mut rt, wm) = two_overlapping_boxes().await;
    let target = body_of(EXTRUDE_A);
    let tool = body_of(EXTRUDE_B);
    let before = head_fingerprint(&rt);

    let candidate = boolean_record(OP_TAIL, op, target, tool);
    let preview = wm
        .preview_op(
            candidate.op.clone(),
            candidate.record_id.to_string(),
            None, // standalone Boolean carries no sketch — operates on existing solids
            None,
            Lod::Coarse,
        )
        .await
        .expect("PreviewOp reaches the worker");

    assert_eq!(
        preview.needs_repair.len(),
        0,
        "{what}: a well-formed target/tool pair resolves cleanly: {:?}",
        preview.needs_repair
    );
    assert_eq!(
        preview.changed_bodies,
        vec![target.to_string()],
        "{what}: changedBodies is exactly the target (id preserved, same as a commit)"
    );
    assert_eq!(
        preview.deleted_bodies,
        vec![tool.to_string()],
        "{what}: deletedBodies is exactly the tool — it is CONSUMED, same as a commit"
    );
    assert_eq!(
        preview.bodies.len(),
        1,
        "{what}: exactly one candidate mesh — the modified target"
    );

    let pv = blob_volume(&preview.bodies[0].mesh, "preview");
    eprintln!("{what}: preview volume = {pv} (expected {expected_vol})");
    assert!(
        (pv - expected_vol).abs() < 1.0,
        "{what}: preview volume must be {expected_vol}, got {pv}"
    );

    // The head is exactly where it was: same snapshot, same signatures, same
    // revision — a preview never fences, prepares, or publishes.
    let after = head_fingerprint(&rt);
    assert_eq!(
        before, after,
        "{what}: a preview must leave the head fingerprint (snapshot id/generation, \
         body signatures, document revision) byte-identical"
    );
    assert_eq!(
        preview.snapshot_id, before.0,
        "{what}: the preview ran against the HEAD snapshot and minted none of its own"
    );
    let untouched_a = mesh_vol(&mut rt, target, Lod::Coarse).await;
    assert!(
        (untouched_a - 20000.0).abs() < 1.0,
        "{what}: the real target body is untouched by the preview (20000), got {untouched_a}"
    );

    // Committing the SAME op must land on the volume the preview promised — the
    // entire point of the lane (SCHEMA §7.6: preview candidate == commit result).
    add_op(&mut rt, candidate);
    let snap = regen_all(&mut rt).await;
    assert_eq!(
        published(&snap, "commit after preview").bodies.len(),
        1,
        "{what}: the boolean modifies the target and consumes the tool — ONE body left"
    );
    let committed = mesh_vol(&mut rt, target, Lod::Coarse).await;
    eprintln!("{what}: preview {pv} vs commit {committed}");
    assert!(
        (committed - pv).abs() < 1.0,
        "{what}: preview {pv} and commit {committed} must agree — that is the entire point"
    );

    wm.shutdown().await;
    eprintln!("{what} PASS: preview {pv} == commit {committed} == {expected_vol}");
    pv
}

// ─────────────────────────────────────────────────────────────────────────────
// 1. Union: preview == A + B − overlap == 30000, changedBodies=[target],
//    deletedBodies=[tool] (a Union still CONSUMES its tool body).
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn boolean_union_preview_matches_the_commit() {
    if real_worker().is_none() {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    }
    // A ∪ B = 20000 + 20000 − 10000 = 30000.
    run_preview_then_commit(BooleanOp::Union, 30_000.0, "Union").await;
}

// ─────────────────────────────────────────────────────────────────────────────
// 2. Cut: preview == A − overlap == 10000. This is the class of defect the whole
//    lane exists for — the old translucent two-body highlight never showed the
//    subtraction; the candidate mesh now does.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn boolean_cut_preview_matches_the_commit() {
    if real_worker().is_none() {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    }
    // A − (A∩B) = 20000 − 10000 = 10000.
    run_preview_then_commit(BooleanOp::Cut, 10_000.0, "Cut").await;
}
