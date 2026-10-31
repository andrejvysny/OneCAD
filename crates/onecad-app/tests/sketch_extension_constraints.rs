//! OnCurve on a line or arc EXTENSION solves through the REAL C++ worker.
//!
//! The sketch snap engine persists `OnCurve(curve)` for snaps on the continuation of a
//! line or arc. The solver must treat the curve as infinite (line) or a full circle
//! (arc), never clamp the point back to the drawn segment. Harness copied from
//! `sketch_constraints.rs`.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use onecad_core::edit::{EditCommand, SketchEditOp};
use onecad_core::ids::{ConstraintId, EntityId, SketchId};
use onecad_core::math::Vec2;
use onecad_core::regen::GeometryEngine;
use onecad_core::sketch::{Constraint, CurvePosition, Sketch, SketchEntity, WorldPlane};

use onecad_app::document_runtime::DocumentRuntime;
use onecad_app::dto::SketchSolveStatus;
use onecad_app::worker::manager::SupervisorConfig;
use onecad_app::worker::{resolve_worker_path, MeshProvider, SolverEngine, WorkerManager};

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
fn point(id: u128, x: f64, y: f64) -> SketchEntity {
    SketchEntity::point(eid(id), Vec2::new_unchecked(x, y), false, false)
}

fn cid(n: u128) -> ConstraintId {
    ConstraintId(Uuid::from_u128(n))
}
/// Pins a free point so the curve stays rigid and only the probe point can move.
fn fixed(id: u128, n: u128, x: f64, y: f64) -> Constraint {
    Constraint::Fixed {
        id: cid(n),
        point: eid(id),
        point_position: CurvePosition::Arbitrary,
        at: Vec2::new_unchecked(x, y),
    }
}

/// Solve `geometry` plus `OnCurve(point 0x01, curve 0x10)` and return the point.
async fn solve_on_curve(
    wm: &WorkerManager,
    sid_n: u128,
    geometry: Vec<SketchEntity>,
    pins: Vec<Constraint>,
    label: &str,
) -> (f64, f64) {
    let mut rt = runtime_over(wm);
    let sid = SketchId(Uuid::from_u128(sid_n));
    rt.apply(EditCommand::AddSketch {
        sketch: Sketch::on_world_plane(sid, label, WorldPlane::XY),
    })
    .expect("AddSketch");
    rt.enter_sketch(sid).await.expect("enter_sketch");
    let ops = geometry
        .into_iter()
        .map(|entity| SketchEditOp::AddEntity { entity })
        .collect();
    rt.sketch_upsert(sid, ops).await.expect("geometry upsert");
    let constraint = Constraint::OnCurve {
        id: cid(0x90),
        point: eid(0x01),
        point_position: CurvePosition::Arbitrary,
        curve: eid(0x10),
        position: CurvePosition::Arbitrary,
    };
    let pinned = pins.into_iter();
    let ops = pinned
        .chain([constraint])
        .map(|constraint| SketchEditOp::AddConstraint { constraint })
        .collect();
    let solved = rt.sketch_upsert(sid, ops).await.expect("OnCurve upsert");
    assert!(
        !matches!(
            solved.status,
            SketchSolveStatus::OverConstrained | SketchSolveStatus::Conflicting
        ),
        "{label}: worker must accept OnCurve (status {:?})",
        solved.status
    );
    let sketch = rt.sketch_snapshot(sid, label).expect("sketch snapshot");
    match sketch.get_entity(eid(0x01)) {
        Some(SketchEntity::Point { at, .. }) => (at.x, at.y),
        other => panic!("{label}: solved point missing: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn on_curve_follows_a_line_past_its_end_through_real_worker() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    // Segment (0,0)-(10,10); Q starts beyond its end: on the line, then slightly off it.
    for (n, start_y) in [15.0, 15.6].into_iter().enumerate() {
        let geometry = vec![
            point(0x01, 15.0, start_y),
            point(0x02, 0.0, 0.0),
            point(0x03, 10.0, 10.0),
            SketchEntity::line(eid(0x10), eid(0x02), eid(0x03), false),
        ];
        let pins = vec![fixed(0x02, 0xa1, 0.0, 0.0), fixed(0x03, 0xa2, 10.0, 10.0)];
        let sid = 0x5e00 + n as u128;
        let (x, y) = solve_on_curve(&wm, sid, geometry, pins, "OnCurve(line extension)").await;
        eprintln!("line extension from y={start_y}: Q = ({x}, {y})");
        assert!(
            (x - y).abs() < 1e-6,
            "Q left the line's extension: ({x}, {y})"
        );
        assert!(x > 10.0 + 1e-3, "Q was clamped to the segment: ({x}, {y})");
    }
}

/// Arc 0 -> pi/2 about the origin, rigid: centre, radius and both ends pinned.
fn rigid_arc(start_radius: f64) -> (Vec<SketchEntity>, Vec<Constraint>) {
    let angle = 135.0_f64.to_radians();
    let arc = SketchEntity::arc(
        eid(0x10),
        eid(0x02),
        10.0,
        0.0,
        std::f64::consts::FRAC_PI_2,
        false,
    )
    .expect("finite arc");
    let geometry = vec![
        point(0x01, start_radius * angle.cos(), start_radius * angle.sin()),
        point(0x02, 0.0, 0.0),
        point(0x03, 10.0, 0.0),
        point(0x04, 0.0, 10.0),
        arc,
    ];
    let weld = |anchor, position, n| Constraint::Coincident {
        id: cid(n),
        point1: eid(anchor),
        point1_position: CurvePosition::Arbitrary,
        point2: eid(0x10),
        point2_position: position,
    };
    let radius = Constraint::Radius {
        id: cid(0xa2),
        entity: eid(0x10),
        value: onecad_core::document::variables::Scalar::new(10.0),
    };
    let pins = vec![
        fixed(0x02, 0xa1, 0.0, 0.0),
        radius,
        fixed(0x03, 0xa3, 10.0, 0.0),
        fixed(0x04, 0xa4, 0.0, 10.0),
        weld(0x03, CurvePosition::Start, 0xa5),
        weld(0x04, CurvePosition::End, 0xa6),
    ];
    (geometry, pins)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn on_curve_follows_an_arc_past_its_sweep_through_real_worker() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    // Q at 135 degrees: exactly on the circle (must stay put), then outside it (the solver
    // is not minimum-norm, so it may slide along the circle; 0.5 degrees bounds that).
    for (n, (start_radius, angle_tolerance)) in [(10.0, 1e-6), (10.5, 0.5)].into_iter().enumerate()
    {
        let (geometry, pins) = rigid_arc(start_radius);
        let sid = 0x5e10 + n as u128;
        let (x, y) = solve_on_curve(&wm, sid, geometry, pins, "OnCurve(arc extension)").await;
        let (radius, degrees) = (x.hypot(y), y.atan2(x).to_degrees());
        eprintln!("arc extension from r={start_radius}: Q = ({x}, {y}) r={radius} angle={degrees}");
        assert!((radius - 10.0).abs() < 1e-6, "radius drifted: {radius}");
        assert!(
            (degrees - 135.0).abs() < angle_tolerance,
            "angle drifted: {degrees}"
        );
    }
}
