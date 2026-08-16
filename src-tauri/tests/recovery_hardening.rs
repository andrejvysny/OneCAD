//! Autosave hardening pins — the three ways an autosave used to be absent, wrong,
//! or unrecoverable when the crash actually came.
//!
//! Companion to `autosave_recovery.rs`, which covers the happy-path lifecycle. Every
//! test here corresponds to a defect that shipped, and each names it, so a
//! green-to-red flip identifies the regression rather than merely reporting one.
//!
//! Driven over [`PendingBackend`] (no OCCT worker): autosave touches the timeline
//! and container IO, never geometry.

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use tokio::sync::{watch, Mutex};
use uuid::Uuid;

use onecad_core::document::record::{
    BooleanMode, ExtrudeMode, ExtrudeParams, KnownOperation, Operation, OperationRecord,
};
use onecad_core::document::variables::Scalar;
use onecad_core::edit::EditCommand;
use onecad_core::ids::RecordId;
use onecad_core::io::recovery::{autosave_path, marker_path, scan_recoverable};
use onecad_core::regen::GeometryEngine;

use onecad_lib::autosave::{self, AutosaveEvent};
use onecad_lib::document_runtime::DocumentRuntime;
use onecad_lib::worker::{MeshProvider, PendingBackend, SolverEngine};

// ─────────────────────────────────────────────────────────────────────────────
// Harness
// ─────────────────────────────────────────────────────────────────────────────

fn pending_runtime() -> DocumentRuntime {
    let backend = Arc::new(PendingBackend);
    let engine: Arc<dyn GeometryEngine> = backend.clone();
    let meshes: Arc<dyn MeshProvider> = backend.clone();
    let solver: Arc<dyn SolverEngine> = backend;
    DocumentRuntime::new_blank(engine, meshes, solver)
}

/// A runtime carrying one extrude op — a non-empty, DIRTY timeline to autosave.
fn dirty_runtime() -> DocumentRuntime {
    let mut rt = pending_runtime();
    rt.apply(EditCommand::AddOperation {
        record: OperationRecord::new(
            RecordId(Uuid::from_u128(0x10)),
            0,
            "Extrude",
            Operation::Known(KnownOperation::Extrude(ExtrudeParams {
                profile: None,
                distance: Scalar::new(25.0),
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
            })),
        ),
        at_cursor: true,
    })
    .expect("AddOperation");
    assert!(rt.is_dirty(), "an applied edit leaves the document dirty");
    rt
}

/// Spawns the driver and returns `(tick sender, observed events, join handle)`.
fn spawn_driver(
    runtime: Arc<Mutex<Option<DocumentRuntime>>>,
    app_data: std::path::PathBuf,
    debounce: Duration,
    max_age: Duration,
) -> (
    watch::Sender<u64>,
    Arc<std::sync::Mutex<Vec<AutosaveEvent>>>,
    tokio::task::JoinHandle<()>,
) {
    let (tx, rx) = watch::channel(0u64);
    let seen = Arc::new(std::sync::Mutex::new(Vec::<AutosaveEvent>::new()));
    let sink = seen.clone();
    let handle = tokio::spawn(autosave::run(
        runtime,
        app_data,
        Arc::new(Mutex::new(())),
        rx,
        debounce,
        max_age,
        move |ev| sink.lock().unwrap().push(ev),
    ));
    (tx, seen, handle)
}

// ─────────────────────────────────────────────────────────────────────────────
// RC-1 — a pure debounce is starvable
// ─────────────────────────────────────────────────────────────────────────────

/// The autosave that never happened.
///
/// The driver used to wait for `debounce` of QUIET with no ceiling, so sustained
/// editing re-armed the window on every edit and no autosave was ever written. The
/// user who models for ten minutes without pausing — the one with the most to lose —
/// was the one guaranteed to lose it.
///
/// A tick every 30 ms against a 500 ms debounce never goes quiet. The write must
/// still land, driven by `max_age` alone.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_continuous_edit_burst_still_autosaves_at_the_deadline() {
    let dir = tempfile::tempdir().unwrap();
    let app_data = dir.path().to_path_buf();

    let rt = dirty_runtime();
    let doc_id = rt.document_uuid();
    let runtime = Arc::new(Mutex::new(Some(rt)));

    let debounce = Duration::from_millis(500); // never satisfied by the burst below
    let max_age = Duration::from_millis(300);
    let (tx, seen, driver) = spawn_driver(runtime, app_data.clone(), debounce, max_age);

    // Edit continuously for well over `max_age` and well under `debounce`.
    let started = Instant::now();
    let burst = tokio::spawn(async move {
        for _ in 0..40 {
            tx.send_modify(|v| *v += 1);
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
        tx // keep the sender alive until the burst ends
    });

    let autosave = autosave_path(&app_data, doc_id);
    let mut landed = None;
    for _ in 0..100 {
        if autosave.exists() && !seen.lock().unwrap().is_empty() {
            landed = Some(started.elapsed());
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let elapsed = landed.expect(
        "a burst that never goes quiet must still autosave — the deadline is the only \
         thing that can fire here",
    );
    assert!(
        elapsed < debounce,
        "the write must come from the deadline, not from the debounce \
         (landed after {elapsed:?}, debounce is {debounce:?})"
    );
    assert!(
        marker_path(&app_data, doc_id).exists(),
        "marker written too"
    );

    let tx = burst.await.unwrap();
    drop(tx);
    let _ = driver.await;
}

// ─────────────────────────────────────────────────────────────────────────────
// RC-6 — a tick is not evidence of unsaved work
// ─────────────────────────────────────────────────────────────────────────────

/// The recovery offer for work that never existed.
///
/// `autosave_current` never read `dirty`, and the loop is woken by a TICK — which a
/// published regen bumps, and `open_document` schedules a regen. So opening a
/// project and touching nothing armed a container plus a crash marker; kill the
/// process and the next launch offered it back as "unsaved changes recovered".
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_clean_document_is_never_autosaved() {
    let dir = tempfile::tempdir().unwrap();
    let app_data = dir.path().to_path_buf();

    // A document with real content that is NOT dirty — exactly the state a just-
    // opened project is in once its initial regen publishes. `mark_saved` at the
    // current revision is the production way to reach it.
    let mut rt = dirty_runtime();
    let revision = rt.revision();
    assert!(
        rt.mark_saved(&dir.path().join("Saved.onecad"), revision),
        "no edit landed, so the save is clean"
    );
    assert!(!rt.is_dirty(), "precondition: clean");
    let doc_id = rt.document_uuid();
    let runtime = Mutex::new(Some(rt));

    let event = autosave::autosave_current(&runtime, &app_data, &Mutex::new(())).await;

    assert!(event.is_none(), "a clean document reports no autosave");
    assert!(
        !autosave_path(&app_data, doc_id).exists(),
        "and writes no container"
    );
    assert!(
        !marker_path(&app_data, doc_id).exists(),
        "and above all no crash marker — a marker is what produces the offer"
    );
    assert!(
        scan_recoverable(&app_data, |_| false, SystemTime::now())
            .unwrap()
            .is_empty(),
        "so the next launch has nothing to offer"
    );
}

/// The dirty gate must not swallow real work: the same document, dirty, still writes.
/// Without this control the test above would pass on an autosave that had simply
/// stopped working.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dirty_document_is_still_autosaved() {
    let dir = tempfile::tempdir().unwrap();
    let app_data = dir.path().to_path_buf();
    let rt = dirty_runtime();
    let doc_id = rt.document_uuid();
    let runtime = Mutex::new(Some(rt));

    let event = autosave::autosave_current(&runtime, &app_data, &Mutex::new(())).await;

    assert!(event.is_some(), "dirty work is autosaved");
    assert!(autosave_path(&app_data, doc_id).exists());
    assert_eq!(
        scan_recoverable(&app_data, |_| false, SystemTime::now())
            .unwrap()
            .len(),
        1,
        "and is offered after a crash"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// RC-10 — a container with no marker is not a completed autosave
// ─────────────────────────────────────────────────────────────────────────────

/// `autosave_current` used to return `Some(event)` even when the marker write
/// failed, so the caller emitted an `autosave` event — and the UI would report the
/// work as protected — for a container discovery could not label and no live session
/// owned. The half-write is now rolled back and reported as a failure.
///
/// The marker write is forced to fail by occupying its path with a DIRECTORY:
/// `write_marker`'s final `rename` cannot replace one, while the container write,
/// which happens first and targets a different name, still succeeds.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_marker_write_fails_the_whole_autosave() {
    let dir = tempfile::tempdir().unwrap();
    let app_data = dir.path().to_path_buf();
    let rt = dirty_runtime();
    let doc_id = rt.document_uuid();
    let runtime = Mutex::new(Some(rt));

    std::fs::create_dir_all(marker_path(&app_data, doc_id)).unwrap();
    std::fs::write(marker_path(&app_data, doc_id).join("occupied"), b"x").unwrap();

    let event = autosave::autosave_current(&runtime, &app_data, &Mutex::new(())).await;

    assert!(
        event.is_none(),
        "an autosave whose marker did not land must report failure, not success"
    );
    assert!(
        !autosave_path(&app_data, doc_id).exists(),
        "and must not leave the container behind as an unowned orphan"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// The dirty gate's own precondition
// ─────────────────────────────────────────────────────────────────────────────

/// Gating the autosave on `dirty` is only safe while everything
/// `build_save_payload` PERSISTS also sets `dirty`. The two known offenders were
/// `promote_selection` and `prepare_edge_op`, which both write `regen.elements` —
/// persisted as `doc.elements` — and used to leave the flag alone, so a promotion
/// was invisible to the close prompt and, once the gate landed, to the autosave too.
/// Both route through `DocumentRuntime::promote_selection`, which is where the flag
/// is now set, so the property holds for any future caller by construction.
///
/// **What this test can and cannot prove.** `PendingBackend` has no worker to
/// resolve a pick, so a promotion here always fails before it can mint — which
/// makes the NEGATIVE direction (a promotion that wrote nothing must not claim
/// unsaved work) the only one provable at this seam. The positive direction is
/// deliberately NOT asserted in the worker-backed slices either: every one of them
/// promotes against an already-dirty document, so the assertion would be vacuous,
/// and the only way to clear the flag first (`mark_saved`) also rewrites the
/// runtime's path and title and would perturb the slice it sits in. Recorded as a
/// gap rather than papered over with an assertion that cannot fail.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_promotion_that_mints_nothing_does_not_claim_unsaved_work() {
    let mut clean = dirty_runtime();
    let revision = clean.revision();
    clean.mark_saved(std::path::Path::new("/tmp/x.onecad"), revision);
    assert!(!clean.is_dirty(), "precondition: clean");

    let promoted = clean
        .promote_selection(
            onecad_core::ids::SnapshotId(1),
            onecad_core::ids::BodyId(Uuid::from_u128(1)),
            vec![(onecad_core::ids::TopoKey::new("f:1".to_string()), None)],
        )
        .await;

    assert!(promoted.is_err(), "no worker ⇒ no promotion");
    assert!(
        !clean.is_dirty(),
        "the flag follows the actual write, not the attempt"
    );
}
