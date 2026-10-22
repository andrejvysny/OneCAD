//! Explicit host resources shared by application services and worker callbacks.

use std::{path::PathBuf, sync::Arc};

use async_trait::async_trait;
use serde::Serialize;

use crate::{autosave::AutosaveEvent, dto::*};

#[derive(Debug, Clone, Default)]
pub struct HostPaths {
    pub app_data: Option<PathBuf>,
    pub app_config: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentChangedEvent {
    #[serde(flatten)]
    pub change: DocumentChange,
    #[cfg_attr(not(debug_assertions), serde(skip_serializing_if = "Option::is_none"))]
    pub render_expectation_id: Option<u64>,
}

#[derive(Debug, Clone)]
pub enum AppEvent {
    ProjectionUpdated(DocumentProjection),
    DocumentChanged(DocumentChangedEvent),
    RegenStarted(RegenStarted),
    RegenFinished(RegenFinished),
    NeedsRepair(NeedsRepairEvent),
    SketchSolved(SketchUpsertDto),
    WorkerStatus(WorkerStatusDto),
    Autosave(AutosaveEvent),
}

pub trait EventSink: Send + Sync {
    fn emit(&self, event: AppEvent) -> Result<(), String>;
}

#[derive(Debug, Clone)]
pub struct FileDialog {
    pub label: String,
    pub extensions: Vec<String>,
    pub save: bool,
    pub multiple: bool,
    pub suggested_filename: Option<String>,
}

impl FileDialog {
    pub fn new(label: &str, extensions: &[&str], save: bool) -> Self {
        Self {
            label: label.to_owned(),
            extensions: extensions.iter().map(|value| (*value).to_owned()).collect(),
            save,
            multiple: false,
            suggested_filename: None,
        }
    }
}

#[async_trait]
pub trait DialogPort: Send + Sync {
    async fn pick(&self, request: FileDialog) -> Vec<String>;
}

#[derive(Clone)]
pub struct HostContext {
    pub paths: HostPaths,
    pub events: Arc<dyn EventSink>,
    pub dialogs: Arc<dyn DialogPort>,
}

impl HostContext {
    pub fn emit(&self, event: AppEvent) -> Result<(), String> {
        self.events.emit(event)
    }
}
