//! Production scheduler/driver commit gate (MODEL-HARDEN W0) against the REAL C++
//! OCCT/PlaneGCS worker.
//!
//! This is the **new coverage class** the reported P0 defect slipped through: every
//! prior worker-backed test drives regen explicitly (`rt.run_regen(...)`), so none
//! ever exercised the apply → `RegenScheduler::handle` → driver → publish chain the
//! real app uses. Here we wire the ACTUAL production driver
//! ([`regen_driver_with_emitter`], the body `make_regen_driver` now delegates to)
//! behind a real [`RegenScheduler`] over an `Arc<Mutex<Option<DocumentRuntime>>>`,
//! exactly as `lib.rs` does, and drive the EXACT frontend commit shape
//! (`AddOperation { at_cursor: false }`) through it. The emitter is a plain channel
//! sink (no `AppHandle`) so the completion is assertable without a webview.
//!
//! * `frontend_shape_commit_publishes_via_scheduler` — a fresh commit at the applied
//!   frontier publishes exactly one body via the scheduler; the undo entry survives
//!   the regen. Before the W0 core fix the frontier append stayed a permanent draft,
//!   the planner clamped it out (`NoOp`), and nothing ever published — this test
//!   would have caught it.
//! * `undo_redo_reconverge_via_scheduler` — undo → `ToEnd{from:0}` republishes with
//!   zero bodies; redo → `ToEnd{from:0}` republishes the body (pins the redo-draft
//!   regression: a re-applied commit must re-join the applied prefix, not become a
//!   draft the from-0 regen clamps away).
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
use onecad_core::document::Document;
use onecad_core::edit::{CommandOutcome, EditCommand, RegenHint};
use onecad_core::ids::{ConstraintId, DocumentId, EntityId, RecordId, RegionId, SketchId};
use onecad_core::io::container::{ContainerCaches, ContainerWriter, SaveMeta};
use onecad_core::math::{Vec2, Vec3};
use onecad_core::regen::{
    CancelToken, GeometryEngine, RegenRequest, RegenScheduler, SchedulerHandle,
};
use onecad_core::sketch::{Constraint, CurvePosition, Sketch, SketchEntity, WorldPlane};

use onecad_lib::document_runtime::{DocumentRuntime, RegenReport};
use onecad_lib::dto::{DocumentChange, DocumentProjection};
use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::wire::sketch_wire;
use onecad_lib::worker::{resolve_worker_path, MeshProvider, SolverEngine, WorkerManager};
use onecad_lib::{regen_driver_with_emitter, RegenEmitter};

// ─────────────────────────────────────────────────────────────────────────────
// Harness (mirrors wire_contract.rs / m2_gate.rs)
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
            region: RegionId::new(""), // empty ⇒ the worker's V1 first-region fallback
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

/// Builds the exact `lib.rs` wiring: the runtime behind the shared mutex, the real
/// production driver ([`regen_driver_with_emitter`]) with a channel emitter, and a
/// spawned [`RegenScheduler`].
async fn wire(wm: &WorkerManager) -> (Runtime, SchedulerHandle, UnboundedReceiver<Emit>) {
    wire_with(Arc::new(Mutex::new(Some(runtime_over(wm))))).await
}

/// [`wire`] over an already-constructed runtime (e.g. one OPENED from a
/// container instead of blank).
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

/// Applies a sketch op then the extrude op — both the EXACT frontend wire shape
/// (`at_cursor: false`) — and returns the extrude's [`CommandOutcome`] (whose
/// [`RegenHint`] `apply_edit_command` feeds to `sched.handle`).
async fn commit_extrude(runtime: &Runtime) -> CommandOutcome {
    let mut guard = runtime.lock().await;
    let rt = guard.as_mut().expect("document open");
    let sa = SketchId(Uuid::from_u128(0xA));
    rt.apply(EditCommand::AddOperation {
        record: sketch_record(SKETCH_A, &rect_sketch(sa, 0x1000, 0.0, 0.0, 40.0, 20.0)),
        at_cursor: false,
    })
    .expect("add sketch op (frontier append)");
    rt.apply(EditCommand::AddOperation {
        record: extrude_record(EXTRUDE_A, sa, 25.0),
        at_cursor: false, // the reported frontend commit shape
    })
    .expect("add extrude op (frontier append)")
}

async fn undo_depth(runtime: &Runtime) -> usize {
    runtime.lock().await.as_ref().expect("open").undo_depth()
}

/// Awaits the next emitter completion (15 s ceiling — a worker regen is well under).
async fn recv(rx: &mut UnboundedReceiver<Emit>) -> Emit {
    tokio::time::timeout(Duration::from_secs(15), rx.recv())
        .await
        .expect("regen completion emitted within 15s")
        .expect("emitter channel stayed open")
}

// ─────────────────────────────────────────────────────────────────────────────
// (1) The exact reported bug: a fresh commit publishes through the real scheduler
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn frontend_shape_commit_publishes_via_scheduler() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let (runtime, sched, mut rx) = wire(&wm).await;

    let outcome = commit_extrude(&runtime).await;
    // The core fix in action: a frontier `at_cursor:false` append regens (`ToEnd`),
    // not `None` — the mapping `sched.handle` relies on to enqueue any work at all.
    assert_eq!(
        outcome.regen,
        RegenHint::ToEnd,
        "a fresh frontier commit must regen ToEnd (was None before W0 — never scheduled)"
    );

    let undo_before = undo_depth(&runtime).await;
    // Mirror api::apply_edit_command :363-365 — hand the outcome to the scheduler.
    sched.handle(&outcome);

    let (outcome_str, change, _proj) = recv(&mut rx).await;
    assert_eq!(
        outcome_str, "published",
        "the commit published a snapshot through the production scheduler/driver"
    );
    let change = change.expect("a published regen carries a document_change");
    assert_eq!(
        change.changed_bodies.len(),
        1,
        "exactly one new body from the extrude, got {:?}",
        change.changed_bodies
    );

    let undo_after = undo_depth(&runtime).await;
    assert_eq!(
        undo_after, undo_before,
        "the regen must not touch the undo stack (the commit's undo entry survives)"
    );

    sched.shutdown();
    wm.shutdown().await;
    eprintln!("COMMIT-VIA-SCHEDULER PASS: at_cursor:false → published 1 body; undo depth intact");
}

// ─────────────────────────────────────────────────────────────────────────────
// (2) Undo → from-0 reconverge (0 bodies); redo → republish (redo-draft regression)
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn undo_redo_reconverge_via_scheduler() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let (runtime, sched, mut rx) = wire(&wm).await;

    // Baseline commit → published, one body.
    let outcome = commit_extrude(&runtime).await;
    sched.handle(&outcome);
    let (s, change, _) = recv(&mut rx).await;
    assert_eq!(s, "published", "baseline commit published");
    assert_eq!(
        change
            .expect("baseline document_change")
            .changed_bodies
            .len(),
        1,
        "baseline: one extrude body"
    );

    // Undo the extrude, reconverge from 0 (mirrors api::undo :384-387).
    {
        let mut guard = runtime.lock().await;
        assert!(
            guard.as_mut().expect("open").undo(),
            "undo pops the extrude commit"
        );
    }
    sched.request(RegenRequest::ToEnd { from: 0 });
    let (s, change, _) = recv(&mut rx).await;
    assert_eq!(
        s, "published",
        "undo reconverges (sketch-only regen publishes)"
    );
    let change = change.expect("undo document_change");
    assert_eq!(
        change.changed_bodies.len(),
        0,
        "no body remains after undo, got {:?}",
        change.changed_bodies
    );
    assert_eq!(
        change.removed_bodies.len(),
        1,
        "the extrude body was removed by the reconverge"
    );

    // Redo the extrude, reconverge from 0 → the body must republish. Before W0 the
    // redo re-created a permanent draft that `ToEnd{from:0}` clamped out, so the body
    // never came back — this pins that regression.
    {
        let mut guard = runtime.lock().await;
        assert!(
            guard.as_mut().expect("open").redo().expect("redo ok"),
            "redo re-applies the extrude commit"
        );
    }
    sched.request(RegenRequest::ToEnd { from: 0 });
    let (s, change, _) = recv(&mut rx).await;
    assert_eq!(s, "published", "redo republishes via the scheduler");
    assert_eq!(
        change.expect("redo document_change").changed_bodies.len(),
        1,
        "the extrude body republished (redo re-joined the applied prefix, not a draft)"
    );

    sched.shutdown();
    wm.shutdown().await;
    eprintln!(
        "UNDO/REDO-RECONVERGE PASS: undo → 0 bodies, redo → 1 body (redo-draft regression pinned)"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// (3) MODEL-HARDEN W0.5: rapid double commit correlates exactly (source_revision)
// ─────────────────────────────────────────────────────────────────────────────

/// The rapid-commit safety Codex MAJOR-3 folded in: commit A then immediately
/// commit B, so A's in-flight regen is superseded by B. We drive the begin/finish
/// phases DIRECTLY (like `document_runtime::edit_during_regen_supersedes_via_live_fencing`)
/// to force the Superseded terminal deterministically, then assert the correlation
/// provenance: A's superseded report is stamped with A's (older) `source_revision`
/// — so the frontend awaiter for A never resolves off it — and the final publish
/// covers BOTH ops' bodies (the from-0 regen covering both commits).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rapid_double_commit_correlates_exactly() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let sa = SketchId(Uuid::from_u128(0xA));
    let sb = SketchId(Uuid::from_u128(0xB));

    // Commit A: sketch + extrude — the exact frontend shape (`at_cursor:false`).
    rt.apply(EditCommand::AddOperation {
        record: sketch_record(SKETCH_A, &rect_sketch(sa, 0x1000, 0.0, 0.0, 40.0, 20.0)),
        at_cursor: false,
    })
    .expect("add sketch A");
    rt.apply(EditCommand::AddOperation {
        record: extrude_record(EXTRUDE_A, sa, 25.0),
        at_cursor: false,
    })
    .expect("add extrude A");
    let rev_a = rt.revision().0;

    // Prepare A's regen (fenced at rev_a) — but do NOT finish it yet.
    let prepared_a = rt
        .begin_regen(RegenRequest::ToEnd { from: 0 })
        .expect("non-empty plan A");

    // Commit B lands DURING A's in-flight regen → bumps the shared fencing revision,
    // superseding A. A second sketch far from the first ⇒ a distinct second body.
    rt.apply(EditCommand::AddOperation {
        record: sketch_record(SKETCH_B, &rect_sketch(sb, 0x2000, 100.0, 0.0, 40.0, 20.0)),
        at_cursor: false,
    })
    .expect("add sketch B");
    rt.apply(EditCommand::AddOperation {
        record: extrude_record(EXTRUDE_B, sb, 30.0),
        at_cursor: false,
    })
    .expect("add extrude B");
    let rev_b = rt.revision().0;
    assert!(
        rev_b > rev_a,
        "commit B advanced the fencing revision past A"
    );

    // Drive + finish A → Superseded, stamped with A's (older) source revision.
    let driven_a = prepared_a.drive(CancelToken::new()).await;
    let report_a = rt.finish_regen(driven_a);
    assert_eq!(
        report_a.outcome_str(),
        "superseded",
        "A's regen was superseded by B"
    );
    assert_eq!(
        report_a.source_revision, rev_a,
        "the superseded report is stamped with A's PREPARED revision"
    );
    assert!(
        report_a.source_revision < report_a.revision,
        "superseded source_revision ({}) < the current/final revision ({})",
        report_a.source_revision,
        report_a.revision
    );
    assert!(
        report_a.document_change().is_none(),
        "a superseded regen publishes nothing (empty changedBodies for the awaiter)"
    );

    // B's regen (from 0) publishes — covering BOTH ops' bodies. The frontend awaiter
    // for commit A (revision rev_a) IGNORES A's superseded report and resolves off
    // THIS publish (revision >= rev_a, which includes A's body); B's awaiter (rev_b)
    // resolves off it too — each commit gets its own body.
    let report_b = rt
        .run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    assert_eq!(
        report_b.outcome_str(),
        "published",
        "the from-0 regen at B publishes"
    );
    assert_eq!(
        report_b.source_revision, rev_b,
        "the publish is stamped with B's revision"
    );
    assert!(
        report_b.source_revision >= rev_a,
        "the final publish covers commit A (source_revision >= rev_a)"
    );
    let change_b = report_b
        .document_change()
        .expect("a published regen carries a document_change");
    assert_eq!(
        change_b.changed_bodies.len(),
        2,
        "the final publish covers BOTH extrude bodies, got {:?}",
        change_b.changed_bodies
    );

    wm.shutdown().await;
    eprintln!(
        "RAPID-DOUBLE-COMMIT PASS: A superseded (source {rev_a} < final {}); final publish covers both bodies",
        report_a.revision
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// (4) MODEL-HARDEN finding 4: an empty-plan (no-op) request STILL emits exactly one
// completion through the production driver — the awaiter's anti-hang terminal.
// ─────────────────────────────────────────────────────────────────────────────

/// A `ToEnd` request on the op-less (empty) document compiles to an EMPTY plan →
/// `begin_regen` yields `None`. Before the fix the driver returned `NoOp` WITHOUT
/// emitting, so the frontend's `regen-finished{noop}` anti-hang terminal never fired
/// in production (the 8 s stall). Now the empty-plan branch synthesizes + emits a NoOp
/// completion. We assert exactly ONE completion, outcome `noop`, no publish.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn empty_plan_request_still_emits_one_noop_completion() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let (_runtime, sched, mut rx) = wire(&wm).await;

    sched.request(RegenRequest::ToEnd { from: 0 });
    let (outcome_str, change, proj) = recv(&mut rx).await;
    assert_eq!(
        outcome_str, "noop",
        "an empty-plan request emits a NoOp completion (not silence)"
    );
    assert!(change.is_none(), "a NoOp publishes nothing");
    assert_eq!(
        proj.total_ops, 0,
        "the projection reflects the op-less document"
    );

    // Exactly one: no second completion arrives for a single no-op request.
    assert!(
        tokio::time::timeout(Duration::from_millis(500), rx.recv())
            .await
            .is_err(),
        "exactly one completion emitted for a single no-op request"
    );

    sched.shutdown();
    wm.shutdown().await;
    eprintln!("EMPTY-PLAN NOOP PASS: one noop completion emitted (anti-hang terminal fires)");
}

// ─────────────────────────────────────────────────────────────────────────────
// (5) THE USER-REPORTED GAP: the INTERACTIVE flow (AddSketch → enter → upsert →
// finish) must mint the sketch's timeline record. No production path authored
// one before (only tests did), so every extrude off an interactively drawn
// sketch failed "profile sketch not found in plan" — autosave forensics of the
// user's document showed 20 Extrude records and ZERO Sketch records.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interactive_sketch_flow_mints_the_timeline_record_and_extrude_commits() {
    use onecad_core::edit::SketchEditOp;

    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let (runtime, sched, mut rx) = wire(&wm).await;
    let sid = SketchId(Uuid::from_u128(0xF00));

    // 1. Interactive create + draw + finish — the app path, NO manual sketch_record.
    let (regions, finish_outcome) = {
        let mut guard = runtime.lock().await;
        let rt = guard.as_mut().expect("open");
        rt.apply(EditCommand::AddSketch {
            sketch: Sketch::on_world_plane(sid, "Sketch 1", WorldPlane::XY),
        })
        .expect("AddSketch");
        rt.enter_sketch(sid).await.expect("enter");
        let drawn = rect_sketch(sid, 0x1000, 0.0, 0.0, 40.0, 20.0);
        let ops: Vec<SketchEditOp> = drawn
            .entities()
            .iter()
            .cloned()
            .map(|entity| SketchEditOp::AddEntity { entity })
            .chain(
                drawn
                    .constraints()
                    .iter()
                    .cloned()
                    .map(|constraint| SketchEditOp::AddConstraint { constraint }),
            )
            .collect();
        rt.sketch_upsert(sid, ops).await.expect("upsert");
        let (dto, outcome) = rt.finish_sketch_with_outcome(sid).await.expect("finish");
        let features = rt.projection().features;
        assert_eq!(
            features.iter().filter(|f| f.op_type == "Sketch").count(),
            1,
            "finish minted exactly ONE Sketch timeline record: {features:#?}"
        );
        (
            dto.regions,
            outcome.expect("first finish appends the record ⇒ Some(outcome)"),
        )
    };
    assert!(!regions.is_empty(), "rect derives a region");
    sched.handle(&finish_outcome);
    let (outcome_str, _, _) = recv(&mut rx).await;
    assert_eq!(outcome_str, "published", "sketch-only regen publishes");

    // 2. Extrude binding the EXACT region id from finish — the frontend commit shape.
    let extrude_outcome = {
        let mut guard = runtime.lock().await;
        let rt = guard.as_mut().expect("open");
        let mut record = extrude_record(0xF01, sid, 25.0);
        let Operation::Known(KnownOperation::Extrude(p)) = &mut record.op else {
            unreachable!();
        };
        p.profile.as_mut().unwrap().region = RegionId::new(regions[0].region_id.clone());
        rt.apply(EditCommand::AddOperation {
            record,
            at_cursor: false,
        })
        .expect("extrude commit")
    };
    sched.handle(&extrude_outcome);
    let (outcome_str, change, _) = recv(&mut rx).await;
    assert_eq!(outcome_str, "published");
    assert_eq!(
        change.expect("published change").changed_bodies.len(),
        1,
        "extrude off an interactively drawn sketch produces its body"
    );

    // 3. Re-finish WITHOUT edits: record unchanged ⇒ no outcome, no dirty regen.
    {
        let mut guard = runtime.lock().await;
        let rt = guard.as_mut().expect("open");
        rt.enter_sketch(sid).await.expect("re-enter");
        let (_, outcome) = rt
            .finish_sketch_with_outcome(sid)
            .await
            .expect("no-op finish");
        assert!(
            outcome.is_none(),
            "an edit-free finish must not dirty regen"
        );
    }

    // 4. Re-finish WITH an edit: the record REFRESHES in place (never duplicates).
    let refresh_outcome = {
        let mut guard = runtime.lock().await;
        let rt = guard.as_mut().expect("open");
        rt.enter_sketch(sid).await.expect("re-enter 2");
        let extra = SketchEntity::point(
            EntityId(Uuid::from_u128(0x9F9F)),
            Vec2::new_unchecked(70.0, 70.0),
            false,
            false,
        );
        rt.sketch_upsert(sid, vec![SketchEditOp::AddEntity { entity: extra }])
            .await
            .expect("edit");
        let (_, outcome) = rt
            .finish_sketch_with_outcome(sid)
            .await
            .expect("refresh finish");
        let features = rt.projection().features;
        assert_eq!(
            features.iter().filter(|f| f.op_type == "Sketch").count(),
            1,
            "refresh must UPDATE the record, never duplicate it"
        );
        outcome.expect("changed content refreshes the record")
    };
    sched.handle(&refresh_outcome);
    let (outcome_str, _, _) = recv(&mut rx).await;
    assert_eq!(
        outcome_str, "published",
        "refreshed sketch replays and the downstream extrude re-runs"
    );

    sched.shutdown();
    wm.shutdown().await;
    eprintln!("INTERACTIVE-FLOW PASS: finish mints/refreshes the Sketch record; extrude commits");
}

// ─────────────────────────────────────────────────────────────────────────────
// (6) LEGACY-CONTAINER BACKFILL: documents saved BEFORE finish_sketch minted the
// sketch's timeline record carry sketches with ZERO Sketch records — the exact
// user forensics (20 Extrude records, 0 Sketch records) — and every extrude
// commit off them failed "profile sketch not found in plan". Opening must
// backfill the missing records so a reopened document extrudes.
// ─────────────────────────────────────────────────────────────────────────────

/// [`ContainerWriter::save`] metadata (mirrors container.rs's test `meta()`).
fn save_meta() -> SaveMeta {
    SaveMeta {
        app_version: "0.1.0-test".into(),
        occt_fingerprint: Some("occt-7.9.3".into()),
        created: "2026-07-30T00:00:00Z".into(),
        modified: "2026-07-30T00:00:00Z".into(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn legacy_container_without_sketch_records_extrudes_after_open() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let sid = SketchId(Uuid::from_u128(0xC00));

    // 1. Author the pre-fix container shape: a sketch in the sketches map and an
    //    EMPTY timeline (the interactive flow never authored a Sketch record).
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("legacy.onecad");
    let mut doc = Document::new(DocumentId::new());
    doc.sketches
        .insert(sid, rect_sketch(sid, 0xC000, 0.0, 0.0, 40.0, 20.0));
    ContainerWriter::save(&path, &doc, &ContainerCaches::none(), &save_meta()).unwrap();

    // 2. Open through the production path — the backfill runs inside.
    let engine: Arc<dyn GeometryEngine> = Arc::new(wm.clone());
    let meshes: Arc<dyn MeshProvider> = Arc::new(wm.clone());
    let solver: Arc<dyn SolverEngine> = Arc::new(wm.clone());
    let rt = DocumentRuntime::open(&path, engine, meshes, solver).expect("open legacy");
    let features = rt.projection().features;
    assert_eq!(
        features.iter().filter(|f| f.op_type == "Sketch").count(),
        1,
        "open backfilled exactly ONE Sketch timeline record: {features:#?}"
    );

    // 3. The frontend arm + commit shape: a PURE region read (no session), then
    //    an `at_cursor: false` AddOperation binding the EXACT region id. Before
    //    the backfill the regen step failed "profile sketch not found in plan".
    let regions = rt
        .prepare_sketch_regions(sid)
        .expect("prepare regions")
        .drive()
        .await
        .expect("regions")
        .regions;
    assert!(!regions.is_empty(), "rect derives a region");

    let runtime: Runtime = Arc::new(Mutex::new(Some(rt)));
    let (runtime, sched, mut rx) = wire_with(runtime).await;
    let outcome = {
        let mut guard = runtime.lock().await;
        let rt = guard.as_mut().expect("open");
        let mut record = extrude_record(0xC01, sid, 25.0);
        let Operation::Known(KnownOperation::Extrude(p)) = &mut record.op else {
            unreachable!();
        };
        p.profile.as_mut().unwrap().region = RegionId::new(regions[0].region_id.clone());
        rt.apply(EditCommand::AddOperation {
            record,
            at_cursor: false, // the frontend commit shape
        })
        .expect("extrude commit on reopened legacy doc")
    };
    sched.handle(&outcome);
    let (outcome_str, change, _) = recv(&mut rx).await;
    assert_eq!(outcome_str, "published");
    assert_eq!(
        change.expect("published change").changed_bodies.len(),
        1,
        "extrude off a backfilled legacy sketch produces its body"
    );

    // 4. Fixed-point: save the opened document and reopen — the persisted record
    //    means the backfill inserts NOTHING (record count stays exactly one).
    let path2 = tmp.path().join("resaved.onecad");
    {
        let mut guard = runtime.lock().await;
        let rt = guard.as_mut().expect("open");
        rt.save(&path2, save_meta()).expect("resave");
    }
    let engine: Arc<dyn GeometryEngine> = Arc::new(wm.clone());
    let meshes: Arc<dyn MeshProvider> = Arc::new(wm.clone());
    let solver: Arc<dyn SolverEngine> = Arc::new(wm.clone());
    let rt2 = DocumentRuntime::open(&path2, engine, meshes, solver).expect("reopen resaved");
    let features2 = rt2.projection().features;
    assert_eq!(
        features2.iter().filter(|f| f.op_type == "Sketch").count(),
        1,
        "a resaved container already carries the record — backfill is a no-op: {features2:#?}"
    );

    sched.shutdown();
    wm.shutdown().await;
    eprintln!(
        "LEGACY-BACKFILL PASS: open backfills the Sketch record; extrude commits; fixed-point"
    );
}
