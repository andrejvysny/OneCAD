//! Several document hosts in one process: ownership of paths and document
//! identities, the open-document cap, and isolation between hosts.
//!
//! Ownership tests run on the no-worker backend (fast, deterministic); the
//! isolation test needs the REAL worker (`ONECAD_REQUIRE_WORKER=1` makes a missing
//! binary fail instead of skip).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use onecad_app::api;
use onecad_app::error::ApiError;
use onecad_app::host::{AppEvent, DialogPort, EventSink, FileDialog, HostContext, HostPaths};
use onecad_app::process::ProcessServices;
use onecad_app::reservations::MAX_OPEN_DOCUMENTS;
use onecad_app::state::{AppState, BackendBundle};
use onecad_app::worker::{resolve_worker_path, PendingBackend};

#[derive(Default)]
struct Events(Mutex<Vec<AppEvent>>);

impl EventSink for Events {
    fn emit(&self, event: AppEvent) -> Result<(), String> {
        self.0.lock().unwrap().push(event);
        Ok(())
    }
}

struct NoDialogs;

#[async_trait]
impl DialogPort for NoDialogs {
    async fn pick(&self, _: FileDialog) -> Vec<String> {
        Vec::new()
    }
}

fn host(root: &Path) -> HostContext {
    HostContext {
        paths: HostPaths {
            app_data: Some(root.join("data")),
            app_config: Some(root.join("config")),
        },
        events: Arc::new(Events::default()),
        dialogs: Arc::new(NoDialogs),
    }
}

fn pending_bundle() -> BackendBundle {
    let b = Arc::new(PendingBackend);
    (
        b.clone(),
        b.clone(),
        b.clone(),
        b.clone(),
        b.clone(),
        b.clone(),
        b.clone(),
        b.clone(),
        b.clone(),
        b,
    )
}

fn pending_host(services: &Arc<ProcessServices>, root: &Path) -> (AppState, HostContext) {
    let state = AppState::new_with_services(Arc::new(pending_bundle), services.clone());
    let ctx = host(root);
    state.start(ctx.clone()).expect("host starts");
    (state, ctx)
}

/// A saved document at `dir/name.onecad`, written by a throwaway host.
async fn saved_document(services: &Arc<ProcessServices>, root: &Path, name: &str) -> PathBuf {
    let (state, ctx) = pending_host(services, root);
    api::new_document(&state, ctx.clone()).await.unwrap();
    let path = root.join(format!("{name}.onecad"));
    api::save_document(
        &state,
        ctx.clone(),
        Some(path.to_string_lossy().into()),
        None,
    )
    .await
    .unwrap();
    api::shutdown_host(&state, ctx, None).await.unwrap();
    path
}

fn s(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_path_open_in_one_host_is_focused_not_reopened_by_another() {
    let root = tempfile::tempdir().unwrap();
    let services = Arc::new(ProcessServices::new());
    let path = saved_document(&services, root.path(), "Bracket").await;
    let (a, a_ctx) = pending_host(&services, root.path());
    let (b, b_ctx) = pending_host(&services, root.path());
    api::open_document(&a, a_ctx.clone(), s(&path), None)
        .await
        .unwrap();

    let refused = api::open_document(&b, b_ctx.clone(), s(&path), None).await;
    match refused {
        Err(ApiError::AlreadyOpen { host, .. }) => assert_eq!(host, a.host_id()),
        other => panic!("expected AlreadyOpen naming A, got {other:?}"),
    }
    assert!(b.runtime.lock().await.is_none(), "B stays empty");

    api::shutdown_host(&a, a_ctx, None).await.unwrap();
    api::open_document(&b, b_ctx.clone(), s(&path), None)
        .await
        .expect("free once A has shut down");
    api::shutdown_host(&b, b_ctx, None).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_copied_file_with_the_same_identity_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let services = Arc::new(ProcessServices::new());
    let path = saved_document(&services, root.path(), "Original").await;
    let copy = root.path().join("Copy.onecad");
    std::fs::copy(&path, &copy).unwrap();
    let (a, a_ctx) = pending_host(&services, root.path());
    let (b, b_ctx) = pending_host(&services, root.path());
    api::open_document(&a, a_ctx.clone(), s(&path), None)
        .await
        .unwrap();

    let refused = api::open_document(&b, b_ctx.clone(), s(&copy), None).await;
    assert!(
        matches!(refused, Err(ApiError::DuplicateDocument { host, .. }) if host == a.host_id()),
        "got {refused:?}"
    );
    assert!(b.runtime.lock().await.is_none());
    api::open_document(&b, b_ctx.clone(), s(&copy), None)
        .await
        .expect_err("still refused while A holds the identity");
    // The refused open released its path lease: A can take the copy's path later.
    assert_eq!(services.reservations().path_owner(&copy), None);
    api::shutdown_host(&a, a_ctx, None).await.unwrap();
    api::shutdown_host(&b, b_ctx, None).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_open_document_cap_refuses_the_next_and_frees_on_close() {
    let root = tempfile::tempdir().unwrap();
    let services = Arc::new(ProcessServices::new());
    let mut hosts = Vec::new();
    for _ in 0..MAX_OPEN_DOCUMENTS {
        let (state, ctx) = pending_host(&services, root.path());
        api::new_document(&state, ctx.clone()).await.unwrap();
        hosts.push((state, ctx));
    }
    let (extra, extra_ctx) = pending_host(&services, root.path());
    let refused = api::new_document(&extra, extra_ctx.clone()).await;
    assert!(
        matches!(refused, Err(ApiError::TooManyDocuments(_))),
        "got {refused:?}"
    );
    assert!(extra.runtime.lock().await.is_none());

    let (first, first_ctx) = hosts.remove(0);
    api::shutdown_host(&first, first_ctx, None).await.unwrap();
    api::new_document(&extra, extra_ctx.clone())
        .await
        .expect("room after one document closed");

    // An occupying host may still replace its own document at the cap.
    let (last, last_ctx) = hosts.last().unwrap();
    api::new_document(last, last_ctx.clone())
        .await
        .expect("a replacement in the same host is not a new document");

    for (state, ctx) in hosts {
        api::shutdown_host(&state, ctx, None).await.unwrap();
    }
    api::shutdown_host(&extra, extra_ctx, None).await.unwrap();
    assert_eq!(services.reservations().open_documents(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn save_as_onto_another_hosts_file_is_refused_and_leaves_it_untouched() {
    let root = tempfile::tempdir().unwrap();
    let services = Arc::new(ProcessServices::new());
    let path = saved_document(&services, root.path(), "Owned").await;
    let before = std::fs::read(&path).unwrap();
    let (a, a_ctx) = pending_host(&services, root.path());
    let (b, b_ctx) = pending_host(&services, root.path());
    api::open_document(&a, a_ctx.clone(), s(&path), None)
        .await
        .unwrap();
    api::new_document(&b, b_ctx.clone()).await.unwrap();

    let refused = api::save_document(&b, b_ctx.clone(), Some(s(&path)), None).await;
    assert!(
        matches!(refused, Err(ApiError::AlreadyOpen { .. })),
        "got {refused:?}"
    );
    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "A's file is untouched"
    );

    // A successful Save As moves B's ownership to the new path.
    let mine = root.path().join("Mine.onecad");
    api::save_document(&b, b_ctx.clone(), Some(s(&mine)), None)
        .await
        .unwrap();
    assert_eq!(services.reservations().path_owner(&mine), Some(b.host_id()));
    let other = root.path().join("Renamed.onecad");
    api::save_document(&b, b_ctx.clone(), Some(s(&other)), None)
        .await
        .unwrap();
    assert_eq!(
        services.reservations().path_owner(&mine),
        None,
        "the previous path is released after Save As"
    );
    api::shutdown_host(&a, a_ctx, None).await.unwrap();
    api::shutdown_host(&b, b_ctx, None).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recents_rename_and_delete_refuse_an_open_file() {
    let root = tempfile::tempdir().unwrap();
    let services = Arc::new(ProcessServices::new());
    let path = saved_document(&services, root.path(), "Busy").await;
    let (a, a_ctx) = pending_host(&services, root.path());
    api::open_document(&a, a_ctx.clone(), s(&path), None)
        .await
        .unwrap();

    assert!(
        api::rename_recent_project(&a, a_ctx.clone(), s(&path), "Other".into())
            .await
            .is_err()
    );
    assert!(api::delete_recent_project(&a, a_ctx.clone(), s(&path))
        .await
        .is_err());
    assert!(path.exists(), "the open document's file is still there");
    api::shutdown_host(&a, a_ctx, None).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_real_hosts_edit_and_close_independently() {
    if resolve_worker_path().is_none() {
        assert!(
            std::env::var("ONECAD_REQUIRE_WORKER").map_or(true, |v| v.is_empty()),
            "ONECAD_REQUIRE_WORKER is set but no worker binary resolved"
        );
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let services = Arc::new(ProcessServices::new());
    let start = |root: &Path| {
        let state = AppState::with_services(services.clone());
        let ctx = host(root);
        state.start(ctx.clone()).unwrap();
        (state, ctx)
    };
    let (a, a_ctx) = start(root.path());
    let (b, b_ctx) = start(root.path());
    api::new_document(&a, a_ctx.clone()).await.unwrap();
    api::new_document(&b, b_ctx.clone()).await.unwrap();
    let a_session = a.runtime.lock().await.as_ref().unwrap().runtime_session();
    let b_revision = b.runtime.lock().await.as_ref().unwrap().revision();

    let a_path = root.path().join("A.onecad");
    api::save_document(&a, a_ctx.clone(), Some(s(&a_path)), None)
        .await
        .unwrap();
    assert_eq!(
        b.runtime.lock().await.as_ref().unwrap().revision(),
        b_revision,
        "work in A never touches B"
    );

    api::shutdown_host(&a, a_ctx, None).await.unwrap();
    let b_worker = b.live_worker().unwrap();
    assert!(b_worker.wait_ready(Duration::from_secs(30)).await);
    assert!(
        !b.runtime
            .lock()
            .await
            .as_ref()
            .unwrap()
            .matches_runtime_session(&a_session),
        "distinct runtime sessions fence cross-host results"
    );
    api::shutdown_host(&b, b_ctx, None).await.unwrap();
    services.retire_warm();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_close_keeps_the_host_whole_and_its_file_owned() {
    let root = tempfile::tempdir().unwrap();
    let services = Arc::new(ProcessServices::new());
    let path = saved_document(&services, root.path(), "Kept").await;
    let (a, a_ctx) = pending_host(&services, root.path());
    let (b, b_ctx) = pending_host(&services, root.path());
    api::open_document(&a, a_ctx.clone(), s(&path), None)
        .await
        .unwrap();

    let refused = api::shutdown_host(&a, a_ctx.clone(), Some("stale-session")).await;
    assert!(refused.is_err(), "stale discard consent is refused");
    assert!(a.runtime.lock().await.is_some(), "A keeps its document");
    assert_eq!(services.reservations().path_owner(&path), Some(a.host_id()));
    assert!(
        api::open_document(&b, b_ctx.clone(), s(&path), None)
            .await
            .is_err(),
        "no second writer can open A's file"
    );
    api::new_document(&a, a_ctx.clone())
        .await
        .expect("admission reopens after a refused close");

    api::shutdown_host(&a, a_ctx, None).await.unwrap();
    api::shutdown_host(&b, b_ctx, None).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn closing_right_after_an_edit_does_not_wait_out_the_autosave_debounce() {
    let root = tempfile::tempdir().unwrap();
    let services = Arc::new(ProcessServices::new());
    let (a, a_ctx) = pending_host(&services, root.path());
    api::new_document(&a, a_ctx.clone()).await.unwrap();
    a.note_mutation();
    tokio::time::sleep(Duration::from_millis(50)).await;
    let started = std::time::Instant::now();
    api::shutdown_host(&a, a_ctx, None).await.unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "close took {:?}; the autosave lane must observe the stop mid-debounce",
        started.elapsed()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_closing_host_admits_no_new_document() {
    let root = tempfile::tempdir().unwrap();
    let services = Arc::new(ProcessServices::new());
    let (a, a_ctx) = pending_host(&services, root.path());
    api::new_document(&a, a_ctx.clone()).await.unwrap();
    api::shutdown_host(&a, a_ctx.clone(), None).await.unwrap();
    assert!(api::new_document(&a, a_ctx).await.is_err());
    assert_eq!(services.reservations().open_documents(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_saved_document_reports_no_changes_and_a_fresh_one_reports_its_state() {
    let root = tempfile::tempdir().unwrap();
    let services = Arc::new(ProcessServices::new());
    let (a, a_ctx) = pending_host(&services, root.path());
    assert_eq!(
        api::unsaved_changes(&a).await.unwrap().count,
        0,
        "nothing open"
    );
    api::new_document(&a, a_ctx.clone()).await.unwrap();
    let path = root.path().join("Saved.onecad");
    api::save_document(&a, a_ctx.clone(), Some(s(&path)), None)
        .await
        .unwrap();
    let changes = api::unsaved_changes(&a).await.unwrap();
    assert_eq!(changes.count, 0);
    assert_eq!(changes.labels, Some(Vec::new()));
    api::shutdown_host(&a, a_ctx, None).await.unwrap();
}
