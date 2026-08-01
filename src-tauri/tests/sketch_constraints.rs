//! Sketch-constraint **serde proof** (S4b) against the REAL C++ OCCT worker.
//!
//! The user-applicable geometric kinds the frontend marshaller
//! (`sketchWireMap.ts toWireConstraint`) newly emits — Fixed / OnCurve / Tangent /
//! Concentric / Symmetric / Equal / Midpoint — must survive the full path: frontend wire JSON →
//! `onecad_core::Constraint` (serde, `constraint.rs`) → worker wire (`worker/wire.rs
//! wire_constraint`) → the C++ `WireSketch`/`PlaneGCS` solver. This test drives the
//! same single-writer [`DocumentRuntime`] as `sketch_reentry.rs` and asserts, for
//! each kind, that the worker ACCEPTS the constraint (never OverConstrained /
//! Conflicting) and that it actually constrains geometry (the solved DOF strictly
//! drops). Landing this proves the wire shapes before the UI relies on them.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use onecad_core::edit::{EditCommand, SketchEditOp};
use onecad_core::ids::{ConstraintId, EntityId, SketchId};
use onecad_core::math::Vec2;
use onecad_core::regen::GeometryEngine;
use onecad_core::sketch::{Constraint, CurvePosition, Sketch, SketchEntity, WorldPlane};

use onecad_lib::document_runtime::DocumentRuntime;
use onecad_lib::dto::SketchSolveStatus;
use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::{resolve_worker_path, MeshProvider, SolverEngine, WorkerManager};

// ── Harness (mirrors sketch_reentry.rs / topology_rebind.rs) ──────────────────

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
fn line(id: u128, s: u128, e: u128) -> SketchEntity {
    SketchEntity::line(eid(id), eid(s), eid(e), false)
}
fn circle(id: u128, c: u128, r: f64) -> SketchEntity {
    SketchEntity::circle(eid(id), eid(c), r, false).expect("finite radius")
}

/// Add + enter a fresh sketch, upsert `geometry` (points first, then curves) as the
/// baseline, then upsert the single `constraint`. Assert the worker accepts it and
/// the solved DOF strictly drops. `base` namespaces the entity ids per case.
async fn assert_constrains(
    wm: &WorkerManager,
    sid_n: u128,
    geometry: Vec<SketchEntity>,
    constraint: Constraint,
    label: &str,
) {
    let mut rt = runtime_over(wm);
    let sid = SketchId(Uuid::from_u128(sid_n));
    rt.apply(EditCommand::AddSketch {
        sketch: Sketch::on_world_plane(sid, label, WorldPlane::XY),
    })
    .expect("AddSketch");
    rt.enter_sketch(sid).await.expect("enter_sketch");

    let before = rt
        .sketch_upsert(
            sid,
            geometry
                .into_iter()
                .map(|entity| SketchEditOp::AddEntity { entity })
                .collect(),
        )
        .await
        .expect("sketch_upsert (geometry)");

    let after = rt
        .sketch_upsert(sid, vec![SketchEditOp::AddConstraint { constraint }])
        .await
        .expect("sketch_upsert (constraint)");

    eprintln!(
        "{label}: dof {} -> {} (status {:?})",
        before.dof, after.dof, after.status
    );
    assert!(
        !matches!(
            after.status,
            SketchSolveStatus::OverConstrained | SketchSolveStatus::Conflicting
        ),
        "{label}: the worker must ACCEPT the constraint (got status {:?})",
        after.status
    );
    assert!(
        after.dof < before.dof,
        "{label}: the constraint must reduce DOF ({} -> {})",
        before.dof,
        after.dof
    );
}

// ── The gate ──────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn user_constraints_round_trip_through_real_worker() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;

    // Fixed — pin a free point (removes 2 DOF).
    assert_constrains(
        &wm,
        0x5c00,
        vec![point(0x01, 3.0, 4.0)],
        Constraint::Fixed {
            id: cid(0x90),
            point: eid(0x01),
            at: Vec2::new_unchecked(3.0, 4.0),
        },
        "Fixed",
    )
    .await;

    // Concentric — two circles with DIFFERENT centers (removes 2 DOF).
    assert_constrains(
        &wm,
        0x5c01,
        vec![
            point(0x01, 0.0, 0.0),
            point(0x02, 20.0, 0.0),
            circle(0x10, 0x01, 5.0),
            circle(0x11, 0x02, 3.0),
        ],
        Constraint::Concentric {
            id: cid(0x90),
            entity1: eid(0x10),
            entity2: eid(0x11),
        },
        "Concentric",
    )
    .await;

    // Tangent — a line + a circle it does NOT currently touch (removes 1 DOF).
    assert_constrains(
        &wm,
        0x5c02,
        vec![
            point(0x01, 10.0, -10.0),
            point(0x02, 10.0, 10.0),
            point(0x03, 0.0, 0.0),
            line(0x10, 0x01, 0x02),
            circle(0x11, 0x03, 5.0),
        ],
        Constraint::Tangent {
            id: cid(0x90),
            entity1: eid(0x10),
            entity2: eid(0x11),
        },
        "Tangent",
    )
    .await;

    // Symmetric — two (non-symmetric) points about an axis line (removes 2 DOF).
    assert_constrains(
        &wm,
        0x5c03,
        vec![
            point(0x01, -5.0, 0.0),
            point(0x02, 4.0, 1.0),
            point(0x03, 0.0, -10.0),
            point(0x04, 0.0, 10.0),
            line(0x10, 0x03, 0x04),
        ],
        Constraint::Symmetric {
            id: cid(0x90),
            point1: eid(0x01),
            point2: eid(0x02),
            axis: eid(0x10),
        },
        "Symmetric",
    )
    .await;

    // OnCurve — a point NOT on a line, pinned Arbitrary (removes 1 DOF).
    assert_constrains(
        &wm,
        0x5c04,
        vec![
            point(0x01, 5.0, 5.0),
            point(0x02, 0.0, 0.0),
            point(0x03, 10.0, 0.0),
            line(0x10, 0x02, 0x03),
        ],
        Constraint::OnCurve {
            id: cid(0x90),
            point: eid(0x01),
            curve: eid(0x10),
            position: CurvePosition::Arbitrary,
        },
        "OnCurve",
    )
    .await;

    // Equal — two circles with DIFFERENT radii (removes 1 DOF).
    assert_constrains(
        &wm,
        0x5c05,
        vec![
            point(0x01, 0.0, 0.0),
            point(0x02, 20.0, 0.0),
            circle(0x10, 0x01, 5.0),
            circle(0x11, 0x02, 3.0),
        ],
        Constraint::Equal {
            id: cid(0x90),
            entity1: eid(0x10),
            entity2: eid(0x11),
        },
        "Equal",
    )
    .await;

    // Midpoint — a free point NOT at the line's midpoint (removes 2 DOF).
    assert_constrains(
        &wm,
        0x5c06,
        vec![
            point(0x01, 3.0, 7.0),
            point(0x02, 0.0, 0.0),
            point(0x03, 10.0, 0.0),
            line(0x10, 0x02, 0x03),
        ],
        Constraint::Midpoint {
            id: cid(0x90),
            point: eid(0x01),
            line: eid(0x10),
        },
        "Midpoint",
    )
    .await;

    wm.shutdown().await;
}

// ── W0b: arc-endpoint Coincidents survive the whole path ──────────────────────

/// The slot shape the frontend's `slotTool` authors, built through the typed core
/// API: two walls over shared `Point` entities and two cap arcs, welded
/// wall-endpoint ↔ arc-endpoint by four `Coincident`s carrying `Start`/`End`
/// positions.
///
/// Before W0b those four were UNEXPRESSIBLE — an arc's endpoints had no wire
/// handle, so the frontend marshaller dropped them silently and the worker would
/// have answered "Coincident: unresolved point handle". This measures the DOF the
/// FE-shaped document actually reaches and asserts the invariant that matters:
/// the four welds remove exactly 8 DOF, and the sketch stays UnderConstrained
/// (never Over/Conflicting — the arc rules are internal and must never surface).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn arc_endpoint_coincidents_weld_a_slot_through_real_worker() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let sid = SketchId(Uuid::from_u128(0x5c07));
    rt.apply(EditCommand::AddSketch {
        sketch: Sketch::on_world_plane(sid, "slot", WorldPlane::XY),
    })
    .expect("AddSketch");
    rt.enter_sketch(sid).await.expect("enter_sketch");

    // Centerline (-50,0) → (50,0), radius 20 — `slotDrafts(p0, p1, r)`.
    const R: f64 = 20.0;
    const X: f64 = 50.0;
    let (a0, a1, b0, b1) = (0x01, 0x02, 0x03, 0x04); // wall endpoints (+n, then −n)
    let (c_p0, c_p1) = (0x05, 0x06); // arc centers
    let (wall0, wall1, cap_p1, cap_p0) = (0x10, 0x11, 0x12, 0x13);

    let geometry = vec![
        point(a0, -X, R),
        point(a1, X, R),
        point(b0, -X, -R),
        point(b1, X, -R),
        point(c_p0, -X, 0.0),
        point(c_p1, X, 0.0),
        line(wall0, a0, a1),
        line(wall1, b0, b1),
        // cap@p1 sweeps CCW from b1 (−pi/2) to a1 (+pi/2); cap@p0 from a0 (+pi/2)
        // back to b0 (−pi/2) — the far side of each end, as `slotDrafts` draws it.
        SketchEntity::arc(
            eid(cap_p1),
            eid(c_p1),
            R,
            -std::f64::consts::FRAC_PI_2,
            std::f64::consts::FRAC_PI_2,
            false,
        )
        .expect("finite arc"),
        SketchEntity::arc(
            eid(cap_p0),
            eid(c_p0),
            R,
            std::f64::consts::FRAC_PI_2,
            -std::f64::consts::FRAC_PI_2,
            false,
        )
        .expect("finite arc"),
    ];

    let baseline = rt
        .sketch_upsert(
            sid,
            geometry
                .into_iter()
                .map(|entity| SketchEditOp::AddEntity { entity })
                .collect(),
        )
        .await
        .expect("sketch_upsert (slot geometry)");

    // The 4 welds, in `SLOT_CONSTRAINTS` order.
    let weld =
        |n: u128, point_id: u128, arc: u128, at: CurvePosition| SketchEditOp::AddConstraint {
            constraint: Constraint::Coincident {
                id: cid(n),
                point1: eid(point_id),
                point2: eid(arc),
                point1_position: CurvePosition::Arbitrary,
                point2_position: at,
            },
        };
    let welded = rt
        .sketch_upsert(
            sid,
            vec![
                weld(0xa0, a1, cap_p1, CurvePosition::End),
                weld(0xa1, b1, cap_p1, CurvePosition::Start),
                weld(0xa2, a0, cap_p0, CurvePosition::Start),
                weld(0xa3, b0, cap_p0, CurvePosition::End),
            ],
        )
        .await
        .expect("sketch_upsert (4 arc-endpoint welds)");

    // MEASURED, not predicted: the FE-shaped document's absolute DOF depends on
    // how many points the round trip mints (see the note below). What is pinned
    // is the DELTA the four welds are worth.
    eprintln!(
        "slot (FE-shaped): dof {} -> {} (status {:?} -> {:?})",
        baseline.dof, welded.dof, baseline.status, welded.status
    );
    assert_eq!(
        i64::from(baseline.dof) - i64::from(welded.dof),
        8,
        "the 4 arc-endpoint Coincidents must remove exactly 8 DOF ({} -> {})",
        baseline.dof,
        welded.dof
    );
    assert!(
        matches!(welded.status, SketchSolveStatus::UnderConstrained),
        "the welded slot must stay UnderConstrained (got {:?}) — the internal arc \
         rules are tag 0 and must never be reported as user redundancy",
        welded.status
    );

    wm.shutdown().await;
}
