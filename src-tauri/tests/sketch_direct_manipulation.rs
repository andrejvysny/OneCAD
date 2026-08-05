//! SP-2 — direct manipulation (SCHEMA §7.4 gesture target KINDS + the `curves`
//! channel) end-to-end against the REAL C++ PlaneGCS worker.
//!
//! Same harness as `sketch_reentry.rs` (real worker via `ONECAD_WORKER_PATH`, else
//! the dev-tree fallback; `ONECAD_REQUIRE_WORKER=1` hard-fails a missing binary).
//! The worker's own suite proves the solver behaviour one layer down; what is
//! under test HERE is that the whole Rust path carries it:
//!
//! * a non-`point` kind reaches the worker as `drag.{kind, entity, role, grab}`,
//! * the answer comes back through BOTH channels, and
//! * `EndGesture` lands it in the **document** as ONE undo command.
//!
//! The `curves` channel is the load-bearing part. `positions` is point-only, and a
//! Rust `Arc` is `{center: EntityId, radius, start_angle, end_angle}` with NO
//! endpoint entities — the worker's `<arcId>.start`/`.end` handles are synthesized
//! children whose keys are not `EntityId`s at all. So before `curves` existed, a
//! reshape was structurally unrepresentable on this side: the worker's store moved
//! and the document did not, and the next `SketchUpsert` silently reverted it.
//! `arc_end_drag_reshapes_sweep` is the regression that pins it.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use onecad_core::document::variables::Scalar;
use onecad_core::edit::{EditCommand, SketchEditOp};
use onecad_core::ids::{ConstraintId, EntityId, SketchId};
use onecad_core::math::Vec2;
use onecad_core::regen::GeometryEngine;
use onecad_core::sketch::{Constraint, Sketch, SketchEntity, WorldPlane};

use onecad_lib::document_runtime::DocumentRuntime;
use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::wire::{DragKind, GestureTarget};
use onecad_lib::worker::{resolve_worker_path, MeshProvider, SolverEngine, WorkerManager};

// ─────────────────────────────────────────────────────────────────────────────
// Harness (mirrors sketch_reentry.rs)
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

/// Builds `sketch` inside a fresh runtime and enters it (the pre-gesture state).
async fn enter_with(wm: &WorkerManager, sketch: Sketch) -> DocumentRuntime {
    let mut rt = runtime_over(wm);
    let sid = sketch.id;
    rt.apply(EditCommand::AddSketch {
        sketch: Sketch::on_world_plane(sid, "DirectManipulation", WorldPlane::XY),
    })
    .expect("AddSketch");
    rt.enter_sketch(sid).await.expect("enter_sketch");
    rt.sketch_upsert(sid, edit_ops(&sketch))
        .await
        .expect("sketch_upsert (build)");
    rt
}

// ── document readers ─────────────────────────────────────────────────────────
//
// `get_sketch` is a pure DOCUMENT read (it projects the stored `Sketch` through
// `wire::sketch_wire` with no worker round-trip), so everything asserted below is
// what the runtime COMMITTED — never what the worker happens to be holding.

fn doc_entity(rt: &DocumentRuntime, sid: SketchId, id: EntityId) -> serde_json::Value {
    let session = rt.get_sketch(sid).expect("get_sketch");
    session
        .entities
        .as_array()
        .expect("entities array")
        .iter()
        .find(|e| e["id"] == serde_json::Value::String(id.to_string()))
        .unwrap_or_else(|| panic!("entity {id} is in the document"))
        .clone()
}

fn num(v: &serde_json::Value) -> f64 {
    v.as_f64()
        .unwrap_or_else(|| panic!("{v:?} is not a number"))
}

fn doc_point(rt: &DocumentRuntime, sid: SketchId, id: EntityId) -> [f64; 2] {
    let e = doc_entity(rt, sid, id);
    assert_eq!(e["type"], "Point");
    [num(&e["at"][0]), num(&e["at"][1])]
}

fn doc_circle_radius(rt: &DocumentRuntime, sid: SketchId, id: EntityId) -> f64 {
    let e = doc_entity(rt, sid, id);
    assert_eq!(e["type"], "Circle");
    num(&e["radius"])
}

/// `(radius, start_angle, end_angle)` of a document arc (angles in RADIANS).
fn doc_arc(rt: &DocumentRuntime, sid: SketchId, id: EntityId) -> (f64, f64, f64) {
    let e = doc_entity(rt, sid, id);
    assert_eq!(e["type"], "Arc");
    (
        num(&e["radius"]),
        num(&e["startAngle"]),
        num(&e["endAngle"]),
    )
}

/// Drives a whole gesture: begin(target) → one drag → pointer-up commit.
async fn drag_with(
    rt: &mut DocumentRuntime,
    sid: SketchId,
    drag_point: EntityId,
    target: GestureTarget,
    to: [f64; 2],
) -> onecad_lib::dto::SketchUpsertDto {
    rt.begin_gesture(sid, drag_point, target)
        .await
        .expect("begin_gesture");
    rt.solve_drag(to).await.expect("solve_drag");
    rt.end_gesture(Some(to)).await.expect("end_gesture")
}

// ─────────────────────────────────────────────────────────────────────────────
// (1) entityBody — the whole entity translates, and its coupling comes with it.
// ─────────────────────────────────────────────────────────────────────────────

const RSID: u128 = 0x5f00;
const R0: u128 = 0x100; // (0,0)   bottom-left
const R1: u128 = 0x101; // (10,0)  bottom-right
const R2: u128 = 0x102; // (10,10) top-right
const R3: u128 = 0x103; // (0,10)  top-left
const RL0: u128 = 0x200; // bottom (dragged)
const RL1: u128 = 0x201; // right
const RL2: u128 = 0x202; // top
const RL3: u128 = 0x203; // left

/// A WELDED 10×10 rectangle: adjacent lines SHARE their corner point entity, and
/// the four H/V constraints are what carry the far corners along on a body drag.
fn welded_rect(sid: SketchId) -> Sketch {
    let mut sk = Sketch::on_world_plane(sid, "Rect", WorldPlane::XY);
    point(&mut sk, R0, 0.0, 0.0);
    point(&mut sk, R1, 10.0, 0.0);
    point(&mut sk, R2, 10.0, 10.0);
    point(&mut sk, R3, 0.0, 10.0);
    for (l, a, b) in [(RL0, R0, R1), (RL1, R1, R2), (RL2, R2, R3), (RL3, R3, R0)] {
        sk.add_entity(SketchEntity::line(eid(l), eid(a), eid(b), false))
            .unwrap();
    }
    for (c, l) in [(0x300, RL0), (0x301, RL2)] {
        sk.add_constraint(Constraint::Horizontal {
            id: cid(c),
            line: eid(l),
        })
        .unwrap();
    }
    for (c, l) in [(0x302, RL1), (0x303, RL3)] {
        sk.add_constraint(Constraint::Vertical {
            id: cid(c),
            line: eid(l),
        })
        .unwrap();
    }
    sk
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn entity_body_drag_translates_a_constrained_rect_line() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let sid = SketchId(Uuid::from_u128(RSID));
    let mut rt = enter_with(&wm, welded_rect(sid)).await;
    let undo_before = rt.undo_depth();

    // Grab the bottom edge at its midpoint and slide it +4 in x. `grab` is what
    // makes this a BODY drag and not a teleport: the delta is `target − grab`.
    let solved = drag_with(
        &mut rt,
        sid,
        eid(R0),
        GestureTarget {
            kind: DragKind::EntityBody,
            entity: eid(RL0),
            role: None,
            grab: Some([5.0, 0.0]),
        },
        [9.0, 0.0],
    )
    .await;

    let (p0, p1, p2, p3) = (
        doc_point(&rt, sid, eid(R0)),
        doc_point(&rt, sid, eid(R1)),
        doc_point(&rt, sid, eid(R2)),
        doc_point(&rt, sid, eid(R3)),
    );
    eprintln!(
        "ENTITY-BODY: p0={p0:?} p1={p1:?} p2={p2:?} p3={p3:?} status={:?}",
        solved.status
    );
    assert!(
        (p0[0] - 4.0).abs() < 1e-3 && (p0[1] - 0.0).abs() < 1e-3,
        "the dragged edge's start moved by exactly the grab delta, got {p0:?}"
    );
    assert!(
        (p1[0] - 14.0).abs() < 1e-3 && (p1[1] - 0.0).abs() < 1e-3,
        "the dragged edge's end moved by exactly the grab delta, got {p1:?}"
    );
    // The COUPLING is the point of the kind: entityBody pins nothing, so the
    // verticals drag the far corners along instead of fighting the drive.
    assert!(
        (p3[0] - p0[0]).abs() < 1e-3,
        "the left Vertical carried the top-left corner ({p3:?} vs {p0:?})"
    );
    assert!(
        (p2[0] - p1[0]).abs() < 1e-3,
        "the right Vertical carried the top-right corner ({p2:?} vs {p1:?})"
    );
    assert!(
        (p0[1] - p1[1]).abs() < 1e-6 && (p2[1] - p3[1]).abs() < 1e-6,
        "both Horizontals still hold"
    );
    assert!(
        (p2[1] - 10.0).abs() < 1e-3 && (p3[1] - 10.0).abs() < 1e-3,
        "the untouched top edge stayed put ({p2:?} {p3:?})"
    );
    // A pure translation curves nothing.
    assert!(
        solved.solved_curves.is_empty(),
        "a line body drag reports no curve members, got {:?}",
        solved.solved_curves
    );

    // ONE undo command for the whole gesture, and it reverts everything.
    assert_eq!(
        rt.undo_depth(),
        undo_before + 1,
        "the whole body drag is exactly ONE undo step"
    );
    assert!(rt.undo().is_some());
    assert_eq!(doc_point(&rt, sid, eid(R0)), [0.0, 0.0]);
    assert_eq!(doc_point(&rt, sid, eid(R1)), [10.0, 0.0]);

    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// (2) arcEnd — the reshape reaches the DOCUMENT. Structurally impossible before
//     the `curves` channel: a Rust Arc has no endpoint entities to move.
// ─────────────────────────────────────────────────────────────────────────────

const ASID: u128 = 0x5f10;
const ACENTER: u128 = 0x110;
const ARC: u128 = 0x210;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn arc_end_drag_reshapes_sweep() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let sid = SketchId(Uuid::from_u128(ASID));
    let mut sk = Sketch::on_world_plane(sid, "Arc", WorldPlane::XY);
    point(&mut sk, ACENTER, 0.0, 0.0);
    sk.add_entity(
        SketchEntity::arc(
            eid(ARC),
            eid(ACENTER),
            20.0,
            0.0,
            std::f64::consts::FRAC_PI_2,
            false,
        )
        .unwrap(),
    )
    .unwrap();
    let mut rt = enter_with(&wm, sk).await;
    assert_eq!(
        doc_arc(&rt, sid, eid(ARC)),
        (20.0, 0.0, std::f64::consts::FRAC_PI_2)
    );

    // Drag the START endpoint to (20,−20): the pin set holds the CENTER but frees
    // both endpoints, so the radius grows to |cursor| and the start angle sweeps
    // to the cursor's bearing (−45°). The sweep stays healthy (135°), well clear
    // of the MIN_ARC_SWEEP refusal.
    let solved = drag_with(
        &mut rt,
        sid,
        eid(ACENTER),
        GestureTarget {
            kind: DragKind::ArcEnd,
            entity: eid(ARC),
            role: Some("start"),
            grab: None, // §7.4: arcEnd IGNORES grab (the handle teleports)
        },
        [20.0, -20.0],
    )
    .await;

    let (radius, start_angle, end_angle) = doc_arc(&rt, sid, eid(ARC));
    let center = doc_point(&rt, sid, eid(ACENTER));
    eprintln!(
        "ARC-END: radius {radius:.6} start {start_angle:.6} end {end_angle:.6} center {center:?} \
         curves={:?}",
        solved.solved_curves
    );
    assert!(
        solved.solved_curves.contains_key(&eid(ARC).to_string()),
        "the reshape rides the `curves` channel (positions cannot carry it)"
    );
    assert!(
        (radius - 20.0_f64.hypot(20.0)).abs() < 1e-3,
        "the DOCUMENT arc's radius grew to the cursor distance, got {radius}"
    );
    assert!(
        (start_angle - (-std::f64::consts::FRAC_PI_4)).abs() < 1e-3,
        "the DOCUMENT arc's start angle swept to the cursor bearing (−45°), got {start_angle}"
    );
    assert!(
        (end_angle - std::f64::consts::FRAC_PI_2).abs() < 0.05,
        "the sibling slid ALONG the reshaped circle — the drag reshaped, it did not \
         rotate the arc (end angle {end_angle})"
    );
    assert!(
        center[0].abs() < 1e-6 && center[1].abs() < 1e-6,
        "the pinned center did NOT translate, got {center:?}"
    );

    // The reshape survives a re-solve: the worker store and the document now
    // agree, so the next upsert cannot silently revert the radius (the latent bug
    // this channel closes).
    rt.sketch_upsert(sid, vec![])
        .await
        .expect("sketch_upsert (re-solve)");
    let (radius_after, start_after, _) = doc_arc(&rt, sid, eid(ARC));
    assert!(
        (radius_after - radius).abs() < 1e-9 && (start_after - start_angle).abs() < 1e-9,
        "a re-solve does NOT revert the reshape ({radius_after} / {start_after})"
    );

    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// (3)+(4) radius — a committed dimension outranks the drive; a free circle tracks.
// ─────────────────────────────────────────────────────────────────────────────

const CSID: u128 = 0x5f20;
const CCENTER: u128 = 0x120;
const CIRC: u128 = 0x220;
const C_RADIUS: u128 = 0x320;

/// A circle at the origin, optionally carrying a `Radius` dimension.
fn circle_sketch(sid: SketchId, dimensioned: bool) -> Sketch {
    let mut sk = Sketch::on_world_plane(sid, "Circle", WorldPlane::XY);
    point(&mut sk, CCENTER, 0.0, 0.0);
    sk.add_entity(SketchEntity::circle(eid(CIRC), eid(CCENTER), 5.0, false).unwrap())
        .unwrap();
    if dimensioned {
        sk.add_constraint(Constraint::Radius {
            id: cid(C_RADIUS),
            entity: eid(CIRC),
            value: Scalar::new(5.0),
        })
        .unwrap();
    }
    sk
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn radius_drag_on_dimensioned_circle_is_held() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let sid = SketchId(Uuid::from_u128(CSID));
    let mut rt = enter_with(&wm, circle_sketch(sid, true)).await;

    // Grab the rim and pull it to 12: the drive is an advisory tag(−1), the
    // dimension is a committed constraint, so the dimension wins.
    let solved = drag_with(
        &mut rt,
        sid,
        eid(CCENTER),
        GestureTarget {
            kind: DragKind::Radius,
            entity: eid(CIRC),
            role: None,
            grab: Some([5.0, 0.0]),
        },
        [12.0, 0.0],
    )
    .await;
    eprintln!(
        "RADIUS-HELD: status={:?} curves={:?}",
        solved.status, solved.solved_curves
    );
    assert!(
        (doc_circle_radius(&rt, sid, eid(CIRC)) - 5.0).abs() < 1e-6,
        "the dimensioned radius is exactly what it was"
    );
    assert!(
        !solved.solved_curves.contains_key(&eid(CIRC).to_string()),
        "nothing changed, so nothing is reported: {:?}",
        solved.solved_curves
    );
    // Being held is NOT an error — the gesture completed and committed normally.
    assert!(
        (doc_point(&rt, sid, eid(CCENTER))[0]).abs() < 1e-6,
        "and the center never slid (the radius kind pins it)"
    );

    wm.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn radius_drag_on_free_circle_commits() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let sid = SketchId(Uuid::from_u128(CSID + 1));
    let mut rt = enter_with(&wm, circle_sketch(sid, false)).await;
    let undo_before = rt.undo_depth();

    // Grab exactly ON the rim ⇒ zero offset ⇒ the radius IS the cursor distance.
    let solved = drag_with(
        &mut rt,
        sid,
        eid(CCENTER),
        GestureTarget {
            kind: DragKind::Radius,
            entity: eid(CIRC),
            role: None,
            grab: Some([5.0, 0.0]),
        },
        [12.0, 0.0],
    )
    .await;
    let radius = doc_circle_radius(&rt, sid, eid(CIRC));
    eprintln!(
        "RADIUS-FREE: radius {radius:.6} curves={:?}",
        solved.solved_curves
    );
    assert!(
        (radius - 12.0).abs() < 1e-6,
        "the DOCUMENT circle tracked the cursor distance, got {radius}"
    );
    assert!(
        solved.solved_positions.is_empty(),
        "a radius drag moves NO point — which is exactly why `curves` had to exist: {:?}",
        solved.solved_positions
    );
    assert_eq!(
        rt.undo_depth(),
        undo_before + 1,
        "the radius drag is ONE undo step"
    );
    assert!(rt.undo().is_some());
    assert!(
        (doc_circle_radius(&rt, sid, eid(CIRC)) - 5.0).abs() < 1e-9,
        "undo reverts the radius too (it rode the same command)"
    );

    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// (5) BACK-COMPAT — a kind-less gesture is still exactly the old point drag.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn legacy_begin_gesture_without_kind_still_drags_a_point() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let sid = SketchId(Uuid::from_u128(RSID + 1));
    let mut rt = enter_with(&wm, welded_rect(sid)).await;

    // `GestureTarget::point` is what an api call with NO `target` resolves to, and
    // it emits the pre-SP-2 request byte for byte (drag.pointId + bare pointId).
    let solved = drag_with(
        &mut rt,
        sid,
        eid(R1),
        GestureTarget::point(eid(R1)),
        [14.0, 6.0],
    )
    .await;
    let p1 = doc_point(&rt, sid, eid(R1));
    eprintln!("LEGACY-POINT: p1={p1:?} status={:?}", solved.status);
    assert!(
        (p1[0] - 14.0).abs() < 1e-3 && (p1[1] - 6.0).abs() < 1e-3,
        "the point still teleports to the cursor, got {p1:?}"
    );
    // The far corner did NOT come along — a point drag pins every other point,
    // which is precisely what entityBody had to stop doing.
    let p3 = doc_point(&rt, sid, eid(R3));
    assert!(
        (p3[0] - 0.0).abs() < 1e-3 && (p3[1] - 10.0).abs() < 1e-3,
        "a point drag still pins the other corners, got {p3:?}"
    );
    assert!(
        solved.solved_curves.is_empty(),
        "no curve in the sketch ⇒ an empty curves map, never a missing key"
    );

    wm.shutdown().await;
}
