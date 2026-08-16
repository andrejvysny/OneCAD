//! Autosave driver + crash-recovery round-trip (M5b deliverable 1 + drill 3b).
//!
//! Drives the app's real [`autosave`](onecad_lib::autosave) module over a
//! [`DocumentRuntime`] backed by [`PendingBackend`] (no OCCT worker — autosave only
//! touches the timeline + container IO, never geometry). Proves:
//!
//! * the debounced driver writes an autosave container + session marker after a
//!   mutation, and emits exactly one [`AutosaveEvent`];
//! * **zero autosave activity when no document is open**;
//! * a clean save/close clears the marker + stale autosave
//!   ([`clear_recovery_state`]);
//! * the recovery round-trip: a crashed session's container surfaces via
//!   [`scan_recoverable`], and reopening it reconstructs the exact document
//!   revision that was autosaved — including its NAME;
//! * an ORPHAN (a container whose marker did not survive) still recovers.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use tokio::sync::{watch, Mutex};
use uuid::Uuid;

use onecad_core::document::record::{
    BooleanMode, ExtrudeMode, ExtrudeParams, KnownOperation, Operation, OperationRecord,
};
use onecad_core::document::variables::Scalar;
use onecad_core::edit::EditCommand;
use onecad_core::ids::{DocumentId, RecordId};
use onecad_core::io::recovery::{autosave_path, marker_path, scan_recoverable};
use onecad_core::regen::GeometryEngine;

use onecad_lib::autosave::{self, AutosaveEvent};
use onecad_lib::document_runtime::DocumentRuntime;
use onecad_lib::worker::{MeshProvider, PendingBackend, SolverEngine};

// ─────────────────────────────────────────────────────────────────────────────
// Harness
// ─────────────────────────────────────────────────────────────────────────────

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

fn pending_runtime() -> DocumentRuntime {
    let backend = Arc::new(PendingBackend);
    let engine: Arc<dyn GeometryEngine> = backend.clone();
    let meshes: Arc<dyn MeshProvider> = backend.clone();
    let solver: Arc<dyn SolverEngine> = backend;
    DocumentRuntime::new_blank(engine, meshes, solver)
}

/// A runtime carrying one extrude op (a non-empty timeline to autosave).
fn runtime_with_op(distance: f64) -> DocumentRuntime {
    let mut rt = pending_runtime();
    rt.apply(EditCommand::AddOperation {
        record: extrude_record(0x10, distance),
        at_cursor: true,
    })
    .expect("AddOperation");
    rt
}

// ─────────────────────────────────────────────────────────────────────────────
// Driver: writes after a mutation + debounce
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn driver_writes_autosave_and_marker_after_debounce() {
    let dir = tempfile::tempdir().unwrap();
    let app_data = dir.path().to_path_buf();

    let rt = runtime_with_op(25.0);
    let doc_id = rt.document_uuid();
    let runtime = Arc::new(Mutex::new(Some(rt)));

    let (tx, rx) = watch::channel(0u64);
    let seen = Arc::new(std::sync::Mutex::new(Vec::<AutosaveEvent>::new()));
    let seen2 = seen.clone();
    let driver = tokio::spawn(autosave::run(
        runtime.clone(),
        app_data.clone(),
        Arc::new(Mutex::new(())),
        rx,
        Duration::from_millis(80), // short debounce for the test
        Duration::from_secs(60),   // ceiling well clear of this test's window
        move |ev| seen2.lock().unwrap().push(ev),
    ));

    // A mutation lands.
    tx.send_modify(|v| *v += 1);

    let autosave = autosave_path(&app_data, doc_id);
    let marker = marker_path(&app_data, doc_id);
    let mut landed = false;
    for _ in 0..60 {
        if autosave.exists() && !seen.lock().unwrap().is_empty() {
            landed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(landed, "autosave must land after the debounce window");
    assert!(autosave.exists(), "autosave container written");
    assert!(marker.exists(), "session crash marker written");

    {
        let events = seen.lock().unwrap();
        assert_eq!(events.len(), 1, "exactly one AUTOSAVE event");
        assert_eq!(events[0].path, autosave.to_string_lossy());
        assert!(events[0].at_ms > 0, "event carries a wall-clock atMs");
    }

    drop(tx);
    let _ = driver.await;
}

// ─────────────────────────────────────────────────────────────────────────────
// Zero autosave activity when no document is open
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn driver_is_silent_with_no_document_open() {
    let dir = tempfile::tempdir().unwrap();
    let app_data = dir.path().to_path_buf();
    let runtime = Arc::new(Mutex::new(None)); // nothing open

    let (tx, rx) = watch::channel(0u64);
    let seen = Arc::new(std::sync::Mutex::new(Vec::<AutosaveEvent>::new()));
    let seen2 = seen.clone();
    let driver = tokio::spawn(autosave::run(
        runtime.clone(),
        app_data.clone(),
        Arc::new(Mutex::new(())),
        rx,
        Duration::from_millis(60),
        Duration::from_secs(60),
        move |ev| seen2.lock().unwrap().push(ev),
    ));

    // Poke the tick with no document open.
    tx.send_modify(|v| *v += 1);
    tokio::time::sleep(Duration::from_millis(300)).await;

    assert!(
        seen.lock().unwrap().is_empty(),
        "no document open ⇒ no AUTOSAVE event"
    );
    // The autosave subdir must not even have been created (zero side effects).
    assert!(
        !app_data.join("autosave").exists(),
        "no autosave directory created when nothing is open"
    );

    drop(tx);
    let _ = driver.await;
}

// ─────────────────────────────────────────────────────────────────────────────
// A clean save/close clears the marker + stale autosave
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn clear_recovery_state_removes_marker_and_autosave() {
    let dir = tempfile::tempdir().unwrap();
    let app_data = dir.path().to_path_buf();
    let rt = runtime_with_op(10.0);
    let doc_id = rt.document_uuid();
    let runtime = Mutex::new(Some(rt));

    // Write one autosave (marker + container).
    let lane = Mutex::new(());
    let ev = autosave::autosave_current(&runtime, &app_data, &lane)
        .await
        .expect("autosave written");
    assert!(autosave_path(&app_data, doc_id).exists());
    assert!(marker_path(&app_data, doc_id).exists());
    assert!(!ev.path.is_empty());

    // A clean save/close supersedes it.
    autosave::clear_recovery_state(&app_data, doc_id);
    assert!(
        !autosave_path(&app_data, doc_id).exists(),
        "autosave deleted on clean save/close"
    );
    assert!(
        !marker_path(&app_data, doc_id).exists(),
        "crash marker removed on clean save/close"
    );
    // Idempotent (a second clear is not an error).
    autosave::clear_recovery_state(&app_data, doc_id);
}

// ─────────────────────────────────────────────────────────────────────────────
// Recovery round-trip: stale marker → scan → recover → document intact (drill 3b)
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recovery_round_trip_reconstructs_the_autosaved_document() {
    let dir = tempfile::tempdir().unwrap();
    let app_data = dir.path().to_path_buf();

    // A document that was edited (extrude 42) and never saved, then "crashed".
    let original_path = dir.path().join("Bracket.onecad");
    let doc_id: DocumentId;
    {
        let mut rt = runtime_with_op(42.0);
        // Pretend it had a real save path (so the marker records `opened_path`).
        rt.mark_recovered(Some(original_path.clone()), None); // sets path/title + dirty
        doc_id = rt.document_uuid();
        let runtime = Mutex::new(Some(rt));
        let lane = Mutex::new(());
        autosave::autosave_current(&runtime, &app_data, &lane)
            .await
            .expect("autosave the pre-crash revision");
    } // the "session" ends here without a clean close (simulated crash — marker stays)

    // Startup scan: the owning process is treated as dead (injected predicate), so
    // the surviving container becomes a recovery offer.
    let offers = scan_recoverable(&app_data, |_| false, SystemTime::now()).expect("scan");
    assert_eq!(offers.len(), 1, "one stale recovery offer");
    let offer = &offers[0];
    assert_eq!(offer.document_id, doc_id);
    let marker = offer
        .marker
        .as_ref()
        .expect("the marker survived the crash");
    assert_eq!(marker.opened_path.as_deref(), Some(original_path.as_path()));
    assert_eq!(
        marker.title.as_deref(),
        Some("Bracket"),
        "the marker records the live title, so recovery can restore it"
    );
    assert!(offer.autosave_path.exists(), "autosave container survives");

    // Recover: reopen the autosave as the live document (the app's recover path).
    let backend = Arc::new(PendingBackend);
    let engine: Arc<dyn GeometryEngine> = backend.clone();
    let meshes: Arc<dyn MeshProvider> = backend.clone();
    let solver: Arc<dyn SolverEngine> = backend;
    let mut recovered = DocumentRuntime::open(&offer.autosave_path, engine, meshes, solver)
        .expect("reopen autosave");
    recovered.mark_recovered(marker.opened_path.clone(), marker.title.clone());

    // The recovered document IS the autosaved revision: same id, same timeline.
    assert_eq!(recovered.document_uuid(), doc_id, "same document id");
    let proj = recovered.projection();
    assert_eq!(proj.features.len(), 1, "the autosaved extrude survives");
    assert_eq!(
        proj.features[0].value_text, "42.0 mm",
        "recovered feature matches the autosaved revision"
    );
    assert!(recovered.is_dirty(), "recovered work is unsaved (dirty)");
    // The real save path is restored so a later Save targets the original file.
    assert_eq!(recovered.path(), Some(original_path.as_path()));
    // ...and so is the NAME. `open` derived it from `<documentId>.onecad`, so
    // without `mark_recovered` restoring it the user is shown a raw UUID and has no
    // way to tell which document they are being offered.
    assert_eq!(
        recovered.title(),
        "Bracket",
        "the recovered document keeps its name, not the autosave file's stem"
    );
    assert!(
        !recovered.title().contains(&doc_id.to_string()),
        "and specifically not the document UUID"
    );
}

/// An autosave whose marker did not survive is still recoverable — only its
/// labelling degrades. The title then falls back to the original path's stem.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_orphaned_container_still_recovers_with_a_usable_title() {
    let dir = tempfile::tempdir().unwrap();
    let app_data = dir.path().to_path_buf();
    let original_path = dir.path().join("Housing.onecad");

    let doc_id = {
        let mut rt = runtime_with_op(7.0);
        rt.mark_recovered(Some(original_path.clone()), None);
        let id = rt.document_uuid();
        let runtime = Mutex::new(Some(rt));
        autosave::autosave_current(&runtime, &app_data, &Mutex::new(()))
            .await
            .expect("autosave");
        id
    };
    // The marker is lost (a crash between the container write and the marker write,
    // or a Restore under the old marker-consuming behaviour).
    std::fs::remove_file(marker_path(&app_data, doc_id)).unwrap();

    let offers = scan_recoverable(&app_data, |_| false, SystemTime::now()).expect("scan");
    assert_eq!(offers.len(), 1, "the container is still discoverable");
    assert!(offers[0].marker.is_none(), "but unlabelled");

    let backend = Arc::new(PendingBackend);
    let engine: Arc<dyn GeometryEngine> = backend.clone();
    let meshes: Arc<dyn MeshProvider> = backend.clone();
    let solver: Arc<dyn SolverEngine> = backend;
    let mut recovered = DocumentRuntime::open(&offers[0].autosave_path, engine, meshes, solver)
        .expect("reopen autosave");
    // No marker ⇒ no recorded path or title. The app supplies what it knows, which
    // here is nothing, and the document keeps the id-derived title rather than
    // inventing one.
    recovered.mark_recovered(None, None);
    assert_eq!(recovered.projection().features.len(), 1, "work intact");
    assert!(recovered.is_dirty());

    // With the path known (e.g. the user picked it), the stem is the fallback title.
    recovered.mark_recovered(Some(original_path.clone()), None);
    assert_eq!(recovered.title(), "Housing");
}
