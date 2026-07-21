//! Sketch-edit **squash transaction** gate (Codex-review B1) against the REAL
//! C++ OCCT/PlaneGCS worker.
//!
//! Drives the app's single-writer [`DocumentRuntime`] exactly like `sketch_edit.rs`
//! / `sketch_reentry.rs` (real worker via `ONECAD_WORKER_PATH`, else the dev-tree
//! fallback; `ONECAD_REQUIRE_WORKER=1` hard-fails a missing binary). Pins that all
//! `SketchEdit` commands committed during one sketch session collapse into ONE net
//! undoable command at finish/cancel:
//!
//! * **finish** — 3 in-session `sketch_upsert`s → `finish_sketch` leaves exactly
//!   ONE extra undo step; `undo()` reverts the WHOLE session (sketch back to the
//!   pre-session state); `redo()` restores the final sketch.
//! * **cancel** — the same collapse happens on the cancel path.
//! * **net-zero** — a session with no edits adds nothing to the undo stack.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use uuid::Uuid;

use onecad_core::edit::{EditCommand, SketchEditOp};
use onecad_core::ids::{ConstraintId, EntityId, SketchId};
use onecad_core::math::Vec2;
use onecad_core::regen::GeometryEngine;
use onecad_core::sketch::{Constraint, Sketch, SketchEntity, WorldPlane};

use onecad_lib::document_runtime::DocumentRuntime;
use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::{resolve_worker_path, MeshProvider, SolverEngine, WorkerManager};

// ── Harness (mirrors sketch_edit.rs) ─────────────────────────────────────────

fn real_worker() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("ONECAD_WORKER_PATH") {
        let path = PathBuf::from(&p);
        assert!(
            path.is_file(),
            "ONECAD_WORKER_PATH={p:?} is set but no worker binary exists there"
        );
        return Some(path);
    }
    if let Some(path) = resolve_worker_path() {
        return Some(path);
    }
    assert!(
        std::env::var("ONECAD_REQUIRE_WORKER").as_deref() != Ok("1"),
        "ONECAD_REQUIRE_WORKER=1 but no worker binary resolved (CI must hard-fail)"
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

// ── Fixed ids ────────────────────────────────────────────────────────────────
const P0: u128 = 0x100;
const P1: u128 = 0x101;
const L0: u128 = 0x200;
const C_H: u128 = 0x300;

fn eid(n: u128) -> EntityId {
    EntityId(Uuid::from_u128(n))
}
fn cid(n: u128) -> ConstraintId {
    ConstraintId(Uuid::from_u128(n))
}

fn entity_count(v: &Value) -> usize {
    v.as_array().map_or(0, Vec::len)
}

/// Runs a 3-upsert session (2 points → a line → a Horizontal constraint) so the
/// final sketch is a fully-connected horizontal line. Returns the runtime after
/// the session but BEFORE finish/cancel, with the watermark still open.
async fn run_three_upserts(rt: &mut DocumentRuntime, sid: SketchId) {
    // upsert 1: two endpoints.
    rt.sketch_upsert(
        sid,
        vec![
            SketchEditOp::AddEntity {
                entity: SketchEntity::point(eid(P0), Vec2::new_unchecked(0.0, 0.0), false, false),
            },
            SketchEditOp::AddEntity {
                entity: SketchEntity::point(eid(P1), Vec2::new_unchecked(40.0, 0.0), false, false),
            },
        ],
    )
    .await
    .expect("upsert 1 (points)");
    // upsert 2: the line between them.
    rt.sketch_upsert(
        sid,
        vec![SketchEditOp::AddEntity {
            entity: SketchEntity::line(eid(L0), eid(P0), eid(P1), false),
        }],
    )
    .await
    .expect("upsert 2 (line)");
    // upsert 3: a horizontal constraint on the line.
    rt.sketch_upsert(
        sid,
        vec![SketchEditOp::AddConstraint {
            constraint: Constraint::Horizontal {
                id: cid(C_H),
                line: eid(L0),
            },
        }],
    )
    .await
    .expect("upsert 3 (constraint)");
}

// ── (a) finish collapses the whole session into ONE undo step ────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn finish_squashes_session_into_one_command() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let sid = SketchId(Uuid::from_u128(0x5A_00));
    rt.apply(EditCommand::AddSketch {
        sketch: Sketch::on_world_plane(sid, "Sq", WorldPlane::XY),
    })
    .expect("AddSketch");
    let base_depth = rt.undo_depth(); // 1 (the AddSketch)

    rt.enter_sketch(sid).await.expect("enter_sketch");
    run_three_upserts(&mut rt, sid).await;
    assert_eq!(
        rt.undo_depth(),
        base_depth + 3,
        "each of the 3 upserts commits its own granular undo step"
    );

    rt.finish_sketch(sid).await.expect("finish_sketch");
    assert_eq!(
        rt.undo_depth(),
        base_depth + 1,
        "finish collapses the 3 granular steps into exactly ONE net command"
    );

    // The net command sits over the final sketch (3 entities, 1 constraint).
    let done = rt.get_sketch(sid).expect("get_sketch after finish");
    assert_eq!(entity_count(&done.entities), 3, "P0,P1,L0 present");
    assert_eq!(entity_count(&done.constraints), 1, "Horizontal present");

    // ONE undo reverts the WHOLE session — sketch back to the pre-session state.
    assert!(rt.undo(), "undo the net sketch command");
    assert_eq!(rt.undo_depth(), base_depth, "back to just the AddSketch");
    let reverted = rt.get_sketch(sid).expect("get_sketch after undo");
    assert_eq!(
        entity_count(&reverted.entities),
        0,
        "undo reverts ALL session entities at once"
    );
    assert_eq!(
        entity_count(&reverted.constraints),
        0,
        "undo reverts ALL session constraints at once"
    );

    // Redo restores the final sketch exactly.
    assert!(rt.redo().expect("redo"), "redo the net sketch command");
    let restored = rt.get_sketch(sid).expect("get_sketch after redo");
    assert_eq!(
        entity_count(&restored.entities),
        3,
        "redo restores entities"
    );
    assert_eq!(
        entity_count(&restored.constraints),
        1,
        "redo restores constraints"
    );
    eprintln!("FINISH-SQUASH PASS: 3 upserts → 1 net undo step; undo/redo exact");

    wm.shutdown().await;
}

// ── (b) cancel collapses the session too ─────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_squashes_session_into_one_command() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let sid = SketchId(Uuid::from_u128(0x5A_01));
    rt.apply(EditCommand::AddSketch {
        sketch: Sketch::on_world_plane(sid, "Sq", WorldPlane::XY),
    })
    .expect("AddSketch");
    let base_depth = rt.undo_depth();

    rt.enter_sketch(sid).await.expect("enter_sketch");
    run_three_upserts(&mut rt, sid).await;
    assert_eq!(rt.undo_depth(), base_depth + 3, "3 granular steps landed");

    rt.cancel_sketch(sid).await.expect("cancel_sketch");
    assert_eq!(
        rt.undo_depth(),
        base_depth + 1,
        "cancel collapses the 3 granular steps into exactly ONE net command"
    );

    assert!(rt.undo(), "undo the net sketch command");
    let reverted = rt.get_sketch(sid).expect("get_sketch after undo");
    assert_eq!(
        entity_count(&reverted.entities),
        0,
        "cancel-path undo reverts ALL session edits at once"
    );
    eprintln!("CANCEL-SQUASH PASS: 3 upserts → 1 net undo step on cancel");

    wm.shutdown().await;
}

// ── (c) net-zero session (no edits) adds nothing ─────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn empty_session_adds_no_command() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let sid = SketchId(Uuid::from_u128(0x5A_02));
    rt.apply(EditCommand::AddSketch {
        sketch: Sketch::on_world_plane(sid, "Sq", WorldPlane::XY),
    })
    .expect("AddSketch");
    let base_depth = rt.undo_depth();

    rt.enter_sketch(sid).await.expect("enter_sketch");
    rt.finish_sketch(sid).await.expect("finish_sketch");
    assert_eq!(
        rt.undo_depth(),
        base_depth,
        "a session with no edits pushes nothing onto the undo stack"
    );
    eprintln!("EMPTY-SESSION PASS: enter → finish with no edits adds nothing");

    wm.shutdown().await;
}
