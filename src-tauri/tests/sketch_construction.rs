//! Construction geometry, end-to-end against the REAL C++ OCCT/PlaneGCS worker.
//!
//! Drives the app's single-writer [`DocumentRuntime`] exactly like
//! `sketch_regions.rs` (real worker via `ONECAD_WORKER_PATH`, else the dev-tree
//! fallback; `ONECAD_REQUIRE_WORKER=1` hard-fails a missing binary).
//!
//! The whole W1-A chain in one place. Rust's `sketch_wire` has ALWAYS emitted
//! `construction` on every entity (SCHEMA §7.3), but the worker's `WireSketch`
//! hardcoded `false`, so `LoopDetector`'s construction filters were dead code and
//! the flag was decorative: a construction rectangle still bounded a region and
//! still extruded. Pinned here:
//!
//! * **excluded** — a closed rectangle of construction lines publishes NO region.
//! * **id stability** — a construction rectangle alongside a real one leaves the
//!   real cell's `regionId` BYTE-IDENTICAL to the real rectangle alone (region
//!   signatures derive only from loop edge wire-ids, so excluding construction
//!   geometry is exactly as id-preserving as never drawing it).
//! * **the edit op** — `SketchEditOp::SetEntityConstruction` flips one edge and
//!   the region set changes loudly and deterministically; undo restores both the
//!   flag and the original region id.
//!
//! This exercises the PRODUCTION wire shape the worker ctest cannot: `p0Ref`/
//! `p1Ref` lines over separate `Point` entities (the ctest uses inline `p0`/`p1`).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use onecad_core::edit::{EditCommand, SketchEditOp};
use onecad_core::ids::{EntityId, SketchId};
use onecad_core::math::Vec2;
use onecad_core::regen::GeometryEngine;
use onecad_core::sketch::{Sketch, SketchEntity, WorldPlane};

use onecad_lib::document_runtime::DocumentRuntime;
use onecad_lib::dto::SketchRegionDto;
use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::{resolve_worker_path, MeshProvider, SolverEngine, WorkerManager};

// ── Harness (mirrors sketch_regions.rs) ──────────────────────────────────────

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

// ── Fixture ──────────────────────────────────────────────────────────────────
//
// Two disjoint 20×20 rectangles: one drawn in construction geometry at x∈[0,20],
// one real at x∈[40,60]. Disjoint so planarization never splits an edge — the
// real cell's identity is then exactly the four real line ids.

const CX_P: u128 = 0x400; // construction rect corner points (…+0..3)
const CX_L: u128 = 0x410; // construction rect lines
const RE_P: u128 = 0x420; // real rect corner points
const RE_L: u128 = 0x430; // real rect lines

fn eid(n: u128) -> EntityId {
    EntityId(Uuid::from_u128(n))
}

/// Four points + four `p0Ref`/`p1Ref` lines closing a rectangle, all flagged
/// `construction` — the shape the frontend authors.
fn push_rect(sk: &mut Sketch, p_base: u128, l_base: u128, x0: f64, side: f64, construction: bool) {
    let corners = [(x0, 0.0), (x0 + side, 0.0), (x0 + side, side), (x0, side)];
    for (i, (x, y)) in corners.iter().enumerate() {
        sk.add_entity(SketchEntity::point(
            eid(p_base + i as u128),
            Vec2::new_unchecked(*x, *y),
            construction,
            false,
        ))
        .expect("add corner point");
    }
    for i in 0..4u128 {
        sk.add_entity(SketchEntity::line(
            eid(l_base + i),
            eid(p_base + i),
            eid(p_base + (i + 1) % 4),
            construction,
        ))
        .expect("add edge");
    }
}

fn mixed_sketch(sid: SketchId, with_construction_rect: bool) -> Sketch {
    let mut sk = Sketch::on_world_plane(sid, "Construction", WorldPlane::XY);
    if with_construction_rect {
        push_rect(&mut sk, CX_P, CX_L, 0.0, 20.0, true);
    }
    push_rect(&mut sk, RE_P, RE_L, 40.0, 20.0, false);
    sk
}

fn add_ops(sk: &Sketch) -> Vec<SketchEditOp> {
    sk.entities()
        .iter()
        .map(|e| SketchEditOp::AddEntity { entity: e.clone() })
        .collect()
}

/// Enters `sid`, upserts the fixture, and leaves the session OPEN so regions can
/// be re-derived after further edits.
async fn open_with(rt: &mut DocumentRuntime, sid: SketchId, with_construction_rect: bool) {
    rt.apply(EditCommand::AddSketch {
        sketch: Sketch::on_world_plane(sid, "Construction", WorldPlane::XY),
    })
    .expect("AddSketch");
    rt.enter_sketch(sid).await.expect("enter_sketch");
    rt.sketch_upsert(sid, add_ops(&mixed_sketch(sid, with_construction_rect)))
        .await
        .expect("sketch_upsert");
}

async fn regions(rt: &mut DocumentRuntime, sid: SketchId) -> Vec<SketchRegionDto> {
    rt.prepare_sketch_regions(sid)
        .expect("prepare_sketch_regions")
        .drive()
        .await
        .expect("drive getSketchRegions")
        .regions
}

// ─────────────────────────────────────────────────────────────────────────────
// (a) Construction geometry is excluded, and excluding it is id-preserving.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn construction_geometry_is_excluded_without_perturbing_region_ids() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    // Baseline: the real rectangle ALONE.
    let baseline_sid = SketchId(Uuid::from_u128(0xC0_00));
    open_with(&mut rt, baseline_sid, false).await;
    let baseline = regions(&mut rt, baseline_sid).await;
    assert_eq!(baseline.len(), 1, "the real rectangle alone is one region");
    let baseline_id = baseline[0].region_id.clone();
    rt.finish_sketch(baseline_sid).await.expect("finish");

    // The same real rectangle, plus a closed CONSTRUCTION rectangle beside it.
    let sid = SketchId(Uuid::from_u128(0xC0_01));
    open_with(&mut rt, sid, true).await;
    let mixed = regions(&mut rt, sid).await;
    assert_eq!(
        mixed.len(),
        1,
        "the construction rectangle bounds nothing — only the real cell is \
         published, got {mixed:#?}"
    );
    assert_eq!(
        mixed[0].region_id, baseline_id,
        "excluding construction geometry leaves the remaining region id \
         BYTE-IDENTICAL (signatures derive only from loop edge wire-ids)"
    );
    eprintln!("CONSTRUCTION-EXCLUDED PASS: 1 region, id {}", baseline_id);

    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// (b) The edit op: flipping a real edge changes the region set; undo restores.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn set_entity_construction_changes_the_region_set_and_undo_restores_it() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let sid = SketchId(Uuid::from_u128(0xC0_02));
    open_with(&mut rt, sid, true).await;
    let before = regions(&mut rt, sid).await;
    assert_eq!(before.len(), 1, "one real cell to start");
    let before_id = before[0].region_id.clone();

    // Flip ONE edge of the REAL rectangle to construction: the loop opens.
    rt.sketch_upsert(
        sid,
        vec![SketchEditOp::SetEntityConstruction {
            entity: eid(RE_L),
            construction: true,
        }],
    )
    .await
    .expect("sketch_upsert setEntityConstruction");
    let after = regions(&mut rt, sid).await;
    assert!(
        after.is_empty(),
        "one construction edge opens the loop — the cell is GONE, not silently \
         re-bound to something else: {after:#?}"
    );

    // Undo restores the flag → the ORIGINAL cell comes back with its original id.
    assert!(rt.undo().is_some(), "undo the flip");
    let restored = regions(&mut rt, sid).await;
    assert_eq!(restored.len(), 1, "undo brings the cell back");
    assert_eq!(
        restored[0].region_id, before_id,
        "the restored cell keeps its original identity — no re-derivation drift"
    );
    eprintln!("SET-CONSTRUCTION PASS: 1 → 0 → 1 region, id {before_id} preserved");

    wm.shutdown().await;
}
