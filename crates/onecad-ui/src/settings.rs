//! Profile-local chrome preferences; document state and transient editors never persist.
use crate::{
    qa::QaPaths,
    ui_state::{Appearance, InspectorTab, UiState},
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    thread::JoinHandle,
};

const VERSION: u32 = 1;
const MAX_BYTES: u64 = 64 * 1024;
const FILE_NAME: &str = "native-settings.json";

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Preferences {
    version: u32,
    appearance: Appearance,
    explorer_open: bool,
    inspector_open: bool,
    inspector_tab: InspectorTab,
    recent_view: usize,
}

impl Preferences {
    fn capture(ui: &UiState) -> Self {
        Self {
            version: VERSION,
            appearance: ui.appearance,
            explorer_open: ui.explorer_open,
            inspector_open: ui.inspector_open,
            inspector_tab: ui.inspector_tab,
            recent_view: ui.recent_view,
        }
    }

    fn apply(&self, ui: &mut UiState) {
        ui.appearance = self.appearance;
        ui.explorer_open = self.explorer_open;
        ui.inspector_open = self.inspector_open;
        ui.inspector_tab = self.inspector_tab;
        ui.recent_view = self.recent_view;
    }
}

pub struct Store {
    path: PathBuf,
    current: Preferences,
    desired: Preferences,
    blocked: bool,
    writer: Option<JoinHandle<Result<Preferences, String>>>,
    context: egui::Context,
    // Shutdown must finish profile writes before releasing the exclusive profile lock.
    _profile: Option<QaPaths>,
}

impl Store {
    pub fn load(paths: &QaPaths, context: egui::Context) -> (Self, UiState, Option<String>) {
        let (mut store, ui, warning) = Self::load_at(&paths.config, context);
        store._profile = Some(paths.clone());
        (store, ui, warning)
    }

    fn load_at(config: &Path, context: egui::Context) -> (Self, UiState, Option<String>) {
        let path = config.join(FILE_NAME);
        let mut ui = UiState::default();
        let warning = match read(&path) {
            Ok(Some(preferences)) => {
                preferences.apply(&mut ui);
                None
            }
            Ok(None) => None,
            Err(error) => Some(format!(
                "Native settings preserved; saving disabled: {error}"
            )),
        };
        let current = Preferences::capture(&ui);
        let store = Self {
            path,
            desired: current.clone(),
            current,
            blocked: warning.is_some(),
            writer: None,
            context,
            _profile: None,
        };
        (store, ui, warning)
    }

    /// Called each frame, but disk work runs only after a persisted preference changes.
    pub fn save_if_changed(&mut self, ui: &UiState) -> Result<(), String> {
        self.finish_ready()?;
        let next = Preferences::capture(ui);
        if next.recent_view > 1 {
            return Err("Native settings recent layout must be Grid or List".into());
        }
        self.desired = next.clone();
        if self.writer.is_some() {
            self.context
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
        if self.blocked || self.writer.is_some() || self.current == next {
            return Ok(());
        }
        let path = self.path.clone();
        let context = self.context.clone();
        let writer = std::thread::Builder::new()
            .name("native-settings".into())
            .spawn(move || {
                let result = write(&path, &next).map(|()| next);
                context.request_repaint();
                result
            })
            .map_err(|error| format!("Native settings writer: {error}"))?;
        self.writer = Some(writer);
        Ok(())
    }

    fn finish_ready(&mut self) -> Result<(), String> {
        if !self.writer.as_ref().is_some_and(JoinHandle::is_finished) {
            return Ok(());
        }
        self.finish()
    }

    fn finish(&mut self) -> Result<(), String> {
        let Some(writer) = self.writer.take() else {
            return Ok(());
        };
        match writer
            .join()
            .unwrap_or_else(|_| Err("Native settings writer panicked".into()))
        {
            Ok(preferences) => {
                self.current = preferences;
                Ok(())
            }
            Err(error) => {
                // A failing disk must not trigger one new writer on every repaint.
                self.blocked = true;
                Err(error)
            }
        }
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        if let Err(error) = self.finish() {
            eprintln!("{error}");
        }
        if !self.blocked
            && self.desired != self.current
            && let Err(error) = write(&self.path, &self.desired)
        {
            eprintln!("{error}");
        }
    }
}

fn read(path: &Path) -> Result<Option<Preferences>, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    if !metadata.is_file() || metadata.len() > MAX_BYTES {
        return Err("settings must be a regular file of at most 64 KiB".into());
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .and_then(|file| file.take(MAX_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("settings exceed 64 KiB".into());
    }
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    if value.get("version").and_then(serde_json::Value::as_u64) != Some(u64::from(VERSION)) {
        return Err("unsupported settings version".into());
    }
    let preferences: Preferences =
        serde_json::from_value(value).map_err(|error| error.to_string())?;
    if preferences.recent_view > 1 {
        return Err("invalid recent layout".into());
    }
    Ok(Some(preferences))
}

fn write(path: &Path, preferences: &Preferences) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(preferences).map_err(|error| error.to_string())?;
    onecad_core::io::durable_write(path, &bytes)
        .map_err(|error| format!("Native settings save failed: {error}"))
}

#[cfg(test)]
#[path = "settings_tests.rs"]
mod tests;
