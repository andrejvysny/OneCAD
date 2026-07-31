//! Sketch **edit-delete** gate against the REAL C++ OCCT/PlaneGCS worker.
//!
//! Drives the app's single-writer [`DocumentRuntime`] exactly like `sketch_reentry.rs`
//! / `m2_gate.rs` (real worker via `ONECAD_WORKER_PATH`, else the dev-tree fallback;
//! `ONECAD_REQUIRE_WORKER=1` hard-fails a missing binary). Covers the deletion lane
//! that had ZERO Rust coverage:
//!
//! * **RemoveConstraint** — dropping a geometric constraint via `sketch_upsert` raises
//!   the dof and re-solves to a valid under-constrained state.
//! * **RemoveEntity cascade** — removing a line drops it AND every constraint that
//!   references it (`cascade_remove_entity`); the post-delete dof equals a from-scratch
//!   sketch built without that line + its constraints.
//! * **Unknown RemoveConstraint** — removing a constraint id that is not present is a
//!   silent no-op (dof + constraint set unchanged), pinning the current behavior.

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
use onecad_lib::dto::SketchSolveStatus;
use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::{resolve_worker_path, MeshProvider, SolverEngine, WorkerManager};

// ─────────────────────────────────────────────────────────────────────────────
// Harness (mirrors sketch_reentry.rs)
// ─────────────────────────────────────────────────────────────────────────────

/// Resolve the worker binary honoring the CI guards: a set-but-missing
/// `ONECAD_WORKER_PATH` PANICS, `ONECAD_REQUIRE_WORKER=1` with no worker PANICS,
/// else a quiet local-dev skip (`None`).
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

/// The ordered `SketchEditOp` batch to build a populated sketch (AddEntity then
/// AddConstraint) — the ops the frontend marshaller emits.
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

// ── Fixed ids shared across the fixtures ──────────────────────────────────────
const P0: u128 = 0x100; // (0,0)
const P1: u128 = 0x101; // (40,0)
const P2: u128 = 0x102; // (40,40)
const L0: u128 = 0x200; // line P0→P1
const L1: u128 = 0x201; // line P1→P2
const C_H: u128 = 0x300; // Horizontal(L0)
const C_P: u128 = 0x301; // Perpendicular(L0,L1)

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

/// A single horizontal line: P0,P1 + L0 + Horizontal(L0).
fn horizontal_line_sketch(sid: SketchId) -> Sketch {
    let mut sk = Sketch::on_world_plane(sid, "HLine", WorldPlane::XY);
    point(&mut sk, P0, 0.0, 0.0);
    point(&mut sk, P1, 40.0, 0.0);
    sk.add_entity(SketchEntity::line(eid(L0), eid(P0), eid(P1), false))
        .unwrap();
    sk.add_constraint(Constraint::Horizontal {
        id: cid(C_H),
        line: eid(L0),
    })
    .unwrap();
    sk
}

/// Two lines with a Horizontal on L0 and a Perpendicular between L0 and L1 — both
/// constraints reference L0, so removing L0 cascades both away.
fn cascade_sketch(sid: SketchId) -> Sketch {
    let mut sk = Sketch::on_world_plane(sid, "Cascade", WorldPlane::XY);
    point(&mut sk, P0, 0.0, 0.0);
    point(&mut sk, P1, 40.0, 0.0);
    point(&mut sk, P2, 40.0, 40.0);
    sk.add_entity(SketchEntity::line(eid(L0), eid(P0), eid(P1), false))
        .unwrap();
    sk.add_entity(SketchEntity::line(eid(L1), eid(P1), eid(P2), false))
        .unwrap();
    sk.add_constraint(Constraint::Horizontal {
        id: cid(C_H),
        line: eid(L0),
    })
    .unwrap();
    sk.add_constraint(Constraint::Perpendicular {
        id: cid(C_P),
        line1: eid(L0),
        line2: eid(L1),
    })
    .unwrap();
    sk
}

/// The cascade sketch WITHOUT L0 + its two constraints (the from-scratch dof
/// reference for the cascade test). P0/P1 survive as free points, exactly as they
/// do after a `RemoveEntity { L0 }` cascade (points are never cascade-removed).
fn line_less_reference(sid: SketchId) -> Sketch {
    let mut sk = Sketch::on_world_plane(sid, "Ref", WorldPlane::XY);
    point(&mut sk, P0, 0.0, 0.0);
    point(&mut sk, P1, 40.0, 0.0);
    point(&mut sk, P2, 40.0, 40.0);
    sk.add_entity(SketchEntity::line(eid(L1), eid(P1), eid(P2), false))
        .unwrap();
    sk
}

// ── wire helpers (SketchSessionDto.entities/constraints are raw JSON arrays) ──

fn entity_by_id<'a>(entities: &'a Value, id: &str) -> Option<&'a Value> {
    entities
        .as_array()?
        .iter()
        .find(|e| e["id"] == Value::String(id.to_string()))
}

fn constraint_by_type<'a>(constraints: &'a Value, ty: &str) -> Option<&'a Value> {
    constraints
        .as_array()?
        .iter()
        .find(|c| c["type"] == Value::String(ty.to_string()))
}

// ─────────────────────────────────────────────────────────────────────────────
// (a) RemoveConstraint — dof rises + valid solve state.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remove_constraint_raises_dof() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let sid = SketchId(Uuid::from_u128(0xED_00));
    let sketch = horizontal_line_sketch(sid);
    rt.apply(EditCommand::AddSketch {
        sketch: Sketch::on_world_plane(sid, "HLine", WorldPlane::XY),
    })
    .expect("AddSketch");
    rt.enter_sketch(sid).await.expect("enter_sketch");
    let before = rt
        .sketch_upsert(sid, edit_ops(&sketch))
        .await
        .expect("sketch_upsert (build)");

    // Drop the Horizontal constraint → the sketch frees a dof.
    let after = rt
        .sketch_upsert(
            sid,
            vec![SketchEditOp::RemoveConstraint {
                constraint: cid(C_H),
            }],
        )
        .await
        .expect("sketch_upsert (remove Horizontal)");

    assert!(
        after.dof > before.dof,
        "removing a constraint raises dof ({} → {})",
        before.dof,
        after.dof
    );
    assert_eq!(
        after.status,
        SketchSolveStatus::UnderConstrained,
        "the freed sketch re-solves under-constrained"
    );
    // The constraint is gone from the projection the panels read.
    let session = rt.enter_sketch(sid).await.expect("re-enter");
    assert!(
        constraint_by_type(&session.constraints, "Horizontal").is_none(),
        "the removed Horizontal is absent from the projection"
    );
    eprintln!(
        "REMOVE-CONSTRAINT PASS: dof {} → {} after dropping Horizontal",
        before.dof, after.dof
    );

    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// (b) RemoveEntity cascade — dependent constraints vanish, dof matches reference.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn remove_entity_cascades_dependent_constraints() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let sid = SketchId(Uuid::from_u128(0xED_01));
    let sketch = cascade_sketch(sid);
    rt.apply(EditCommand::AddSketch {
        sketch: Sketch::on_world_plane(sid, "Cascade", WorldPlane::XY),
    })
    .expect("AddSketch");
    rt.enter_sketch(sid).await.expect("enter_sketch");
    rt.sketch_upsert(sid, edit_ops(&sketch))
        .await
        .expect("sketch_upsert (build)");

    // Remove L0 → cascade drops L0 AND both constraints that reference it.
    rt.sketch_upsert(sid, vec![SketchEditOp::RemoveEntity { entity: eid(L0) }])
        .await
        .expect("sketch_upsert (remove L0)");
    let after = rt.enter_sketch(sid).await.expect("re-enter after remove");

    assert!(
        entity_by_id(&after.entities, &eid(L0).to_string()).is_none(),
        "L0 is gone after RemoveEntity"
    );
    assert!(
        entity_by_id(&after.entities, &eid(L1).to_string()).is_some(),
        "L1 (does not reference L0) survives"
    );
    assert!(
        constraint_by_type(&after.constraints, "Horizontal").is_none(),
        "Horizontal(L0) cascaded away with L0"
    );
    assert!(
        constraint_by_type(&after.constraints, "Perpendicular").is_none(),
        "Perpendicular(L0,L1) cascaded away with L0"
    );

    // dof correct: a from-scratch sketch WITHOUT L0 + its constraints solves to the
    // SAME dof (P0/P1 survive as free points in BOTH, so they cancel out).
    let mut rt2 = runtime_over(&wm);
    let sid2 = SketchId(Uuid::from_u128(0xED_02));
    let reference = line_less_reference(sid2);
    rt2.apply(EditCommand::AddSketch {
        sketch: Sketch::on_world_plane(sid2, "Ref", WorldPlane::XY),
    })
    .expect("AddSketch (ref)");
    rt2.enter_sketch(sid2).await.expect("enter_sketch (ref)");
    let ref_solved = rt2
        .sketch_upsert(sid2, edit_ops(&reference))
        .await
        .expect("sketch_upsert (ref)");
    assert_eq!(
        after.dof, ref_solved.dof,
        "post-cascade dof ({}) equals the from-scratch line-less reference ({})",
        after.dof, ref_solved.dof
    );
    eprintln!(
        "CASCADE PASS: L0 removed, both dependent constraints gone, dof={} matches reference",
        after.dof
    );

    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// (c) Unknown RemoveConstraint — silent no-op (pins current behavior).
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remove_unknown_constraint_is_noop() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let sid = SketchId(Uuid::from_u128(0xED_03));
    let sketch = horizontal_line_sketch(sid);
    rt.apply(EditCommand::AddSketch {
        sketch: Sketch::on_world_plane(sid, "HLine", WorldPlane::XY),
    })
    .expect("AddSketch");
    rt.enter_sketch(sid).await.expect("enter_sketch");
    let before = rt
        .sketch_upsert(sid, edit_ops(&sketch))
        .await
        .expect("sketch_upsert (build)");

    // Remove a constraint id that is NOT in the sketch → silent no-op.
    let after = rt
        .sketch_upsert(
            sid,
            vec![SketchEditOp::RemoveConstraint {
                constraint: cid(0xDEAD_BEEF),
            }],
        )
        .await
        .expect("sketch_upsert (remove unknown) must not error");

    assert_eq!(
        after.dof, before.dof,
        "removing an unknown constraint does not change dof"
    );
    let session = rt.enter_sketch(sid).await.expect("re-enter");
    assert!(
        constraint_by_type(&session.constraints, "Horizontal").is_some(),
        "the real constraint is untouched by the unknown removal"
    );
    eprintln!("UNKNOWN-REMOVE PASS: no-op, dof stayed {}", after.dof);

    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// (d) W3 — an ELLIPSE upserts, and its presence switches the whole sketch onto
//     the NAIVE dof path (SCHEMA §7.4 deviation).
//
// `SolverAdapter::populateSolver` deliberately skips ellipses (legacy parity —
// the OneCAD-CPP oracle solver has zero ellipse references), so
// `Sketch::hasSolverUnsupportedEntities()` routes `getDegreesOfFreedom()` /
// `isOverConstrained()` / `hasRedundantConstraints()` to static counting. The
// observable consequence, pinned here: the SAME redundant constraint pair that
// reports `OverConstrained` on a pure line/point sketch reports nothing once an
// ellipse joins it. That is a documented deviation, not a regression.
// ─────────────────────────────────────────────────────────────────────────────

const PE: u128 = 0x120; // ellipse centre
const ELL: u128 = 0x400;
const C_H2: u128 = 0x302; // the DUPLICATE Horizontal(L0)
const C_FIX: u128 = 0x303; // Fixed(P0)

/// The `sketch_redundant.ndjson` shape in core terms: a line anchored at one end
/// with a DUPLICATE Horizontal — solvable, but PlaneGCS flags the second
/// Horizontal as benign redundancy ⇒ `OverConstrained`.
fn redundant_line_sketch(sid: SketchId) -> Sketch {
    let mut sk = horizontal_line_sketch(sid);
    sk.add_constraint(Constraint::Fixed {
        id: cid(C_FIX),
        point: eid(P0),
        at: Vec2::new_unchecked(0.0, 0.0),
    })
    .unwrap();
    sk.add_constraint(Constraint::Horizontal {
        id: cid(C_H2),
        line: eid(L0),
    })
    .unwrap();
    sk
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_ellipse_switches_the_sketch_onto_the_naive_dof_path() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;

    // (1) Baseline — no ellipse: PlaneGCS diagnoses the duplicate Horizontal.
    let mut rt = runtime_over(&wm);
    let sid = SketchId(Uuid::from_u128(0xED_04));
    rt.apply(EditCommand::AddSketch {
        sketch: Sketch::on_world_plane(sid, "Redundant", WorldPlane::XY),
    })
    .expect("AddSketch");
    rt.enter_sketch(sid).await.expect("enter_sketch");
    let baseline = rt
        .sketch_upsert(sid, edit_ops(&redundant_line_sketch(sid)))
        .await
        .expect("sketch_upsert (baseline)");
    assert_eq!(
        baseline.status,
        SketchSolveStatus::OverConstrained,
        "PlaneGCS reports the duplicate Horizontal as benign redundancy"
    );

    // (2) The same sketch plus a disjoint ellipse.
    let mut rt2 = runtime_over(&wm);
    let sid2 = SketchId(Uuid::from_u128(0xED_05));
    let mut with_ellipse = redundant_line_sketch(sid2);
    point(&mut with_ellipse, PE, 100.0, 100.0);
    with_ellipse
        .add_entity(
            SketchEntity::ellipse(eid(ELL), eid(PE), 6.0, 3.0, 0.25, false).expect("ellipse"),
        )
        .unwrap();

    rt2.apply(EditCommand::AddSketch {
        sketch: Sketch::on_world_plane(sid2, "RedundantEllipse", WorldPlane::XY),
    })
    .expect("AddSketch");
    rt2.enter_sketch(sid2).await.expect("enter_sketch");
    let solved = rt2
        .sketch_upsert(sid2, edit_ops(&with_ellipse))
        .await
        .expect("an ellipse-bearing sketch upserts (it used to fail wire translation)");

    assert_ne!(
        solved.status,
        SketchSolveStatus::OverConstrained,
        "SCHEMA §7.4: redundancy is a PlaneGCS notion and goes UNREPORTED on the \
         naive path an ellipse forces"
    );
    assert_eq!(
        solved.status,
        SketchSolveStatus::UnderConstrained,
        "naive dof > 0 ⇒ UnderConstrained"
    );

    // Naive arithmetic, entity by entity: P0,P1 (4) + line (0) + the core centre
    // Point (2) + the centre the worker mints from the inlined `center` (2) +
    // ellipse shape (3) = 11, minus Fixed (2) and two Horizontals (1 each) = 7.
    // The redundant Horizontal still subtracts here — that is exactly the
    // documented naive-count inaccuracy.
    assert_eq!(
        solved.dof, 7,
        "naive dof = 11 entity DOF − 4 constraint arity"
    );
    // The contrast that proves the two paths are genuinely different: naive
    // counting on the BASELINE would give 4 − 4 = 0 (FullyConstrained), because
    // the redundant Horizontal subtracts a DOF it does not actually remove.
    // PlaneGCS knows better and reports 1.
    assert_eq!(
        baseline.dof, 1,
        "the ellipse-free sketch is diagnosed by PlaneGCS, not counted"
    );
    eprintln!(
        "ELLIPSE-DOF PASS: baseline status={:?} dof={}, with-ellipse status={:?} dof={}",
        baseline.status, baseline.dof, solved.status, solved.dof
    );

    wm.shutdown().await;
}
