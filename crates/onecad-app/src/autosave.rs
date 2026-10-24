//! Autosave + crash-recovery driver (app-side).
//!
//! Wraps [`onecad_core::io::recovery`] (the layout + marker lifecycle) with the
//! app's scheduling, filesystem writes and event emission. The core module owns
//! **where** autosaves live (`<app_data>/autosave/<documentId>.onecad`) and the
//! `SessionMarker` (pid crash marker); this module owns **when** and **how** they
//! are written.
//!
//! ## The driver
//!
//! [`run`] is a debounced loop the app spawns once at startup. It subscribes to a
//! `watch` "mutation tick" (bumped by every document-mutating command via
//! [`AppState::note_mutation`](crate::state::AppState::note_mutation)): after the
//! first change it waits [`AUTOSAVE_DEBOUNCE`] of quiet — but never longer than
//! [`AUTOSAVE_MAX_AGE`] from that first change — then writes an autosave for the
//! currently-open document and emits
//! [`events::AUTOSAVE`](crate::events::AUTOSAVE).
//!
//! Two gates decide whether a wake produces a write, and they answer different
//! questions:
//!
//! * **no document open** ⇒ skip (zero autosave activity on the start screen);
//! * **document not dirty** ⇒ skip. A tick means "something ran", not "there is
//!   unsaved work" — a published regen ticks the loop, and opening a project
//!   schedules one. Writing on that armed a crash marker for documents the user had
//!   only looked at, and the next launch offered them back as "unsaved changes".
//!
//! Explicit host recovery flushes use the same admission lane and dirty gates.
//! The periodic driver retries failed dirty checkpoints on a fixed deadline without
//! requiring another edit. The age limit bounds scheduling; admission and IO add time.
//!
//! ## Recovery lifecycle
//!
//! * a clean [`save_document`](crate::api::save_document) or clean
//!   [`close_document`](crate::api::close_document) calls [`clear_recovery_state`]
//!   (remove the marker + delete the stale autosave);
//! * dirty close preserves the container and releases marker ownership with
//!   `pid = 0`; only an explicit discard authorizes cleanup of dirty work;
//! * the next launch scans the autosave directory for CONTAINERS no live session
//!   owns — a released marker, a dead `pid` (per [`pid_alive`]), or no
//!   marker at all — and offers them
//!   ([`check_recovery`](crate::api::check_recovery)). Discovery is keyed to the
//!   container, not the marker, so losing a marker costs a label rather than the
//!   whole document.
//!
//! ## The persistence lane (VF-B7 / M2 / M3)
//!
//! Autosave acquires the persistence lane before its short runtime snapshot so
//! queued older work cannot overwrite a newer checkpoint. Both writers build a
//! [`SavePayload`](crate::document_runtime::SavePayload) under the runtime lock, release
//! that lock, then do the
//! container write on a `spawn_blocking` thread. That keeps serialization + deflate of
//! up to a 256 MiB import blob off the single-writer lock, so an edit is never blocked
//! by a save.
//!
//! Two writers off one lock need their own ordering, which is the `lane` parameter
//! both take: a `tokio::sync::Mutex<()>` (`AppState::persistence`) serializing
//! {container write + recovery-marker mutation}. No function awaits the lane
//! while holding the runtime lock. Autosave admits its lane first, then briefly
//! locks the runtime for the current accepted snapshot and session fence. Explicit
//! save snapshots before its lane admission. Neither holds the runtime during I/O.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[path = "autosave_checkpoint.rs"]
mod checkpoint_store;

use crate::host::HostContext;
use tokio::sync::{watch, Mutex};

use onecad_core::ids::DocumentId;
use onecad_core::io::container::SaveMeta;
use onecad_core::io::recovery::{
    autosave_dir, autosave_path, remove_marker, write_marker, SessionMarker,
};
use onecad_core::io::IoError;

use crate::document_runtime::{DocumentRuntime, SaveCaches, SavePayload};

/// Quiet window after the last document mutation before an autosave fires
/// (V1/V2 lifecycle). A debounce, not a fixed cadence: a busy editor does not
/// autosave mid-burst, and an idle-but-dirty document autosaves once, ~30 s after
/// the last edit.
pub const AUTOSAVE_DEBOUNCE: Duration = Duration::from_secs(30);

/// Ceiling on how long the debounce may defer a write once the first unsaved edit
/// has landed.
///
/// A debounce ALONE starves: sustained modelling re-arms the quiet window on every
/// edit, so a user who never pauses for [`AUTOSAVE_DEBOUNCE`] never gets an
/// autosave at all, and a crash takes the whole session with it. The deadline turns
/// "30 s after you stop" into "30 s after you stop, or 60 s after you started,
/// whichever comes first" — the debounce still wins in the common case, and it can
/// no longer be starved.
///
/// This bounds scheduling, not durable completion: admission and IO take time,
/// and failed writes are surfaced and retried rather than claimed as recoverable.
pub const AUTOSAVE_MAX_AGE: Duration =
    Duration::from_secs(onecad_core::io::recovery::AUTOSAVE_INTERVAL_SECS);

/// A failed checkpoint retries without an edit; mutations never postpone this delay.
pub const AUTOSAVE_RETRY_DELAY: Duration = Duration::from_secs(5);

static NEXT_ATTEMPT: AtomicU64 = AtomicU64::new(1);

/// One autosave ATTEMPT — the payload of the [`events::AUTOSAVE`](crate::events::AUTOSAVE)
/// event, fenced to its document, runtime session and accepted revision.
/// `error` absent ⇒ the container + marker were
/// written; present ⇒ the write at `path` was attempted and failed (`path` is the
/// INTENDED target, not a written one) — see [`autosave_current_reporting`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutosaveEvent {
    pub document_id: String,
    pub runtime_session: String,
    /// The accepted snapshot attempted, even when its write fails.
    pub document_revision: u64,
    /// Process-local ordering across independent checkpoint event producers.
    pub attempt_id: u64,
    /// The autosave container written, or (on failure) the one that was attempted.
    pub path: String,
    /// Wall-clock time of the write attempt, in Unix-epoch milliseconds.
    pub at_ms: u64,
    /// Why the attempt failed. Absent on a completed autosave.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// The app-data root the autosave layout lives under
/// (`<app_data>/autosave/...`), or `None` in a headless / permission-denied
/// environment (autosave then degrades to a no-op).
#[must_use]
pub fn autosave_root(app: &HostContext) -> Option<PathBuf> {
    app.paths.app_data.clone()
}

/// Whether `pid` names a live process — the liveness predicate
/// [`scan_recoverable`](onecad_core::io::recovery::scan_recoverable) injects.
///
/// On Unix, `kill(pid, 0)`: `0` ⇒ alive; `EPERM` ⇒ alive (exists, not ours);
/// `ESRCH` ⇒ dead. Elsewhere the conservative answer is **alive** — it merely
/// defers recovery to a later launch, never a false "safe to discard".
///
/// iOS: the same-pid rule. One app process owns every scene, so a marker or
/// workspace stamped with another pid belongs to a previous launch. `kill` is
/// not asked: in the sandbox a recycled pid of another app answers `EPERM`,
/// which the rule above would read as alive and so never offer recovery.
#[must_use]
pub fn pid_alive(pid: u32) -> bool {
    #[cfg(target_os = "ios")]
    {
        pid == std::process::id()
    }
    #[cfg(all(unix, not(target_os = "ios")))]
    {
        let ret = unsafe { libc::kill(pid as libc::pid_t, 0) };
        if ret == 0 {
            return true;
        }
        matches!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::EPERM)
        )
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        true
    }
}

/// The outcome of one [`attempt`] — richer than [`autosave_current`]'s collapsed
/// `Option`, so a caller that surfaces autosave state to the user can tell "there
/// was nothing to do" from "we tried and it broke".
pub enum AutosaveAttempt {
    /// No document open, or the open one is not dirty — nothing was attempted.
    Skipped,
    /// The container + marker were written.
    Written(AutosaveEvent),
    /// A write was attempted and failed (already logged); `AutosaveEvent.error`
    /// carries why and `AutosaveEvent.path` is the target that was attempted.
    Failed(AutosaveEvent),
}

/// Writes an autosave container + refreshes the session crash marker for the
/// currently-open document. Returns the [`AutosaveEvent`], or `None` when **no
/// document is open**, when the document is **not dirty**, or when the write
/// failed (logged) — the frozen contract `tests/recovery_hardening.rs` RC-10 pins:
/// a failed attempt must never be mistaken for a completed one. Callers that also
/// need to SURFACE a failure (rather than merely never report a false success) use
/// [`autosave_current_reporting`] instead — both share [`attempt`].
///
/// ## Why the dirty gate
///
/// The driver is woken by a mutation TICK, and a tick is not evidence of unsaved
/// work: a published regen ticks it (`crate::make_regen_driver`), and
/// `open_document` schedules an initial regen — so merely OPENING a project used
/// to arm an autosave container plus a crash marker for a document with zero user
/// edits. Kill the process and the next launch offered "unsaved changes" for work
/// that was never touched. `dirty` is the authority on whether anything is at
/// risk; the tick only says when to look.
///
/// This gate is only safe while every mutation that `build_save_payload` persists
/// also sets `dirty` — see the classification table in
/// `tests/recovery_hardening.rs`.
///
/// The document's live save path and dirty flag are untouched (this is a recovery
/// snapshot, not a real save — see [`DocumentRuntime::build_save_payload`]).
pub async fn autosave_current(
    runtime: &Mutex<Option<DocumentRuntime>>,
    app_data: &Path,
    lane: &Mutex<()>,
) -> Option<AutosaveEvent> {
    match attempt(runtime, app_data, lane, None).await {
        AutosaveAttempt::Written(ev) => Some(ev),
        AutosaveAttempt::Skipped | AutosaveAttempt::Failed(_) => None,
    }
}

/// Same as [`autosave_current`], except a FAILED attempt is also reported (as an
/// [`AutosaveEvent`] carrying `error`) rather than collapsed to `None`. Used by the
/// debounced [`run`] driver, which emits [`events::AUTOSAVE`](crate::events::AUTOSAVE) —
/// so a broken autosave is visible instead of silently absent.
pub async fn autosave_current_reporting(
    runtime: &Mutex<Option<DocumentRuntime>>,
    app_data: &Path,
    lane: &Mutex<()>,
) -> Option<AutosaveEvent> {
    match attempt(runtime, app_data, lane, None).await {
        AutosaveAttempt::Written(ev) | AutosaveAttempt::Failed(ev) => Some(ev),
        AutosaveAttempt::Skipped => None,
    }
}

/// The shared attempt logic — see [`autosave_current`] for the gate/lock-order
/// contract both public wrappers rely on.
///
/// **Off the runtime lock.** The lock is held only for the
/// [`SavePayload`](crate::document_runtime::SavePayload) snapshot; the container
/// write runs on a blocking thread under `lane` (see the module docs), and the join
/// handle is **awaited** — a detached write would let two writers race one target
/// path.
///
/// The marker is written only after a successful container write, and under the same
/// `lane` acquisition, so a marker never advertises an autosave that does not exist.
/// Captures only the requested runtime session; replacements cannot redirect a flush.
pub async fn checkpoint(
    runtime: &Mutex<Option<DocumentRuntime>>,
    app_data: &Path,
    lane: &Mutex<()>,
    session: &str,
) -> AutosaveAttempt {
    attempt(runtime, app_data, lane, Some(session)).await
}

/// Stops waiting for persistence admission when the host closes. Once admitted,
/// the full disk transaction remains awaited under its lane guard.
pub async fn checkpoint_until(
    runtime: &Mutex<Option<DocumentRuntime>>,
    app_data: &Path,
    lane: &Mutex<()>,
    session: Option<&str>,
    stop: impl std::future::Future<Output = ()>,
) -> AutosaveAttempt {
    let guard = tokio::select! {
        biased;
        () = stop => return AutosaveAttempt::Skipped,
        guard = lane.lock() => guard,
    };
    attempt_admitted(runtime, app_data, session, guard).await
}

async fn attempt(
    runtime: &Mutex<Option<DocumentRuntime>>,
    app_data: &Path,
    lane: &Mutex<()>,
    expected_session: Option<&str>,
) -> AutosaveAttempt {
    let guard = lane.lock().await;
    attempt_admitted(runtime, app_data, expected_session, guard).await
}

async fn attempt_admitted(
    runtime: &Mutex<Option<DocumentRuntime>>,
    app_data: &Path,
    expected_session: Option<&str>,
    _lane: tokio::sync::MutexGuard<'_, ()>,
) -> AutosaveAttempt {
    let now = now_rfc3339();
    let meta = SaveMeta {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        occt_fingerprint: None,
        created: now.clone(),
        modified: now.clone(),
    };
    // ── Under the runtime lock: snapshot only. No IO, no serialization. ──
    //    `SaveCaches::none()`: an autosave writes no mesh/preview cache. It fires on
    //    a timer against a document the user is actively editing, and its container
    //    is only ever read by crash recovery — which regens from 0 regardless. The
    //    "paint at open" caches belong to an explicit save.
    let (payload, doc_id, session, opened, title, identity) = {
        let mut guard = runtime.lock().await;
        let Some(rt) = guard.as_mut() else {
            return AutosaveAttempt::Skipped; // no document open.
        };
        if expected_session.is_some_and(|session| !rt.matches_runtime_session(session))
            || !rt.is_dirty()
        {
            return AutosaveAttempt::Skipped; // nothing at risk.
        }
        let doc_id = rt.document_uuid();
        let opened = rt.path().map(Path::to_path_buf);
        let title = rt.title().to_string();
        let identity = AttemptIdentity {
            document_id: doc_id.to_string(),
            runtime_session: rt.runtime_session(),
            document_revision: rt.revision().0,
            attempt_id: NEXT_ATTEMPT.fetch_add(1, Ordering::Relaxed),
        };
        (
            rt.build_save_payload(meta, SaveCaches::none()),
            doc_id,
            rt.runtime_session(),
            opened,
            title,
            identity,
        )
    };
    let path = autosave_path(app_data, doc_id);

    // The marker writer creates the dir, but the container writer needs it first.
    if let Err(e) = std::fs::create_dir_all(autosave_dir(app_data)) {
        tracing::warn!("autosave: create dir failed: {e}");
        return AutosaveAttempt::Failed(identity.event(&path, Some(format!("create dir: {e}"))));
    }

    // ── Persistence lane: container write + marker, serialized against an
    //    explicit save's write + recovery clearing (M2/M3). ──
    // A snapshot queued behind replacement cleanup must not resurrect discarded work.
    if !runtime
        .lock()
        .await
        .as_ref()
        .is_some_and(|runtime| runtime.matches_runtime_session(&session) && runtime.is_dirty())
    {
        return AutosaveAttempt::Skipped;
    }
    let marker = SessionMarker {
        document_id: doc_id,
        pid: std::process::id(),
        opened_path: opened,
        last_autosave: now,
        title: Some(title),
    };
    let target = path.clone();
    let root = app_data.to_owned();
    let write = tokio::task::spawn_blocking(move || {
        checkpoint_store::preserve_pair(
            &target,
            &onecad_core::io::recovery::marker_path(&root, doc_id),
            || {
                DocumentRuntime::write_payload(&target, &payload)?;
                write_marker(&root, &marker)
            },
        )
    })
    .await;
    match write {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            return AutosaveAttempt::Failed(
                identity.event(&path, Some(format!("write checkpoint: {error}"))),
            );
        }
        Err(error) => {
            return AutosaveAttempt::Failed(
                identity.event(&path, Some(format!("write task: {error}"))),
            );
        }
    }
    AutosaveAttempt::Written(identity.event(&path, None))
}

struct AttemptIdentity {
    document_id: String,
    runtime_session: String,
    document_revision: u64,
    attempt_id: u64,
}

impl AttemptIdentity {
    fn event(self, path: &Path, error: Option<String>) -> AutosaveEvent {
        AutosaveEvent {
            document_id: self.document_id,
            runtime_session: self.runtime_session,
            document_revision: self.document_revision,
            attempt_id: self.attempt_id,
            path: path.to_string_lossy().into_owned(),
            at_ms: now_ms(),
            error,
        }
    }
}

/// The explicit save's **persistence half**: container write (off the runtime lock,
/// on a blocking thread) followed by recovery-state clearing, both under `lane`.
///
/// Split out of [`save_document`](crate::api::save_document) so the two lanes share
/// one ordering primitive and one code path. Clearing the marker INSIDE the lane is
/// what closes M2 — an autosave that already holds the lane finishes first and its
/// marker is then removed, instead of landing after the clear and resurrecting a
/// bogus recovery offer for a cleanly-saved document.
///
/// Residual (accepted, documented): an autosave that snapshotted its payload *before*
/// this save but only reaches the lane *after* it re-writes its marker. The offer that
/// produces is spurious but harmless — the marker still points at a real autosave
/// container holding a superset of the saved work, so recovery cannot lose anything.
///
/// `recovery_root` `None` ⇒ headless / no app-data dir; nothing to clear.
///
/// # Errors
/// [`IoError`] when the container write fails — the target is untouched, and the
/// recovery state is deliberately NOT cleared (a failed save must not discard the
/// crash snapshot that is now the only copy of the work).
pub async fn write_save_payload(
    lane: &Mutex<()>,
    path: &Path,
    payload: SavePayload,
    recovery_root: Option<&Path>,
    document_id: DocumentId,
) -> Result<(), IoError> {
    let _lane = lane.lock().await;
    let target = path.to_path_buf();
    match tokio::task::spawn_blocking(move || DocumentRuntime::write_payload(&target, &payload))
        .await
    {
        Ok(result) => result?,
        Err(e) => return Err(IoError::Io(format!("save task did not complete: {e}"))),
    }
    // A clean save supersedes any crash-recovery state for this document.
    if let Some(root) = recovery_root {
        clear_recovery_state(root, document_id);
    }
    Ok(())
}

/// Clears a document's crash marker + stale autosave after a clean save/close or
/// explicit discard. Best-effort: a missing marker/file is not an error.
pub fn clear_recovery_state(app_data: &Path, document_id: DocumentId) {
    if let Err(e) = remove_marker(app_data, document_id) {
        tracing::warn!("clear recovery: remove marker failed: {e}");
    }
    let autosave = autosave_path(app_data, document_id);
    match std::fs::remove_file(&autosave) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => tracing::warn!("clear recovery: remove autosave {autosave:?}: {e}"),
    }
}

/// The debounced autosave loop (spawned once at startup). Blocks on `tick` until a
/// mutation lands, waits `debounce` of quiet — but no longer than `max_age` from the
/// mutation that opened the cycle — autosaves the open document, and hands the
/// [`AutosaveEvent`] to `emit`. Returns when every `tick` sender is dropped (app
/// shutdown).
///
/// **The deadline is what makes this loop honest.** A pure debounce is starvable:
/// sustained editing re-arms the quiet window forever, so the one user who most
/// needs an autosave — the one who has been modelling without pause for ten minutes
/// — is the one who never gets one. `max_age` bounds that at the cost of a write
/// during a long burst.
///
/// `lane` is the shared persistence lane (`AppState::persistence`) — see the module
/// docs for the lock order.
pub async fn run<F>(
    runtime: Arc<Mutex<Option<DocumentRuntime>>>,
    app_data: PathBuf,
    lane: Arc<Mutex<()>>,
    tick: watch::Receiver<u64>,
    debounce: Duration,
    max_age: Duration,
    emit: F,
) where
    F: Fn(AutosaveEvent),
{
    let timing = (debounce, max_age);
    run_until(
        runtime,
        app_data,
        lane,
        tick,
        timing,
        std::future::pending(),
        emit,
    )
    .await;
}

/// [`run`], also returning when `stop` resolves. `stop` is observed only while the
/// loop waits, never during a write, so an admitted autosave always completes —
/// this is how a closing document host stops its own lane on a shared runtime.
pub async fn run_until<F>(
    runtime: Arc<Mutex<Option<DocumentRuntime>>>,
    app_data: PathBuf,
    lane: Arc<Mutex<()>>,
    tick: watch::Receiver<u64>,
    timing: (Duration, Duration),
    stop: impl std::future::Future<Output = ()>,
    emit: F,
) where
    F: Fn(AutosaveEvent),
{
    run_driver(
        runtime,
        app_data,
        lane,
        tick,
        (timing.0, timing.1, AUTOSAVE_RETRY_DELAY),
        stop,
        emit,
    )
    .await;
}

async fn run_driver<F>(
    runtime: Arc<Mutex<Option<DocumentRuntime>>>,
    app_data: PathBuf,
    lane: Arc<Mutex<()>>,
    mut tick: watch::Receiver<u64>,
    (debounce, max_age, retry_delay): (Duration, Duration, Duration),
    stop: impl std::future::Future<Output = ()>,
    emit: F,
) where
    F: Fn(AutosaveEvent),
{
    let mut stop = std::pin::pin!(stop);
    let mut retry: Option<(tokio::time::Instant, String)> = None;
    while wait_for_attempt(
        &mut tick,
        stop.as_mut(),
        (debounce, max_age),
        retry.as_ref().map(|(deadline, _)| *deadline),
    )
    .await
    {
        let session = retry.take().map(|(_, session)| session);
        let mut outcome = attempt(&runtime, &app_data, &lane, session.as_deref()).await;
        if session.is_some() && matches!(outcome, AutosaveAttempt::Skipped) {
            // A replacement's mutation may already have been consumed while the
            // old retry waited. Snapshot current dirty work without deferring it again.
            outcome = attempt(&runtime, &app_data, &lane, None).await;
        }
        match outcome {
            AutosaveAttempt::Failed(event) => {
                retry = Some((
                    tokio::time::Instant::now() + retry_delay,
                    event.runtime_session.clone(),
                ));
                emit(event);
            }
            AutosaveAttempt::Written(event) => emit(event),
            AutosaveAttempt::Skipped => {}
        }
    }
}

async fn wait_for_attempt(
    tick: &mut watch::Receiver<u64>,
    mut stop: std::pin::Pin<&mut impl std::future::Future<Output = ()>>,
    (debounce, max_age): (Duration, Duration),
    retry: Option<tokio::time::Instant>,
) -> bool {
    if retry.is_none() {
        tokio::select! {
            biased;
            () = stop.as_mut() => return false,
            result = tick.changed() => if result.is_err() { return false; },
        }
    }
    let deadline = retry.unwrap_or_else(|| tokio::time::Instant::now() + max_age);
    loop {
        let quiet = retry.unwrap_or_else(|| tokio::time::Instant::now() + debounce);
        tokio::select! {
            biased;
            () = stop.as_mut() => return false,
            () = tokio::time::sleep_until(quiet) => return true,
            () = tokio::time::sleep_until(deadline) => return true,
            result = tick.changed() => if result.is_err() { return false; },
        }
    }
}

/// Current wall-clock time in Unix-epoch milliseconds.
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Current UTC time as an RFC-3339 string (shares the calendar-free helper the
/// save path uses).
fn now_rfc3339() -> String {
    crate::api::rfc3339_from_secs(now_ms() / 1000)
}

#[cfg(test)]
#[path = "autosave_driver_tests.rs"]
mod driver_tests;
