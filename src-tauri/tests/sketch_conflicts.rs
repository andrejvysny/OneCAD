//! Per-constraint **conflict-id** proof (SCHEMA §7.4) against the REAL C++ OCCT
//! worker. A genuinely conflicting sketch must surface the offending constraint
//! ids on BOTH solve surfaces the frontend reads: the `sketch_upsert` re-solve and
//! the session-enter DTO (`enter_sketch`). Mirrors the `sketch_constraints.rs`
//! harness/skip-guards.
//!
//! The conflict is corpus (g)'s `testConflictingConstraintsDetected`: two `Fixed`
//! points at x=0 and x=10 plus a `HorizontalDistance(25)` between them is
//! unsatisfiable ⇒ status `Conflicting` with a non-empty `conflicting[]` id list
//! (the wire ids are the constraint UUID strings).

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use onecad_core::document::variables::Scalar;
use onecad_core::edit::{EditCommand, SketchEditOp};
use onecad_core::ids::{ConstraintId, EntityId, SketchId};
use onecad_core::math::Vec2;
use onecad_core::regen::GeometryEngine;
use onecad_core::sketch::{Constraint, CurvePosition, Sketch, SketchEntity, WorldPlane};

use onecad_lib::document_runtime::DocumentRuntime;
use onecad_lib::dto::SketchSolveStatus;
use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::{resolve_worker_path, MeshProvider, SolverEngine, WorkerManager};

// ── Harness (mirrors sketch_constraints.rs) ───────────────────────────────────

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
fn point(id: u128, x: f64, y: f64) -> SketchEntity {
    SketchEntity::point(eid(id), Vec2::new_unchecked(x, y), false, false)
}

// ── The gate ──────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn conflicting_ids_surface_on_upsert_and_session_enter() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;

    let mut rt = runtime_over(&wm);
    let sid = SketchId(Uuid::from_u128(0xc0ff));
    rt.apply(EditCommand::AddSketch {
        sketch: Sketch::on_world_plane(sid, "conflict", WorldPlane::XY),
    })
    .expect("AddSketch");
    rt.enter_sketch(sid).await.expect("enter_sketch");

    // Two points, then the contradictory constraint set.
    rt.sketch_upsert(
        sid,
        vec![
            SketchEditOp::AddEntity {
                entity: point(0x01, 0.0, 0.0),
            },
            SketchEditOp::AddEntity {
                entity: point(0x02, 10.0, 0.0),
            },
        ],
    )
    .await
    .expect("sketch_upsert (geometry)");

    // Fixed(p1@x=0) + Fixed(p2@x=10) + HorizontalDistance(p1,p2)=25 is unsatisfiable.
    let solved = rt
        .sketch_upsert(
            sid,
            vec![
                SketchEditOp::AddConstraint {
                    constraint: Constraint::Fixed {
                        id: cid(0x90),
                        point: eid(0x01),
                        point_position: CurvePosition::Arbitrary,
                        at: Vec2::new_unchecked(0.0, 0.0),
                    },
                },
                SketchEditOp::AddConstraint {
                    constraint: Constraint::Fixed {
                        id: cid(0x91),
                        point: eid(0x02),
                        point_position: CurvePosition::Arbitrary,
                        at: Vec2::new_unchecked(10.0, 0.0),
                    },
                },
                SketchEditOp::AddConstraint {
                    constraint: Constraint::HorizontalDistance {
                        id: cid(0x92),
                        point1: eid(0x01),
                        point1_position: CurvePosition::Arbitrary,
                        point2: eid(0x02),
                        point2_position: CurvePosition::Arbitrary,
                        value: Scalar::new(25.0),
                    },
                },
            ],
        )
        .await
        .expect("sketch_upsert (conflicting)");

    eprintln!(
        "upsert: status {:?}, conflicting {:?}",
        solved.status, solved.conflicting
    );
    assert_eq!(
        solved.status,
        SketchSolveStatus::Conflicting,
        "the contradictory constraint set must solve Conflicting"
    );
    assert!(
        !solved.conflicting.is_empty(),
        "sketch_upsert must surface the conflicting constraint ids"
    );

    // Every reported id is one of the three constraints we authored (wire = uuid str).
    let known: HashSet<String> = [cid(0x90), cid(0x91), cid(0x92)]
        .iter()
        .map(ToString::to_string)
        .collect();
    for id in &solved.conflicting {
        assert!(
            known.contains(id),
            "conflicting id {id} must be a known authored constraint"
        );
    }

    // A fresh session-enter must surface the SAME conflicting set (SketchSessionDto).
    let session = rt.enter_sketch(sid).await.expect("re-enter_sketch");
    eprintln!(
        "enter: status {:?}, conflicting {:?}",
        session.status, session.conflicting
    );
    assert_eq!(
        session.status,
        SketchSolveStatus::Conflicting,
        "enter_sketch must report the conflicting solve state"
    );
    assert!(
        !session.conflicting.is_empty(),
        "enter_sketch (SketchSessionDto) must surface the conflicting ids"
    );

    let upsert_set: HashSet<&String> = solved.conflicting.iter().collect();
    let enter_set: HashSet<&String> = session.conflicting.iter().collect();
    assert_eq!(
        upsert_set, enter_set,
        "session-enter conflicting ids must match the upsert conflicting ids"
    );
}

/// The clean-sketch counterpart: the ndjson fixture's `"conflicting":[]` only
/// asserts presence + array-type (the subset matcher can't pin emptiness), so THIS
/// is the assertion that a satisfiable sketch reports an EMPTY conflict set on
/// both solve surfaces.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn clean_sketch_reports_empty_conflicting() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;

    let mut rt = runtime_over(&wm);
    let sid = SketchId(Uuid::from_u128(0xc1ea));
    rt.apply(EditCommand::AddSketch {
        sketch: Sketch::on_world_plane(sid, "clean", WorldPlane::XY),
    })
    .expect("AddSketch");
    rt.enter_sketch(sid).await.expect("enter_sketch");

    // Two points + ONE satisfiable HorizontalDistance(10) — solvable, no conflict.
    let solved = rt
        .sketch_upsert(
            sid,
            vec![
                SketchEditOp::AddEntity {
                    entity: point(0x01, 0.0, 0.0),
                },
                SketchEditOp::AddEntity {
                    entity: point(0x02, 10.0, 0.0),
                },
                SketchEditOp::AddConstraint {
                    constraint: Constraint::HorizontalDistance {
                        id: cid(0x90),
                        point1: eid(0x01),
                        point1_position: CurvePosition::Arbitrary,
                        point2: eid(0x02),
                        point2_position: CurvePosition::Arbitrary,
                        value: Scalar::new(10.0),
                    },
                },
            ],
        )
        .await
        .expect("sketch_upsert (clean)");

    assert_ne!(
        solved.status,
        SketchSolveStatus::Conflicting,
        "a satisfiable sketch must not report Conflicting"
    );
    assert!(
        solved.conflicting.is_empty(),
        "a clean sketch_upsert must report an EMPTY conflicting list, got {:?}",
        solved.conflicting
    );

    let session = rt.enter_sketch(sid).await.expect("re-enter_sketch");
    assert!(
        session.conflicting.is_empty(),
        "a clean enter_sketch must report an EMPTY conflicting list, got {:?}",
        session.conflicting
    );
}
