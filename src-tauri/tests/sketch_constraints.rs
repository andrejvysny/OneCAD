//! Sketch-constraint **serde proof** (S4b) against the REAL C++ OCCT worker.
//!
//! The five user-applicable geometric kinds the frontend marshaller
//! (`sketchWireMap.ts toWireConstraint`) newly emits — Fixed / OnCurve / Tangent /
//! Concentric / Symmetric — must survive the full path: frontend wire JSON →
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

    wm.shutdown().await;
}
