//! WP-S1: a refused sketch profile names the entities at fault, END TO END.
//!
//! The worker half and the wire fixture prove the refusal leaves the C++ side
//! carrying `reasonCode` + `evidence.entityIds`. This proves the Rust side does
//! not throw them away before the frontend can read them — which is exactly what
//! it used to do, and on the one lane that matters most.
//!
//! `finish_sketch_with_outcome` upserts the timeline record BEFORE it asks for
//! regions, so a region refusal cannot lose the record; it then returned
//! `op_failed(format!("finishSketch: {e}"))`, and `op_failed` mints an EMPTY
//! diagnostics vector while `Display` skips the field. Every reason code and
//! every entity id died one frame after `wire::parse_diagnostics` parsed it, and
//! the user got an opaque sentence naming no geometry — measured on a real
//! session, `logs/dev.jsonl` 2026-09-09 17:30.
//!
//! Red-first: with `prefixed_engine_error` reverted to the old `op_failed`
//! flattening, `diagnostics` comes back EMPTY and the reason-code assertion
//! fails.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use onecad_core::edit::{EditCommand, SketchEditOp};
use onecad_core::ids::{EntityId, SketchId};
use onecad_core::math::Vec2;
use onecad_core::regen::{EngineError, GeometryEngine};
use onecad_core::sketch::{Sketch, SketchEntity, WorldPlane};

use onecad_lib::document_runtime::DocumentRuntime;
use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::{resolve_worker_path, MeshProvider, SolverEngine, WorkerManager};

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

fn eid(n: u128) -> EntityId {
    EntityId(Uuid::from_u128(n))
}

const P: u128 = 0x900;
const L: u128 = 0x910;

/// Two collinear segments that genuinely OVERLAP over [10,20], inside a closing
/// chain. A point tangency is NOT this case: two collinear segments that merely
/// touch end-to-end collapse to one split and publish normally (pinned in
/// `worker/tests/test_region_table.cpp`).
fn overlapping_sketch(sid: SketchId) -> Sketch {
    let mut sk = Sketch::on_world_plane(sid, "Overlap", WorldPlane::XY);
    let pts = [
        (0.0, 0.0),
        (20.0, 0.0),
        (10.0, 0.0),
        (40.0, 0.0),
        (40.0, 20.0),
        (0.0, 20.0),
    ];
    for (i, (x, y)) in pts.iter().enumerate() {
        sk.add_entity(SketchEntity::point(
            eid(P + i as u128),
            Vec2::new_unchecked(*x, *y),
            false,
            false,
        ))
        .expect("add point");
    }
    // L0 (0,0)->(20,0) and L1 (10,0)->(40,0) share one support and overlap.
    for (i, (a, b)) in [(0u128, 1u128), (2, 3), (3, 4), (4, 5), (5, 0)]
        .iter()
        .enumerate()
    {
        sk.add_entity(SketchEntity::line(
            eid(L + i as u128),
            eid(P + a),
            eid(P + b),
            false,
        ))
        .expect("add line");
    }
    sk
}

#[tokio::test]
async fn a_refused_profile_keeps_its_reason_code_and_entity_ids_through_rust() {
    let Some(bin) = real_worker() else {
        return;
    };
    let wm = WorkerManager::spawn(SupervisorConfig::production(bin));
    assert!(
        wm.wait_ready(Duration::from_secs(10)).await,
        "real worker must connect + handshake + OpenSession"
    );
    let engine: Arc<dyn GeometryEngine> = Arc::new(wm.clone());
    let meshes: Arc<dyn MeshProvider> = Arc::new(wm.clone());
    let solver: Arc<dyn SolverEngine> = Arc::new(wm.clone());
    let mut rt = DocumentRuntime::new_blank(engine, meshes, solver);

    let sid = SketchId(Uuid::from_u128(0x9001));
    rt.apply(EditCommand::AddSketch {
        sketch: Sketch::on_world_plane(sid, "Overlap", WorldPlane::XY),
    })
    .expect("AddSketch");
    rt.enter_sketch(sid).await.expect("enter_sketch");
    let ops: Vec<SketchEditOp> = overlapping_sketch(sid)
        .entities()
        .iter()
        .map(|e| SketchEditOp::AddEntity { entity: e.clone() })
        .collect();
    rt.sketch_upsert(sid, ops).await.expect("sketch_upsert");

    let err = rt
        .finish_sketch(sid)
        .await
        .expect_err("an overlapping profile must refuse");

    let EngineError::OpFailed {
        message,
        diagnostics,
        ..
    } = &err
    else {
        panic!("expected a recoverable OpFailed, got {err:?}");
    };
    assert!(
        message.starts_with("finishSketch: "),
        "the lane prefix survives: {message}"
    );

    let named = diagnostics
        .iter()
        .find(|d| d.reason_code.as_deref() == Some("SKETCH_PROFILE_OVERLAPPING_CURVES"))
        .unwrap_or_else(|| {
            panic!("the refusal must reach Rust with its reason code intact, got {diagnostics:?}")
        });
    assert_eq!(
        named.code, "OP_FAILED",
        "the §8 code stays the taxonomy value"
    );
    assert_eq!(named.stage.as_deref(), Some("profile"));

    // The whole point: WHICH curves. Wire ids, mapped worker-side.
    let ids = named
        .evidence
        .as_ref()
        .and_then(|e| e.get("entityIds"))
        .and_then(|v| v.as_array())
        .unwrap_or_else(|| panic!("evidence.entityIds must survive, got {named:?}"))
        .iter()
        .filter_map(|v| v.as_str())
        .collect::<Vec<_>>();
    let expected = [eid(L).0.to_string(), eid(L + 1).0.to_string()];
    assert_eq!(
        ids,
        expected.iter().map(String::as_str).collect::<Vec<_>>(),
        "both overlapping lines are named, in normative source order"
    );
}
