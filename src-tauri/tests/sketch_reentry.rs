//! Sketch re-entry **gate** (BUG-5 + BUG-2) against the REAL C++ OCCT worker.
//!
//! Drives the app's single-writer [`DocumentRuntime`] the same way `m2_gate.rs` /
//! `topology_rebind.rs` do (real worker via `ONECAD_WORKER_PATH`, else the dev-tree
//! fallback; `ONECAD_REQUIRE_WORKER=1` hard-fails a missing binary). Proves that a
//! re-entered sketch is behaviorally identical to a fresh one:
//!
//! * **BUG-5 ownership** — on re-entry the wire circle/arc carry a `centerRef` (the
//!   center point uuid) and lines carry `p0Ref`/`p1Ref`, so hydration can re-own the
//!   child points instead of leaking stray Points.
//! * **BUG-5 drag** — dragging a line endpoint AND a circle center (by the center
//!   Point uuid) moves the geometry (the circle actually relocates).
//! * **BUG-5 delete** — removing a circle (+ its center point, as the marshaller
//!   emits) leaves NO orphan child point; the re-solved dof equals a from-scratch
//!   sketch without the circle.
//! * **BUG-2 angle** — the Angle value crosses the wire in RADIANS: an authored π/2
//!   comes back π/2, and a `SetDimension` to π/4 comes back π/4.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use uuid::Uuid;

use onecad_core::document::variables::Scalar;
use onecad_core::edit::{EditCommand, SketchEditOp};
use onecad_core::ids::{ConstraintId, EntityId, SketchId};
use onecad_core::math::Vec2;
use onecad_core::regen::GeometryEngine;
use onecad_core::sketch::{Constraint, Sketch, SketchEntity, WorldPlane};

use onecad_lib::document_runtime::DocumentRuntime;
use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::wire::GestureTarget;
use onecad_lib::worker::{resolve_worker_path, MeshProvider, SolverEngine, WorkerManager};

// ─────────────────────────────────────────────────────────────────────────────
// Harness (mirrors topology_rebind.rs)
// ─────────────────────────────────────────────────────────────────────────────

/// Resolve the worker binary, honoring the CI / misconfiguration guards: a set-but-
/// missing `ONECAD_WORKER_PATH` PANICS, `ONECAD_REQUIRE_WORKER=1` with no worker
/// PANICS, else a quiet local-dev skip (`None`).
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

/// Derive the ordered `SketchEditOp` batch from a populated sketch (AddEntity points
/// + curves, then AddConstraint) — the ops the frontend marshaller emits.
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

// ─────────────────────────────────────────────────────────────────────────────
// Fixed ids for the re-entry sketch (line l0 + line l1 + circle + arc).
// ─────────────────────────────────────────────────────────────────────────────

const SID: u128 = 0x5e00;
// Line l0 endpoints, line l1 endpoints, circle center, arc center.
const P0: u128 = 0x100; // l0.Start (0,0)
const P1: u128 = 0x101; // l0.End   (40,0)  ← dragged
const Q0: u128 = 0x110; // l1.Start (0,0)
const Q1: u128 = 0x111; // l1.End   (0,40)
const CCENTER: u128 = 0x120; // circle center (20,20) ← dragged
const ACENTER: u128 = 0x130; // arc center (70,10)
const L0: u128 = 0x200;
const L1: u128 = 0x201;
const CIRC: u128 = 0x210;
const ARC: u128 = 0x220;
const C_ANGLE: u128 = 0x300;
const C_RADIUS: u128 = 0x301;

fn eid(n: u128) -> EntityId {
    EntityId(Uuid::from_u128(n))
}
fn cid(n: u128) -> ConstraintId {
    ConstraintId(Uuid::from_u128(n))
}

/// The re-entry sketch: two lines (perpendicular, tied by an Angle dimension), a
/// free circle (Radius dim), and an arc. Angle carries the WIRE value (radians).
fn reentry_sketch(angle_rad: f64) -> Sketch {
    let mut sk = Sketch::on_world_plane(SketchId(Uuid::from_u128(SID)), "ReEntry", WorldPlane::XY);
    let pt = |sk: &mut Sketch, id: u128, x: f64, y: f64| {
        sk.add_entity(SketchEntity::point(
            eid(id),
            Vec2::new_unchecked(x, y),
            false,
            false,
        ))
        .unwrap();
    };
    pt(&mut sk, P0, 0.0, 0.0);
    pt(&mut sk, P1, 40.0, 0.0);
    pt(&mut sk, Q0, 0.0, 0.0);
    pt(&mut sk, Q1, 0.0, 40.0);
    pt(&mut sk, CCENTER, 20.0, 20.0);
    pt(&mut sk, ACENTER, 70.0, 10.0);
    sk.add_entity(SketchEntity::line(eid(L0), eid(P0), eid(P1), false))
        .unwrap();
    sk.add_entity(SketchEntity::line(eid(L1), eid(Q0), eid(Q1), false))
        .unwrap();
    sk.add_entity(SketchEntity::circle(eid(CIRC), eid(CCENTER), 5.0, false).unwrap())
        .unwrap();
    sk.add_entity(
        SketchEntity::arc(
            eid(ARC),
            eid(ACENTER),
            4.0,
            0.0,
            std::f64::consts::FRAC_PI_2,
            false,
        )
        .unwrap(),
    )
    .unwrap();
    // Angle between the two lines (authored in the wire domain: radians).
    sk.add_constraint(Constraint::Angle {
        id: cid(C_ANGLE),
        line1: eid(L0),
        line2: eid(L1),
        value: Scalar::new(angle_rad),
    })
    .unwrap();
    sk.add_constraint(Constraint::Radius {
        id: cid(C_RADIUS),
        entity: eid(CIRC),
        value: Scalar::new(5.0),
    })
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

fn xy(v: &Value) -> [f64; 2] {
    [v[0].as_f64().unwrap(), v[1].as_f64().unwrap()]
}

fn constraint_by_type<'a>(constraints: &'a Value, ty: &str) -> Option<&'a Value> {
    constraints
        .as_array()?
        .iter()
        .find(|c| c["type"] == Value::String(ty.to_string()))
}

/// Count the free-standing `Point` entities on the wire (orphan detector).
fn point_ids(entities: &Value) -> Vec<String> {
    entities
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter(|e| e["type"] == "Point")
                .map(|e| e["id"].as_str().unwrap_or("").to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// Drive a full drag gesture (begin → one solve → end-commit) on `drag_point`.
async fn drag(rt: &mut DocumentRuntime, sid: SketchId, drag_point: EntityId, target: [f64; 2]) {
    rt.begin_gesture(sid, drag_point, GestureTarget::point(drag_point))
        .await
        .expect("begin_gesture");
    rt.solve_drag(target).await.expect("solve_drag");
    rt.end_gesture(Some(target)).await.expect("end_gesture");
}

// ─────────────────────────────────────────────────────────────────────────────
// The gate.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sketch_reentry_ownership_drag_delete() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    // ── Build + enter the sketch, sync the geometry to the solver lane. ─────────
    let sid = SketchId(Uuid::from_u128(SID));
    let sketch = reentry_sketch(std::f64::consts::FRAC_PI_2); // 90° authored
    rt.apply(EditCommand::AddSketch {
        sketch: Sketch::on_world_plane(sid, "ReEntry", WorldPlane::XY),
    })
    .expect("AddSketch");
    rt.enter_sketch(sid).await.expect("enter_sketch");
    rt.sketch_upsert(sid, edit_ops(&sketch))
        .await
        .expect("sketch_upsert (build)");

    // ── EXIT + RE-ENTER: the wire carries ownership info (BUG-5). ────────────────
    let session = rt.enter_sketch(sid).await.expect("re-enter_sketch");
    let circ_id = eid(CIRC).to_string();
    let arc_id = eid(ARC).to_string();
    let l0_id = eid(L0).to_string();

    let circ = entity_by_id(&session.entities, &circ_id).expect("circle on re-entry");
    assert_eq!(
        circ["centerRef"].as_str(),
        Some(eid(CCENTER).to_string().as_str()),
        "BUG-5: the re-entry circle carries its center point uuid (centerRef)"
    );
    let arc = entity_by_id(&session.entities, &arc_id).expect("arc on re-entry");
    assert_eq!(
        arc["centerRef"].as_str(),
        Some(eid(ACENTER).to_string().as_str()),
        "BUG-5: the re-entry arc carries its center point uuid (centerRef)"
    );
    let l0 = entity_by_id(&session.entities, &l0_id).expect("line on re-entry");
    assert_eq!(l0["p0Ref"].as_str(), Some(eid(P0).to_string().as_str()));
    assert_eq!(l0["p1Ref"].as_str(), Some(eid(P1).to_string().as_str()));
    // Every backend point IS present on the wire (as a Point entity) so hydration can
    // re-own them; the FRONTEND (`sketchWireMap.ts`) is what skips the owned ones.
    let pts = point_ids(&session.entities);
    assert!(
        pts.contains(&eid(CCENTER).to_string()) && pts.contains(&eid(P1).to_string()),
        "the center + endpoint points ride on the wire, keyed to their owners"
    );
    eprintln!("RE-ENTRY PASS: circle/arc carry centerRef, line carries p0Ref/p1Ref");

    // ── DRAG a line endpoint (P1) — it moves. ───────────────────────────────────
    // A wire line inlines p0Ref/p1Ref (not coords), so read the endpoint Point's `at`.
    let p1_start = xy(&entity_by_id(&session.entities, &eid(P1).to_string()).unwrap()["at"]);
    drag(&mut rt, sid, eid(P1), [60.0, 5.0]).await;
    let after_line = rt
        .enter_sketch(sid)
        .await
        .expect("re-enter after line drag");
    let p1_now = xy(&entity_by_id(&after_line.entities, &eid(P1).to_string()).unwrap()["at"]);
    let moved = ((p1_now[0] - p1_start[0]).powi(2) + (p1_now[1] - p1_start[1]).powi(2)).sqrt();
    assert!(
        moved > 1.0,
        "BUG-5: dragging the line endpoint P1 moved it ({p1_start:?} → {p1_now:?})"
    );
    eprintln!("LINE-DRAG PASS: P1 {p1_start:?} → {p1_now:?}");

    // ── DRAG the circle center (by the center Point uuid) — the CIRCLE relocates. ─
    let circ_center_before = xy(&entity_by_id(&after_line.entities, &circ_id).unwrap()["center"]);
    drag(&mut rt, sid, eid(CCENTER), [25.0, 35.0]).await;
    let after_circle = rt
        .enter_sketch(sid)
        .await
        .expect("re-enter after circle drag");
    let circ_center_now = xy(&entity_by_id(&after_circle.entities, &circ_id).unwrap()["center"]);
    assert!(
        (circ_center_now[0] - 25.0).abs() < 1.0 && (circ_center_now[1] - 35.0).abs() < 1.0,
        "BUG-5: dragging the circle center relocated the CIRCLE to ~(25,35), got {circ_center_now:?} (was {circ_center_before:?})"
    );
    // The moved center also rides back as centerRef (ownership survives the drag).
    assert_eq!(
        entity_by_id(&after_circle.entities, &circ_id).unwrap()["centerRef"].as_str(),
        Some(eid(CCENTER).to_string().as_str())
    );
    eprintln!("CIRCLE-DRAG PASS: center {circ_center_before:?} → {circ_center_now:?}");

    // ── DELETE the circle → re-enter → no orphan child point, dof correct. ──────
    // The marshaller removes the circle AND its center point (BUG-5). Cascade also
    // drops the Radius dim (it references the circle).
    rt.sketch_upsert(
        sid,
        vec![
            SketchEditOp::RemoveEntity { entity: eid(CIRC) },
            SketchEditOp::RemoveEntity {
                entity: eid(CCENTER),
            },
        ],
    )
    .await
    .expect("sketch_upsert (delete circle + center)");
    let after_delete = rt.enter_sketch(sid).await.expect("re-enter after delete");

    assert!(
        entity_by_id(&after_delete.entities, &circ_id).is_none(),
        "the circle is gone after delete"
    );
    let remaining_pts = point_ids(&after_delete.entities);
    assert!(
        !remaining_pts.contains(&eid(CCENTER).to_string()),
        "BUG-5: the circle's center point is NOT orphaned after delete (pts={remaining_pts:?})"
    );
    // dof correct: a from-scratch sketch WITHOUT the circle solves to the SAME dof
    // (an orphan center point would add 2 free dof — this proves there is none).
    let mut rt2 = runtime_over(&wm);
    let sid2 = SketchId(Uuid::from_u128(SID + 1));
    // The SAME sketch minus the circle (+ its center + the Radius dim).
    let no_circle = circle_less_sketch(sid2);
    rt2.apply(EditCommand::AddSketch {
        sketch: Sketch::on_world_plane(sid2, "NoCircle", WorldPlane::XY),
    })
    .expect("AddSketch (ref)");
    rt2.enter_sketch(sid2).await.expect("enter_sketch (ref)");
    let ref_solved = rt2
        .sketch_upsert(sid2, edit_ops(&no_circle))
        .await
        .expect("sketch_upsert (ref)");
    assert_eq!(
        after_delete.dof, ref_solved.dof,
        "BUG-5: post-delete dof ({}) equals a from-scratch circle-less sketch ({}) — no orphan dof",
        after_delete.dof, ref_solved.dof
    );
    eprintln!(
        "DELETE PASS: circle removed, no orphan center point, dof={} matches circle-less sketch",
        after_delete.dof
    );

    wm.shutdown().await;
}

/// The re-entry sketch minus the circle, its center point, and its Radius dim.
fn circle_less_sketch(new_sid: SketchId) -> Sketch {
    let mut sk = Sketch::on_world_plane(new_sid, "NoCircle", WorldPlane::XY);
    let pt = |sk: &mut Sketch, id: u128, x: f64, y: f64| {
        sk.add_entity(SketchEntity::point(
            eid(id),
            Vec2::new_unchecked(x, y),
            false,
            false,
        ))
        .unwrap();
    };
    pt(&mut sk, P0, 0.0, 0.0);
    pt(&mut sk, P1, 40.0, 0.0);
    pt(&mut sk, Q0, 0.0, 0.0);
    pt(&mut sk, Q1, 0.0, 40.0);
    pt(&mut sk, ACENTER, 70.0, 10.0);
    sk.add_entity(SketchEntity::line(eid(L0), eid(P0), eid(P1), false))
        .unwrap();
    sk.add_entity(SketchEntity::line(eid(L1), eid(Q0), eid(Q1), false))
        .unwrap();
    sk.add_entity(
        SketchEntity::arc(
            eid(ARC),
            eid(ACENTER),
            4.0,
            0.0,
            std::f64::consts::FRAC_PI_2,
            false,
        )
        .unwrap(),
    )
    .unwrap();
    sk.add_constraint(Constraint::Angle {
        id: cid(C_ANGLE),
        line1: eid(L0),
        line2: eid(L1),
        value: Scalar::new(std::f64::consts::FRAC_PI_2),
    })
    .unwrap();
    sk
}

// ─────────────────────────────────────────────────────────────────────────────
// BUG-2 — the Angle value crosses the wire in RADIANS, through SetDimension too.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sketch_reentry_angle_is_radians() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let sid = SketchId(Uuid::from_u128(SID + 2));
    let sketch = reentry_sketch(std::f64::consts::FRAC_PI_2); // 90° == π/2 on the wire
    rt.apply(EditCommand::AddSketch {
        sketch: Sketch::on_world_plane(sid, "Angle", WorldPlane::XY),
    })
    .expect("AddSketch");
    rt.enter_sketch(sid).await.expect("enter_sketch");
    rt.sketch_upsert(sid, edit_ops(&sketch))
        .await
        .expect("sketch_upsert");

    // Authored 90° → the wire Angle value is π/2 radians.
    let s1 = rt.enter_sketch(sid).await.expect("re-enter (π/2)");
    let a1 = constraint_by_type(&s1.constraints, "Angle").expect("Angle on wire");
    let v1 = a1["value"].as_f64().expect("Angle value is a number");
    assert!(
        (v1 - std::f64::consts::FRAC_PI_2).abs() < 1e-9,
        "BUG-2: authored 90° comes back as π/2 radians on the wire, got {v1}"
    );

    // SetDimension to 45° (== π/4 on the wire) → it comes back π/4.
    rt.sketch_upsert(
        sid,
        vec![SketchEditOp::SetDimension {
            constraint: cid(C_ANGLE),
            value: Scalar::new(std::f64::consts::FRAC_PI_4),
        }],
    )
    .await
    .expect("sketch_upsert (setDimension π/4)");
    let s2 = rt.enter_sketch(sid).await.expect("re-enter (π/4)");
    let v2 = constraint_by_type(&s2.constraints, "Angle").unwrap()["value"]
        .as_f64()
        .unwrap();
    assert!(
        (v2 - std::f64::consts::FRAC_PI_4).abs() < 1e-9,
        "BUG-2: SetDimension to 45° (π/4) comes back π/4 radians, got {v2}"
    );
    eprintln!("ANGLE PASS: wire carries radians (π/2 → SetDimension → π/4)");

    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// W3 — an ELLIPSE survives a full re-entry round trip.
//
// Same BUG-5 ownership contract as circle/arc: the re-entry wire carries the
// ellipse's `centerRef` (its backend center-point uuid) alongside the inlined
// `center`, so hydration re-owns the child Point instead of leaking a stray one,
// and the center stays draggable through the gesture verbs. The three shape
// scalars (`majorR`/`minorR`/`rotation`) ride back verbatim — an ellipse is not
// registered with PlaneGCS, so nothing may silently rewrite them.
// ─────────────────────────────────────────────────────────────────────────────

const ESID: u128 = 0x5e50;
const ECENTER: u128 = 0x140; // ellipse center (12,8) ← dragged
const ELL: u128 = 0x230;

fn ellipse_reentry_sketch(sid: SketchId) -> Sketch {
    let mut sk = Sketch::on_world_plane(sid, "EllipseReEntry", WorldPlane::XY);
    sk.add_entity(SketchEntity::point(
        eid(ECENTER),
        Vec2::new_unchecked(12.0, 8.0),
        false,
        false,
    ))
    .unwrap();
    sk.add_entity(
        SketchEntity::ellipse(eid(ELL), eid(ECENTER), 6.0, 3.0, 0.25, false).expect("ellipse"),
    )
    .unwrap();
    sk
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ellipse_reentry_carries_center_ref_and_drags() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let sid = SketchId(Uuid::from_u128(ESID));
    rt.apply(EditCommand::AddSketch {
        sketch: Sketch::on_world_plane(sid, "EllipseReEntry", WorldPlane::XY),
    })
    .expect("AddSketch");
    rt.enter_sketch(sid).await.expect("enter_sketch");
    rt.sketch_upsert(sid, edit_ops(&ellipse_reentry_sketch(sid)))
        .await
        .expect("sketch_upsert (build) — an ellipse now survives wire translation");

    // ── EXIT + RE-ENTER: ownership + shape scalars ride back. ───────────────────
    let session = rt.enter_sketch(sid).await.expect("re-enter_sketch");
    let ell_id = eid(ELL).to_string();
    let ell = entity_by_id(&session.entities, &ell_id).expect("ellipse on re-entry");
    assert_eq!(ell["type"], "Ellipse");
    assert_eq!(
        ell["centerRef"].as_str(),
        Some(eid(ECENTER).to_string().as_str()),
        "BUG-5 parity: the re-entry ellipse carries its center point uuid"
    );
    assert_eq!(xy(&ell["center"]), [12.0, 8.0]);
    assert_eq!(ell["majorR"].as_f64(), Some(6.0));
    assert_eq!(ell["minorR"].as_f64(), Some(3.0));
    assert_eq!(
        ell["rotation"].as_f64(),
        Some(0.25),
        "rotation crosses the wire in RADIANS, unrewritten"
    );
    assert!(
        point_ids(&session.entities).contains(&eid(ECENTER).to_string()),
        "the center Point rides on the wire so hydration can re-own it"
    );

    // ── DRAG the center by its Point uuid — the ELLIPSE relocates. ──────────────
    drag(&mut rt, sid, eid(ECENTER), [30.0, 22.0]).await;
    let after = rt.enter_sketch(sid).await.expect("re-enter after drag");
    let moved = entity_by_id(&after.entities, &ell_id).expect("ellipse after drag");
    let now = xy(&moved["center"]);
    assert!(
        (now[0] - 30.0).abs() < 1.0 && (now[1] - 22.0).abs() < 1.0,
        "dragging the center relocated the ELLIPSE to ~(30,22), got {now:?}"
    );
    assert_eq!(
        moved["centerRef"].as_str(),
        Some(eid(ECENTER).to_string().as_str()),
        "ownership survives the drag"
    );
    // The drag must not perturb the shape — the gesture only moves the center.
    assert_eq!(moved["majorR"].as_f64(), Some(6.0));
    assert_eq!(moved["minorR"].as_f64(), Some(3.0));
    assert_eq!(moved["rotation"].as_f64(), Some(0.25));

    eprintln!(
        "ELLIPSE RE-ENTRY PASS: centerRef preserved, center dragged to {now:?}, shape intact"
    );
    wm.shutdown().await;
}
