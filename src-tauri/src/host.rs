//! Tauri adapters for the application's typed host ports.

use std::sync::Arc;

use async_trait::async_trait;
use onecad_app::host::{AppEvent, DialogPort, EventSink, FileDialog, HostContext, HostPaths};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_dialog::DialogExt;

use crate::events;

struct TauriEvents(AppHandle);

impl EventSink for TauriEvents {
    fn emit(&self, event: AppEvent) -> Result<(), String> {
        let result = match event {
            AppEvent::ProjectionUpdated(value) => self.0.emit(events::PROJECTION_UPDATED, value),
            AppEvent::DocumentChanged(value) => self.0.emit(events::DOCUMENT_CHANGED, value),
            AppEvent::RegenStarted(value) => self.0.emit(events::REGEN_STARTED, value),
            AppEvent::RegenFinished(value) => self.0.emit(events::REGEN_FINISHED, value),
            AppEvent::NeedsRepair(value) => self.0.emit(events::NEEDS_REPAIR, value),
            AppEvent::SketchSolved(value) => self.0.emit(events::SKETCH_SOLVED, value),
            AppEvent::WorkerStatus(value) => self.0.emit(events::WORKER_STATUS, value),
            AppEvent::Autosave(value) => self.0.emit(events::AUTOSAVE, value),
        };
        result.map_err(|error| error.to_string())
    }
}

struct TauriDialogs(AppHandle);

#[async_trait]
impl DialogPort for TauriDialogs {
    async fn pick(&self, request: FileDialog) -> Vec<String> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let extensions: Vec<&str> = request.extensions.iter().map(String::as_str).collect();
        let dialog = self
            .0
            .dialog()
            .file()
            .add_filter(&request.label, &extensions);
        if request.multiple {
            dialog.pick_files(move |files| {
                let _ = tx.send(files.unwrap_or_default());
            });
        } else {
            let callback = move |file: Option<tauri_plugin_dialog::FilePath>| {
                let _ = tx.send(file.into_iter().collect());
            };
            if request.save {
                dialog.save_file(callback);
            } else {
                dialog.pick_file(callback);
            }
        }
        rx.await
            .unwrap_or_default()
            .into_iter()
            .filter_map(|file| file.into_path().ok())
            .map(|path| path.to_string_lossy().into_owned())
            .collect()
    }
}

pub(crate) fn context(app: &AppHandle) -> HostContext {
    HostContext {
        paths: HostPaths {
            app_data: app.path().app_data_dir().ok(),
            app_config: app.path().app_config_dir().ok(),
        },
        events: Arc::new(TauriEvents(app.clone())),
        dialogs: Arc::new(TauriDialogs(app.clone())),
    }
}
