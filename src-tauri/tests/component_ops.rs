//! Component Library WP-0.2 `PlaceComponent` integration gate against the REAL
//! C++ OCCT worker, driven through the app's [`DocumentRuntime`] exactly like
//! `hole_ops.rs`.
//!
//! Proves the op end to end (Rust core params → wire lowering → worker
//! dispatch → OCCT → published body) with the EXACT kernel volume from
//! `QueryMassProperties` (SCHEMA §7.5), not a mesh-chord approximation — the
//! same discipline `hole_ops.rs` follows.
//!
//! * `place_component_generator_source_mints_one_body` — a `generator`-source
//!   `PlaceComponent` (P0/WP-0.2's hardcoded M6 SHCS) mints exactly one NewBody
//!   at the exact analytic volume.
//! * `place_component_survives_save_and_a_fresh_worker_reopen` — save → reopen
//!   on a FRESH worker replays to the same volume (P0's "zero library
//!   dependency" framing starts here: nothing in this op reads a library root).
//!
//! Gated on `ONECAD_WORKER_PATH` (else the dev-tree fallback); a missing
//! binary skips cleanly (`ONECAD_REQUIRE_WORKER=1` makes that a hard failure).

use std::f64::consts::PI;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use onecad_core::document::record::{
    ComponentParamValue, ComponentSourceRef, DetachComponentParams, FrozenPlacement,
    KnownOperation, Operation, OperationRecord, PlaceComponentParams,
};
use onecad_core::document::variables::Scalar;
use onecad_core::edit::EditCommand;
use onecad_core::ids::{BodyId, RecordId};
use onecad_core::io::container::SaveMeta;
use onecad_core::regen::{CancelToken, GeometryEngine, ModelSnapshot, Outcome, RegenRequest};

use onecad_lib::document_runtime::{DocumentRuntime, RegenReport};
use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::{
    resolve_worker_path, ElementQuery, MeshProvider, SolverEngine, WorkerManager,
};

// ─────────────────────────────────────────────────────────────────────────────
// Harness (mirrors hole_ops.rs)
// ─────────────────────────────────────────────────────────────────────────────

fn real_worker() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("ONECAD_WORKER_PATH") {
        let path = PathBuf::from(&p);
        assert!(
            path.is_file(),
            "ONECAD_WORKER_PATH={p:?} is set but no worker binary exists there \
             (misconfiguration — refusing to skip as green)"
        );
        return Some(path);
    }
    if let Some(path) = resolve_worker_path() {
        return Some(path);
    }
    assert!(
        std::env::var("ONECAD_REQUIRE_WORKER").as_deref() != Ok("1"),
        "ONECAD_REQUIRE_WORKER=1 but no worker binary resolved (CI must hard-fail here)"
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

fn open_over(wm: &WorkerManager, path: &std::path::Path) -> DocumentRuntime {
    let engine: Arc<dyn GeometryEngine> = Arc::new(wm.clone());
    let meshes: Arc<dyn MeshProvider> = Arc::new(wm.clone());
    let solver: Arc<dyn SolverEngine> = Arc::new(wm.clone());
    DocumentRuntime::open(path, engine, meshes, solver).expect("reopen the saved container")
}

fn add_op(rt: &mut DocumentRuntime, record: OperationRecord) {
    rt.apply(EditCommand::AddOperation {
        record,
        at_cursor: true,
    })
    .expect("AddOperation");
}

async fn regen_all(rt: &mut DocumentRuntime) -> RegenReport {
    rt.run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await
}

/// The record's exact `opType` string, read through `rt.projection()`
/// (`DocumentRuntime` exposes no direct timeline accessor to test code).
fn feature_op_type(rt: &DocumentRuntime, record: RecordId) -> String {
    let id = record.to_string();
    rt.projection()
        .features
        .into_iter()
        .find(|f| f.id == id)
        .unwrap_or_else(|| panic!("no feature for record {id}"))
        .op_type
}

fn published<'a>(report: &'a RegenReport, what: &str) -> &'a Arc<ModelSnapshot> {
    match &report.outcome {
        Outcome::Published(s) => s,
        other => panic!("{what}: expected Published, got {other:?}"),
    }
}

fn save_meta() -> SaveMeta {
    SaveMeta {
        app_version: "0.1.0-test".into(),
        occt_fingerprint: Some("occt-8.0.1".into()),
        created: "2026-08-12T00:00:00Z".into(),
        modified: "2026-08-12T00:00:00Z".into(),
    }
}

/// Exact kernel volume of a body (`QueryMassProperties`, SCHEMA §7.5) — read
/// from the BRep, never re-derived from the tessellation.
async fn exact_volume(wm: &WorkerManager, body: BodyId) -> f64 {
    ElementQuery::query_mass_properties(wm, body, "b".into())
        .await
        .expect("QueryMassProperties")
        .volume
}

/// The hardcoded ISO 4762 M6x20 solid's exact analytic volume (see
/// `worker/tests/test_component_ops.cpp` — the C++ side pins the same
/// constant, so a divergence between the two shows up on both sides).
fn m6_shcs_volume() -> f64 {
    PI * 5.0 * 5.0 * 6.0 + PI * 3.0 * 3.0 * 20.0
}

fn place_component_record(rec: u128) -> OperationRecord {
    let mut generator_params = std::collections::BTreeMap::new();
    generator_params.insert(
        "thread".to_string(),
        ComponentParamValue::Text("M6".to_string()),
    );
    let op = Operation::Known(KnownOperation::PlaceComponent(PlaceComponentParams {
        component_id: "onecad.std.iso4762".to_string(),
        component_version: "1.0.0".to_string(),
        component_revision: format!("sha256:{}", "0".repeat(64)),
        params: generator_params,
        source: ComponentSourceRef::Generator {
            generator_id: "iso4762".to_string(),
            generator_version: 1,
            params: std::collections::BTreeMap::new(),
            extra: Default::default(),
        },
        mate: None,
        placement: FrozenPlacement {
            translate: [Scalar::new(0.0), Scalar::new(0.0), Scalar::new(0.0)],
            rotate: Default::default(),
        },
        extra: Default::default(),
    }));
    OperationRecord::new(RecordId(Uuid::from_u128(rec)), 0, "Place Component", op)
}

fn detach_component_op() -> Operation {
    Operation::Known(KnownOperation::DetachComponent(DetachComponentParams {
        source: ComponentSourceRef::Generator {
            generator_id: "iso4762".to_string(),
            generator_version: 1,
            params: std::collections::BTreeMap::new(),
            extra: Default::default(),
        },
        placement: FrozenPlacement {
            translate: [Scalar::new(0.0), Scalar::new(0.0), Scalar::new(0.0)],
            rotate: Default::default(),
        },
        extra: Default::default(),
    }))
}

// ─────────────────────────────────────────────────────────────────────────────
// The gate
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn place_component_generator_source_mints_one_body() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: real worker binary not found (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    add_op(&mut rt, place_component_record(0xc1));
    let report = regen_all(&mut rt).await;
    let snap = published(&report, "place_component");
    assert_eq!(
        snap.bodies.len(),
        1,
        "PlaceComponent mints exactly one body"
    );
    let body = snap.bodies[0].body;

    let vol = exact_volume(&wm, body).await;
    assert!(
        (vol - m6_shcs_volume()).abs() < 1.0,
        "PlaceComponent volume: got {vol}, want {} (M6 SHCS head+shank)",
        m6_shcs_volume()
    );

    wm.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn place_component_survives_save_and_a_fresh_worker_reopen() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: real worker binary not found (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm1 = spawn_worker(bin.clone()).await;
    let mut rt = runtime_over(&wm1);
    add_op(&mut rt, place_component_record(0xc2));
    let report = regen_all(&mut rt).await;
    let body_before = published(&report, "place_component").bodies[0].body;
    let vol_before = exact_volume(&wm1, body_before).await;

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("component.onecad");
    rt.save(&path, save_meta()).expect("save");
    wm1.shutdown().await;

    // A FRESH worker process — proves the op replays from the document alone,
    // with no dependency on anything but the saved container (spec §0
    // invariant #4: "geometry is always cached locally").
    let wm2 = spawn_worker(bin).await;
    let mut rt2 = open_over(&wm2, &path);
    let report2 = regen_all(&mut rt2).await;
    let snap2 = published(&report2, "reopen");
    assert_eq!(
        snap2.bodies.len(),
        1,
        "reopen still publishes exactly one body"
    );
    let body_after = snap2.bodies[0].body;
    let vol_after = exact_volume(&wm2, body_after).await;
    assert!(
        (vol_after - vol_before).abs() < 1e-6,
        "reopen volume {vol_after} != original {vol_before}"
    );

    wm2.shutdown().await;
}

/// WP-1.2: the sanctioned `PlaceComponent` → `DetachComponent` swap, end to
/// end through the REAL worker — not just the Rust-core `op_type_edit_allowed`
/// unit test. Same `RecordId`, same `BodyId`, same volume, before and after;
/// only the op's identity provenance (component_id/version/revision/mate)
/// disappears — spec §3.4's "inert provenance."
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn detach_component_preserves_body_and_volume_across_the_swap() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: real worker binary not found (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let record_id = RecordId(Uuid::from_u128(0xc3));
    add_op(&mut rt, place_component_record(0xc3));
    let report1 = regen_all(&mut rt).await;
    let snap1 = published(&report1, "place_component");
    assert_eq!(snap1.bodies.len(), 1);
    let body_before = snap1.bodies[0].body;
    let vol_before = exact_volume(&wm, body_before).await;
    assert_eq!(feature_op_type(&rt, record_id), "PlaceComponent");

    rt.apply(EditCommand::UpdateOperationParams {
        record: record_id,
        op: detach_component_op(),
    })
    .expect("the sanctioned PlaceComponent -> DetachComponent swap is accepted");
    assert_eq!(
        feature_op_type(&rt, record_id),
        "DetachComponent",
        "the record's op type actually changed"
    );

    let report2 = regen_all(&mut rt).await;
    let snap2 = published(&report2, "detach_component");
    assert_eq!(snap2.bodies.len(), 1, "still exactly one body after detach");
    let body_after = snap2.bodies[0].body;
    assert_eq!(
        body_after, body_before,
        "same BodyId across the swap (same RecordId)"
    );

    let vol_after = exact_volume(&wm, body_after).await;
    assert!(
        (vol_after - vol_before).abs() < 1e-6,
        "detach volume {vol_after} != pre-detach volume {vol_before}"
    );

    wm.shutdown().await;
}
