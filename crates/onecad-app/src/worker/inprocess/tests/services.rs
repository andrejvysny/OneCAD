//! Production call sites start their workers on the PROCESS's backend
//! (`ProcessServices::with_backend`), never on the build default.

use std::sync::Arc;

use crate::host::{AppEvent, DialogPort, EventSink, FileDialog, HostContext, HostPaths};
use crate::process::ProcessServices;
use crate::state::AppState;

use super::*;

struct Quiet;

impl EventSink for Quiet {
    fn emit(&self, _: AppEvent) -> Result<(), String> {
        Ok(())
    }
}

#[async_trait::async_trait]
impl DialogPort for Quiet {
    async fn pick(&self, _: FileDialog) -> Vec<String> {
        Vec::new()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_app_ingest_runs_on_the_process_backend() {
    let _gate = GATE.lock().await;
    if !kernel_linked() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    // This test build's default backend is the sidecar; the process says otherwise.
    let state = AppState::with_services(Arc::new(ProcessServices::with_backend(embedded(None))));
    let host = HostContext {
        paths: HostPaths {
            app_data: Some(dir.path().into()),
            app_config: Some(dir.path().join("config")),
        },
        events: Arc::new(Quiet),
        dialogs: Arc::new(Quiet),
    };
    let before = in_process_counters();
    let defaults = crate::dto::IngestDefaultsDto {
        vendor: "test".into(),
        category: Vec::new(),
        tags: Vec::new(),
    };
    let library = dir.path().join("library").to_string_lossy().into_owned();
    let report = crate::api::ingest_components(&state, host, Vec::new(), Some(library), defaults)
        .await
        .expect("an empty ingest succeeds");
    assert!(report.parts.is_empty(), "{report:?}");
    assert_eq!(
        in_process_counters().launched,
        before.launched + 1,
        "the ingest worker ran in-process"
    );
    assert!(
        eventually(Duration::from_secs(5), || in_process_counters().live
            == before.live)
        .await,
        "and was retired with the ingest"
    );
}
