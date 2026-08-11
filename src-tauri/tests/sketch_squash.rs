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

use onecad_core::document::record::{
    BooleanMode, ExtrudeMode, ExtrudeParams, KnownOperation, Operation, OperationRecord,
};
use onecad_core::document::refs::SketchRegionRef;
use onecad_core::document::variables::Scalar;
use onecad_core::edit::{EditCommand, SketchEditOp};
use onecad_core::ids::{ConstraintId, EntityId, RecordId, RegionId, SketchId};
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

/// The `construction` flag the wire carries for `id` (SCHEMA §7.3), or `None` if
/// the entity is absent.
fn construction_flag(entities: &Value, id: EntityId) -> Option<bool> {
    let want = id.to_string();
    entities
        .as_array()?
        .iter()
        .find(|e| e.get("id").and_then(Value::as_str) == Some(want.as_str()))
        .map(|e| {
            e.get("construction")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        })
}

/// A minimal NewBody extrude op referencing `sketch`'s first region — a valid
/// timeline `AddOperation` (reference existence is a regen-time concern, so the
/// command commits without a solved region). Used to interleave a MODEL op into a
/// sketch session (the reported undo-corruption repro).
fn extrude_record(rec: u128, sketch: SketchId) -> OperationRecord {
    let params = ExtrudeParams {
        profile: Some(SketchRegionRef {
            sketch,
            region: RegionId::new(""),
            region_identity_version: None,
            extra: Default::default(),
        }),
        distance: Scalar::new(25.0),
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
        base_depth + 2,
        "finish collapses the 3 granular steps into exactly ONE net command \
         (+1 = the minted Sketch timeline record)"
    );

    // The net command sits over the final sketch (3 entities, 1 constraint).
    let done = rt.get_sketch(sid).expect("get_sketch after finish");
    assert_eq!(entity_count(&done.entities), 3, "P0,P1,L0 present");
    assert_eq!(entity_count(&done.constraints), 1, "Horizontal present");

    // The newest entry is the minted Sketch record; beneath it ONE undo reverts
    // the WHOLE session — sketch back to the pre-session state.
    assert!(rt.undo().is_some(), "undo the minted sketch record");
    assert!(rt.undo().is_some(), "undo the net sketch command");
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
    assert!(
        rt.redo().expect("redo").is_some(),
        "redo the net sketch command"
    );
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

    assert!(rt.undo().is_some(), "undo the net sketch command");
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
        base_depth + 1,
        "a session with no edits squashes nothing; the single push is the sketch's \
         FIRST-finish timeline record (a re-finish with no changes adds nothing)"
    );
    let depth_after_first = rt.undo_depth();
    rt.enter_sketch(sid).await.expect("re-enter");
    rt.finish_sketch(sid).await.expect("re-finish");
    assert_eq!(
        rt.undo_depth(),
        depth_after_first,
        "an unchanged re-finish pushes nothing (record upsert is a no-op)"
    );
    eprintln!("EMPTY-SESSION PASS: only the first finish mints the record; re-finish adds nothing");

    wm.shutdown().await;
}

// ── (d) MODEL-HARDEN W0.5: a stray finish must NOT eat a model op's undo entry ──
//
// The exact reported corruption: an arm path re-opens a sketch session (enterSketch,
// never closed), a MODEL op commits into that watermark range, then a stray
// finish_sketch (a SketchStaticSync refetch) squashes — the OLD net-zero branch
// popped the extrude's own undo entry with no replacement. The B1 all-or-nothing
// guard refuses: the trailing run from head is the extrude, not a same-sketch edit.

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stray_finish_after_model_op_preserves_op_undo_entry() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let sid = SketchId(Uuid::from_u128(0x5A_10));
    rt.apply(EditCommand::AddSketch {
        sketch: Sketch::on_world_plane(sid, "Sq", WorldPlane::XY),
    })
    .expect("AddSketch");
    let base_depth = rt.undo_depth(); // 1 (the AddSketch — not a timeline op)

    // Arm path re-opens a sketch session it never closes (the bug's entry point).
    rt.enter_sketch(sid).await.expect("enter_sketch");
    // A MODEL op commits INTO that still-open session's watermark range.
    rt.apply(EditCommand::AddOperation {
        record: extrude_record(0xE_10, sid),
        at_cursor: false,
    })
    .expect("extrude commit");
    assert_eq!(
        rt.undo_depth(),
        base_depth + 1,
        "the extrude committed exactly one undo step"
    );
    assert_eq!(
        rt.projection().features.len(),
        1,
        "the extrude is a timeline op"
    );

    // The stray finish MUST NOT squash away the extrude. (+1 = the sketch's
    // first-finish timeline record; the extrude's entry survives beneath it.)
    rt.finish_sketch(sid).await.expect("stray finish_sketch");
    assert_eq!(
        rt.undo_depth(),
        base_depth + 2,
        "the stray finish did NOT pop the extrude's undo entry (B1 guard held)"
    );

    // Undo order: newest first — the minted Sketch record, THEN the extrude.
    assert!(rt.undo().is_some(), "undo pops the sketch record");
    assert!(rt.undo().is_some(), "undo pops the extrude");
    assert_eq!(rt.undo_depth(), base_depth, "back to just the AddSketch");
    assert_eq!(
        rt.projection().features.len(),
        0,
        "the extrude op is gone from the timeline"
    );
    assert!(rt.get_sketch(sid).is_ok(), "the sketch itself survives");
    eprintln!("STRAY-FINISH PASS: model op's undo entry survives a stray finish; undo removes the extrude");

    wm.shutdown().await;
}

// ── (f) The construction flag survives the squash replay ─────────────────────
//
// B1 squash rebuilds the net session as whole-entity `AddEntity` ops (every prior
// entity removed, every final entity re-added). Nothing there is aware of
// `SetEntityConstruction`, so the flag survives only because it rides ON the
// entity. If the replay ever loses it, a construction line silently becomes real
// geometry and starts bounding regions again — pinned here end-to-end.

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn construction_flag_survives_the_session_squash() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let sid = SketchId(Uuid::from_u128(0x5A_12));
    rt.apply(EditCommand::AddSketch {
        sketch: Sketch::on_world_plane(sid, "Sq", WorldPlane::XY),
    })
    .expect("AddSketch");
    let base_depth = rt.undo_depth();

    rt.enter_sketch(sid).await.expect("enter_sketch");
    run_three_upserts(&mut rt, sid).await;
    // A 4th in-session edit flips the line to construction geometry.
    rt.sketch_upsert(
        sid,
        vec![SketchEditOp::SetEntityConstruction {
            entity: eid(L0),
            construction: true,
        }],
    )
    .await
    .expect("upsert 4 (setEntityConstruction)");
    assert_eq!(
        construction_flag(&rt.get_sketch(sid).expect("get_sketch").entities, eid(L0)),
        Some(true),
        "the flip reaches the wire the worker consumes"
    );
    assert_eq!(rt.undo_depth(), base_depth + 4, "4 granular steps landed");

    rt.finish_sketch(sid).await.expect("finish_sketch");
    assert_eq!(
        rt.undo_depth(),
        base_depth + 2,
        "the 4 granular steps collapse into ONE net command (+1 timeline record)"
    );
    let done = rt.get_sketch(sid).expect("get_sketch after finish");
    assert_eq!(
        construction_flag(&done.entities, eid(L0)),
        Some(true),
        "the squash replay (whole-entity AddEntity) preserved the construction flag"
    );
    assert_eq!(
        construction_flag(&done.entities, eid(P0)),
        Some(false),
        "the untouched endpoints stay real geometry"
    );

    // Undo the whole session, then redo it: the flag round-trips through the
    // memento AND the replayed net command.
    assert!(rt.undo().is_some(), "undo the minted sketch record");
    assert!(rt.undo().is_some(), "undo the net sketch command");
    let reverted = rt.get_sketch(sid).expect("get_sketch after undo");
    assert_eq!(
        entity_count(&reverted.entities),
        0,
        "session fully reverted"
    );
    assert!(
        rt.redo().expect("redo").is_some(),
        "redo the net sketch command"
    );
    assert_eq!(
        construction_flag(&rt.get_sketch(sid).expect("get_sketch").entities, eid(L0)),
        Some(true),
        "redo restores the construction flag too"
    );
    eprintln!("SQUASH-CONSTRUCTION PASS: flag survives squash + undo/redo");

    wm.shutdown().await;
}

// ── (e) Interleaved sketch→op→sketch: edits stay granular; the op survives ──────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interleaved_session_keeps_edits_granular_and_op_survives() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let sid = SketchId(Uuid::from_u128(0x5A_11));
    rt.apply(EditCommand::AddSketch {
        sketch: Sketch::on_world_plane(sid, "Sq", WorldPlane::XY),
    })
    .expect("AddSketch");
    let base_depth = rt.undo_depth();

    rt.enter_sketch(sid).await.expect("enter_sketch");
    // (1) in-session sketch edit …
    rt.sketch_upsert(
        sid,
        vec![SketchEditOp::AddEntity {
            entity: SketchEntity::point(eid(P0), Vec2::new_unchecked(0.0, 0.0), false, false),
        }],
    )
    .await
    .expect("upsert 1 (P0)");
    // (2) … an INTERLEAVED model op …
    rt.apply(EditCommand::AddOperation {
        record: extrude_record(0xE_11, sid),
        at_cursor: false,
    })
    .expect("extrude commit");
    // (3) … then another in-session sketch edit.
    rt.sketch_upsert(
        sid,
        vec![SketchEditOp::AddEntity {
            entity: SketchEntity::point(eid(P1), Vec2::new_unchecked(40.0, 0.0), false, false),
        }],
    )
    .await
    .expect("upsert 2 (P1)");
    assert_eq!(
        rt.undo_depth(),
        base_depth + 3,
        "3 granular steps landed: edit, extrude, edit"
    );

    // Stray finish: the interleaved extrude breaks the trailing run → NO squash.
    // (+1 = the sketch's first-finish timeline record on top.)
    rt.finish_sketch(sid).await.expect("stray finish_sketch");
    assert_eq!(
        rt.undo_depth(),
        base_depth + 4,
        "no squash across the interleaved op — the sketch edits stay granular"
    );

    // Undo reverts ONLY the head sketch edit (P1), never the whole session; the
    // interleaved extrude survives (this is the correctness the clamp would break).
    assert!(rt.undo().is_some(), "undo the minted sketch record");
    assert!(rt.undo().is_some(), "undo the head sketch edit");
    let s = rt.get_sketch(sid).expect("get_sketch after undo");
    assert_eq!(
        entity_count(&s.entities),
        1,
        "only P1 reverted; P0 survives (no spurious pre-session restore)"
    );
    assert_eq!(
        rt.projection().features.len(),
        1,
        "the interleaved extrude op survives the sketch-edit undo"
    );
    eprintln!("INTERLEAVED PASS: sketch→op→sketch stays granular; op survives");

    wm.shutdown().await;
}
