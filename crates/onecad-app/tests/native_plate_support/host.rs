use async_trait::async_trait;
use onecad_app::{
    api,
    dto::{DocumentProjection, FeatureStatus, RegenFinished},
    host::{AppEvent, DialogPort, EventSink, FileDialog, HostContext, HostPaths},
    state::AppState,
};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Default)]
pub struct Events(Mutex<Vec<AppEvent>>);
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
        panic!("workflow supplies explicit QA paths; no dialog is expected")
    }
}

pub struct Harness {
    pub state: AppState,
    pub host: HostContext,
    events: Arc<Events>,
}

impl Harness {
    pub async fn new(root: &Path) -> Self {
        std::fs::create_dir_all(root).unwrap();
        let events = Arc::new(Events::default());
        let host = HostContext {
            paths: HostPaths {
                app_data: Some(root.join("data")),
                app_config: Some(root.join("config")),
            },
            events: events.clone(),
            dialogs: Arc::new(NoDialogs),
        };
        let state = AppState::default();
        state
            .start(host.clone())
            .expect("shared application startup");
        Self {
            state,
            host,
            events,
        }
    }

    pub async fn projection(&self) -> DocumentProjection {
        api::get_projection(&self.state).await.unwrap()
    }

    pub fn snapshot_id(&self) -> u64 {
        self.events
            .0
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find_map(|event| match event {
                AppEvent::DocumentChanged(event) => Some(event.change.snapshot_id),
                _ => None,
            })
            .expect("scheduler published a document-changed event")
    }

    pub fn document_event_count(&self) -> usize {
        self.events
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    AppEvent::DocumentChanged(_) | AppEvent::ProjectionUpdated(_)
                )
            })
            .count()
    }

    pub async fn settled(&self, source: &DocumentProjection) -> DocumentProjection {
        let report = self.terminal(source).await;
        assert!(
            matches!(report.outcome.as_str(), "published" | "noop"),
            "{report:?}"
        );
        assert!(report.failed_steps.is_empty(), "{report:?}");
        assert!(report.repair_steps.is_empty(), "{report:?}");
        let projection = self.projection().await;
        assert!(
            projection
                .features
                .iter()
                .all(|feature| feature.status == FeatureStatus::Ok),
            "{projection:?}"
        );
        projection
    }

    pub async fn terminal(&self, source: &DocumentProjection) -> RegenFinished {
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                if let Some(report) = self.completion(source) {
                    return report;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("real-worker scheduler must complete requested revision")
    }

    fn completion(&self, source: &DocumentProjection) -> Option<RegenFinished> {
        self.events
            .0
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find_map(|event| match event {
                AppEvent::RegenFinished(report)
                    if report.runtime_session == source.runtime_session
                        && (report.source_revision >= source.revision
                            || report.revision >= source.revision)
                        && report.outcome != "superseded" =>
                {
                    Some(report.clone())
                }
                _ => None,
            })
    }

    pub async fn close(&self) {
        let worker = self.state.live_worker().expect("real worker was adopted");
        api::close_document(&self.state, self.host.clone())
            .await
            .unwrap();
        self.state.retire_all();
        assert!(
            worker.wait_torn_down(Duration::from_secs(10)).await,
            "worker process must exit"
        );
        assert!(self.projection().await.document_id.is_empty());
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.state.retire_all();
    }
}

pub fn required_worker() -> PathBuf {
    let path = PathBuf::from(
        std::env::var_os("ONECAD_WORKER_PATH")
            .expect("ONECAD_WORKER_PATH is required; this workflow never skips the real worker"),
    );
    assert!(path.is_file(), "real worker missing: {}", path.display());
    path
}

/// Each host gets private import-sweep, library and logging paths before startup.
pub fn run_isolated_phase(root: &Path, phase: &str, test_name: &str) {
    let host_root = root.join(format!("{phase}-host"));
    let temp = host_root.join("temp");
    let library = host_root.join("data/library");
    let logs = host_root.join("logs");
    for directory in [&temp, &library, &logs] {
        std::fs::create_dir_all(directory).unwrap();
    }
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test_name, "--nocapture"])
        .env("ONECAD_NATIVE_PLATE_TEST_PHASE", phase)
        .env("ONECAD_NATIVE_PLATE_TEST_ROOT", root)
        .env("TMPDIR", temp)
        .env("ONECAD_LIBRARY_ROOT", library)
        .env("ONECAD_LOG_DIR", logs)
        .status()
        .unwrap();
    assert!(
        status.success(),
        "fresh-process {phase} phase failed: {status}"
    );
}
