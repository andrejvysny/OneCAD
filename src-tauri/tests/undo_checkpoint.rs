//! HISTORY-HARDEN **H8** — undo/redo stop forcing blind full-history replays.
//!
//! Driven against the REAL C++ OCCT/PlaneGCS worker. Three of the five cases run
//! straight through [`DocumentRuntime`] (so the request the runtime NAMES is the
//! thing under test); the rollback-mint case runs through the PRODUCTION scheduler
//! + driver wiring, because the mint lives in the driver's completion seam.
//!
//! What each case pins:
//!
//! * `undo_restores_from_a_checkpoint_below_its_floor` — an undo whose dirty floor is
//!   `k` regens `[k, end]` and RESTORES from a checkpoint at `≤ k − 1` instead of
//!   replaying the whole history, and the geometry really reverts. Before H8 the
//!   request was `ToEnd { from: 0 }`, whose checkpoint ceiling is `0.checked_sub(1)
//!   == None` — a from-0 request can never select a checkpoint, so every undo was a
//!   full replay by construction.
//! * `undo_of_an_append_publishes_the_removal` — the **B1 clamp**. `from` must be
//!   `min(floor, applied − 1)` over the cursor AFTER the inverse applied; the raw
//!   floor of an undone append equals `applied`, which the planner answers with an
//!   empty plan ⇒ `NoOp` ⇒ the undone body stays on screen.
//! * `undo_evicts_the_checkpoints_above_its_floor` — undo/redo now run the same
//!   truncation eviction the edit lane does (they used to bypass it wholesale), and
//!   the checkpoints strictly BELOW the floor survive so the replay stays fast.
//! * `redo_reuses_the_checkpoint_below_its_floor` — the redo half: its floor comes
//!   from the re-executed forward commands, whose outcomes used to be discarded.
//! * `a_published_rollback_mints_a_checkpoint_at_the_new_head` — the roll-back →
//!   insert-at-cursor → roll-forward loop. The mint is invisible to a repeat
//!   rollback (a `ToStep(k)` preview picks a checkpoint at `≤ k − 1`); what it
//!   accelerates is the `ToEnd { from: c }` the insert-at-cursor schedules, whose
//!   ceiling `c − 1` is exactly the minted step.
//!
//! The H6a half of H8 — "an undo must not arm the edit-scoped congruent-twin veto" —
//! is pinned in `topology_rebind.rs::h6a_edit_lane_vetoes_a_drifted_twin`, where the
//! symmetric-geometry scene already lives.
//!
//! Gated on `ONECAD_WORKER_PATH` (else the dev-tree fallback);
//! `ONECAD_REQUIRE_WORKER=1` hard-fails a missing binary (CI). A missing worker is a
//! quiet local-dev skip.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};
use tokio::sync::{watch, Mutex};

use onecad_core::document::record::{
    BooleanMode, ExtrudeMode, ExtrudeParams, KnownOperation, Operation, OperationRecord, PlaneKind,
    SketchOpParams, SketchPlaneRef,
};
use onecad_core::document::refs::SketchRegionRef;
use onecad_core::document::variables::Scalar;
use onecad_core::edit::{CommandOutcome, EditCommand};
use onecad_core::ids::{BodyId, ConstraintId, EntityId, RecordId, RegionId, SketchId};
use onecad_core::math::{Vec2, Vec3};
use onecad_core::regen::{
    CancelToken, GeometryEngine, Lod, ModelSnapshot, Outcome, RegenRequest, RegenScheduler,
    SchedulerHandle,
};
use onecad_core::sketch::{Constraint, CurvePosition, Sketch, SketchEntity, WorldPlane};
use onecad_protocol::mesh::{f32_le, u32_le, validate_mesh_blob, MeshHeaderView};

use onecad_lib::document_runtime::{DocumentRuntime, RegenReport};
use onecad_lib::dto::{DocumentChange, DocumentProjection};
use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::wire::sketch_wire;
use onecad_lib::worker::{resolve_worker_path, MeshProvider, SolverEngine, WorkerManager};
use onecad_lib::{regen_driver_with_emitter, RegenEmitter};

// ─────────────────────────────────────────────────────────────────────────────
// Harness (mirrors checkpoints.rs / rollback_lane.rs)
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

async fn regen(rt: &mut DocumentRuntime, request: RegenRequest) -> RegenReport {
    rt.run_regen(request, CancelToken::new()).await
}

fn published<'a>(report: &'a RegenReport, what: &str) -> &'a Arc<ModelSnapshot> {
    match &report.outcome {
        Outcome::Published(s) => s,
        other => panic!("{what}: expected Published, got {other:?}"),
    }
}

/// The regen the runtime NAMES for an undo (already floored, clamped and on the
/// revert lane) — the exact value `api::undo` hands the scheduler.
fn undo_request(rt: &mut DocumentRuntime) -> RegenRequest {
    rt.undo()
        .expect("a step was undone")
        .regen
        .expect("undoing a timeline op schedules a regen")
}

fn redo_request(rt: &mut DocumentRuntime) -> RegenRequest {
    rt.redo()
        .expect("redo ok")
        .expect("a step was redone")
        .regen
        .expect("redoing a timeline op schedules a regen")
}

// ── mesh volume (breadth_ops.rs) ─────────────────────────────────────────────

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

/// The volume of `body`'s current published mesh.
async fn body_volume(rt: &mut DocumentRuntime, body: BodyId) -> f64 {
    let mesh = rt
        .get_mesh(body, Lod::Coarse, None)
        .await
        .expect("fetch mesh");
    let view = validate_mesh_blob(&mesh).expect("MESH1 validates");
    mesh_volume(&view, &mesh)
}

// ─────────────────────────────────────────────────────────────────────────────
// Record builders (rectangle → first-region extrude, as checkpoints.rs)
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

/// A fully-constrained (dof 0) rectangle at sketch-space `(x0, y0)`, size `w × h`.
fn rect_sketch(sid: SketchId, base: u128, x0: f64, y0: f64, w: f64, h: f64) -> Sketch {
    let e = |n: u128| EntityId(Uuid::from_u128(base + n));
    let c = |n: u128| ConstraintId(Uuid::from_u128(base + 0x40 + n));
    let (p0s, p0e) = (e(0), e(1));
    let (p1s, p1e) = (e(2), e(3));
    let (p2s, p2e) = (e(4), e(5));
    let (p3s, p3e) = (e(6), e(7));
    let (l0, l1, l2, l3) = (e(0x10), e(0x11), e(0x12), e(0x13));
    let mut sk = Sketch::on_world_plane(sid, "Rect", WorldPlane::XY);
    let pt = |sk: &mut Sketch, id, x, y| {
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
    let params = ExtrudeParams {
        profile: Some(SketchRegionRef {
            sketch,
            region: RegionId::new(""),
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
    };
    OperationRecord::new(
        RecordId(Uuid::from_u128(rec)),
        0,
        "Extrude",
        Operation::Known(KnownOperation::Extrude(params)),
    )
}

// Fixed record ids for the 4-op history: sketch A, extrude A, sketch B, extrude B.
const SK_A: u128 = 0xA00;
const EX_A: u128 = 0xA01;
const SK_B: u128 = 0xB00;
const EX_B: u128 = 0xB01;
const SID_A: u128 = 0xA;
const SID_B: u128 = 0xB;

const BOX_B_DEPTH: f64 = 25.0;
/// 30 × 15 × 25 — box B's volume at the authored depth.
const BOX_B_VOL: f64 = 30.0 * 15.0 * BOX_B_DEPTH;

/// The 4-record two-box history: `[sketchA, extrudeA, sketchB, extrudeB]`, cursor 4.
fn build_two_boxes(rt: &mut DocumentRuntime) {
    let sa = SketchId(Uuid::from_u128(SID_A));
    let sb = SketchId(Uuid::from_u128(SID_B));
    add_op(
        rt,
        sketch_record(SK_A, &rect_sketch(sa, 0x1000, 0.0, 0.0, 40.0, 20.0)),
    );
    add_op(rt, extrude_record(EX_A, sa, 25.0));
    add_op(
        rt,
        sketch_record(SK_B, &rect_sketch(sb, 0x2000, 60.0, 0.0, 30.0, 15.0)),
    );
    add_op(rt, extrude_record(EX_B, sb, BOX_B_DEPTH));
}

/// Plants a checkpoint at **step 1** (box A's extrude — a feature boundary) by
/// rolling the bar back to cursor 2, minting at the rolled-back head, then rolling
/// forward again. Cursor motion rewrites no record bytes, so the checkpoint stays
/// hash-valid at the frontier: it is exactly the acceleration base a later
/// `ToEnd { from: 2 }` / `{ from: 3 }` is allowed to restore from.
///
/// **Step 1, not step 2, and that is load-bearing.** A checkpoint restores bodies,
/// never the worker's sketch/region state, and the worker resolves a profile against
/// THE PLAN'S ops — so a base that swallows the `Sketch` op an executed step consumes
/// makes that step fail (`"Extrude: profile sketch not found in plan"`). The planner
/// refuses such a checkpoint outright (`suffix_sketches_stay_planned`); planting at a
/// feature boundary is what keeps the acceleration available at all.
async fn plant_checkpoint_at_step_1(rt: &mut DocumentRuntime) {
    rt.apply(EditCommand::SetRollback { cursor: 2 })
        .expect("roll back");
    let rolled = regen(rt, RegenRequest::ToStep(1)).await;
    assert_eq!(
        published(&rolled, "roll to step 1").step_index,
        Some(1),
        "the rolled-back head is step 1"
    );
    rt.take_checkpoint_at_head().await;
    assert_eq!(
        rt.checkpoint_steps(),
        vec![1],
        "a checkpoint was minted at the rolled-back head"
    );
    rt.apply(EditCommand::SetRollback { cursor: 4 })
        .expect("roll forward");
    let _ = published(&regen(rt, RegenRequest::ToStep(3)).await, "roll forward");
    assert_eq!(
        rt.checkpoint_steps(),
        vec![1],
        "rolling forward evicts nothing (no record byte moved)"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// (a) An undo replays from its floor and RESTORES from the checkpoint below it
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn undo_restores_from_a_checkpoint_below_its_floor() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    build_two_boxes(&mut rt);
    let base = regen(&mut rt, RegenRequest::ToEnd { from: 0 }).await;
    let _ = published(&base, "baseline");
    let box_b = *base
        .changed
        .iter()
        .map(|(b, _)| b)
        .nth(1)
        .expect("two bodies at the frontier");
    assert!(
        (body_volume(&mut rt, box_b).await - BOX_B_VOL).abs() < 1.0,
        "baseline box B is 30×15×25"
    );

    plant_checkpoint_at_step_1(&mut rt).await;

    // Edit the LAST op (step 3): double box B's depth. The edit lane evicts from 3,
    // so the step-1 checkpoint survives and accelerates the edit's own regen.
    rt.apply(EditCommand::UpdateOperationParams {
        record: RecordId(Uuid::from_u128(EX_B)),
        op: extrude_record(EX_B, SketchId(Uuid::from_u128(SID_B)), BOX_B_DEPTH * 2.0).op,
    })
    .expect("edit box B depth");
    assert_eq!(
        rt.checkpoint_steps(),
        vec![1],
        "an edit at step 3 evicts only checkpoints at/above 3"
    );
    let edited = regen(&mut rt, RegenRequest::ToEnd { from: 3 }).await;
    let _ = published(&edited, "edited");
    assert!(
        rt.last_regen_used_checkpoint(),
        "the edit's own regen already restores from the step-1 checkpoint"
    );
    let edited_vol = body_volume(&mut rt, box_b).await;
    assert!(
        (edited_vol - BOX_B_VOL * 2.0).abs() < 1.0,
        "the edit really doubled box B (got {edited_vol})"
    );

    // ── UNDO. The floor is the edited step (3); the checkpoint below it (2) is
    //    still valid, so the replay is INCREMENTAL, not from 0.
    let request = undo_request(&mut rt);
    assert_eq!(
        request,
        RegenRequest::RevertToEnd { from: 3 },
        "H8: the undo threads the edited step as its floor (was a blind ToEnd{{from:0}})"
    );
    assert_eq!(
        request.edited_from(),
        None,
        "a revert never claims SCHEMA §7.2 editedFrom"
    );
    let reverted = regen(&mut rt, request).await;
    let snap = published(&reverted, "undo");
    assert!(
        rt.last_regen_used_checkpoint(),
        "H8 PAYOFF: the undo regen RESTORED from the step-1 checkpoint instead of \
         replaying the whole history (a from-0 request can never select one — its \
         ceiling is `0.checked_sub(1) == None`)"
    );
    assert_eq!(
        snap.step_index,
        Some(3),
        "the undo republished the full model"
    );
    let reverted_vol = body_volume(&mut rt, box_b).await;
    assert!(
        (reverted_vol - BOX_B_VOL).abs() < 1.0,
        "H8: the geometry really reverted (got {reverted_vol}, want {BOX_B_VOL})"
    );

    wm.shutdown().await;
    eprintln!(
        "H8 PASS (a): undo floor=3 → restored from ckpt@1, volume {edited_vol} → {reverted_vol}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// (b) The B1 clamp: undoing an append must PUBLISH the body's removal
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn undo_of_an_append_publishes_the_removal() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    build_two_boxes(&mut rt);
    let base = regen(&mut rt, RegenRequest::ToEnd { from: 0 }).await;
    let bodies: Vec<BodyId> = published(&base, "baseline")
        .bodies
        .iter()
        .map(|b| b.body)
        .collect();
    assert_eq!(bodies.len(), 2, "two boxes at the frontier");
    let box_b = bodies[1];

    // Undo the box-B extrude (the appended record at index 3). The RAW floor is 3
    // and the cursor lands on 3, so an unclamped `ToEnd { from: 3 }` starts PAST the
    // applied end ⇒ empty plan ⇒ NoOp ⇒ box B would stay on screen.
    let request = undo_request(&mut rt);
    assert_eq!(
        request,
        RegenRequest::RevertToEnd { from: 2 },
        "B1 CLAMP: from = min(floor 3, applied 3 − 1) = 2"
    );
    let reverted = regen(&mut rt, request).await;
    let snap = published(
        &reverted,
        "B1: undoing an append must PUBLISH, never NoOp (the unclamped floor NoOps)",
    );
    assert_eq!(
        snap.step_index,
        Some(2),
        "the new head is the sketch-B step"
    );
    assert_eq!(
        reverted.removed,
        vec![box_b],
        "B1: the published change REMOVES the undone body"
    );
    assert_eq!(
        rt.head_body_ids(),
        vec![bodies[0]],
        "only box A survives the undo"
    );

    // The same clamp at the timeline floor: undoing everything down to cursor 0 must
    // reach the planner's CLEAR path (`from: 0`), not an empty-plan NoOp.
    for _ in 0..3 {
        let request = undo_request(&mut rt);
        let _ = regen(&mut rt, request).await;
    }
    assert!(
        rt.head_body_ids().is_empty(),
        "an empty applied prefix publishes no geometry"
    );

    wm.shutdown().await;
    eprintln!("H8 PASS (b): undo-of-append published the removal (clamped from 3 → 2)");
}

// ─────────────────────────────────────────────────────────────────────────────
// (c) A revert runs the truncation eviction the edit lane runs
// ─────────────────────────────────────────────────────────────────────────────

/// Undo/redo used to bypass checkpoint eviction wholesale (WP-FIX W2). That was
/// tolerable while a revert asked for `ToEnd { from: 0 }` and therefore could never
/// select a checkpoint at all. H8 threads a real floor, so it can — and the stale
/// entry it would be reading is one minted over the very state the revert undid,
/// with a stored prefix hash that matches the POST-edit records. A revert now evicts
/// from its raw floor like any other mutation, while everything strictly below it
/// survives so the replay stays fast.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn undo_evicts_the_checkpoints_above_its_floor() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    build_two_boxes(&mut rt);
    let _ = published(
        &regen(&mut rt, RegenRequest::ToEnd { from: 0 }).await,
        "baseline",
    );

    // A checkpoint BELOW the coming floor (step 1, a feature boundary) and one AT
    // the head (step 3) — the one the revert must evict.
    plant_checkpoint_at_step_1(&mut rt).await;
    rt.take_checkpoint_at_head().await;
    assert_eq!(
        rt.checkpoint_steps(),
        vec![1, 3],
        "checkpoints below the coming floor and at the head"
    );

    // Edit the SKETCH the head extrude consumes — through the record the worker
    // actually executes, so box B becomes 60 × 15 instead of 30 × 15.
    let wider = rect_sketch(
        SketchId(Uuid::from_u128(SID_B)),
        0x2000,
        60.0,
        0.0,
        60.0,
        15.0,
    );
    rt.apply(EditCommand::UpdateOperationParams {
        record: RecordId(Uuid::from_u128(SK_B)),
        op: sketch_record(SK_B, &wider).op,
    })
    .expect("edit sketch B");
    assert_eq!(
        rt.checkpoint_steps(),
        vec![1],
        "the edit at step 2 evicted the head checkpoint, not the one below it"
    );
    let edited = regen(&mut rt, RegenRequest::ToEnd { from: 2 }).await;
    let box_b = published(&edited, "wider box B").bodies[1].body;
    let wide_vol = body_volume(&mut rt, box_b).await;
    assert!(
        (wide_vol - BOX_B_VOL * 2.0).abs() < 1.0,
        "the edit really widened box B (got {wide_vol})"
    );

    // Re-mint at the head. THIS is the checkpoint the undo has to get rid of: it
    // holds the POST-edit geometry, and its stored prefix hash matches the POST-edit
    // records, so nothing else would ever reject it.
    rt.take_checkpoint_at_head().await;
    assert_eq!(rt.checkpoint_steps(), vec![1, 3]);

    // UNDO. Floor 2 (the sketch record's step) ⇒ evict at/above 2, keep step 1.
    let request = undo_request(&mut rt);
    assert_eq!(request, RegenRequest::RevertToEnd { from: 2 });
    assert_eq!(
        rt.checkpoint_steps(),
        vec![1],
        "H8: the revert evicted the stale post-edit head checkpoint and KEPT the \
         valid one below its floor (undo/redo used to evict nothing at all)"
    );
    let reverted = regen(&mut rt, request).await;
    let _ = published(&reverted, "undo of the sketch edit");
    assert!(
        rt.last_regen_used_checkpoint(),
        "and the surviving step-1 checkpoint accelerates the revert"
    );
    let reverted_vol = body_volume(&mut rt, box_b).await;
    assert!(
        (reverted_vol - BOX_B_VOL).abs() < 1.0,
        "H8: the sketch edit really reverted (got {reverted_vol}, want {BOX_B_VOL})"
    );

    wm.shutdown().await;
    eprintln!("H8 PASS (c): revert evicts at/above its floor, keeps what is below");
}

// ─────────────────────────────────────────────────────────────────────────────
// (d) The redo half — the floor comes from the RE-EXECUTED forward commands
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn redo_reuses_the_checkpoint_below_its_floor() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    build_two_boxes(&mut rt);
    let base = regen(&mut rt, RegenRequest::ToEnd { from: 0 }).await;
    let box_b = published(&base, "baseline").bodies[1].body;

    plant_checkpoint_at_step_1(&mut rt).await;

    rt.apply(EditCommand::UpdateOperationParams {
        record: RecordId(Uuid::from_u128(EX_B)),
        op: extrude_record(EX_B, SketchId(Uuid::from_u128(SID_B)), BOX_B_DEPTH * 2.0).op,
    })
    .expect("edit box B depth");
    let _ = published(
        &regen(&mut rt, RegenRequest::ToEnd { from: 3 }).await,
        "edited",
    );
    let request = undo_request(&mut rt);
    let _ = published(&regen(&mut rt, request).await, "undo");
    assert!(
        (body_volume(&mut rt, box_b).await - BOX_B_VOL).abs() < 1.0,
        "the undo reverted the depth"
    );

    // REDO. Its floor comes from the re-executed `UpdateOperationParams`' dirty span
    // (step 3) — outcomes the redo path used to throw away.
    let request = redo_request(&mut rt);
    assert_eq!(
        request,
        RegenRequest::RevertToEnd { from: 3 },
        "H8: redo folds the re-executed commands' dirty spans"
    );
    let redone = regen(&mut rt, request).await;
    let _ = published(&redone, "redo");
    assert!(
        rt.last_regen_used_checkpoint(),
        "the redo restores from the step-1 checkpoint too"
    );
    let redone_vol = body_volume(&mut rt, box_b).await;
    assert!(
        (redone_vol - BOX_B_VOL * 2.0).abs() < 1.0,
        "H8: the redo re-applied the deeper box (got {redone_vol})"
    );

    wm.shutdown().await;
    eprintln!("H8 PASS (d): redo floor=3 → restored from ckpt@1, volume {redone_vol}");
}

// ─────────────────────────────────────────────────────────────────────────────
// (e) A published rollback mints a checkpoint at the new head
// ─────────────────────────────────────────────────────────────────────────────

type Runtime = Arc<Mutex<Option<DocumentRuntime>>>;
/// What the emitter captures per completion: `(outcome_str, document_change, projection)`.
type Emit = (String, Option<DocumentChange>, DocumentProjection);

async fn wire(wm: &WorkerManager) -> (Runtime, SchedulerHandle, UnboundedReceiver<Emit>) {
    let runtime: Runtime = Arc::new(Mutex::new(Some(runtime_over(wm))));
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

async fn recv(rx: &mut UnboundedReceiver<Emit>) -> Emit {
    tokio::time::timeout(Duration::from_secs(15), rx.recv())
        .await
        .expect("regen completion emitted within 15s")
        .expect("emitter channel stayed open")
}

async fn apply(runtime: &Runtime, cmd: EditCommand) -> CommandOutcome {
    let mut guard = runtime.lock().await;
    guard
        .as_mut()
        .expect("document open")
        .apply(cmd)
        .expect("command applied")
}

/// The mint runs AFTER the completion is emitted (unlocked worker round-trip), so a
/// test that reads the cache the instant `recv` returns can race it. Poll instead.
async fn await_checkpoint_steps(runtime: &Runtime, want: &[usize], what: &str) {
    for _ in 0..150 {
        let steps = {
            let guard = runtime.lock().await;
            guard.as_ref().expect("document open").checkpoint_steps()
        };
        if steps == want {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let steps = {
        let guard = runtime.lock().await;
        guard.as_ref().expect("document open").checkpoint_steps()
    };
    panic!("{what}: expected checkpoints at {want:?}, got {steps:?}");
}

/// The roll-back → insert-at-cursor → roll-forward loop, driven through the real
/// scheduler + driver (the mint lives in the driver's completion seam).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_published_rollback_mints_a_checkpoint_at_the_new_head() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let (runtime, sched, mut rx) = wire(&wm).await;
    {
        let mut guard = runtime.lock().await;
        build_two_boxes(guard.as_mut().expect("document open"));
    }
    sched.request(RegenRequest::ToEnd { from: 0 });
    let (s, _, _) = recv(&mut rx).await;
    assert_eq!(s, "published", "baseline commit published");
    {
        let guard = runtime.lock().await;
        assert_eq!(
            guard.as_ref().expect("open").checkpoint_steps(),
            Vec::<usize>::new(),
            "a plain commit mints nothing (the SaveCheckpoint policy is save-only)"
        );
    }

    // Roll the bar back to row 3 → publishes step 2 → the driver mints ckpt@2.
    let outcome = apply(&runtime, EditCommand::SetRollback { cursor: 3 }).await;
    sched.handle(&outcome);
    let (s, _, proj) = recv(&mut rx).await;
    assert_eq!(s, "published", "the roll published the prefix geometry");
    assert_eq!(proj.applied_ops, 3);
    await_checkpoint_steps(&runtime, &[2], "H8: a published rollback mints at its head").await;

    // Insert a feature AT THE CURSOR — the gesture the mint exists to accelerate. It
    // dirties `[3, len)`, so the scheduler asks for `ToEnd { from: 3 }`, whose
    // checkpoint ceiling is 2: exactly the checkpoint the rollback just minted.
    let sc = SketchId(Uuid::from_u128(0xC));
    let outcome = apply(
        &runtime,
        EditCommand::AddOperation {
            record: sketch_record(0xC00, &rect_sketch(sc, 0x3000, 120.0, 0.0, 20.0, 20.0)),
            at_cursor: true,
        },
    )
    .await;
    assert_eq!(
        outcome.dirty.map(|d| d.from),
        Some(3),
        "an insert at cursor 3 dirties from step 3"
    );
    {
        let guard = runtime.lock().await;
        assert_eq!(
            guard.as_ref().expect("open").checkpoint_steps(),
            vec![2],
            "the insert evicts only at/above 3 — the rollback's checkpoint survives"
        );
    }
    sched.handle(&outcome);
    let (s, _, _) = recv(&mut rx).await;
    assert_eq!(s, "published", "the insert-at-cursor regen published");
    {
        let guard = runtime.lock().await;
        assert!(
            guard.as_ref().expect("open").last_regen_used_checkpoint(),
            "H8 PAYOFF: the insert-at-cursor regen RESTORED from the checkpoint the \
             rollback minted, instead of replaying from 0"
        );
    }

    sched.shutdown();
    wm.shutdown().await;
    eprintln!("H8 PASS (e): rollback minted ckpt@2 → the insert-at-cursor restored from it");
}
