//! MULTI-OBJECT SKETCH SESSION gate against the REAL C++ OCCT/PlaneGCS worker.
//!
//! User report: draw a rectangle, then a SEPARATE disconnected line in the SAME
//! sketch session — both render while editing — press Finish → only the
//! latest-drawn object survives.
//!
//! Drives the EXACT verb sequence the frontend issues (derived from
//! `SketchController.commitNow` → `tauriClient.sketchUpsert` →
//! `sketchWireMap.marshalUpsert`, and `SketchController.exit()`'s
//! cancel-then-finish pair), not an idealized one:
//!
//!   apply(AddSketch) → enter_sketch
//!   → sketch_upsert(rect: 8 points + 4 lines + H/V + 4 Coincident)
//!   → sketch_upsert(line: 2 points + 1 line + H)          <- second gesture
//!   → cancel_sketch (worker-gesture teardown + take-once squash)
//!   → finish_sketch (solve + regions + Sketch timeline-record upsert)
//!
//! and asserts the DOCUMENT sketch still owns BOTH objects afterwards, plus that
//! a re-entry (`enter_sketch`) reports the same set.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use uuid::Uuid;

use onecad_core::edit::{EditCommand, SketchEditOp};
use onecad_core::ids::{ConstraintId, EntityId, SketchId};
use onecad_core::math::Vec2;
use onecad_core::regen::GeometryEngine;
use onecad_core::sketch::{Constraint, CurvePosition, Sketch, SketchEntity, WorldPlane};

use onecad_lib::document_runtime::DocumentRuntime;
use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::{resolve_worker_path, MeshProvider, SolverEngine, WorkerManager};

// ── Harness (mirrors sketch_squash.rs) ───────────────────────────────────────

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

fn eid(n: u128) -> EntityId {
    EntityId(Uuid::from_u128(n))
}
fn cid(n: u128) -> ConstraintId {
    ConstraintId(Uuid::from_u128(n))
}

fn point(n: u128, x: f64, y: f64) -> SketchEditOp {
    SketchEditOp::AddEntity {
        entity: SketchEntity::point(eid(n), Vec2::new_unchecked(x, y), false, false),
    }
}
fn line(n: u128, a: u128, b: u128) -> SketchEditOp {
    SketchEditOp::AddEntity {
        entity: SketchEntity::line(eid(n), eid(a), eid(b), false),
    }
}
fn horizontal(c: u128, l: u128) -> SketchEditOp {
    SketchEditOp::AddConstraint {
        constraint: Constraint::Horizontal {
            id: cid(c),
            line: eid(l),
        },
    }
}
fn vertical(c: u128, l: u128) -> SketchEditOp {
    SketchEditOp::AddConstraint {
        constraint: Constraint::Vertical {
            id: cid(c),
            line: eid(l),
        },
    }
}
fn coincident(c: u128, p1: u128, p2: u128) -> SketchEditOp {
    SketchEditOp::AddConstraint {
        constraint: Constraint::Coincident {
            id: cid(c),
            point1: eid(p1),
            point2: eid(p2),
            point1_position: CurvePosition::Arbitrary,
            point2_position: CurvePosition::Arbitrary,
        },
    }
}

/// GESTURE 1 — the rectangle tool's single commit: 4 lines, each with its OWN
/// synthesized endpoints (`marshalUpsert.addEntityOps` mints a Point per line
/// endpoint), tied by the auto-inferred Coincident/H/V constraints
/// (`autoConstrain.inferConstraints`). Corners (0,0)→(40,20).
fn rect_gesture_ops() -> Vec<SketchEditOp> {
    vec![
        // L1 (0,0)→(40,0)
        point(0x11, 0.0, 0.0),
        point(0x12, 40.0, 0.0),
        line(0x10, 0x11, 0x12),
        // L2 (40,0)→(40,20)
        point(0x21, 40.0, 0.0),
        point(0x22, 40.0, 20.0),
        line(0x20, 0x21, 0x22),
        // L3 (40,20)→(0,20)
        point(0x31, 40.0, 20.0),
        point(0x32, 0.0, 20.0),
        line(0x30, 0x31, 0x32),
        // L4 (0,20)→(0,0)
        point(0x41, 0.0, 20.0),
        point(0x42, 0.0, 0.0),
        line(0x40, 0x41, 0x42),
        horizontal(0x1000, 0x10),
        vertical(0x1001, 0x20),
        horizontal(0x1002, 0x30),
        vertical(0x1003, 0x40),
        coincident(0x1004, 0x21, 0x12),
        coincident(0x1005, 0x31, 0x22),
        coincident(0x1006, 0x41, 0x32),
        coincident(0x1007, 0x42, 0x11),
    ]
}

/// GESTURE 2 — the line tool's commit: ONE standalone, disconnected horizontal
/// segment far from the rectangle (no coincidence with anything).
fn standalone_line_ops() -> Vec<SketchEditOp> {
    vec![
        point(0x51, 80.0, 60.0),
        point(0x52, 120.0, 60.0),
        line(0x50, 0x51, 0x52),
        horizontal(0x1008, 0x50),
    ]
}

/// Every id the two gestures authored — the full expected entity set.
const ALL_ENTITY_IDS: [u128; 15] = [
    0x11, 0x12, 0x10, 0x21, 0x22, 0x20, 0x31, 0x32, 0x30, 0x41, 0x42, 0x40, 0x51, 0x52, 0x50,
];

/// Entity ids present in a `SketchSessionDto`'s wire entity array.
fn wire_ids(entities: &Value) -> Vec<String> {
    entities
        .as_array()
        .expect("entities array")
        .iter()
        .filter_map(|e| e.get("id").and_then(Value::as_str).map(str::to_owned))
        .collect()
}

fn assert_has_both_objects(entities: &Value, ctx: &str) {
    let ids = wire_ids(entities);
    for want in ALL_ENTITY_IDS {
        let want_str = eid(want).to_string();
        assert!(
            ids.contains(&want_str),
            "{ctx}: entity {want_str} missing — the sketch lost an object it was drawn \
             with (have {} entities: {ids:?})",
            ids.len()
        );
    }
    assert_eq!(
        ids.len(),
        ALL_ENTITY_IDS.len(),
        "{ctx}: unexpected entity count"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_disconnected_objects_in_one_session_both_survive_finish() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let sid = SketchId(Uuid::from_u128(0x5B_00));

    rt.apply(EditCommand::AddSketch {
        sketch: Sketch::on_world_plane(sid, "Sketch 1", WorldPlane::XY),
    })
    .expect("AddSketch");
    rt.enter_sketch(sid).await.expect("enter");

    // Gesture 1: the rectangle.
    rt.sketch_upsert(sid, rect_gesture_ops())
        .await
        .expect("upsert 1 (rect)");
    assert_eq!(
        wire_ids(&rt.get_sketch(sid).expect("get").entities).len(),
        12,
        "after the rect gesture the sketch holds 8 points + 4 lines"
    );

    // Gesture 2: a SEPARATE disconnected line, same session.
    rt.sketch_upsert(sid, standalone_line_ops())
        .await
        .expect("upsert 2 (standalone line)");
    assert_has_both_objects(
        &rt.get_sketch(sid).expect("get").entities,
        "mid-session (both objects render)",
    );

    // SketchController.exit(): cancel FIRST (worker-gesture teardown + take-once
    // squash), then finish (solve + regions + Sketch timeline-record upsert).
    rt.cancel_sketch(sid).await.expect("cancel");
    let (dto, _outcome) = rt.finish_sketch_with_outcome(sid).await.expect("finish");
    assert!(
        !dto.regions.is_empty(),
        "the rectangle still derives a region"
    );

    assert_has_both_objects(
        &rt.get_sketch(sid).expect("get").entities,
        "after Finish sketch",
    );

    // The minted Sketch timeline record must carry the FULL entity set (the regen
    // planner resolves every downstream profile from it).
    let features = rt.projection().features;
    let sketch_records: Vec<_> = features.iter().filter(|f| f.op_type == "Sketch").collect();
    assert_eq!(
        sketch_records.len(),
        1,
        "finish mints exactly one Sketch record"
    );
    let record_id =
        onecad_core::ids::RecordId(Uuid::parse_str(&sketch_records[0].id).expect("record id uuid"));
    let params = rt
        .operation_params(record_id)
        .expect("Sketch record params");
    assert_has_both_objects(
        params.get("entities").expect("record entities"),
        "Sketch timeline record",
    );

    // Re-entry (tree double-click) must report the same set.
    let re = rt.enter_sketch(sid).await.expect("re-enter");
    assert_has_both_objects(&re.entities, "re-entering the sketch");

    wm.shutdown().await;
    eprintln!("MULTI-OBJECT PASS: both sub-sketches survive Finish");
}

/// The RE-ENTRY half — the shape the user actually hit. Each object is drawn in
/// its OWN session (draw → Finish → re-open → draw → Finish), which on the
/// frontend is where the per-sketch id-map has to be rebased onto the
/// authoritative `enter_sketch` wire (`sketchWireMap.seedIdMapFromWire`). A stale
/// binding there marshalled a `removeEntity` for every entity of the previous
/// session, so the first edit after re-opening deleted everything drawn before.
/// This pins the BACKEND contract that half depends on: the add-only op stream a
/// correct client sends across two sessions loses nothing, the sketch-session
/// squash spans both, and the timeline record ends up carrying the union.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_second_session_adds_to_the_sketch_instead_of_replacing_it() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let sid = SketchId(Uuid::from_u128(0x5B_01));

    rt.apply(EditCommand::AddSketch {
        sketch: Sketch::on_world_plane(sid, "Sketch 1", WorldPlane::XY),
    })
    .expect("AddSketch");

    // Session 1 — the rectangle, then SketchController.exit()'s cancel+finish.
    rt.enter_sketch(sid).await.expect("enter 1");
    rt.sketch_upsert(sid, rect_gesture_ops())
        .await
        .expect("upsert (rect)");
    rt.cancel_sketch(sid).await.expect("cancel 1");
    rt.finish_sketch(sid).await.expect("finish 1");

    // Session 2 — re-open the SAME sketch and draw a second, separate object.
    let re = rt.enter_sketch(sid).await.expect("enter 2");
    assert_eq!(
        wire_ids(&re.entities).len(),
        12,
        "re-entry hydrates the rectangle (8 points + 4 lines)"
    );
    rt.sketch_upsert(sid, standalone_line_ops())
        .await
        .expect("upsert (standalone line)");
    rt.cancel_sketch(sid).await.expect("cancel 2");
    rt.finish_sketch(sid).await.expect("finish 2");

    assert_has_both_objects(
        &rt.get_sketch(sid).expect("get").entities,
        "after the SECOND session's Finish",
    );

    // The refreshed timeline record carries the union, and a third re-entry agrees.
    let features = rt.projection().features;
    let sketch_records: Vec<_> = features.iter().filter(|f| f.op_type == "Sketch").collect();
    assert_eq!(
        sketch_records.len(),
        1,
        "the second finish REFRESHES the record, never duplicates it"
    );
    let record_id =
        onecad_core::ids::RecordId(Uuid::parse_str(&sketch_records[0].id).expect("record id uuid"));
    let params = rt
        .operation_params(record_id)
        .expect("Sketch record params");
    assert_has_both_objects(
        params.get("entities").expect("record entities"),
        "Sketch timeline record after two sessions",
    );

    let re2 = rt.enter_sketch(sid).await.expect("enter 3");
    assert_has_both_objects(&re2.entities, "third re-entry");

    wm.shutdown().await;
    eprintln!("MULTI-OBJECT RE-ENTRY PASS: a second session adds, never replaces");
}
