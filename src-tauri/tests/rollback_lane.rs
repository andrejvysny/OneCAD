//! Rollback ("roll to here") publish gate (HISTORY-HARDEN H1) against the REAL C++
//! OCCT/PlaneGCS worker, driven through the PRODUCTION scheduler + driver wiring
//! (`scheduler_commit.rs` harness) so the `RegenHint` → `RegenRequest` mapping is
//! exercised, not bypassed.
//!
//! The defect this pins: `SetRollback { cursor }` emitted `PreviewTo(cursor)` — a
//! COUNT — while `PreviewTo`/`ToStep` take a step INDEX. Every non-zero roll asked
//! the planner for a step at/past the applied end, got an empty plan, and published
//! nothing: the viewport kept the full-history geometry for every cursor except 0.
//!
//! * `roll_to_row_republishes_prefix_geometry` — (a) roll to mid publishes ONLY the
//!   prefix body, (b) roll forward to the end brings the full model back, (c) roll to
//!   0 clears.
//! * `roll_into_all_suppressed_prefix_clears` — the empty-plan CLEAR decision must
//!   key off the compiled plan's target range, not the request's start, or a roll
//!   into a fully-suppressed prefix reports `NoOp` and strands stale bodies.
//! * `roll_to_current_row_is_a_full_noop` — a redundant roll changes nothing: no
//!   regen scheduled, no undo entry, no checkpoint eviction. A REAL roll keeps the
//!   checkpoints too (cursor motion rewrites no record bytes).
//!
//! Gated on `ONECAD_WORKER_PATH` (else the dev-tree fallback); `ONECAD_REQUIRE_WORKER=1`
//! hard-fails a missing binary (CI). A missing worker is a quiet local-dev skip.

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
use onecad_core::edit::{CommandOutcome, EditCommand, RegenHint};
use onecad_core::ids::{ConstraintId, EntityId, RecordId, RegionId, SketchId};
use onecad_core::math::{Vec2, Vec3};
use onecad_core::regen::GeometryEngine;
use onecad_core::sketch::{Constraint, CurvePosition, Sketch, SketchEntity, WorldPlane};

use onecad_lib::document_runtime::{DocumentRuntime, RegenReport};
use onecad_lib::dto::{DocumentChange, DocumentProjection};
use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::wire::sketch_wire;
use onecad_lib::worker::{resolve_worker_path, MeshProvider, SolverEngine, WorkerManager};
use onecad_lib::{regen_driver_with_emitter, RegenEmitter};

use onecad_core::regen::{RegenScheduler, SchedulerHandle};

// ─────────────────────────────────────────────────────────────────────────────
// Harness (mirrors scheduler_commit.rs)
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

// ─────────────────────────────────────────────────────────────────────────────
// Sketch + op record builders (rectangle → first-region extrude, as wire_contract)
// ─────────────────────────────────────────────────────────────────────────────

const SKETCH_A: u128 = 0xA00;
const EXTRUDE_A: u128 = 0xA01;
const SKETCH_B: u128 = 0xB00;
const EXTRUDE_B: u128 = 0xB01;

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
            region: RegionId::new(""),
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

// ─────────────────────────────────────────────────────────────────────────────
// Production wiring (Arc<Mutex<Option<DocumentRuntime>>> + real scheduler/driver)
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

/// Awaits the next emitter completion (15 s ceiling — a worker regen is well under).
async fn recv(rx: &mut UnboundedReceiver<Emit>) -> Emit {
    tokio::time::timeout(Duration::from_secs(15), rx.recv())
        .await
        .expect("regen completion emitted within 15s")
        .expect("emitter channel stayed open")
}

/// Applies one command through the single writer and returns its outcome.
async fn apply(runtime: &Runtime, cmd: EditCommand) -> CommandOutcome {
    let mut guard = runtime.lock().await;
    guard
        .as_mut()
        .expect("document open")
        .apply(cmd)
        .expect("command applied")
}

/// The body ids the published regen mirror currently holds, sorted — the geometry
/// actually on screen, read from the runtime rather than from any record's fields.
async fn head_bodies(runtime: &Runtime) -> Vec<String> {
    let guard = runtime.lock().await;
    let mut ids: Vec<String> = guard
        .as_ref()
        .expect("document open")
        .head_body_ids()
        .iter()
        .map(ToString::to_string)
        .collect();
    ids.sort();
    ids
}

/// Commits the sketch + extrude pair for one box (both the frontend wire shape) and
/// returns the extrude's outcome.
async fn commit_box(
    runtime: &Runtime,
    sketch_rec: u128,
    extrude_rec: u128,
    sketch_seed: u128,
    x0: f64,
    w: f64,
    h: f64,
) -> CommandOutcome {
    let sid = SketchId(Uuid::from_u128(sketch_seed));
    apply(
        runtime,
        EditCommand::AddOperation {
            record: sketch_record(
                sketch_rec,
                &rect_sketch(sid, sketch_seed << 8, x0, 0.0, w, h),
            ),
            at_cursor: false,
        },
    )
    .await;
    apply(
        runtime,
        EditCommand::AddOperation {
            record: extrude_record(extrude_rec, sid, 25.0),
            at_cursor: false,
        },
    )
    .await
}

/// Builds the two-box history (4 records, cursor at the end) through the scheduler,
/// returning `(body_a, body_b)` — the box-A and box-B body ids.
async fn build_two_boxes(
    runtime: &Runtime,
    sched: &SchedulerHandle,
    rx: &mut UnboundedReceiver<Emit>,
) -> (String, String) {
    let outcome = commit_box(runtime, SKETCH_A, EXTRUDE_A, 0xA, 0.0, 40.0, 20.0).await;
    sched.handle(&outcome);
    let (s, _, _) = recv(rx).await;
    assert_eq!(s, "published", "box A commit published");
    let after_a = head_bodies(runtime).await;
    assert_eq!(after_a.len(), 1, "box A alone, got {after_a:?}");

    let outcome = commit_box(runtime, SKETCH_B, EXTRUDE_B, 0xB, 60.0, 30.0, 15.0).await;
    sched.handle(&outcome);
    let (s, _, _) = recv(rx).await;
    assert_eq!(s, "published", "box B commit published");
    let after_b = head_bodies(runtime).await;
    assert_eq!(after_b.len(), 2, "box A + box B, got {after_b:?}");

    let body_a = after_a[0].clone();
    let body_b = after_b
        .iter()
        .find(|id| **id != body_a)
        .expect("box B minted a second body")
        .clone();
    (body_a, body_b)
}

// ─────────────────────────────────────────────────────────────────────────────
// (a)/(b)/(c) roll back → prefix only; roll forward → full; roll to 0 → clear
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn roll_to_row_republishes_prefix_geometry() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let (runtime, sched, mut rx) = wire(&wm).await;
    let (body_a, body_b) = build_two_boxes(&runtime, &sched, &mut rx).await;

    // (a) Roll the bar to row 2 (= after box A's extrude). The hint is a step INDEX,
    //     so a 2-op applied prefix previews to step 1 — the count-vs-index defect.
    let outcome = apply(&runtime, EditCommand::SetRollback { cursor: 2 }).await;
    assert_eq!(
        outcome.regen,
        RegenHint::PreviewTo(1),
        "cursor 2 (an applied-op COUNT) previews to step INDEX 1"
    );
    sched.handle(&outcome);
    let (s, change, proj) = recv(&mut rx).await;
    assert_eq!(
        s, "published",
        "rolling to a row must PUBLISH the prefix geometry (was a silent noop)"
    );
    assert_eq!(
        head_bodies(&runtime).await,
        vec![body_a.clone()],
        "only box A survives the roll to row 2"
    );
    assert_eq!(
        change
            .expect("a published roll carries a document_change")
            .removed_bodies,
        vec![body_b.clone()],
        "box B was removed by the roll"
    );
    assert_eq!(proj.applied_ops, 2, "the projection's rollback bar moved");
    assert_eq!(proj.total_ops, 4, "the rolled-past records are still there");

    // (b) Roll forward to the end → the full model returns.
    let outcome = apply(&runtime, EditCommand::SetRollback { cursor: 4 }).await;
    assert_eq!(outcome.regen, RegenHint::PreviewTo(3));
    sched.handle(&outcome);
    let (s, _, _) = recv(&mut rx).await;
    assert_eq!(s, "published", "rolling forward republishes");
    let mut both = vec![body_a.clone(), body_b.clone()];
    both.sort();
    assert_eq!(
        head_bodies(&runtime).await,
        both,
        "both boxes are back at the frontier"
    );

    // (c) Roll to 0 → a CLEAR publish (no applied op ⇒ no geometry).
    let outcome = apply(&runtime, EditCommand::SetRollback { cursor: 0 }).await;
    assert_eq!(outcome.regen, RegenHint::PreviewTo(0));
    sched.handle(&outcome);
    let (s, _, proj) = recv(&mut rx).await;
    assert_eq!(s, "published", "roll to 0 publishes the empty model");
    assert!(
        head_bodies(&runtime).await.is_empty(),
        "no geometry at cursor 0"
    );
    assert_eq!(proj.applied_ops, 0);

    sched.shutdown();
    wm.shutdown().await;
    eprintln!("ROLLBACK-LANE PASS: mid → prefix only; end → full; 0 → clear");
}

// ─────────────────────────────────────────────────────────────────────────────
// (d) A roll whose whole applied range is suppressed must CLEAR, not NoOp
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn roll_into_all_suppressed_prefix_clears() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let (runtime, sched, mut rx) = wire(&wm).await;
    let (_body_a, body_b) = build_two_boxes(&runtime, &sched, &mut rx).await;

    // Suppress box A's pair, extrude first so no live op is ever left pointing at a
    // suppressed sketch.
    for rec in [EXTRUDE_A, SKETCH_A] {
        let outcome = apply(
            &runtime,
            EditCommand::SetOperationSuppression {
                record: RecordId(Uuid::from_u128(rec)),
                suppressed: true,
                cascade: false,
            },
        )
        .await;
        sched.handle(&outcome);
        let (s, _, _) = recv(&mut rx).await;
        assert_eq!(s, "published", "suppression regen published");
    }
    assert_eq!(
        head_bodies(&runtime).await,
        vec![body_b.clone()],
        "only box B remains once box A's ops are suppressed"
    );

    // Roll to row 2 → ToStep(1) over records[0..=1], both suppressed ⇒ the compiled
    // plan is EMPTY. Deciding the clear off the REQUEST's start (which is 1, not 0)
    // reported NoOp and left box B on screen; deciding it off the plan's target range
    // publishes the empty model.
    let outcome = apply(&runtime, EditCommand::SetRollback { cursor: 2 }).await;
    assert_eq!(outcome.regen, RegenHint::PreviewTo(1));
    sched.handle(&outcome);
    let (s, change, proj) = recv(&mut rx).await;
    assert_eq!(
        s, "published",
        "a roll into an all-suppressed prefix CLEARS (must not be a stale-geometry noop)"
    );
    assert!(
        head_bodies(&runtime).await.is_empty(),
        "every applied op in [0, 1] is suppressed ⇒ no geometry"
    );
    assert_eq!(
        change
            .expect("the clear carries a document_change")
            .removed_bodies,
        vec![body_b],
        "box B was removed by the clear"
    );
    assert_eq!(proj.applied_ops, 2);

    sched.shutdown();
    wm.shutdown().await;
    eprintln!("ROLLBACK-LANE PASS: all-suppressed prefix roll → CLEAR publish");
}

// ─────────────────────────────────────────────────────────────────────────────
// (e) A redundant roll is a full no-op; a real roll preserves checkpoints
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn roll_to_current_row_is_a_full_noop() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let (runtime, sched, mut rx) = wire(&wm).await;
    build_two_boxes(&runtime, &sched, &mut rx).await;

    // Mint a checkpoint of the head (step 3) — the state the roll must not evict.
    let (undo_before, checkpoints_before) = {
        let mut guard = runtime.lock().await;
        let rt = guard.as_mut().expect("document open");
        rt.take_checkpoint_at_head().await;
        (rt.undo_depth(), rt.checkpoint_count())
    };
    assert_eq!(checkpoints_before, 1, "a checkpoint was taken at the head");

    // Roll to the row the bar already sits on: nothing dirties, nothing regens.
    let outcome = apply(&runtime, EditCommand::SetRollback { cursor: 4 }).await;
    assert_eq!(
        outcome.regen,
        RegenHint::None,
        "a redundant roll schedules no regen"
    );
    assert_eq!(outcome.dirty, None, "a redundant roll dirties no step");
    assert!(
        !outcome.projection_delta.cursor_changed,
        "the cursor did not move"
    );
    sched.handle(&outcome);
    assert!(
        tokio::time::timeout(Duration::from_millis(1500), rx.recv())
            .await
            .is_err(),
        "RegenHint::None enqueues nothing, so no completion may fire"
    );
    {
        let guard = runtime.lock().await;
        let rt = guard.as_ref().expect("document open");
        assert_eq!(
            rt.undo_depth(),
            undo_before,
            "a redundant roll pushes no undo entry"
        );
        assert_eq!(
            rt.checkpoint_count(),
            checkpoints_before,
            "a redundant roll evicts no checkpoint"
        );
    }

    // A REAL roll moves the cursor but rewrites no record bytes, so every checkpoint
    // stays hash-valid (the roll-forward replay must not lose its acceleration base).
    let outcome = apply(&runtime, EditCommand::SetRollback { cursor: 2 }).await;
    sched.handle(&outcome);
    let (s, _, _) = recv(&mut rx).await;
    assert_eq!(s, "published", "the real roll published");
    {
        let guard = runtime.lock().await;
        assert_eq!(
            guard.as_ref().expect("document open").checkpoint_count(),
            checkpoints_before,
            "cursor motion changes no record, so checkpoints survive the roll"
        );
    }

    sched.shutdown();
    wm.shutdown().await;
    eprintln!("ROLLBACK-LANE PASS: redundant roll = no regen/undo/eviction; real roll keeps ckpts");
}
