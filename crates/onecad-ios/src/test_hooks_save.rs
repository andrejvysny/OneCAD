//! `save_artifact` and the injected save failure of the semantic hooks
//! (`test-hooks` only; catalogue in docs/ios/TESTING.md). The save is the
//! editor's own File › Save As; only its name prompt is answered here.
use crate::app::IosApp;
use crate::host::SaveAnswer;
use crate::test_hooks_probe as probe;
use crate::test_hooks_semantic::{
    Ticket, add_ticket, artifact_context, name, save_ticket, ticket_room,
};
use serde_json::{Value, json};
use std::fs::Permissions;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use tokio::sync::oneshot;

/// File inside the directory that stands in for an injected save failure.
const SAVE_BLOCKER: &str = "fault-injection";
/// What the container write reports when a directory holds the target's name
/// (EISDIR); any other failure did not come from the injected fault.
const SAVE_BLOCKED: &str = "Is a directory";
/// What the container write reports when its directory is read-only (EACCES).
const SAVE_DENIED: &str = "Permission denied";
/// Application Support record of a read-only run directory and its previous
/// mode, so `fault clear` and the next launch restore it when no ticket did
/// (a timed-out test, an aborted run, a killed app).
const READ_ONLY_MARKER: &str = "save-fault.read-only.json";
const READ_ONLY: u32 = 0o555;

/// How an armed save failure blocks the target.
pub enum Blocker {
    /// A new target: a directory takes its name, so the final rename fails.
    Directory(PathBuf),
    /// An existing file (an overwrite): its directory is made read-only, so
    /// the write fails before any byte reaches the directory and the previous
    /// file must survive byte-identical. Holds the marker recording the
    /// directory's previous mode.
    ReadOnly(PathBuf),
}

impl Blocker {
    fn arm(target: &Path, app_support: &Path) -> Result<Self, String> {
        if target.is_file() {
            let dir = target.parent().ok_or("artifact has no directory")?;
            let mode = std::fs::metadata(dir)
                .map_err(|e| e.to_string())?
                .permissions()
                .mode();
            let marker = app_support.join(READ_ONLY_MARKER);
            if marker.exists() {
                return Err("a read-only save failure is already armed".into());
            }
            let record = json!({ "dir": dir, "mode": mode });
            std::fs::write(&marker, record.to_string()).map_err(|e| e.to_string())?;
            if let Err(e) = std::fs::set_permissions(dir, Permissions::from_mode(READ_ONLY)) {
                let _ = std::fs::remove_file(&marker);
                return Err(e.to_string());
            }
            return Ok(Self::ReadOnly(marker));
        }
        std::fs::create_dir_all(target).map_err(|e| e.to_string())?;
        std::fs::write(target.join(SAVE_BLOCKER), b"save failure").map_err(|e| e.to_string())?;
        Ok(Self::Directory(target.to_path_buf()))
    }

    fn remove(&self) {
        match self {
            Self::Directory(target) => {
                let _ = std::fs::remove_file(target.join(SAVE_BLOCKER));
                let _ = std::fs::remove_dir(target);
            }
            Self::ReadOnly(marker) => restore_read_only(marker),
        }
    }

    /// The editor status the injected fault must produce, and its name.
    fn expected(&self) -> (&'static str, &'static str) {
        match self {
            Self::Directory(_) => (SAVE_BLOCKED, "directory"),
            Self::ReadOnly(_) => (SAVE_DENIED, "readOnly"),
        }
    }
}

impl Drop for Blocker {
    // An unread ticket (timed-out test, `fault clear`, app exit) must not
    // leave the run directory blocked; removing twice is a no-op.
    fn drop(&mut self) {
        self.remove();
    }
}

/// Restores the directory an armed overwrite fault left read-only (called by
/// `fault clear`, at exit and before the editor starts). Does nothing without a
/// record, and leaves a directory whose mode changed since.
pub fn repair_save_fault(app_support: &Path) {
    restore_read_only(&app_support.join(READ_ONLY_MARKER));
}

fn restore_read_only(marker: &Path) {
    let Ok(bytes) = std::fs::read(marker) else {
        return;
    };
    let record: Value = serde_json::from_slice(&bytes).unwrap_or_default();
    let mode = record["mode"]
        .as_u64()
        .and_then(|mode| u32::try_from(mode).ok());
    if let (Some(dir), Some(mode)) = (record["dir"].as_str(), mode) {
        let blocked =
            std::fs::metadata(dir).is_ok_and(|meta| meta.permissions().mode() & 0o777 == READ_ONLY);
        if blocked {
            let _ = std::fs::set_permissions(dir, Permissions::from_mode(mode));
        }
    }
    let _ = std::fs::remove_file(marker);
}

/// Where `answer_save` points the next save dialog: inside Documents (the
/// only place saves and exports may go) but outside every run directory,
/// which the gate copies and checks.
const EXPORTS: &str = "acceptance-exports";

/// Answers the next save dialog (Save As or Export) with
/// `Documents/acceptance-exports/<name>`, so a test can drive the editor's own
/// menu rows; only the name prompt is skipped. Nothing is attributed to the
/// run.
pub fn answer_save(app: &mut IosApp, request: &Value) -> Result<Value, String> {
    let name = name(request)?;
    let access = app
        .editor
        .as_ref()
        .and_then(|editor| editor.document_access())
        .ok_or("no document host is running")?;
    let directory = access.documents.join(EXPORTS);
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let path = directory.join(name);
    if let Ok(mut slot) = app.host.save_answer.lock() {
        *slot = Some(SaveAnswer {
            path: path.to_string_lossy().into_owned(),
            hold: None,
        });
    }
    Ok(json!({ "path": path }))
}

/// A Save As the editor runs; finished when the editor reports it.
pub struct SaveTicket {
    /// The tab whose Save this is; its result is read there, whichever tab
    /// is shown when the test polls.
    tab: u64,
    target: PathBuf,
    run_id: String,
    blocker: Option<Blocker>,
    /// Keeps a held save's dialog open; sending or dropping releases it.
    release: Option<oneshot::Sender<()>>,
}

impl SaveTicket {
    /// Lets a held save continue; `false` when it was not held or already
    /// released.
    pub fn release(&mut self) -> bool {
        self.release.take().is_some_and(|sender| {
            let _ = sender.send(());
            true
        })
    }
}

/// `{"cmd":"release_save","ticket":n}`: a save started with `hold` continues.
pub fn release_save(app: &mut IosApp, request: &Value) -> Result<Value, String> {
    let id = request
        .get("ticket")
        .and_then(Value::as_u64)
        .ok_or("ticket")?;
    let save = save_ticket(app, id).ok_or_else(|| format!("no unread save ticket {id}"))?;
    Ok(json!({ "released": save.release() }))
}

/// Saves the active document to `<run dir>/<name>.onecad` with the editor's
/// own File › Save As; only the name prompt is answered by the hook. An armed
/// `save_failure` blocks the target (see [`Blocker`]), so the save fails where
/// a full or read-only volume would: in the container write. With `"hold":
/// true` the save stays pending (the editor busy) until `release_save`.
pub fn save_artifact(app: &mut IosApp, request: &Value) -> Result<Value, String> {
    let name = name(request)?;
    let (access, run_id) = artifact_context(app)?;
    ticket_room(app)?;
    let target = probe::run_directory(&access, &run_id)?.join(format!("{name}.onecad"));
    let blocker = if std::mem::take(&mut app.semantic.save_failure_armed) {
        Some(Blocker::arm(&target, app.semantic.app_support())?)
    } else {
        None
    };
    let held = request.get("hold").and_then(Value::as_bool) == Some(true);
    let (release, hold) = if held {
        let (sender, receiver) = oneshot::channel();
        (Some(sender), Some(receiver))
    } else {
        (None, None)
    };
    if let Ok(mut slot) = app.host.save_answer.lock() {
        *slot = Some(SaveAnswer {
            path: target.to_string_lossy().into_owned(),
            hold,
        });
    }
    let started = match app.editor.as_mut() {
        Some(editor) => editor.request_save_as(),
        None => Err("no editor".into()),
    };
    let tab = match started {
        Ok(tab) => tab,
        Err(status) => {
            finish_save(app, blocker.as_ref());
            return Err(format!("the editor refused Save As: {status}"));
        }
    };
    let save = SaveTicket {
        tab,
        target,
        run_id,
        blocker,
        release,
    };
    add_ticket(app, Ticket::Save(save))
}

/// Drops an unused dialog answer and the injected blocker.
fn finish_save(app: &IosApp, blocker: Option<&Blocker>) {
    if let Ok(mut slot) = app.host.save_answer.lock() {
        slot.take();
    }
    if let Some(blocker) = blocker {
        blocker.remove();
    }
}

/// The editor's verdict on a Save As ticket; `None` while it runs.
pub fn save_result(app: &IosApp, save: &SaveTicket) -> Option<Result<Value, String>> {
    let outcome = app.editor.as_ref()?.save_outcome(save.tab)?;
    finish_save(app, save.blocker.as_ref());
    let Some(blocker) = &save.blocker else {
        return Some(match outcome {
            Err(status) => Err(format!("save failed: {status}")),
            Ok((clean, status)) => saved(app, save, clean, status),
        });
    };
    let (expected, kind) = blocker.expected();
    Some(match outcome {
        Err(status) if status.contains(expected) => Ok(json!({
            "saved": false,
            "injectedFailure": true,
            "blocked": kind,
            "status": status,
        })),
        Err(status) => Err(format!(
            "injected save failure: the save failed elsewhere: {status}"
        )),
        Ok(_) => Err("injected save failure: the save unexpectedly succeeded".into()),
    })
}

fn saved(app: &IosApp, save: &SaveTicket, clean: bool, status: String) -> Result<Value, String> {
    let access = app
        .editor
        .as_ref()
        .and_then(|editor| editor.document_access())
        .ok_or("the document host stopped after saving")?;
    if !save.target.is_file() {
        return Err(format!(
            "saved, but {} is not a file",
            save.target.display()
        ));
    }
    let provenance = probe::write_provenance(&save.target, "document", &save.run_id, &access)?;
    Ok(json!({
        "saved": true,
        "path": save.target,
        "clean": clean,
        "status": status,
        "provenance": provenance,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A run directory and an Application Support directory.
    fn scratch(label: &str) -> (PathBuf, PathBuf, PathBuf) {
        let root =
            std::env::temp_dir().join(format!("onecad-ios-save-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (run, support) = (root.join("run"), root.join("support"));
        std::fs::create_dir_all(&run).unwrap();
        std::fs::create_dir_all(&support).unwrap();
        (root, run, support)
    }

    fn overwrite_target(run: &Path) -> PathBuf {
        let target = run.join("part.onecad");
        std::fs::write(&target, b"previous").unwrap();
        target
    }

    fn mode(dir: &Path) -> u32 {
        std::fs::metadata(dir).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn a_new_target_is_blocked_by_a_directory_and_restored() {
        let (root, run, support) = scratch("new");
        let target = run.join("part.onecad");
        let blocker = Blocker::arm(&target, &support).unwrap();
        assert!(target.is_dir());
        assert_eq!(blocker.expected().1, "directory");
        blocker.remove();
        assert!(!target.exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn an_overwrite_is_denied_without_touching_the_previous_file() {
        let (root, run, support) = scratch("overwrite");
        let target = overwrite_target(&run);
        let blocker = Blocker::arm(&target, &support).unwrap();
        assert_eq!(blocker.expected().1, "readOnly");
        let denied = std::fs::write(run.join(".part.onecad.tmp-x"), b"new").unwrap_err();
        assert_eq!(denied.kind(), std::io::ErrorKind::PermissionDenied);
        blocker.remove();
        assert_eq!(std::fs::read(&target).unwrap(), b"previous");
        assert!(!support.join(READ_ONLY_MARKER).exists());
        std::fs::write(run.join("after"), b"writable again").unwrap();
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn an_unread_blocker_restores_the_directory_when_dropped() {
        let (root, run, support) = scratch("dropped");
        let blocker = Blocker::arm(&overwrite_target(&run), &support).unwrap();
        assert_eq!(mode(&run), READ_ONLY);
        drop(blocker);
        assert_ne!(mode(&run), READ_ONLY, "the run directory stayed read-only");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn fault_clear_and_the_next_launch_restore_a_lost_blocker() {
        let (root, run, support) = scratch("lost");
        let target = overwrite_target(&run);
        let before = mode(&run);
        // A ticket that never drops (killed app): only the record remains.
        std::mem::forget(Blocker::arm(&target, &support).unwrap());
        assert!(Blocker::arm(&target, &support).is_err(), "armed twice");
        let mut state = crate::test_hooks_semantic::State::new(support.clone());
        state.clear();
        assert_eq!(mode(&run), before, "fault clear restores it");
        std::mem::forget(Blocker::arm(&target, &support).unwrap());
        repair_save_fault(&support);
        assert_eq!(mode(&run), before, "the next launch restores it");
        assert!(!support.join(READ_ONLY_MARKER).exists());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
