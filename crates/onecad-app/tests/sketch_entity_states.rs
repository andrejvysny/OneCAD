//! SCHEMA §7.4 `entityStates` — the per-entity constrained state, end-to-end
//! through the REAL C++ PlaneGCS worker and out onto the Rust DTOs.
//!
//! Same harness as `sketch_direct_manipulation.rs` (real worker via
//! `ONECAD_WORKER_PATH`, else the dev-tree fallback; `ONECAD_REQUIRE_WORKER=1`
//! hard-fails a missing binary). The worker's own suite and the canonical fixture
//! `protocol/fixtures/sketch_entity_states.ndjson` pin the DERIVATION one layer
//! down; what is under test HERE is that the whole Rust path carries it:
//!
//! * `SketchUpsert` lands a per-entity map on `SketchUpsertDto`, discriminating
//!   within a single `UnderConstrained` sketch (the entire argument for the field),
//! * `BeginGesture` lands one on `BeginGestureDto` and `EndGesture` ECHOES it
//!   byte-identically (the field is gesture-fixed; `SolveDrag` carries none), and
//! * an ellipse-bearing sketch arrives as an EMPTY map — the worker omits the whole
//!   thing, and "empty" must mean *nothing to say*, never "all unconstrained".
//!
//! Typed curve centers share their authored Point through `centerRef` (solver
//! policy 3). Per-entity state must reflect that shared solver identity.

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

use onecad_app::document_runtime::DocumentRuntime;
use onecad_app::dto::EntityConstrainedState as S;
use onecad_app::worker::manager::SupervisorConfig;
use onecad_app::worker::wire::GestureTarget;
use onecad_app::worker::{resolve_worker_path, MeshProvider, SolverEngine, WorkerManager};

// ─────────────────────────────────────────────────────────────────────────────
// Harness (mirrors sketch_direct_manipulation.rs)
// ─────────────────────────────────────────────────────────────────────────────

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

fn edit_ops(sk: &Sketch) -> Vec<SketchEditOp> {
    let mut ops = Vec::new();
    for ent in sk.entities() {
        ops.push(SketchEditOp::AddEntity {
            entity: ent.clone(),
        });
    }
    for con in sk.constraints() {
        ops.push(SketchEditOp::AddConstraint {
            constraint: con.clone(),
        });
    }
    ops
}

fn eid(n: u128) -> EntityId {
    EntityId(Uuid::from_u128(n))
}
fn cid(n: u128) -> ConstraintId {
    ConstraintId(Uuid::from_u128(n))
}

fn point(sk: &mut Sketch, id: u128, x: f64, y: f64) {
    sk.add_entity(SketchEntity::point(
        eid(id),
        Vec2::new_unchecked(x, y),
        false,
        false,
    ))
    .unwrap();
}

/// Builds `sketch` inside a fresh runtime, enters it, and returns the runtime plus
/// the building upsert's solve.
async fn enter_with(
    wm: &WorkerManager,
    sketch: Sketch,
) -> (DocumentRuntime, onecad_app::dto::SketchUpsertDto) {
    let mut rt = runtime_over(wm);
    let sid = sketch.id;
    rt.apply(EditCommand::AddSketch {
        sketch: Sketch::on_world_plane(sid, "EntityStates", WorldPlane::XY),
    })
    .expect("AddSketch");
    rt.enter_sketch(sid).await.expect("enter_sketch");
    let solved = rt
        .sketch_upsert(sid, edit_ops(&sketch))
        .await
        .expect("sketch_upsert (build)");
    (rt, solved)
}

// ─────────────────────────────────────────────────────────────────────────────
// The §7.4 round-1 shape: ONE under-constrained sketch, a pinned line, a free circle
// ─────────────────────────────────────────────────────────────────────────────

const SID: u128 = 0x6f00;
const P1: u128 = 0x100; // (0,0), Fixed
const P2: u128 = 0x101; // (10,0)
const LINE: u128 = 0x200;
const CENTER: u128 = 0x102; // the circle's centre POINT entity
const CIRCLE: u128 = 0x201;

/// p1 Fixed at the origin; Horizontal(L) fixes p2.y; Distance(p1,p2)=10 fixes p2.x.
/// The line owns no parameter of its own, so both its endpoints being pinned is
/// exactly what makes it `fullyConstrained`. The circle carries all the remaining
/// freedom.
fn pinned_line_free_circle(sid: SketchId) -> Sketch {
    let mut sk = Sketch::on_world_plane(sid, "EntityStates", WorldPlane::XY);
    point(&mut sk, P1, 0.0, 0.0);
    point(&mut sk, P2, 10.0, 0.0);
    sk.add_entity(SketchEntity::line(eid(LINE), eid(P1), eid(P2), false))
        .unwrap();
    point(&mut sk, CENTER, 20.0, 20.0);
    sk.add_entity(SketchEntity::circle(eid(CIRCLE), eid(CENTER), 5.0, false).unwrap())
        .unwrap();
    sk.add_constraint(Constraint::Fixed {
        id: cid(0x300),
        point: eid(P1),
        point_position: CurvePosition::Arbitrary,
        at: Vec2::new_unchecked(0.0, 0.0),
    })
    .unwrap();
    sk.add_constraint(Constraint::Horizontal {
        id: cid(0x301),
        line: eid(LINE),
    })
    .unwrap();
    sk.add_constraint(Constraint::Distance {
        id: cid(0x302),
        entity1: eid(P1),
        entity1_position: CurvePosition::Arbitrary,
        entity2: eid(P2),
        entity2_position: CurvePosition::Arbitrary,
        value: Scalar::new(10.0),
    })
    .unwrap();
    sk
}

/// Radius plus Fixed on the authored center fully constrain the circle.
fn pin_the_circle_the_frontend_way(sk: &mut Sketch) {
    sk.add_constraint(Constraint::Radius {
        id: cid(0x310),
        entity: eid(CIRCLE),
        value: Scalar::new(5.0),
    })
    .unwrap();
    sk.add_constraint(Constraint::Fixed {
        id: cid(0x312),
        point: eid(CENTER),
        point_position: CurvePosition::Arbitrary,
        at: Vec2::new_unchecked(20.0, 20.0),
    })
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_authorable_circle_pin_reads_fully_constrained() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let sid = SketchId(Uuid::from_u128(SID));
    let mut sk = pinned_line_free_circle(sid);
    pin_the_circle_the_frontend_way(&mut sk);
    let (_rt, solved) = enter_with(&wm, sk).await;

    eprintln!(
        "FE-SHAPE: dof={} states={:?}",
        solved.dof, solved.entity_states
    );
    assert_eq!(state(&solved, CENTER), S::FullyConstrained);
    assert_eq!(state(&solved, CIRCLE), S::FullyConstrained);
    assert_eq!(solved.dof, 0);
    assert_eq!(state(&solved, LINE), S::FullyConstrained);
}

fn state(
    solved: &onecad_app::dto::SketchUpsertDto,
    id: u128,
) -> onecad_app::dto::EntityConstrainedState {
    let key = eid(id).to_string();
    *solved
        .entity_states
        .get(&key)
        .unwrap_or_else(|| panic!("{key} is in the map: {:?}", solved.entity_states))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sketch_upsert_reports_a_pinned_line_and_a_free_circle_in_one_sketch() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let sid = SketchId(Uuid::from_u128(SID));
    let (mut rt, solved) = enter_with(&wm, pinned_line_free_circle(sid)).await;

    eprintln!(
        "ROUND 1: dof={} status={:?} states={:?}",
        solved.dof, solved.status, solved.entity_states
    );
    assert_eq!(
        solved.dof, 3,
        "the circle carries the whole remaining freedom"
    );
    assert_eq!(
        solved.status,
        onecad_app::dto::SketchSolveStatus::UnderConstrained
    );
    // THE assertion: within ONE UnderConstrained sketch the entities differ.
    assert_eq!(state(&solved, P1), S::FullyConstrained);
    assert_eq!(state(&solved, P2), S::FullyConstrained);
    assert_eq!(
        state(&solved, LINE),
        S::FullyConstrained,
        "a Line owns no parameter of its own — both endpoints pinned IS pinned"
    );
    assert_eq!(state(&solved, CIRCLE), S::UnderConstrained);
    assert_eq!(state(&solved, CENTER), S::UnderConstrained);
    assert_eq!(
        solved.entity_states.len(),
        5,
        "keyed by authored wire entity id only: {:?}",
        solved.entity_states
    );

    // Fully constrain the circle too ⇒ every entity flips.
    let mut pinned = pinned_line_free_circle(sid);
    pin_the_circle_the_frontend_way(&mut pinned);
    let extra: Vec<SketchEditOp> = pinned
        .constraints()
        .iter()
        .skip(3)
        .map(|c| SketchEditOp::AddConstraint {
            constraint: c.clone(),
        })
        .collect();
    assert_eq!(extra.len(), 2);
    let solved = rt.sketch_upsert(sid, extra).await.expect("sketch_upsert");
    eprintln!(
        "ROUND 2: dof={} status={:?} states={:?}",
        solved.dof, solved.status, solved.entity_states
    );
    assert_eq!(solved.dof, 0);
    assert_eq!(
        solved.status,
        onecad_app::dto::SketchSolveStatus::FullyConstrained
    );
    for id in [P1, P2, LINE, CIRCLE, CENTER] {
        assert_eq!(
            state(&solved, id),
            S::FullyConstrained,
            "entity {:?} in a dof-0 sketch",
            eid(id)
        );
    }

    wm.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_gesture_map_is_fixed_at_begin_and_echoed_verbatim_at_end() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let sid = SketchId(Uuid::from_u128(SID + 1));
    let (mut rt, _) = enter_with(&wm, pinned_line_free_circle(sid)).await;

    // Drag the free centre point. `BeginGesture` answers for the COMMITTED sketch.
    let begin = rt
        .begin_gesture(sid, eid(CENTER), GestureTarget::point(eid(CENTER)))
        .await
        .expect("begin_gesture");
    eprintln!(
        "BEGIN: ready={} states={:?}",
        begin.ready, begin.entity_states
    );
    assert!(begin.ready);
    assert_eq!(
        begin.entity_states.len(),
        5,
        "the map rides BeginGesture: {:?}",
        begin.entity_states
    );
    assert_eq!(
        begin.entity_states[&eid(LINE).to_string()],
        S::FullyConstrained
    );
    assert_eq!(
        begin.entity_states[&eid(CENTER).to_string()],
        S::UnderConstrained
    );

    rt.solve_drag([26.0, 20.0]).await.expect("solve_drag");
    rt.solve_drag([30.0, 20.0]).await.expect("solve_drag");
    let end = rt
        .end_gesture(Some([30.0, 20.0]))
        .await
        .expect("end_gesture");
    eprintln!("END: dof={} states={:?}", end.dof, end.entity_states);
    // Gesture-FIXED: `EndGesture` echoes the BeginGesture map rather than
    // re-deriving it (a drag adds no constraint, and by pointer-up the drag steps
    // have invalidated the diagnosis it comes from).
    assert_eq!(
        end.entity_states, begin.entity_states,
        "EndGesture echoes the BeginGesture map byte-identically"
    );

    wm.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_ellipse_bearing_sketch_arrives_with_an_empty_map() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let sid = SketchId(Uuid::from_u128(SID + 2));

    // Same pinned line, plus an Ellipse. An ellipse is not registered with PlaneGCS
    // (§7.4 documented deviation), so the sketch falls back to a naive static dof
    // count and NO diagnosis exists — the worker omits the WHOLE map rather than
    // answering for the entities that happen to have one.
    let mut sk = pinned_line_free_circle(sid);
    point(&mut sk, 0x103, 30.0, 30.0);
    sk.add_entity(SketchEntity::ellipse(eid(0x202), eid(0x103), 8.0, 4.0, 0.0, false).unwrap())
        .unwrap();
    let (_rt, solved) = enter_with(&wm, sk).await;

    eprintln!(
        "ELLIPSE: dof={} status={:?} states={:?}",
        solved.dof, solved.status, solved.entity_states
    );
    assert!(
        solved.entity_states.is_empty(),
        "an ellipse omits the WHOLE map — empty means NOTHING TO SAY, never \
         'everything unconstrained': {:?}",
        solved.entity_states
    );

    wm.shutdown().await;
}
