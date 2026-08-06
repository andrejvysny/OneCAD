//! Persisted recent-projects store for the start screen.
//!
//! A tiny JSON file at `<app_config_dir>/recents.json` holding the most-recently
//! opened/saved `.onecad` projects (newest first, capped). [`open_document`] and
//! [`save_document`] call [`record`] on success; [`list_recents`] reads it and maps
//! to the frontend-facing [`RecentProjectDto`]. The webview has zero fs capability,
//! so all of this happens in Rust.
//!
//! [`open_document`]: crate::api::open_document
//! [`save_document`]: crate::api::save_document
//! [`list_recents`]: crate::api::list_recents

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use onecad_core::io::container;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::dto::RecentProjectDto;

/// Newest-first cap on the recents list.
const MAX_RECENTS: usize = 10;

/// One persisted recents entry (`recents.json` element).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecentEntry {
    /// Absolute project path (the dedup key).
    path: String,
    /// File stem, shown as the card title.
    name: String,
    /// Unix-epoch milliseconds of the last open/save.
    last_opened_ms: u64,
}

/// The `recents.json` path under the app config dir, or `None` if it cannot be
/// resolved (a headless / permission-denied environment — recents degrade to []).
fn recents_file(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|d| d.join("recents.json"))
}

/// Loads the persisted entries (missing / corrupt file ⇒ empty).
fn load(app: &AppHandle) -> Vec<RecentEntry> {
    let Some(path) = recents_file(app) else {
        return Vec::new();
    };
    let Ok(bytes) = std::fs::read(&path) else {
        return Vec::new();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

/// Atomically writes `entries` (temp file + rename), creating the config dir if
/// needed. Best-effort: a write failure is swallowed (recents are non-critical).
fn store(app: &AppHandle, entries: &[RecentEntry]) {
    let Some(path) = recents_file(app) else {
        return;
    };
    if let Some(parent) = path.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
    }
    let Ok(json) = serde_json::to_vec_pretty(entries) else {
        return;
    };
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, &json).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

/// Records a successful open/save of `project_path`: dedup by path, move it to the
/// front (newest first), cap at [`MAX_RECENTS`]. Best-effort persistence.
pub fn record(app: &AppHandle, project_path: &Path) {
    let path = project_path.to_string_lossy().into_owned();
    let name = project_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.clone());
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    let mut entries = load(app);
    entries.retain(|e| e.path != path);
    entries.insert(
        0,
        RecentEntry {
            path,
            name,
            last_opened_ms: now_ms,
        },
    );
    entries.truncate(MAX_RECENTS);
    store(app, &entries);
}

/// The recents list mapped to the frontend `RecentProject` DTO shape (missing file
/// ⇒ empty). `id` is the path (unique + stable); `modifiedAt` is the ISO timestamp.
///
/// `thumbnail` is left `None` here — this function touches only `recents.json`.
/// Filling it means opening up to [`MAX_RECENTS`] containers, which is blocking IO;
/// see [`with_thumbnails`].
pub fn list(app: &AppHandle) -> Vec<RecentProjectDto> {
    load(app)
        .into_iter()
        .map(|e| RecentProjectDto {
            id: e.path.clone(),
            name: e.name,
            path: e.path,
            modified_at: crate::api::rfc3339_from_secs(e.last_opened_ms / 1000),
            thumbnail: None,
        })
        .collect()
}

/// Fills each entry's `thumbnail` from its container's `preview.png`, as a
/// `data:image/png;base64,…` URL the start screen can put straight in an `<img>`.
///
/// **Blocking** — up to [`MAX_RECENTS`] file reads. Call it inside one
/// `spawn_blocking` for the whole batch (see [`crate::api::list_recents`]); a
/// per-entry spawn would cost ten thread hops for ten small reads.
///
/// Deliberately takes no [`AppHandle`]: keeping it a pure `Vec → Vec` is what makes
/// it `Send + 'static` and movable onto the blocking pool.
///
/// A project with no preview (never saved by a build that captures one, saved
/// headless, deleted since, corrupt) simply keeps `None` and the frontend renders
/// its existing hatch placeholder. `read_preview` collapses every failure mode to
/// `None` for exactly this reason — a bad file must never break the listing.
#[must_use]
pub fn with_thumbnails(mut entries: Vec<RecentProjectDto>) -> Vec<RecentProjectDto> {
    use base64::Engine as _;

    for entry in &mut entries {
        entry.thumbnail = container::read_preview(Path::new(&entry.path)).map(|png| {
            format!(
                "data:image/png;base64,{}",
                base64::engine::general_purpose::STANDARD.encode(png)
            )
        });
    }
    entries
}
