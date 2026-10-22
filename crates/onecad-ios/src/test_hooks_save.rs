//! `save_artifact` and the injected save failure of the semantic hooks
//! (`test-hooks` only; catalogue in docs/ios/TESTING.md). The save is the
//! editor's own File › Save As; only its name prompt is answered here.
use crate::app::IosApp;
use crate::test_hooks_probe as probe;
use crate::test_hooks_semantic::{Ticket, add_ticket, artifact_context, name, ticket_room};
use serde_json::{Value, json};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// File inside the directory that stands in for an injected save failure.
const SAVE_BLOCKER: &str = "fault-injection";
/// What the container write reports when a directory holds the target's name
/// (EISDIR); any other failure did not come from the injected fault.
const SAVE_BLOCKED: &str = "Is a directory";
/// What the container write reports when its directory is read-only (EACCES).
const SAVE_DENIED: &str = "Permission denied";

/// How an armed save failure blocks the target.
pub enum Blocker {
    /// A new target: a directory takes its name, so the final rename fails.
    Directory(PathBuf),
    /// An existing file (an overwrite): its directory is made read-only, so
    /// the write fails before any byte reaches the directory and the previous
    /// file must survive byte-identical. Holds the directory's mode to restore.
    ReadOnly(PathBuf, u32),
}

impl Blocker {
    fn arm(target: &Path) -> Result<Self, String> {
        if target.is_file() {
            let dir = target.parent().ok_or("artifact has no directory")?;
            let mode = std::fs::metadata(dir)
                .map_err(|e| e.to_string())?
                .permissions()
                .mode();
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o555))
                .map_err(|e| e.to_string())?;
            return Ok(Self::ReadOnly(dir.to_path_buf(), mode));
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
            Self::ReadOnly(dir, mode) => {
                let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(*mode));
            }
        }
    }

    /// The editor status the injected fault must produce, and its name.
    fn expected(&self) -> (&'static str, &'static str) {
        match self {
            Self::Directory(_) => (SAVE_BLOCKED, "directory"),
            Self::ReadOnly(..) => (SAVE_DENIED, "readOnly"),
        }
    }
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
        *slot = Some(path.to_string_lossy().into_owned());
    }
    Ok(json!({ "path": path }))
}

/// A Save As the editor runs; finished when the editor reports it.
pub struct SaveTicket {
    target: PathBuf,
    run_id: String,
    blocker: Option<Blocker>,
}

/// Saves the active document to `<run dir>/<name>.onecad` with the editor's
/// own File › Save As; only the name prompt is answered by the hook. An armed
/// `save_failure` blocks the target (see [`Blocker`]), so the save fails where
/// a full or read-only volume would: in the container write.
pub fn save_artifact(app: &mut IosApp, request: &Value) -> Result<Value, String> {
    let name = name(request)?;
    let (access, run_id) = artifact_context(app)?;
    ticket_room(app)?;
    let target = probe::run_directory(&access, &run_id)?.join(format!("{name}.onecad"));
    let blocker = if std::mem::take(&mut app.semantic.save_failure_armed) {
        Some(Blocker::arm(&target)?)
    } else {
        None
    };
    if let Ok(mut slot) = app.host.save_answer.lock() {
        *slot = Some(target.to_string_lossy().into_owned());
    }
    let started = match app.editor.as_mut() {
        Some(editor) => editor.request_save_as(),
        None => Err("no editor".into()),
    };
    if let Err(status) = started {
        finish_save(app, blocker.as_ref());
        return Err(format!("the editor refused Save As: {status}"));
    }
    let save = SaveTicket {
        target,
        run_id,
        blocker,
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
    let outcome = app.editor.as_ref()?.save_outcome()?;
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

    fn scratch(label: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("onecad-ios-save-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn a_new_target_is_blocked_by_a_directory_and_restored() {
        let root = scratch("new");
        let target = root.join("part.onecad");
        let blocker = Blocker::arm(&target).unwrap();
        assert!(target.is_dir());
        assert_eq!(blocker.expected().1, "directory");
        blocker.remove();
        assert!(!target.exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn an_overwrite_is_denied_without_touching_the_previous_file() {
        let root = scratch("overwrite");
        let target = root.join("part.onecad");
        std::fs::write(&target, b"previous").unwrap();
        let blocker = Blocker::arm(&target).unwrap();
        assert_eq!(blocker.expected().1, "readOnly");
        let denied = std::fs::write(root.join(".part.onecad.tmp-x"), b"new").unwrap_err();
        assert_eq!(denied.kind(), std::io::ErrorKind::PermissionDenied);
        blocker.remove();
        assert_eq!(std::fs::read(&target).unwrap(), b"previous");
        std::fs::write(root.join("after"), b"writable again").unwrap();
        std::fs::remove_dir_all(&root).unwrap();
    }
}
