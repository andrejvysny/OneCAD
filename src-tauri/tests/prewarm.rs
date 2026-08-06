//! Pre-warm adoption — the second half of the worker-lifetime fix.
//!
//! `AppState::prewarm` spawns the sidecar at app start so the FIRST document open
//! adopts a worker that has already handshaken instead of paying cold start. The
//! contract asserted here is that adoption is exactly that — an *adoption*, not a
//! second spawn — and that the `live` slot it lands in retires the worker it
//! replaces (the process leak this work exists to close).
//!
//! Runs against the REAL worker binary (`ONECAD_WORKER_PATH` / the dev-tree
//! fallback), like `real_worker_smoke.rs`: `ONECAD_REQUIRE_WORKER=1` turns a missing
//! binary into a hard failure instead of a vacuously green skip.

use std::sync::Arc;
use std::time::Duration;

use onecad_core::regen::{CancelToken, GeometryEngine, Outcome, RegenRequest};

use onecad_lib::document_runtime::DocumentRuntime;
use onecad_lib::state::AppState;
use onecad_lib::worker::{resolve_worker_path, WorkerState};

/// `ONECAD_REQUIRE_WORKER` — set in CI/gate runs so "no worker binary" fails loudly
/// rather than skipping.
fn worker_required() -> bool {
    std::env::var("ONECAD_REQUIRE_WORKER").is_ok_and(|v| !v.is_empty())
}

/// `false` ⇒ skip (no binary and none demanded).
fn have_worker() -> bool {
    if resolve_worker_path().is_some() {
        return true;
    }
    assert!(
        !worker_required(),
        "ONECAD_REQUIRE_WORKER is set but no worker binary resolved"
    );
    eprintln!("skip: no worker binary resolved");
    false
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_document_open_adopts_the_prewarmed_worker_instead_of_spawning_a_second() {
    if !have_worker() {
        return;
    }
    let state = AppState::default();

    state.prewarm();
    let warm = state
        .warm_worker()
        .expect("prewarm installs a warm worker when the binary resolves");
    assert!(
        warm.wait_ready(Duration::from_secs(30)).await,
        "the pre-warmed worker completes its OCW1 handshake in the background"
    );
    let warm_epoch = warm.epoch();

    // The document-open path.
    let _backend = state.make_backend();
    state.commit_backend();

    let live = state
        .live_worker()
        .expect("opening a document installs a live worker");
    assert!(
        live.is_same(&warm),
        "the open ADOPTED the warm worker — a second spawn would be a different supervisor"
    );
    assert_eq!(
        live.epoch(),
        warm_epoch,
        "adoption is not a restart: same supervisor, same epoch"
    );
    assert_eq!(
        live.state(),
        WorkerState::Ready,
        "and the document starts on an ALREADY-ready worker (the whole point)"
    );
    assert!(
        state.warm_worker().is_none(),
        "the warm slot is consumed exactly once"
    );
    assert!(!live.is_retired());

    state.retire_all();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_next_open_retires_the_previous_documents_worker() {
    if !have_worker() {
        return;
    }
    let state = AppState::default();
    state.prewarm();
    let first = state.warm_worker().expect("warm worker");
    assert!(first.wait_ready(Duration::from_secs(30)).await);

    let _open_a = state.make_backend();
    state.commit_backend();
    assert!(state.live_worker().expect("live").is_same(&first));

    // A second open with an empty warm slot cold-spawns — and committing it MUST
    // retire the worker the previous document was using. Before this existed that
    // process stayed alive for the lifetime of the app.
    let _open_b = state.make_backend();
    let second = state.live_worker().expect("live");
    assert!(
        !second.is_same(&first),
        "the second open spawned a new worker"
    );
    assert!(
        !first.is_retired(),
        "staging alone must not retire — the open can still fail from here"
    );
    state.commit_backend();
    assert!(
        first.is_retired(),
        "THE leak: the previous document's worker must be retired once the new one commits"
    );
    assert!(
        first.wait_torn_down(Duration::from_secs(5)).await,
        "and its process must actually be reaped, not merely flagged"
    );
    assert!(!second.is_retired());

    state.retire_all();
    assert!(
        second.is_retired(),
        "quit retires the open document's worker"
    );
    assert!(state.live_worker().is_none() && state.warm_worker().is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rewarm_retires_the_closed_documents_worker_and_readies_the_next_open() {
    if !have_worker() {
        return;
    }
    let state = AppState::default();
    state.prewarm();
    let opened = state.warm_worker().expect("warm worker");
    assert!(opened.wait_ready(Duration::from_secs(30)).await);
    let _open = state.make_backend();
    state.commit_backend();
    assert!(state.live_worker().expect("live").is_same(&opened));

    // The `close_document` lane.
    state.rewarm();
    assert!(
        opened.is_retired(),
        "a closed document's worker can serve nothing — retire it"
    );
    assert!(opened.wait_torn_down(Duration::from_secs(5)).await);
    assert!(state.live_worker().is_none(), "the live slot is cleared");

    let next = state
        .warm_worker()
        .expect("close pre-warms the replacement so the next open is instant");
    assert!(!next.is_same(&opened));
    assert!(next.wait_ready(Duration::from_secs(30)).await);

    // Pre-warming twice must never stack processes.
    state.prewarm();
    assert!(state.warm_worker().expect("warm").is_same(&next));

    state.retire_all();
}

// ─────────────────────────────────────────────────────────────────────────────
// THE REGRESSION: a failed open must leave the surviving document whole
// ─────────────────────────────────────────────────────────────────────────────

/// Every open command calls `make_backend` **before** the work that can fail, while
/// the PREVIOUS document is still the live one. Retiring the displaced worker there
/// killed a perfectly good document's sidecar whenever a user picked an unreadable
/// file — and `appStore.importStep` deliberately keeps the user on that document
/// when the probe fails, so this was reachable from the UI.
///
/// Driven through [`open_runtime_over_new_backend`] — the real expression
/// `open_document` runs, backend swap and all — so this pins the production error
/// path rather than a transcription of it. (The command itself takes a concrete
/// `AppHandle`, which `tauri::test::mock_app`'s `MockRuntime` cannot produce; that is
/// why the fallible half is factored out.)
///
/// [`open_runtime_over_new_backend`]: onecad_lib::api::open_runtime_over_new_backend
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_open_leaves_the_previous_documents_worker_alive() {
    if !have_worker() {
        return;
    }
    let state = AppState::default();
    state.prewarm();
    let warm = state.warm_worker().expect("warm worker");
    assert!(warm.wait_ready(Duration::from_secs(30)).await);

    // Document A is open, on the pre-warmed worker.
    let (engine, meshes, solver) = state.make_backend();
    state.commit_backend();
    *state.runtime.lock().await = Some(DocumentRuntime::new_blank(engine, meshes, solver));
    let live_before = state.live_worker().expect("the open document has a worker");
    assert!(
        live_before.is_same(&warm),
        "document A adopted the warm worker"
    );

    // Snapshot the side seams document A reads through, so the pre-existing
    // half-break (they were swapped out from under it) is pinned too.
    let seams_before = (
        state.exporter(),
        state.element_query(),
        state.preview(),
        state.face_projection(),
        state.step_import(),
        state.circuit(),
    );

    // …and now the user picks a file that is not there. This is the exact expression
    // `open_document` runs, backend swap and all.
    let missing = std::env::temp_dir().join("onecad-no-such-document-9f3a.onecad");
    assert!(!missing.exists());
    let result = onecad_lib::api::open_runtime_over_new_backend(&state, &missing);
    assert!(
        result.is_err(),
        "opening a nonexistent container must fail, not succeed silently"
    );

    // Document A is still the open document, and it must still WORK.
    let live_after = state
        .live_worker()
        .expect("the surviving document keeps a worker");
    assert!(
        live_after.is_same(&live_before),
        "the failed open must hand document A its own worker back"
    );
    assert!(
        !live_before.is_retired(),
        "REGRESSION: a failed open retired the surviving document's worker"
    );
    assert_eq!(live_before.state(), WorkerState::Ready);
    // A real wire round-trip: the sidecar process is alive and serving, not merely
    // un-flagged. This is the assertion a transcribed test could not make.
    GeometryEngine::ping(&live_before)
        .await
        .expect("document A's engine still answers over OCW1");
    // The assertion that actually matters: the engine Arc DOCUMENT A HOLDS — the one
    // its regen dispatches through — still answers. A retired manager would fail this
    // with `EngineError::Crashed { "worker retired" }`.
    let doc_engine = state
        .runtime
        .lock()
        .await
        .as_ref()
        .expect("document A is still open")
        .engine_arc();
    doc_engine
        .ping()
        .await
        .expect("document A's OWN engine still serves after the failed open");
    let outcome = state
        .runtime
        .lock()
        .await
        .as_mut()
        .expect("document A is still open")
        .run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await
        .outcome;
    assert!(
        !matches!(outcome, Outcome::EngineFailed(_)),
        "document A's regen must not fail on a retired worker, got {outcome:?}"
    );

    let seams_after = (
        state.exporter(),
        state.element_query(),
        state.preview(),
        state.face_projection(),
        state.step_import(),
        state.circuit(),
    );
    assert!(
        Arc::ptr_eq(&seams_before.0, &seams_after.0),
        "exporter restored"
    );
    assert!(
        Arc::ptr_eq(&seams_before.1, &seams_after.1),
        "element query restored"
    );
    assert!(
        Arc::ptr_eq(&seams_before.2, &seams_after.2),
        "preview restored"
    );
    assert!(
        Arc::ptr_eq(&seams_before.3, &seams_after.3),
        "face projection restored"
    );
    assert!(
        Arc::ptr_eq(&seams_before.4, &seams_after.4),
        "step import restored"
    );
    assert!(
        Arc::ptr_eq(&seams_before.5, &seams_after.5),
        "circuit control restored"
    );

    state.retire_all();
}
