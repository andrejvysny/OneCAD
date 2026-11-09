use async_trait::async_trait;
use onecad_app::{
    api,
    dto::{DocumentProjection, FeatureStatus, RegenFinished},
    host::{AppEvent, DialogPort, EventSink, FileDialog, HostContext, HostPaths},
    state::AppState,
};
use std::{
    collections::VecDeque,
    io::Write,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

pub struct Events(
    Mutex<VecDeque<AppEvent>>,
    Mutex<std::fs::File>,
    Mutex<usize>,
);
impl Events {
    pub fn dropped(&self) -> usize {
        *self.2.lock().unwrap()
    }
    pub fn records(&self) -> Vec<AppEvent> {
        self.0.lock().unwrap().iter().cloned().collect()
    }
}
impl EventSink for Events {
    fn emit(&self, event: AppEvent) -> Result<(), String> {
        if let AppEvent::RegenFinished(ref report) = event {
            let mut file = self.1.lock().map_err(|e| e.to_string())?;
            serde_json::to_writer(&mut *file, report).map_err(|e| e.to_string())?;
            writeln!(file).map_err(|e| e.to_string())?;
            file.flush().map_err(|e| e.to_string())?;
        }
        if matches!(event, AppEvent::RegenFinished(_)) {
            let mut rows = self.0.lock().map_err(|e| e.to_string())?;
            if rows.len() == 128 {
                rows.pop_front();
                *self.2.lock().map_err(|e| e.to_string())? += 1;
            }
            rows.push_back(event);
        }
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
    pub events: Arc<Events>,
    root: std::path::PathBuf,
}

impl Harness {
    pub async fn new(root: &Path) -> Self {
        std::fs::create_dir_all(root).unwrap();
        let journal = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(root.join("terminal-events.jsonl"))
            .unwrap();
        let events = Arc::new(Events(
            Mutex::new(VecDeque::new()),
            Mutex::new(journal),
            Mutex::new(0),
        ));
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
            root: root.to_path_buf(),
        }
    }

    pub async fn projection(&self) -> DocumentProjection {
        tokio::time::timeout(Duration::from_secs(5), api::get_projection(&self.state))
            .await
            .expect("projection exceeded 5 seconds")
            .unwrap()
    }

    pub async fn manual_save(&self, label: &str) -> Result<(), String> {
        let path = self.root.join("native-authoring.onecad");
        let target = if path.exists() {
            None
        } else {
            Some(path.to_string_lossy().into_owned())
        };
        let result = tokio::time::timeout(
            Duration::from_secs(30),
            api::save_document(&self.state, self.host.clone(), target, None),
        )
        .await;
        let status = match &result {
            Ok(Ok(outcome)) if outcome.clean => "saved_clean",
            Ok(Ok(_)) => "saved_dirty",
            Ok(Err(_)) => "save_error",
            Err(_) => "save_timeout",
        };
        let mut journal = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(self.root.join("manual-saves.jsonl"))
            .map_err(|e| e.to_string())?;
        serde_json::to_writer(&mut journal, &serde_json::json!({"label":label,"status":status,
            "detail":format!("{result:?}"),"checkpoint":"best-effort; inspect worker protocol log"})).map_err(|e| e.to_string())?;
        writeln!(journal).map_err(|e| e.to_string())?;
        journal.flush().map_err(|e| e.to_string())?;
        if status == "saved_clean" {
            Ok(())
        } else {
            Err(format!("{label}: {status}: {result:?}"))
        }
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

    pub async fn settled(&self, source: &DocumentProjection) -> RegenFinished {
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
        report
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
                        && report.source_revision == source.revision
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
