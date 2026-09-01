//! Persisted recent-projects store for the start screen.
//!
//! A tiny JSON file at `<app_config_dir>/recents.json` holding the most-recently
//! opened/saved `.onecad` projects (newest first, capped). [`open_document`] and
//! [`save_document`] call [`record`] on success; [`list_recents`] reads it and maps
//! to the frontend-facing [`RecentProjectDto`]. The webview has zero fs capability,
//! so all of this happens in Rust.
//!
//! Every public fn takes an [`AppHandle`] and resolves `recents.json`'s path from
//! it, then delegates to a private `*_at(recents_path, ...)` twin that does the
//! real work over an explicit path. That split exists for testability: this
//! crate's `AppHandle` is pinned to the concrete `Wry` runtime (same as every
//! other Tauri-command-adjacent module here), which `tauri::test::mock_app`'s
//! `MockRuntime` cannot produce — and even if the types lined up, a mock app's
//! default (empty) bundle identifier makes `app_config_dir()` resolve to the
//! REAL `~/Library/Application Support` (or platform equivalent), not a sandbox.
//! The `*_at` twins are exercised directly against a tempdir in `tests` below,
//! never touching `AppHandle` or the host's real config dir.
//!
//! [`open_document`]: crate::api::open_document
//! [`save_document`]: crate::api::save_document
//! [`list_recents`]: crate::api::list_recents

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use onecad_core::io::{container, durable_write};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::dto::RecentProjectDto;
use crate::error::ApiError;

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

/// Loads the persisted entries at `recents_path`.
///
/// - Missing file ⇒ empty.
/// - Present but unparseable ⇒ [`quarantine_corrupt`] preserves the bytes (never
///   silently discarded, never later clobbered by [`store_at`]) and this returns
///   empty, so the caller starts a fresh list.
fn load_at(recents_path: &Path) -> Vec<RecentEntry> {
    let Ok(bytes) = std::fs::read(recents_path) else {
        return Vec::new();
    };
    match serde_json::from_slice(&bytes) {
        Ok(entries) => entries,
        Err(e) => {
            quarantine_corrupt(recents_path, &e);
            Vec::new()
        }
    }
}

/// Renames an unparseable `recents.json` aside to `recents.json.corrupt-<unix
/// seconds>` (or `…-<unix seconds>-<n>` when that name is taken), so the next
/// [`store_at`] — which only ever targets `recents_path` itself — can never
/// overwrite the evidence. An existing `.corrupt-*` sibling is never clobbered
/// either: a free name is always chosen. If the rename itself fails, the corrupt
/// file is left in place and only logged (losing it would be worse than leaving
/// stale junk on disk).
fn quarantine_corrupt(recents_path: &Path, parse_err: &serde_json::Error) {
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let base = format!("recents.json.corrupt-{now_secs}");
    let mut quarantined = recents_path.with_file_name(&base);
    let mut n = 1u32;
    while quarantined.exists() {
        quarantined = recents_path.with_file_name(format!("{base}-{n}"));
        n += 1;
    }
    match std::fs::rename(recents_path, &quarantined) {
        Ok(()) => tracing::warn!(
            "recents.json was corrupt ({parse_err}); original preserved at {}",
            quarantined.display()
        ),
        Err(rename_err) => tracing::warn!(
            "recents.json is corrupt ({parse_err}) and could not be quarantined ({rename_err}); left in place"
        ),
    }
}

/// Durably (tmp + `fsync` + rename + parent `fsync`) writes `entries` to
/// `recents_path`, creating its parent dir if needed. Best-effort: a write
/// failure is logged and swallowed (recents are non-critical, unlike the
/// document itself).
fn store_at(recents_path: &Path, entries: &[RecentEntry]) {
    let Ok(json) = serde_json::to_vec_pretty(entries) else {
        return;
    };
    if let Err(e) = durable_write(recents_path, &json) {
        tracing::warn!("recents.json write failed: {e}");
    }
}

fn record_at(recents_path: &Path, project_path: &Path) {
    let path = project_path.to_string_lossy().into_owned();
    let name = project_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.clone());
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    let mut entries = load_at(recents_path);
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
    store_at(recents_path, &entries);
}

/// Records a successful open/save of `project_path`: dedup by path, move it to the
/// front (newest first), cap at [`MAX_RECENTS`]. Best-effort persistence.
pub fn record(app: &AppHandle, project_path: &Path) {
    if let Some(recents_path) = recents_file(app) {
        record_at(&recents_path, project_path);
    }
}

/// The recents list mapped to the frontend `RecentProject` DTO shape (missing file
/// ⇒ empty). `id` is the path (unique + stable); `modifiedAt` is the ISO timestamp.
///
/// **Prunes dead entries first**: a project deleted outside the app (Finder, `rm`,
/// an external tool) is dropped here rather than left to error when opened, and the
/// drop is persisted back to `recents.json` so it does not reappear next launch.
/// This is the app's only "sync to filesystem" — re-verified on every call, which
/// already fires on boot and after every open/save/close/rename/delete.
fn list_at(recents_path: &Path) -> Vec<RecentProjectDto> {
    let entries = load_at(recents_path);
    let live: Vec<RecentEntry> = entries
        .iter()
        .filter(|e| Path::new(&e.path).exists())
        .cloned()
        .collect();
    if live.len() != entries.len() {
        store_at(recents_path, &live);
    }
    live.into_iter()
        .map(|e| RecentProjectDto {
            id: e.path.clone(),
            name: e.name,
            path: e.path,
            modified_at: crate::api::rfc3339_from_secs(e.last_opened_ms / 1000),
            thumbnail: None,
            // Filled by `api::list_recents`, which is where the pending offers live.
            // This function reads `recents.json` and nothing else.
            has_recovery: false,
        })
        .collect()
}

/// See [`list_at`]; `thumbnail` is left `None` here — this function touches only
/// `recents.json`. Filling it means opening up to [`MAX_RECENTS`] containers,
/// which is blocking IO; see [`with_thumbnails`].
pub fn list(app: &AppHandle) -> Vec<RecentProjectDto> {
    match recents_file(app) {
        Some(p) => list_at(&p),
        None => Vec::new(),
    }
}

fn rename_at(recents_path: &Path, old_path: &Path, new_name: &str) -> Result<PathBuf, ApiError> {
    let trimmed = new_name.trim();
    if trimmed.is_empty() {
        return Err(ApiError::Io("project name cannot be empty".into()));
    }
    if trimmed.contains('/') || trimmed.contains('\\') {
        return Err(ApiError::Io(
            "project name cannot contain a path separator".into(),
        ));
    }

    let new_path = old_path.with_file_name(format!("{trimmed}.onecad"));
    if new_path != old_path && new_path.exists() {
        return Err(ApiError::Io(format!(
            "a project named \"{trimmed}\" already exists here"
        )));
    }

    std::fs::rename(old_path, &new_path).map_err(|e| ApiError::Io(e.to_string()))?;

    let old = old_path.to_string_lossy().into_owned();
    let new = new_path.to_string_lossy().into_owned();
    let mut entries = load_at(recents_path);
    for entry in &mut entries {
        if entry.path == old {
            entry.path = new.clone();
            entry.name = trimmed.to_string();
        }
    }
    store_at(recents_path, &entries);

    Ok(new_path)
}

/// Renames a project's `.onecad` file on disk (same directory, new file stem) and
/// updates its `recents.json` entry. Refuses an empty/whitespace name or one
/// containing a path separator (`new_name` is a filename, never a path); refuses a
/// target that already exists under a different path (no silent overwrite).
pub fn rename(app: &AppHandle, old_path: &Path, new_name: &str) -> Result<PathBuf, ApiError> {
    let recents_path =
        recents_file(app).ok_or_else(|| ApiError::Io("no app config directory".into()))?;
    rename_at(&recents_path, old_path, new_name)
}

fn remove_at(recents_path: &Path, path: &Path) -> Result<(), ApiError> {
    trash::delete(path).map_err(|e| ApiError::Io(e.to_string()))?;

    let target = path.to_string_lossy().into_owned();
    let mut entries = load_at(recents_path);
    entries.retain(|e| e.path != target);
    store_at(recents_path, &entries);
    Ok(())
}

/// Moves a project's `.onecad` file to the OS trash (recoverable, unlike a
/// permanent delete) and drops its `recents.json` entry. On a trash failure the
/// entry is left untouched — if the file is actually gone, the next [`list`] prunes
/// it anyway; if it isn't, the card must keep pointing at a real file.
pub fn remove(app: &AppHandle, path: &Path) -> Result<(), ApiError> {
    let recents_path =
        recents_file(app).ok_or_else(|| ApiError::Io("no app config directory".into()))?;
    remove_at(&recents_path, path)
}

/// Reveals a project's `.onecad` file in the OS file manager (Finder / Explorer /
/// whatever the Linux desktop offers), selected if the platform supports it.
///
/// Every arm passes `path` as a separate process argument (never interpolated into
/// a shell string), so there is no shell-injection surface here.
pub fn reveal(path: &Path) -> Result<(), ApiError> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("-R")
            .arg(path)
            .status()
            .map_err(|e| ApiError::Io(e.to_string()))?;
    }
    #[cfg(target_os = "windows")]
    {
        // `explorer` routinely exits non-zero even on a successful reveal — the
        // spawn succeeding is the only signal worth checking.
        let mut arg = std::ffi::OsString::from("/select,");
        arg.push(path.as_os_str());
        std::process::Command::new("explorer")
            .arg(arg)
            .status()
            .map_err(|e| ApiError::Io(e.to_string()))?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let uri = format!("file://{}", path.display());
        let by_dbus = std::process::Command::new("dbus-send")
            .args([
                "--session",
                "--dest=org.freedesktop.FileManager1",
                "--type=method_call",
                "/org/freedesktop/FileManager1",
                "org.freedesktop.FileManager1.ShowItems",
                &format!("array:string:{uri}"),
                "string:\"\"",
            ])
            .status();
        let ok = matches!(by_dbus, Ok(s) if s.success());
        if !ok {
            let parent = path.parent().unwrap_or(path);
            std::process::Command::new("xdg-open")
                .arg(parent)
                .status()
                .map_err(|e| ApiError::Io(e.to_string()))?;
        }
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(path: &Path) {
        std::fs::write(path, b"not a real container").unwrap();
    }

    #[test]
    fn list_prunes_a_manually_deleted_project_and_persists_the_prune() {
        let scratch = tempfile::tempdir().unwrap();
        let recents_path = scratch.path().join("recents.json");
        let dir = tempfile::tempdir().unwrap();
        let alive = dir.path().join("Alive.onecad");
        let dead = dir.path().join("Dead.onecad");
        touch(&alive);
        touch(&dead);

        record_at(&recents_path, &alive);
        record_at(&recents_path, &dead);
        std::fs::remove_file(&dead).unwrap();

        let names: Vec<_> = list_at(&recents_path).into_iter().map(|d| d.name).collect();
        assert_eq!(names, vec!["Alive".to_string()]);

        // The prune persisted — a fresh read of the file on disk agrees.
        let names_again: Vec<_> = list_at(&recents_path).into_iter().map(|d| d.name).collect();
        assert_eq!(names_again, vec!["Alive".to_string()]);
    }

    #[test]
    fn rename_moves_the_file_and_updates_the_entry() {
        let scratch = tempfile::tempdir().unwrap();
        let recents_path = scratch.path().join("recents.json");
        let dir = tempfile::tempdir().unwrap();
        let old_path = dir.path().join("Old.onecad");
        touch(&old_path);
        record_at(&recents_path, &old_path);

        let new_path = rename_at(&recents_path, &old_path, "New Name").unwrap();
        assert_eq!(new_path, dir.path().join("New Name.onecad"));
        assert!(new_path.exists());
        assert!(!old_path.exists());

        let entries = list_at(&recents_path);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "New Name");
        assert_eq!(entries[0].path, new_path.to_string_lossy());
    }

    #[test]
    fn rename_refuses_a_collision_with_an_existing_file() {
        let scratch = tempfile::tempdir().unwrap();
        let recents_path = scratch.path().join("recents.json");
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("A.onecad");
        let b = dir.path().join("B.onecad");
        touch(&a);
        touch(&b);
        record_at(&recents_path, &a);

        let err = rename_at(&recents_path, &a, "B").unwrap_err();
        assert!(matches!(err, ApiError::Io(_)));
        assert!(a.exists(), "source must be left in place on refusal");
    }

    #[test]
    fn rename_refuses_an_empty_or_path_separator_name() {
        let scratch = tempfile::tempdir().unwrap();
        let recents_path = scratch.path().join("recents.json");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("A.onecad");
        touch(&path);

        assert!(rename_at(&recents_path, &path, "   ").is_err());
        assert!(rename_at(&recents_path, &path, "sub/dir").is_err());
        assert!(path.exists());
    }

    #[test]
    fn remove_missing_file_does_not_touch_recents_json() {
        let scratch = tempfile::tempdir().unwrap();
        let recents_path = scratch.path().join("recents.json");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Ghost.onecad");
        touch(&path);
        record_at(&recents_path, &path);
        std::fs::remove_file(&path).unwrap();

        // trash::delete on a nonexistent path errors; the entry must survive so
        // the next `list()` prune (not this failed remove) is what drops it.
        assert!(remove_at(&recents_path, &path).is_err());
        let entries = load_at(&recents_path);
        assert_eq!(entries.len(), 1);
    }

    /// Finds the single `recents.json.corrupt-*` sibling next to `recents_path`
    /// (panics if there isn't exactly one — the tests below only ever produce one).
    fn find_quarantine_file(recents_path: &Path) -> PathBuf {
        let dir = recents_path.parent().unwrap();
        let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("recents.json.corrupt-"))
            })
            .collect();
        assert_eq!(found.len(), 1, "expected exactly one quarantine file");
        found.pop().unwrap()
    }

    #[test]
    fn a_corrupt_recents_json_is_quarantined_and_never_overwritten_by_the_next_store() {
        let scratch = tempfile::tempdir().unwrap();
        let recents_path = scratch.path().join("recents.json");
        let dir = tempfile::tempdir().unwrap();

        // 3 valid entries.
        for name in ["A", "B", "C"] {
            let p = dir.path().join(format!("{name}.onecad"));
            touch(&p);
            record_at(&recents_path, &p);
        }
        assert_eq!(load_at(&recents_path).len(), 3);

        // Corrupt it in place (simulating a torn/partial write from before this
        // durability fix).
        let corrupt_bytes: &[u8] = b"{ this is not valid json at all";
        std::fs::write(&recents_path, corrupt_bytes).unwrap();

        // Load returns empty AND the corrupt bytes survive untouched at a
        // `.corrupt-*` sibling.
        assert!(load_at(&recents_path).is_empty());
        let quarantine = find_quarantine_file(&recents_path);
        assert_eq!(std::fs::read(&quarantine).unwrap(), corrupt_bytes);

        // Store 1 fresh entry: recents.json holds exactly that entry, and the
        // quarantine file is untouched.
        let fresh = dir.path().join("D.onecad");
        touch(&fresh);
        record_at(&recents_path, &fresh);
        let entries = load_at(&recents_path);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "D");
        assert_eq!(
            std::fs::read(&quarantine).unwrap(),
            corrupt_bytes,
            "the quarantine file must survive the next store untouched"
        );
    }

    #[test]
    fn quarantine_never_clobbers_an_existing_corrupt_file_at_the_same_second() {
        let scratch = tempfile::tempdir().unwrap();
        let recents_path = scratch.path().join("recents.json");

        std::fs::write(&recents_path, b"first corruption").unwrap();
        assert!(load_at(&recents_path).is_empty());
        let first = find_quarantine_file(&recents_path);
        assert_eq!(std::fs::read(&first).unwrap(), b"first corruption");

        // A second corruption event in the same wall-clock second: the base name
        // collides, so a `-<n>` suffix is chosen — BOTH corrupt files survive and
        // `recents.json` is gone, so the next store cannot destroy either.
        std::fs::write(&recents_path, b"second corruption").unwrap();
        quarantine_corrupt(
            &recents_path,
            &serde_json::from_slice::<Vec<RecentEntry>>(b"x").unwrap_err(),
        );
        assert!(
            !recents_path.exists(),
            "the corrupt file must be moved aside"
        );
        assert_eq!(std::fs::read(&first).unwrap(), b"first corruption");
        let second = first.with_file_name(format!(
            "{}-1",
            first.file_name().unwrap().to_string_lossy()
        ));
        assert_eq!(std::fs::read(&second).unwrap(), b"second corruption");
    }
}
