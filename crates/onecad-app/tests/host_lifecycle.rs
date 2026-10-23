//! Document-host lifetime on a shared process: several `AppState` hosts share one
//! Tokio runtime and one `ProcessServices`, so closing one host must stop its own
//! lanes and reap its own worker without touching the others or the shared pool.
//!
//! Runs against the REAL worker binary; `ONECAD_REQUIRE_WORKER=1` turns a missing
//! binary into a failure instead of a skip.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use onecad_app::api;
use onecad_app::host::{AppEvent, DialogPort, EventSink, FileDialog, HostContext, HostPaths};
use onecad_app::process::ProcessServices;
use onecad_app::state::AppState;
use onecad_app::worker::{resolve_worker_path, WorkerState};

fn have_worker() -> bool {
    if resolve_worker_path().is_some() {
        return true;
    }
    assert!(
        std::env::var("ONECAD_REQUIRE_WORKER").map_or(true, |v| v.is_empty()),
        "ONECAD_REQUIRE_WORKER is set but no worker binary resolved"
    );
    eprintln!("skip: no worker binary resolved");
    false
}

#[derive(Default)]
struct Events(Mutex<Vec<AppEvent>>);

impl EventSink for Events {
    fn emit(&self, event: AppEvent) -> Result<(), String> {
        self.0.lock().unwrap().push(event);
        Ok(())
    }
}

impl Events {
    fn worker_statuses(&self) -> usize {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|e| matches!(e, AppEvent::WorkerStatus(_)))
            .count()
    }
}

struct NoDialogs;

#[async_trait]
impl DialogPort for NoDialogs {
    async fn pick(&self, _: FileDialog) -> Vec<String> {
        Vec::new()
    }
}

fn host(root: &Path, events: Arc<Events>) -> HostContext {
    HostContext {
        paths: HostPaths {
            app_data: Some(root.join("data")),
            app_config: Some(root.join("config")),
        },
        events,
        dialogs: Arc::new(NoDialogs),
    }
}

async fn started(
    services: &Arc<ProcessServices>,
    root: &Path,
) -> (AppState, HostContext, Arc<Events>) {
    let events = Arc::new(Events::default());
    let ctx = host(root, events.clone());
    let state = AppState::with_services(services.clone());
    state.start(ctx.clone()).expect("host starts");
    (state, ctx, events)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutting_one_host_down_reaps_its_worker_and_spares_the_other() {
    if !have_worker() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let services = Arc::new(ProcessServices::new());
    let (a, a_ctx, _) = started(&services, root.path()).await;
    let (b, b_ctx, _) = started(&services, root.path()).await;
    api::new_document(&a, a_ctx.clone()).await.expect("A opens");
    api::new_document(&b, b_ctx.clone()).await.expect("B opens");
    let a_worker = a.live_worker().expect("A has a worker");
    let b_worker = b.live_worker().expect("B has a worker");
    assert!(!a_worker.is_same(&b_worker), "one worker per open document");

    api::shutdown_host(&a, a_ctx, None)
        .await
        .expect("A shuts down");

    assert!(a.live_worker().is_none());
    assert!(
        a_worker.is_torn_down(),
        "A's process is reaped before shutdown returns"
    );
    assert!(a.runtime.lock().await.is_none());
    assert!(b_worker.wait_ready(Duration::from_secs(30)).await);
    assert_eq!(b_worker.state(), WorkerState::Ready, "B keeps its worker");
    assert!(b.runtime.lock().await.is_some(), "B keeps its document");

    api::shutdown_host(&b, b_ctx, None)
        .await
        .expect("B shuts down");
    services.retire_warm();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repeated_open_close_on_one_runtime_leaks_no_worker() {
    if !have_worker() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let services = Arc::new(ProcessServices::new());
    let mut workers = Vec::new();
    for _ in 0..8 {
        let (state, ctx, _) = started(&services, root.path()).await;
        api::new_document(&state, ctx.clone()).await.expect("opens");
        workers.push(state.live_worker().expect("worker"));
        api::shutdown_host(&state, ctx, None).await.expect("closes");
    }
    assert!(
        workers.iter().all(|w| w.is_torn_down()),
        "every closed host's worker process was reaped"
    );
    let warm = services.warm_worker();
    assert!(
        warm.iter().all(|w| !w.is_retired()),
        "the shared pool is never retired by a host close"
    );
    services.retire_warm();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_shutdown_retires_its_worker_while_disk_cleanup_waits() {
    if !have_worker() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let services = Arc::new(ProcessServices::new());
    let (state, ctx, _) = started(&services, root.path()).await;
    api::new_document(&state, ctx.clone()).await.unwrap();
    let worker = state.live_worker().expect("document has a worker");
    let lane = state.persistence.lock().await;
    let mut shutdown = Box::pin(api::shutdown_host(&state, ctx, None));
    std::future::poll_fn(|cx| {
        assert!(std::future::Future::poll(shutdown.as_mut(), cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    assert!(state.runtime.lock().await.is_none());
    assert!(state.live_worker().is_none());
    assert!(worker.is_retired());
    drop(shutdown);
    drop(lane);
    assert!(worker.wait_torn_down(Duration::from_secs(30)).await);
    tokio::time::timeout(Duration::from_secs(5), async {
        while services.reservations().open_documents() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("detached cleanup releases the closed claims");
    services.retire_warm();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pooled_worker_reports_status_only_to_the_host_that_adopts_it() {
    if !have_worker() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let services = Arc::new(ProcessServices::new());
    let (a, a_ctx, a_events) = started(&services, root.path()).await;
    let warm = services
        .warm_worker()
        .expect("A's start pre-warmed the pool");
    let (b, b_ctx, b_events) = started(&services, root.path()).await;
    assert!(
        services.warm_worker().is_some_and(|w| w.is_same(&warm)),
        "a second host never stacks a second warm worker"
    );

    api::new_document(&b, b_ctx.clone()).await.expect("B opens");
    let adopted = b.live_worker().expect("B adopted");
    assert!(adopted.is_same(&warm), "B adopted the pool's worker");
    tokio::time::sleep(Duration::from_millis(200)).await;

    assert!(
        b_events.worker_statuses() > 0,
        "the adopter receives worker status"
    );
    assert_eq!(
        a_events.worker_statuses(),
        0,
        "the host that merely pre-warmed never hears from a worker it does not own"
    );

    api::shutdown_host(&b, b_ctx, None)
        .await
        .expect("B shuts down");
    api::shutdown_host(&a, a_ctx, None)
        .await
        .expect("A shuts down");
    services.retire_warm();
}
