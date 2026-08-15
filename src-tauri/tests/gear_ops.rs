//! `Gear` (SCHEMA §7.3, Gear Generator G1) against the REAL C++ OCCT worker.
//!
//! The generator's tooth mathematics is proven exactly in the worker ctest
//! (`worker/tests/test_gear_tool.cpp` for the solid, `test_gear_op.cpp` for the
//! operation's D1 lineage, placement fence and refusal matrix). What was NEVER
//! covered from the Rust side is the wire path itself: typed `GearParams` →
//! `Operation::Known` serde → OCW1 → worker dispatch → OCCT → body adoption →
//! MESH1. A rename or a dropped field on that path round-trips happily in the
//! core's own unit tests and still produces no body in the app.
//!
//! Two proofs, deliberately thin:
//! * `gear_frame_placement_publishes_one_solid` — a frame-placed involute gear
//!   (teeth 20, module 2, height 5) publishes exactly one worker-minted body whose
//!   mesh sits on the frame and spans the addendum circle. Pitch diameter is
//!   `teeth × module = 40`; the addendum adds one module of radius, so the tip
//!   circle is `40 + 2×2 = 44` across.
//! * `gear_helical_request_is_refused_before_the_worker` — `helixAngleDeg != 0` is
//!   forward-compatibility payload, not a supported build. It must be refused at
//!   the authoring boundary, so no half-supported gear can enter history.
//!
//! Gated on `ONECAD_WORKER_PATH` (else the dev-tree fallback); a missing binary
//! skips cleanly unless `ONECAD_REQUIRE_WORKER=1` (CI hard-fails).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use onecad_core::document::record::{
    GearParams, GearPlacement, GearRecipe, InvoluteExternalParams, KnownOperation, Operation,
    OperationRecord,
};
use onecad_core::document::variables::Scalar;
use onecad_core::edit::EditCommand;
use onecad_core::ids::{BodyId, RecordId};
use onecad_core::math::Vec3;
use onecad_core::regen::{CancelToken, GeometryEngine, Lod, Outcome, RegenRequest};

use onecad_lib::document_runtime::DocumentRuntime;
use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::{resolve_worker_path, MeshProvider, SolverEngine, WorkerManager};

// ─────────────────────────────────────────────────────────────────────────────
// Harness (mirrors datum_planes.rs / revolve_ops.rs)
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

// ── fixture ──────────────────────────────────────────────────────────────────

const TEETH: u32 = 20;
const MODULE: f64 = 2.0;
const HEIGHT: f64 = 5.0;
/// Pitch diameter `teeth × module`, plus one module of addendum on each side.
const TIP_DIAMETER: f64 = TEETH as f64 * MODULE + 2.0 * MODULE;

fn involute_block() -> InvoluteExternalParams {
    InvoluteExternalParams {
        teeth: TEETH,
        module: Scalar::new(MODULE),
        height: Scalar::new(HEIGHT),
        pressure_angle_deg: Scalar::new(20.0),
        shift: 0.0,
        helix_angle_deg: 0.0,
        double_helix: false,
        properties_from_tool: false,
        undercut: false,
        backlash: 0.0,
        clearance: 0.25,
        head: 0.0,
        sample_count: 20,
        axle_hole: false,
        axle_hole_diameter: None,
        offset_hole: false,
        offset_hole_diameter: None,
        offset_hole_offset: None,
    }
}

/// A gear on an explicit frozen frame at the world origin — the datum/world
/// placement path, which needs no host body and so isolates the Gear wire itself.
fn gear_op(rec: RecordId, involute: InvoluteExternalParams) -> OperationRecord {
    OperationRecord::new(
        rec,
        0,
        "Gear",
        Operation::Known(KnownOperation::Gear(GearParams {
            recipe: GearRecipe::InvoluteExternal,
            placement: GearPlacement {
                face: None,
                frame: Some(onecad_core::document::record::GearFrame {
                    origin: Vec3::new_unchecked(0.0, 0.0, 0.0),
                    axis: Vec3::new_unchecked(0.0, 0.0, 1.0),
                    x_dir: Vec3::new_unchecked(1.0, 0.0, 0.0),
                }),
                point: Vec3::new_unchecked(0.0, 0.0, 0.0),
            },
            involute_external: Some(involute),
            extra: Default::default(),
        })),
    )
}

// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gear_frame_placement_publishes_one_solid() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let rec = RecordId(Uuid::from_u128(0x6EA1));
    rt.apply(EditCommand::AddOperation {
        record: gear_op(rec, involute_block()),
        at_cursor: true,
    })
    .expect("AddOperation Gear");

    let report = rt
        .run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    let Outcome::Published(snapshot) = &report.outcome else {
        panic!("Gear did not publish: {:?}", report.outcome);
    };
    assert_eq!(
        snapshot.bodies.len(),
        1,
        "a gear is exactly one connected solid"
    );

    // The body id is worker-minted from the Gear record (decision D1, `body_<opId>`).
    let mesh = rt
        .get_mesh(BodyId(rec.0), Lod::Coarse, None)
        .await
        .expect("fetch the gear's mesh");
    let view = onecad_protocol::mesh::validate_mesh_blob(&mesh).expect("MESH1 validates");

    // The MESH1 bbox is a slightly padded tessellation bound, not a BRep extent, so
    // these are placement assertions with a faceting allowance — exact dimensions
    // belong to the worker ctest. The allowance is far below the 5 mm height, so a
    // dropped frame (or a gear built on the wrong plane) still fails loudly.
    const PAD: f64 = 0.05;
    let zmin = f64::from(view.bbox_min[2]);
    let zmax = f64::from(view.bbox_max[2]);
    assert!(
        zmin.abs() < PAD && (zmax - HEIGHT).abs() < PAD,
        "the gear must sit on its frame, z ∈ [0, {HEIGHT}]; got [{zmin}, {zmax}]"
    );

    // The tooth tips are sampled points on the profile, so a faceted bbox reaches
    // the tip circle from below and never meaningfully past it.
    let dx = f64::from(view.bbox_max[0]) - f64::from(view.bbox_min[0]);
    let dy = f64::from(view.bbox_max[1]) - f64::from(view.bbox_min[1]);
    eprintln!("gear tip span = {dx} × {dy} (tip diameter {TIP_DIAMETER})");
    for (span, axis) in [(dx, "x"), (dy, "y")] {
        assert!(
            span > TIP_DIAMETER - 2.0 && span <= TIP_DIAMETER + PAD,
            "the {axis} span must reach the tip circle {TIP_DIAMETER} without exceeding \
             it; got {span}"
        );
    }

    wm.shutdown().await;
}

/// `helixAngleDeg` exists so the payload will not change shape when the Frenet
/// sweep lands. Until then a non-zero value is refused at the authoring boundary,
/// never quietly built as a spur gear.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gear_helical_request_is_refused_before_the_worker() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let mut helical = involute_block();
    helical.helix_angle_deg = 15.0;
    let err = rt
        .apply(EditCommand::AddOperation {
            record: gear_op(RecordId(Uuid::from_u128(0x6EA2)), helical),
            at_cursor: true,
        })
        .expect_err("a helical gear must not enter the timeline");
    assert!(
        err.to_string().contains("helixAngleDeg"),
        "the refusal must name the unsupported parameter: {err}"
    );
    assert!(
        rt.projection().features.is_empty(),
        "nothing partially applied"
    );

    wm.shutdown().await;
}
