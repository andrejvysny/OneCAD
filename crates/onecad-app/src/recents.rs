//! Durable recent-project storage rooted at the host's explicit configuration path.
//!
//! Filesystem operations stay in Rust; tests use temporary directories.

use std::path::{Path, PathBuf};
use std::sync::{Mutex as StdMutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::host::HostContext;
use onecad_core::{
    ids::DocumentId,
    io::{container, durable_write},
};
use serde::{Deserialize, Serialize};

use crate::dto::RecentProjectDto;
use crate::error::ApiError;

/// Newest-first cap on the recents list.
const MAX_RECENTS: usize = 10;

/// One persisted recents entry (`recents.json` element).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecentEntry {
    /// Absolute project path (the dedup key).
    path: String,
    /// File stem, shown as the card title.
    name: String,
    /// Unix-epoch milliseconds of the last open/save.
    last_opened_ms: u64,
    /// Identity lets container relocation distinguish the saved document from an
    /// unrelated file at the same relative path. Older entries lack this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    document_id: Option<DocumentId>,
}

/// The `recents.json` path under the app config dir, or `None` if it cannot be
/// resolved (a headless / permission-denied environment — recents degrade to []).
/// Serializes every read-modify-write of `recents.json` within the process.
///
/// Each write is atomic, but the file is read, edited and written back, and several
/// open documents record, list (which prunes and rewrites) and rename concurrently:
/// two unserialized cycles would lose one edit. Process-wide rather than per host
/// because every host shares the one file.
static RECENTS_LANE: StdMutex<()> = StdMutex::new(());

fn recents_lane() -> MutexGuard<'static, ()> {
    RECENTS_LANE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn recents_file(app: &HostContext) -> Option<PathBuf> {
    app.paths.app_config.clone().map(|d| d.join("recents.json"))
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

/// Updates a successfully migrated working path without deleting its source.
pub fn relocate(app: &HostContext, source: &Path, destination: &Path) {
    let Some(path) = recents_file(app) else {
        return;
    };
    let _lane = recents_lane();
    let mut entries = load_at(&path);
    let source = source.to_string_lossy();
    let target = destination.to_string_lossy().into_owned();
    entries.retain(|entry| entry.path != source.as_ref() && entry.path != target);
    entries.insert(
        0,
        RecentEntry {
            path: target,
            document_id: saved_identity(destination),
            name: destination
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            last_opened_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|time| time.as_millis() as u64)
                .unwrap_or(0),
        },
    );
    entries.truncate(MAX_RECENTS);
    store_at(&path, &entries);
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
    quarantine_corrupt_at(recents_path, parse_err, now_secs);
}

fn quarantine_corrupt_at(recents_path: &Path, parse_err: &serde_json::Error, now_secs: u64) {
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
    let _lane = recents_lane();
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
            document_id: saved_identity(project_path),
        },
    );
    entries.truncate(MAX_RECENTS);
    store_at(recents_path, &entries);
}

/// Records a successful open/save of `project_path`: dedup by path, move it to the
/// front (newest first), cap at [`MAX_RECENTS`]. Best-effort persistence.
pub fn record(app: &HostContext, project_path: &Path) {
    if let Some(recents_path) = recents_file(app) {
        record_at(&recents_path, project_path);
    }
}

/// The recents list mapped to the frontend `RecentProject` DTO shape (missing file
/// ⇒ empty). `id` is the path (unique + stable); `modifiedAt` is the ISO timestamp.
///
/// **Prunes dead external entries**: a project deleted outside the app (Finder, `rm`,
/// an external tool) is dropped here rather than left to error when opened, and the
/// drop is persisted back to `recents.json` so it does not reappear next launch.
/// Missing app-container references remain stored until a host can resolve their
/// relocation with the saved document identity; listing cannot discard that evidence.
/// This is the app's only "sync to filesystem" — re-verified on every call, which
/// already fires on boot and after every open/save/close/rename/delete.
fn list_at(recents_path: &Path) -> Vec<RecentProjectDto> {
    list_at_in_documents(recents_path, None)
}

fn saved_identity(path: &Path) -> Option<DocumentId> {
    container::ContainerReader::open(path)
        .ok()
        .map(|reader| reader.document().id)
}

fn list_at_in_documents(recents_path: &Path, documents: Option<&Path>) -> Vec<RecentProjectDto> {
    let _lane = recents_lane();
    let entries = load_at(recents_path);
    let mut retained = entries.clone();
    for entry in &mut retained {
        resolve_entry(entry, documents);
    }
    let mut seen = std::collections::HashSet::new();
    retained.retain(|entry| {
        let path = Path::new(&entry.path);
        (path.exists() || crate::document_paths::is_container_document(path))
            && seen.insert(entry.path.clone())
    });
    if retained != entries {
        store_at(recents_path, &retained);
    }
    retained
        .into_iter()
        .filter(|entry| Path::new(&entry.path).exists())
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

fn resolve_entry(entry: &mut RecentEntry, documents: Option<&Path>) {
    let path = Path::new(&entry.path);
    if path.exists() {
        if entry.document_id.is_none() {
            entry.document_id = saved_identity(path);
        }
        return;
    }
    if let (Some(documents), Some(id)) = (documents, entry.document_id) {
        match crate::document_paths::relocate_saved_path(documents, path, id) {
            Ok(Some(relocated)) => entry.path = relocated.to_string_lossy().into_owned(),
            Err(error) => {
                tracing::warn!(%error, "Recent document relocation refused; reference retained")
            }
            Ok(None) => {}
        }
    }
}

/// See [`list_at`]; `thumbnail` is left `None` here — this function touches only
/// `recents.json`. Filling it means opening up to [`MAX_RECENTS`] containers,
/// which is blocking IO; see [`with_thumbnails`].
pub fn list(app: &HostContext) -> Vec<RecentProjectDto> {
    match recents_file(app) {
        Some(p) => list_at(&p),
        None => Vec::new(),
    }
}

/// Resolve application-container relocation before pruning missing recent files.
/// Unknown legacy identities and rejected candidates retain their stored reference
/// for recovery/manual resolution; they are never guessed from a filename.
pub fn list_in_documents(app: &HostContext, documents: &Path) -> Vec<RecentProjectDto> {
    match recents_file(app) {
        Some(path) => list_at_in_documents(&path, Some(documents)),
        None => Vec::new(),
    }
}

fn rename_at(recents_path: &Path, old_path: &Path, new_name: &str) -> Result<PathBuf, ApiError> {
    let _lane = recents_lane();
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
pub fn rename(app: &HostContext, old_path: &Path, new_name: &str) -> Result<PathBuf, ApiError> {
    let recents_path =
        recents_file(app).ok_or_else(|| ApiError::Io("no app config directory".into()))?;
    rename_at(&recents_path, old_path, new_name)
}

fn remove_at(recents_path: &Path, path: &Path) -> Result<(), ApiError> {
    let _lane = recents_lane();
    move_to_trash(path)?;

    let target = path.to_string_lossy().into_owned();
    let mut entries = load_at(recents_path);
    entries.retain(|e| e.path != target);
    store_at(recents_path, &entries);
    Ok(())
}

#[cfg(not(target_os = "ios"))]
fn move_to_trash(path: &Path) -> Result<(), ApiError> {
    trash::delete(path).map_err(|e| ApiError::Io(e.to_string()))
}

/// iPadOS apps have no programmatic trash; a permanent delete would break the
/// recoverable-removal promise below, so the move is refused.
#[cfg(target_os = "ios")]
fn move_to_trash(_path: &Path) -> Result<(), ApiError> {
    Err(ApiError::Io(
        "moving documents to the trash is not available on iPadOS".into(),
    ))
}

/// Moves a project's `.onecad` file to the OS trash (recoverable, unlike a
/// permanent delete) and drops its `recents.json` entry. On a trash failure the
/// entry is left untouched — if the file is actually gone, the next [`list`] prunes
/// it anyway; if it isn't, the card must keep pointing at a real file.
pub fn remove(app: &HostContext, path: &Path) -> Result<(), ApiError> {
    let recents_path =
        recents_file(app).ok_or_else(|| ApiError::Io("no app config directory".into()))?;
    remove_at(&recents_path, path)
}

/// Reveals a project's `.onecad` file in the OS file manager (Finder / Explorer /
/// whatever the Linux desktop offers), selected if the platform supports it.
///
/// Every arm passes `path` as a separate process argument (never interpolated into
/// a shell string), so there is no shell-injection surface here. On iOS there is
/// no file manager to spawn (and no child processes): a no-op returning `Ok`;
/// the UIKit host presents files itself.
pub fn reveal(path: &Path) -> Result<(), ApiError> {
    #[cfg(target_os = "ios")]
    let _ = path;
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
    #[cfg(all(unix, not(any(target_os = "macos", target_os = "ios"))))]
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
/// Deliberately takes no [`HostContext`]: keeping it a pure `Vec → Vec` is what makes
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
#[path = "recents_relocation_tests.rs"]
mod relocation_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn touch(path: &Path) {
        std::fs::write(path, b"not a real container").unwrap();
    }

    #[test]
    fn concurrent_records_and_prunes_from_several_hosts_lose_no_entry() {
        let scratch = tempfile::tempdir().unwrap();
        let recents_path = Arc::new(scratch.path().join("recents.json"));
        let dir = tempfile::tempdir().unwrap();
        let dead = dir.path().join("Dead.onecad");
        touch(&dead);
        record_at(&recents_path, &dead);
        std::fs::remove_file(&dead).unwrap();
        let projects: Vec<_> = (0..MAX_RECENTS)
            .map(|i| {
                let path = dir.path().join(format!("P{i}.onecad"));
                touch(&path);
                path
            })
            .collect();
        let handles: Vec<_> = projects
            .iter()
            .cloned()
            .map(|project| {
                let recents_path = recents_path.clone();
                std::thread::spawn(move || {
                    record_at(&recents_path, &project);
                    list_at(&recents_path);
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
        let mut names: Vec<_> = list_at(&recents_path).into_iter().map(|d| d.name).collect();
        names.sort();
        let mut expected: Vec<_> = (0..MAX_RECENTS).map(|i| format!("P{i}")).collect();
        expected.sort();
        assert_eq!(names, expected, "every record survives and the prune holds");
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
        let error = serde_json::from_slice::<Vec<RecentEntry>>(b"x").unwrap_err();
        let timestamp = 1_800_000_000;

        std::fs::write(&recents_path, b"first corruption").unwrap();
        quarantine_corrupt_at(&recents_path, &error, timestamp);
        let first = find_quarantine_file(&recents_path);
        assert_eq!(std::fs::read(&first).unwrap(), b"first corruption");

        // A second corruption event in the same wall-clock second: the base name
        // collides, so a `-<n>` suffix is chosen — BOTH corrupt files survive and
        // `recents.json` is gone, so the next store cannot destroy either.
        std::fs::write(&recents_path, b"second corruption").unwrap();
        quarantine_corrupt_at(&recents_path, &error, timestamp);
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
