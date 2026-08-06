//! WP-FIX W3 — save/autosave run OFF the single-writer runtime lock (VF-B7 + F1a),
//! serialized against each other by the **persistence lane** (`AppState::persistence`).
//!
//! Before W3 both lanes held the runtime lock across container serialization +
//! deflate of every import blob, so a save stalled every edit for its whole duration.
//! The write now happens on a `spawn_blocking` thread from a detached
//! [`SavePayload`](onecad_lib::document_runtime::SavePayload); the runtime lock is
//! held only for the snapshot. That opens two races the lane closes:
//!
//! * **M3** — two writers on one target path eat each other's temps
//!   (`ContainerWriter` sweeps sibling temps before its rename), so a concurrent
//!   autosave pair could both fail;
//! * **M2** — an autosave landing after a save cleared the crash marker resurrects a
//!   bogus recovery offer.
//!
//! What each test pins:
//!
//! * `autosave_holds_no_runtime_lock_while_waiting_on_the_persistence_lane` —
//!   deterministic lock-order pin: with the lane held by the test, an edit still
//!   acquires the runtime lock promptly, and the autosaved container carries the
//!   PRE-edit revision (proving the snapshot was taken before the lane wait).
//! * `concurrent_autosaves_on_one_path_all_land` — the M3 pin (three writers, one
//!   path, a multi-megabyte blob to widen the window).
//! * `save_and_autosave_leave_a_consistent_recovery_state` — the M2 pin: whatever
//!   order the lane grants, a surviving marker always names an autosave that exists.
//! * `autosave_with_a_real_import_blob_completes_while_edits_land` — real worker +
//!   the checked-in colored STEP fixture: edits keep landing during the write, both
//!   sides complete, and the autosave container + marker are intact.
//!
//! The last test is REQUIRE_WORKER-guarded (CI hard-fails without a worker binary;
//! local dev skips cleanly). Everything else runs over [`PendingBackend`] — the lane
//! is pure IO + locking and never touches geometry.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;
use uuid::Uuid;

use onecad_core::document::record::{
    BooleanMode, ExtrudeMode, ExtrudeParams, ImportSourceCodec, ImportStepParams, KnownOperation,
    Operation, OperationRecord, IMPORT_HEAL_POLICY_V1,
};
use onecad_core::document::variables::Scalar;
use onecad_core::edit::EditCommand;
use onecad_core::ids::RecordId;
use onecad_core::io::container::{ContainerReader, SaveMeta};
use onecad_core::io::imports::ImportBlob;
use onecad_core::io::recovery::{autosave_path, marker_path};
use onecad_core::regen::GeometryEngine;

use onecad_lib::autosave;
use onecad_lib::document_runtime::{DocumentRuntime, SaveCaches};
use onecad_lib::imports::{prepare_import, PreparedImport};
use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::{resolve_worker_path, MeshProvider, SolverEngine, WorkerManager};

// ─────────────────────────────────────────────────────────────────────────────
// Harness
// ─────────────────────────────────────────────────────────────────────────────

type SharedRuntime = Arc<Mutex<Option<DocumentRuntime>>>;
type Lane = Arc<Mutex<()>>;

fn extrude_record(seed: u128, distance: f64) -> OperationRecord {
    let op = Operation::Known(KnownOperation::Extrude(ExtrudeParams {
        profile: None,
        distance: Scalar::new(distance),
        draft_angle_deg: Scalar::new(0.0),
        mode: ExtrudeMode::Blind,
        boolean_mode: BooleanMode::NewBody,
        target_body: None,
        target_face: None,
        two_directions: false,
        mode2: ExtrudeMode::Blind,
        distance2: Scalar::new(0.0),
        target_face2: None,
        extra: Default::default(),
    }));
    OperationRecord::new(RecordId(Uuid::from_u128(seed)), 0, "Extrude", op)
}

fn add_extrude(seed: u128, distance: f64) -> EditCommand {
    EditCommand::AddOperation {
        record: extrude_record(seed, distance),
        at_cursor: true,
    }
}

fn pending_runtime() -> DocumentRuntime {
    let backend = Arc::new(onecad_lib::worker::PendingBackend);
    let engine: Arc<dyn GeometryEngine> = backend.clone();
    let meshes: Arc<dyn MeshProvider> = backend.clone();
    let solver: Arc<dyn SolverEngine> = backend;
    DocumentRuntime::new_blank(engine, meshes, solver)
}

fn record_count(container: &Path) -> usize {
    ContainerReader::open(container)
        .expect("the autosaved container must open cleanly")
        .document()
        .timeline
        .records()
        .len()
}

/// Deterministic pseudo-random (and therefore near-incompressible) bytes — a blob
/// whose deflate actually costs something, so the concurrent-write window is real
/// rather than a nanosecond.
fn incompressible(len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len);
    let mut x: u64 = 0x2545_F491_4F6C_DD1D;
    while out.len() < len {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        out.extend_from_slice(&x.to_le_bytes());
    }
    out.truncate(len);
    out
}

/// Authors an `ImportStep` record over synthetic bytes, so the autosave carries a
/// multi-megabyte blob through the real refcounted carrier.
fn seed_synthetic_import(rt: &mut DocumentRuntime, bytes: Vec<u8>) {
    use sha2::{Digest, Sha256};
    let sha = format!("{:x}", Sha256::digest(&bytes));
    let params = ImportStepParams {
        source_sha256: sha.clone(),
        source_codec: ImportSourceCodec::Step,
        source_name: "synthetic.step".into(),
        heal_policy: IMPORT_HEAL_POLICY_V1.to_string(),
        unit_scale: Scalar::new(1.0),
        brep_format: None,
        provenance_sha256: None,
        extra: Default::default(),
    };
    let prepared = PreparedImport {
        params,
        blobs: vec![(
            sha,
            ImportBlob {
                codec: ImportSourceCodec::Step,
                bytes: Arc::new(bytes),
            },
        )],
        solid_count: 1,
        product_names: Vec::new(),
        diagnostics: Vec::new(),
    };
    rt.add_import_record(&prepared, false)
        .expect("author the synthetic ImportStep record");
}

fn spawn_autosave(
    runtime: &SharedRuntime,
    app_data: &Path,
    lane: &Lane,
) -> tokio::task::JoinHandle<Option<autosave::AutosaveEvent>> {
    let runtime = runtime.clone();
    let lane = lane.clone();
    let app_data = app_data.to_path_buf();
    tokio::spawn(async move { autosave::autosave_current(&runtime, &app_data, &lane).await })
}

fn save_meta() -> SaveMeta {
    SaveMeta {
        app_version: "0.1.0-test".into(),
        occt_fingerprint: None,
        created: "2026-08-03T00:00:00Z".into(),
        modified: "2026-08-03T00:00:00Z".into(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Lock order: runtime → release → lane, never nested
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn autosave_holds_no_runtime_lock_while_waiting_on_the_persistence_lane() {
    let dir = tempfile::tempdir().unwrap();
    let app_data = dir.path().to_path_buf();

    let mut rt = pending_runtime();
    rt.apply(add_extrude(0x10, 25.0)).unwrap();
    let doc_id = rt.document_uuid();
    let runtime: SharedRuntime = Arc::new(Mutex::new(Some(rt)));
    let lane: Lane = Arc::new(Mutex::new(()));

    // Hold the lane, so the autosave gets exactly as far as "payload built, waiting
    // to write" and parks there.
    let held = lane.lock().await;
    let task = spawn_autosave(&runtime, &app_data, &lane);
    tokio::time::sleep(Duration::from_millis(200)).await;

    // THE PIN: the runtime lock must be free while the autosave waits on the lane.
    // Holding it across the lane wait (or across the write) would hang here.
    let mut guard = tokio::time::timeout(Duration::from_secs(5), runtime.lock())
        .await
        .expect("the runtime lock must NOT be held across the persistence lane wait");
    guard
        .as_mut()
        .unwrap()
        .apply(add_extrude(0x11, 40.0))
        .unwrap();
    drop(guard);

    drop(held); // release the lane; the parked write proceeds.
    let ev = task.await.unwrap().expect("the autosave completes");

    let path = autosave_path(&app_data, doc_id);
    assert_eq!(ev.path, path.to_string_lossy());
    assert!(marker_path(&app_data, doc_id).exists(), "marker written");
    assert_eq!(
        record_count(&path),
        1,
        "the container holds the PRE-edit snapshot — proof the payload was built \
         under the runtime lock and the edit landed while the write was pending"
    );
    let guard = runtime.lock().await;
    assert_eq!(
        guard.as_ref().unwrap().projection().features.len(),
        2,
        "the edit really landed in the live document"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// M3: concurrent writers on one path do not eat each other's temps
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_autosaves_on_one_path_all_land() {
    let dir = tempfile::tempdir().unwrap();
    let app_data = dir.path().to_path_buf();

    let mut rt = pending_runtime();
    rt.apply(add_extrude(0x10, 25.0)).unwrap();
    // 6 MiB of incompressible source bytes ⇒ the deflate window is wide enough for
    // three writers to genuinely overlap.
    seed_synthetic_import(&mut rt, incompressible(6 * 1024 * 1024));
    let doc_id = rt.document_uuid();
    let runtime: SharedRuntime = Arc::new(Mutex::new(Some(rt)));
    let lane: Lane = Arc::new(Mutex::new(()));

    let tasks: Vec<_> = (0..3)
        .map(|_| spawn_autosave(&runtime, &app_data, &lane))
        .collect();
    for (i, t) in tasks.into_iter().enumerate() {
        assert!(
            t.await.unwrap().is_some(),
            "autosave #{i} must complete — a lost temp (cleanup_stale_temps) is the M3 defect"
        );
    }

    let path = autosave_path(&app_data, doc_id);
    assert_eq!(record_count(&path), 2, "import + extrude survive the race");
    assert!(marker_path(&app_data, doc_id).exists(), "marker written");
    // No temp debris survives a fully-serialized run.
    let debris: Vec<_> = std::fs::read_dir(path.parent().unwrap())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains(".tmp"))
        .collect();
    assert!(debris.is_empty(), "leftover temp files: {debris:?}");
}

// ─────────────────────────────────────────────────────────────────────────────
// M2: a save's recovery-clear and an autosave's marker never disagree
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn save_and_autosave_leave_a_consistent_recovery_state() {
    let dir = tempfile::tempdir().unwrap();
    let app_data = dir.path().to_path_buf();
    let target = dir.path().join("Bracket.onecad");

    let mut rt = pending_runtime();
    rt.apply(add_extrude(0x10, 25.0)).unwrap();
    seed_synthetic_import(&mut rt, incompressible(2 * 1024 * 1024));
    let doc_id = rt.document_uuid();
    let runtime: SharedRuntime = Arc::new(Mutex::new(Some(rt)));
    let lane: Lane = Arc::new(Mutex::new(()));

    // An earlier autosave already left recovery state behind.
    autosave::autosave_current(&runtime, &app_data, &lane)
        .await
        .expect("seed autosave");
    assert!(marker_path(&app_data, doc_id).exists());

    // Now a save and an autosave race for the lane.
    let autosaving = spawn_autosave(&runtime, &app_data, &lane);
    let saving = {
        let payload = {
            let mut guard = runtime.lock().await;
            guard
                .as_mut()
                .unwrap()
                .build_save_payload(save_meta(), SaveCaches::explicit())
        };
        let lane = lane.clone();
        let app_data = app_data.clone();
        let target = target.clone();
        tokio::spawn(async move {
            autosave::write_save_payload(&lane, &target, payload, Some(&app_data), doc_id).await
        })
    };

    saving.await.unwrap().expect("the explicit save writes");
    autosaving.await.unwrap(); // may legitimately be Some or (never) None

    assert_eq!(
        record_count(&target),
        2,
        "the saved container opens cleanly"
    );
    // The invariant: a marker may or may not survive the race, but if it does it MUST
    // name an autosave container that exists — never a recovery offer pointing at a
    // file the save deleted.
    if marker_path(&app_data, doc_id).exists() {
        assert!(
            autosave_path(&app_data, doc_id).exists(),
            "a surviving crash marker must name a surviving autosave container"
        );
        assert_eq!(record_count(&autosave_path(&app_data, doc_id)), 2);
    } else {
        assert!(
            !autosave_path(&app_data, doc_id).exists(),
            "the save cleared the marker, so it must have cleared the autosave too"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Real worker: an autosave carrying a real import blob, with edits landing
// throughout the write
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn autosave_with_a_real_import_blob_completes_while_edits_land() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let step = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/colored_boxes.step");
    assert!(step.is_file(), "checked-in fixture missing at {step:?}");

    let dir = tempfile::tempdir().unwrap();
    let app_data = dir.path().to_path_buf();

    let wm = WorkerManager::spawn(SupervisorConfig::production(bin));
    assert!(
        wm.wait_ready(Duration::from_secs(10)).await,
        "real worker must connect + handshake + OpenSession"
    );
    let engine: Arc<dyn GeometryEngine> = Arc::new(wm.clone());
    let meshes: Arc<dyn MeshProvider> = Arc::new(wm.clone());
    let solver: Arc<dyn SolverEngine> = Arc::new(wm.clone());
    let mut rt = DocumentRuntime::new_blank(engine, meshes, solver);

    let prepared = prepare_import(&wm, &step).await.expect("prepare_import");
    rt.add_import_record(&prepared, false)
        .expect("author the ImportStep record");
    let doc_id = rt.document_uuid();
    let runtime: SharedRuntime = Arc::new(Mutex::new(Some(rt)));
    let lane: Lane = Arc::new(Mutex::new(()));

    // The autosave runs while a writer keeps taking the runtime lock. Under the old
    // (lock-held) shape the edits could not interleave at all.
    let autosaving = spawn_autosave(&runtime, &app_data, &lane);
    let editing = {
        let runtime = runtime.clone();
        tokio::spawn(async move {
            for i in 0..40u128 {
                {
                    let mut guard = runtime.lock().await;
                    guard
                        .as_mut()
                        .unwrap()
                        .apply(add_extrude(0x200 + i, 5.0 + i as f64))
                        .expect("edit lands during the autosave write");
                }
                tokio::task::yield_now().await;
            }
        })
    };

    let ev = tokio::time::timeout(Duration::from_secs(30), autosaving)
        .await
        .expect("the autosave must not deadlock against the edit loop")
        .unwrap()
        .expect("the autosave completes");
    tokio::time::timeout(Duration::from_secs(30), editing)
        .await
        .expect("the edit loop must not deadlock against the autosave")
        .unwrap();

    let path = autosave_path(&app_data, doc_id);
    assert_eq!(ev.path, path.to_string_lossy());
    assert!(marker_path(&app_data, doc_id).exists(), "marker written");

    // The container is internally consistent (it opens, hashes verify) and carries
    // the import blob, whatever revision it caught.
    let loaded = ContainerReader::open(&path).expect("autosave container opens clean");
    assert!(
        !loaded.document().timeline.records().is_empty(),
        "the autosaved timeline is non-empty"
    );
    assert!(
        !loaded.import_blobs().is_empty(),
        "the import source blob is persisted into the autosave"
    );
    let sha = &prepared.params.source_sha256;
    let bytes = loaded
        .read_import_blob(sha)
        .expect("the blob reads back and content-verifies");
    assert!(!bytes.is_empty());

    let guard = runtime.lock().await;
    assert_eq!(
        guard.as_ref().unwrap().projection().features.len(),
        41,
        "the import plus all 40 concurrent edits are in the live document"
    );
}
