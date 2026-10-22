//! Semantic acceptance commands of `onecad_ios_test_command` (`test-hooks`
//! only; catalogue in docs/ios/TESTING.md). Commands that wait on document
//! services never block the main thread: they return `{"pending":true,
//! "ticket":n}` and the test polls `{"cmd":"result","ticket":n}` while the
//! display link keeps running frames.
use crate::app::IosApp;
use crate::test_hooks_probe as probe;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Unfinished or unread results kept at once; more is a test bug.
const TICKETS_MAX: usize = 64;
/// Environment names of the gate's run id: XCTest strips `TEST_RUNNER_` before
/// the hosted app sees it; a UI test passes it on in `launchEnvironment`.
const RUN_ID_ENV: [&str; 2] = ["ONECAD_RUN_ID", "TEST_RUNNER_ONECAD_RUN_ID"];
const JOURNAL: &str = "kernel-journal";
const JOURNAL_HELD: &str = "kernel-journal.fault-held";
/// Content of the regular file that blocks the journal directory's name.
const JOURNAL_MARKER: &[u8] = b"onecad acceptance journal fault";
/// The held journal's in-flight record describes the moment the fault was
/// injected, not any later job; restoring it would strike an innocent op at
/// the next launch (ADR-0018 decision 5). The worker rewrites it at its next
/// journal event.
const INFLIGHT: &str = "inflight.json";
/// File inside the directory that stands in for an injected save failure.
const SAVE_BLOCKER: &str = "fault-injection";
/// What the container write reports when a directory holds the target's name
/// (EISDIR); any other failure did not come from the injected fault.
const SAVE_BLOCKED: &str = "Is a directory";
/// Files only `save_artifact` and `export_step_artifact` may attribute to the
/// run: sealing one would let a copied or forged model pass as saved here.
const UNSEALABLE: [&str; 2] = [".onecad", ".step"];

type Slot = Arc<Mutex<Option<Result<Value, String>>>>;

pub const COMMANDS: &[&str] = &[
    "run_id",
    "kernel_identity",
    "document_summary",
    "save_artifact",
    "export_step_artifact",
    "write_semantic",
    "seal_artifact",
    "fault",
    "result",
];

/// A Save As the editor runs; finished when the editor reports it.
struct SaveTicket {
    target: PathBuf,
    run_id: String,
    /// The directory standing in the target's place (injected save failure).
    blocker: Option<PathBuf>,
}

enum Ticket {
    Job(Slot),
    Save(SaveTicket),
}

pub struct State {
    run_id: Option<String>,
    app_support: PathBuf,
    next_ticket: u64,
    tickets: HashMap<u64, Ticket>,
    save_failure_armed: bool,
    journal_fault: bool,
}

impl State {
    pub fn new(app_support: PathBuf) -> Self {
        let run_id = RUN_ID_ENV
            .iter()
            .find_map(|name| std::env::var(name).ok())
            .filter(|id| valid_name(id, 128));
        Self {
            run_id,
            app_support,
            next_ticket: 0,
            tickets: HashMap::new(),
            save_failure_armed: false,
            journal_fault: false,
        }
    }

    /// Journal write failure: the journal directory is moved aside and a
    /// regular file takes its name, so every temp + rename into it fails (the
    /// condition onecad-app's own journal test builds). Counted, never fatal.
    fn inject_journal_fault(&mut self) -> Result<(), String> {
        if self.journal_fault {
            return Ok(());
        }
        let journal = self.app_support.join(JOURNAL);
        if journal.is_dir() {
            std::fs::rename(&journal, self.app_support.join(JOURNAL_HELD))
                .map_err(|e| e.to_string())?;
        }
        std::fs::write(&journal, JOURNAL_MARKER).map_err(|e| e.to_string())?;
        self.journal_fault = true;
        Ok(())
    }

    fn clear_journal_fault(&mut self) {
        if std::mem::take(&mut self.journal_fault) {
            repair_journal(&self.app_support);
        }
    }
}

impl Drop for State {
    fn drop(&mut self) {
        // Never leave the next launch without its crash journal.
        self.clear_journal_fault();
    }
}

/// Undoes an injected journal fault, also one a crash left armed (called
/// before the editor starts a kernel). Touches nothing it did not create.
pub fn repair_journal(app_support: &Path) {
    let journal = app_support.join(JOURNAL);
    let held = app_support.join(JOURNAL_HELD);
    if std::fs::read(&journal).is_ok_and(|bytes| bytes == JOURNAL_MARKER) {
        let _ = std::fs::remove_file(&journal);
    }
    if held.is_dir() && !journal.exists() {
        let _ = std::fs::remove_file(held.join(INFLIGHT));
        let _ = std::fs::rename(&held, &journal);
    }
}

fn valid_name(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        && !value.starts_with('.')
}

fn name(request: &Value) -> Result<String, String> {
    request
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| valid_name(name, 64))
        .map(str::to_string)
        .ok_or_else(|| "name must be 1-64 of [A-Za-z0-9._-]".into())
}

pub fn command(app: &mut IosApp, cmd: &str, request: &Value) -> Result<Value, String> {
    match cmd {
        "run_id" => run_id(app, request),
        "kernel_identity" => {
            let access = app.editor.as_ref().and_then(|e| e.document_access());
            Ok(json!({"kernelIdentity": probe::kernel_identity(access.as_ref())}))
        }
        "fault" => fault(app, request),
        "result" => result(app, request),
        "save_artifact" => save_artifact(app, request),
        "seal_artifact" => seal_artifact(app, request),
        _ => start(app, cmd, request),
    }
}

fn run_id(app: &mut IosApp, request: &Value) -> Result<Value, String> {
    if let Some(value) = request.get("set") {
        let id = value.as_str().filter(|id| valid_name(id, 128));
        app.semantic.run_id = Some(id.ok_or("run id must be 1-128 of [A-Za-z0-9._-]")?.into());
    }
    Ok(json!({"runId": app.semantic.run_id}))
}

fn fault(app: &mut IosApp, request: &Value) -> Result<Value, String> {
    let state = &mut app.semantic;
    match request.get("kind").and_then(Value::as_str) {
        Some("journal_write") => state.inject_journal_fault()?,
        Some("save_failure") => state.save_failure_armed = true,
        Some("clear") => {
            state.clear_journal_fault();
            state.save_failure_armed = false;
        }
        _ => return Err("fault kind must be journal_write, save_failure or clear".into()),
    }
    Ok(json!({
        "journalWrite": state.journal_fault,
        "saveFailureArmed": state.save_failure_armed,
    }))
}

/// The document host and run id every artifact command needs.
fn artifact_context(app: &IosApp) -> Result<(onecad_ui::host::DocumentAccess, String), String> {
    let access = app
        .editor
        .as_ref()
        .and_then(|editor| editor.document_access())
        .ok_or("no document host is running")?;
    let run_id = app
        .semantic
        .run_id
        .clone()
        .ok_or("no run id: set ONECAD_RUN_ID or send run_id with set")?;
    Ok((access, run_id))
}

fn add_ticket(app: &mut IosApp, ticket: Ticket) -> Result<Value, String> {
    if app.semantic.tickets.len() >= TICKETS_MAX {
        return Err("too many unread results".into());
    }
    app.semantic.next_ticket += 1;
    let id = app.semantic.next_ticket;
    app.semantic.tickets.insert(id, ticket);
    Ok(json!({"pending": true, "ticket": id}))
}

/// Saves the active document to `<run dir>/<name>.onecad` with the editor's
/// own File › Save As; only the name prompt is answered by the hook. An armed
/// `save_failure` puts a directory at the target, so that save fails where a
/// full or read-only volume would: in the container write.
fn save_artifact(app: &mut IosApp, request: &Value) -> Result<Value, String> {
    let name = name(request)?;
    let (access, run_id) = artifact_context(app)?;
    if app.semantic.tickets.len() >= TICKETS_MAX {
        return Err("too many unread results".into());
    }
    let target = probe::run_directory(&access, &run_id)?.join(format!("{name}.onecad"));
    let blocker = if std::mem::take(&mut app.semantic.save_failure_armed) {
        std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
        std::fs::write(target.join(SAVE_BLOCKER), b"save failure").map_err(|e| e.to_string())?;
        Some(target.clone())
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
        finish_save(app, blocker.as_deref());
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
fn finish_save(app: &IosApp, blocker: Option<&Path>) {
    if let Ok(mut slot) = app.host.save_answer.lock() {
        slot.take();
    }
    if let Some(blocker) = blocker {
        let _ = std::fs::remove_file(blocker.join(SAVE_BLOCKER));
        let _ = std::fs::remove_dir(blocker);
    }
}

/// The editor's verdict on a Save As ticket; `None` while it runs.
fn save_result(app: &IosApp, save: &SaveTicket) -> Option<Result<Value, String>> {
    let outcome = app.editor.as_ref()?.save_outcome()?;
    finish_save(app, save.blocker.as_deref());
    Some(match (outcome, save.blocker.is_some()) {
        (Err(status), true) if status.contains(SAVE_BLOCKED) => {
            Ok(json!({"saved": false, "injectedFailure": true, "status": status}))
        }
        (Err(status), true) => Err(format!(
            "injected save failure: the save failed elsewhere: {status}"
        )),
        (Ok(_), true) => Err("injected save failure: the save unexpectedly succeeded".into()),
        (Err(status), false) => Err(format!("save failed: {status}")),
        (Ok((clean, status)), false) => saved(app, save, clean, status),
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

fn result(app: &mut IosApp, request: &Value) -> Result<Value, String> {
    let id = request
        .get("ticket")
        .and_then(Value::as_u64)
        .ok_or("ticket")?;
    let finished = match app.semantic.tickets.get(&id) {
        None => return Err(format!("unknown ticket {id}")),
        Some(Ticket::Job(slot)) => slot.lock().map_err(|_| "ticket lock poisoned")?.take(),
        Some(Ticket::Save(save)) => save_result(app, save),
    };
    let Some(outcome) = finished else {
        return Ok(json!({"pending": true, "ticket": id}));
    };
    app.semantic.tickets.remove(&id);
    let run_id = app.semantic.run_id.clone();
    outcome.map(|mut value| {
        value["runId"] = json!(run_id);
        value
    })
}

/// Writes `<name>.provenance.json` (kind `trace`) for a trace record a test
/// already wrote into the run directory, so the gate can attribute it to this
/// run. Documents and STEP files are attributed only by the commands that
/// write them.
fn seal_artifact(app: &mut IosApp, request: &Value) -> Result<Value, String> {
    let name = name(request)?;
    sealable(&name)?;
    let (access, run_id) = artifact_context(app)?;
    let file = probe::run_directory(&access, &run_id)?.join(&name);
    let provenance = probe::write_provenance(&file, "trace", &run_id, &access)?;
    Ok(json!({"runId": run_id, "provenance": provenance}))
}

fn sealable(name: &str) -> Result<(), String> {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".json") {
        return Err("JSON artifacts carry runId and kernelIdentity themselves".into());
    }
    if UNSEALABLE.iter().any(|suffix| lower.ends_with(suffix)) {
        return Err(
            "documents and STEP files come from save_artifact and export_step_artifact".into(),
        );
    }
    Ok(())
}

/// Starts a service-backed command on the editor's service runtime.
fn start(app: &mut IosApp, cmd: &str, request: &Value) -> Result<Value, String> {
    let (access, run_id, name) = if cmd == "document_summary" {
        let access = app.editor.as_ref().and_then(|e| e.document_access());
        let run_id = app.semantic.run_id.clone().unwrap_or_default();
        (
            access.ok_or("no document host is running")?,
            run_id,
            String::new(),
        )
    } else {
        let (access, run_id) = artifact_context(app)?;
        (access, run_id, name(request)?)
    };
    let job = Job {
        cmd: cmd.to_string(),
        name,
        run_id,
        tabs: app.editor.as_ref().map_or(0, |e| e.open_tab_count()),
    };
    let slot: Slot = Arc::default();
    let reply = add_ticket(app, Ticket::Job(slot.clone()))?;
    let ctx = app.ctx.clone();
    access.runtime.clone().spawn(async move {
        let outcome = job.run(&access).await;
        if let Ok(mut slot) = slot.lock() {
            *slot = Some(outcome);
        }
        ctx.request_repaint();
    });
    Ok(reply)
}

struct Job {
    cmd: String,
    name: String,
    run_id: String,
    tabs: usize,
}

impl Job {
    async fn run(self, access: &onecad_ui::host::DocumentAccess) -> Result<Value, String> {
        match self.cmd.as_str() {
            "document_summary" => Ok(json!({
                "summary": probe::document_summary(access, self.tabs).await?,
                "kernelIdentity": probe::kernel_identity(Some(access)),
            })),
            "export_step_artifact" => {
                probe::export_step_artifact(access, &self.run_id, &self.name).await
            }
            "write_semantic" => {
                probe::write_semantic(access, &self.run_id, &self.name, self.tabs).await
            }
            other => Err(format!("unsupported command: {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_plain_file_stems() {
        assert!(valid_name("20261022T101500Z-f585dab0-1a2b", 128));
        assert!(valid_name("plate-fillet", 64));
        for bad in ["", "../x", "a/b", ".hidden", "sp ace", &"x".repeat(65)] {
            assert!(!valid_name(bad, 64), "{bad}");
        }
    }

    #[test]
    fn only_trace_records_can_be_sealed() {
        sealable("touch-trace.jsonl").unwrap();
        for name in ["plate.onecad", "plate.STEP", "plate.step", "summary.json"] {
            assert!(sealable(name).is_err(), "{name}");
        }
    }

    fn scratch(label: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("onecad-ios-fault-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(JOURNAL)).unwrap();
        std::fs::write(root.join(JOURNAL).join("poison.json"), b"{}").unwrap();
        std::fs::write(root.join(JOURNAL).join(INFLIGHT), b"{}").unwrap();
        root
    }

    #[test]
    fn the_journal_fault_blocks_the_directory_and_clear_restores_it() {
        let root = scratch("clear");
        let journal = root.join(JOURNAL);
        let mut state = State::new(root.clone());
        state.inject_journal_fault().unwrap();
        assert!(journal.is_file(), "a regular file holds the journal's name");
        assert!(std::fs::write(journal.join(INFLIGHT), b"x").is_err());
        drop(state);
        assert!(
            journal.join("poison.json").is_file(),
            "dropping the state restores it"
        );
        assert!(
            !journal.join(INFLIGHT).exists(),
            "the in-flight record of the fault window is not evidence"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_fault_left_armed_by_a_crash_is_repaired_at_the_next_launch() {
        let root = scratch("crash");
        let mut state = State::new(root.clone());
        state.inject_journal_fault().unwrap();
        // A crash: no Drop runs.
        std::mem::forget(state);
        repair_journal(&root);
        assert!(root.join(JOURNAL).join("poison.json").is_file());
        assert!(!root.join(JOURNAL_HELD).exists());
        // Someone else's regular file is never removed.
        std::fs::remove_dir_all(root.join(JOURNAL)).unwrap();
        std::fs::write(root.join(JOURNAL), b"not ours").unwrap();
        repair_journal(&root);
        assert!(root.join(JOURNAL).is_file());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
