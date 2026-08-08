//! Tauri command handlers — the webview → Rust API surface.
//!
//! Commands are **thin**: they lock the single-writer runtime, delegate to a
//! [`DocumentRuntime`] method (all the domain logic — testable without a webview),
//! emit the projection/document events, and return a DTO. The command set mirrors
//! the frontend `CadClient` seam (`src/ipc/client.ts`); F-WP8 swaps its mock for a
//! `tauriClient` that calls these. No webview capability is widened — Rust does all
//! filesystem/dialog IO (capabilities stay `core:default`).
//!
//! ## Tracing (DEV-OBSERVABILITY wave R)
//!
//! The **baseline is free**: tauri's `tracing` cargo feature wraps every
//! `#[tauri::command]` in an `ipc::request::handler` debug span carrying the command
//! name + timing, so a new command is observable the day it is added. Only commands
//! whose *arguments* identify the work are hand-`#[instrument]`ed on top (always
//! `skip_all` + explicit id fields — payloads never enter the log).
//!
//! Two deliberate omissions:
//! * `err(…)` is only ever attached to a `Result`-returning command (it is a compile
//!   error otherwise), and
//! * NEVER to a drag-frequency command (`preview_op`/`solve_drag`/`get_mesh`/
//!   `get_sketch_regions`): `err` records at ERROR, and a stale-preview race is
//!   routine, so it would ERROR-flood once per drag frame. Those keep their
//!   hand-rolled debug/warn match.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use tauri::{AppHandle, Emitter, Manager, State};

use onecad_core::document::record::Operation;
use onecad_core::document::refs::{AnchorIntent, ElementKind, ElementRef, PrimaryRef};
use onecad_core::edit::{EditCommand, SketchEditOp};
use onecad_core::ids::{
    BodyId, ConstraintId, ElementId, EntityId, RecordId, SketchId, SnapshotId, TopoKey,
};
use onecad_core::io::container::SaveMeta;
use onecad_core::io::recovery::{scan_stale_markers, RecoveryOffer};
use onecad_core::regen::{RegenRequest, ResolveRef, ResolveRequest};

use crate::autosave;
use crate::document_runtime::{DocumentRuntime, RegenReport, SaveCaches};
use crate::dto::{
    BeginGestureDto, DocumentProjection, DocumentSnapshotDto, DragSolveDto, FeatureDependenciesDto,
    FinishSketchDto, PromotedElementDto, RecentProjectDto, RecoveryInfoDto, ResolveRefDto,
    SketchOnFaceDto, SketchSessionDto, SketchUpsertDto,
};
use crate::error::ApiError;
use crate::events;
use crate::recents;
use crate::state::AppState;
use crate::worker::{lod_from_str, wire};

// ─────────────────────────────────────────────────────────────────────────────
// Lifecycle
// ─────────────────────────────────────────────────────────────────────────────

/// Builds this document's backend and opens `path` over it, rolling the staged
/// backend swap back if the container will not load
/// ([`AppState::rollback_backend`]) — so a bad file leaves the CURRENTLY open
/// document untouched, worker included.
///
/// On success the swap is left **staged**: the caller publishes the runtime and only
/// then calls [`AppState::commit_backend`].
///
/// Factored out of [`open_document`] / [`recover_document`] deliberately. Those take
/// a concrete `AppHandle`, which `tauri::test::mock_app` (a `MockRuntime`) cannot
/// produce, so the rollback contract would otherwise be untestable — see
/// `tests/prewarm.rs`.
pub fn open_runtime_over_new_backend(
    state: &AppState,
    path: &Path,
) -> Result<DocumentRuntime, ApiError> {
    let (engine, meshes, solver) = state.make_backend();
    DocumentRuntime::open(path, engine, meshes, solver).map_err(|e| {
        state.rollback_backend();
        ApiError::from(e)
    })
}

/// Async sibling of [`open_runtime_over_new_backend`] — the same swap + open +
/// rollback-on-`Err` contract, but off the async executor thread.
///
/// `DocumentRuntime::open` does sync zip parsing (`ContainerReader::open`) plus
/// import-blob materialization to temp files: blocking IO that, run directly on a
/// `#[tauri::command]`'s task, stalls every OTHER in-flight async command sharing
/// that worker thread with it (there is no `spawn_blocking` boundary today).
/// [`tokio::task::spawn_blocking`] moves that work to the blocking pool instead.
///
/// Rollback fires on BOTH failure paths: `DocumentRuntime::open` returning `Err`
/// (unchanged contract) and the blocking task itself not completing
/// (`JoinError` — a panic) — either way [`AppState::make_backend`]'s staged swap
/// must not be left dangling, or a surviving document loses its worker for
/// nothing.
pub async fn open_runtime_over_new_backend_blocking(
    state: &AppState,
    path: PathBuf,
) -> Result<DocumentRuntime, ApiError> {
    let (engine, meshes, solver) = state.make_backend();
    match tokio::task::spawn_blocking(move || DocumentRuntime::open(&path, engine, meshes, solver))
        .await
    {
        Ok(Ok(rt)) => Ok(rt),
        Ok(Err(e)) => {
            state.rollback_backend();
            Err(ApiError::from(e))
        }
        Err(join_err) => {
            state.rollback_backend();
            Err(ApiError::Internal(format!(
                "document open task did not complete: {join_err}"
            )))
        }
    }
}

/// Fires the `from: 0` regen a freshly opened/imported/recovered document needs,
/// gated on the backend's [`WorkerReadiness`](crate::worker::WorkerReadiness)
/// rather than enqueued immediately.
///
/// **The cold-worker regen race**: an immediate request can beat a still-
/// connecting worker — `execute_plan` on a not-yet-`Ready` worker fails fast
/// ([`EngineError::NotConnected`](onecad_core::regen::EngineError::NotConnected))
/// as `PlanEvent::Failed`, and a
/// from-0 plan has **no retry**: the executor's checkpoint-retry path needs a
/// prior successful attempt to fall back to, and there isn't one yet. With
/// [`AppState::prewarm`] the wait below resolves near-instantly in the common
/// case — it only pays real latency on a genuinely cold start.
///
/// Spawned rather than awaited so the command still returns immediately; on
/// timeout the request still fires (best-effort), and a failure surfaces through
/// the normal `regen-finished` event path instead of silently never running.
///
/// Production timeout is fixed at 8 s ([`schedule_initial_regen`]); the wait
/// itself is factored out with an explicit `timeout` so a test can inject a
/// short one instead of paying out the real window.
///
/// `still_current` is consulted AFTER the wait: the gate can outlive its document
/// — a close/open during the window retires the worker this readiness watches and
/// installs a NEW runtime whose own gate is pending — and a stale gate firing then
/// would race that document's handshake with a from-0 replay it never asked for
/// (or supersede its active regen). The scheduler is global, so the request alone
/// carries no document identity; the probe supplies it.
fn spawn_gated_regen<C, Fut>(
    readiness: std::sync::Arc<dyn crate::worker::WorkerReadiness>,
    sched: crate::state::SharedScheduler,
    timeout: std::time::Duration,
    still_current: C,
) where
    C: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = bool> + Send,
{
    tauri::async_runtime::spawn(async move {
        let _ = readiness.wait_ready(timeout).await;
        if !still_current().await {
            return;
        }
        if let Some(handle) = sched.get() {
            handle.request(RegenRequest::ToEnd { from: 0 });
        }
    });
}

/// Production entry point for [`spawn_gated_regen`] — the 8 s window is generous
/// relative to [`AppState::prewarm`]'s near-instant common case, while still
/// bounding how long a genuinely stuck cold start can silently withhold the
/// document's first regen. The document uuid captured here is the staleness probe:
/// the gate fires only while the SAME document is still the open one.
async fn schedule_initial_regen(state: &AppState) {
    let expected = match state.runtime.lock().await.as_ref() {
        Some(rt) => rt.document_uuid(),
        None => return, // no document — nothing to rebuild
    };
    let runtime = state.runtime.clone();
    spawn_gated_regen(
        state.readiness(),
        state.scheduler.clone(),
        std::time::Duration::from_secs(8),
        move || async move {
            runtime
                .lock()
                .await
                .as_ref()
                .is_some_and(|rt| rt.document_uuid() == expected)
        },
    );
}

/// Creates a blank document and opens it (`CadClient.newDocument`).
#[tauri::command]
pub async fn new_document(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<DocumentSnapshotDto, ApiError> {
    let (engine, meshes, solver) = state.make_backend();
    let (snapshot, projection) = {
        let mut guard = state.runtime.lock().await;
        *guard = Some(DocumentRuntime::new_blank(engine, meshes, solver));
        let rt = guard.as_ref().unwrap();
        rt.adopt_current_epoch(); // VF-B4: a restart before this insert never reached it.
        (snapshot_of(rt), rt.projection())
    };
    // The new runtime is published, so the previous document's worker is now
    // unreachable — retire it (see `AppState::make_backend`'s two-phase contract).
    // Nothing between the swap and here can fail, so there is no rollback arm.
    state.commit_backend();
    let _ = app.emit(events::PROJECTION_UPDATED, &projection);
    Ok(snapshot)
}

/// Opens an existing `.onecad` project (`CadClient.openDocument`).
///
/// **Recovery seam (V1 scope).** Crash recovery is startup-only: a crashed session
/// is surfaced by [`check_recovery`] before any document is opened. The
/// "open a path that has a *newer* autosave than the file on disk" case is **not**
/// handled here yet — the autosave layout is keyed by `documentId`, not by the
/// on-disk path, so matching a just-opened file to a stale autosave needs a
/// path→documentId index. Deferred to a later WP; the startup scan covers the
/// crash-then-relaunch flow that matters most.
#[tauri::command]
#[tracing::instrument(skip_all, fields(path = %path), err(Display))]
pub async fn open_document(
    state: State<'_, AppState>,
    app: AppHandle,
    path: String,
) -> Result<DocumentSnapshotDto, ApiError> {
    let rt = open_runtime_over_new_backend_blocking(&state, PathBuf::from(&path)).await?;
    let (snapshot, projection) = {
        let mut guard = state.runtime.lock().await;
        *guard = Some(rt);
        let rt = guard.as_ref().unwrap();
        rt.adopt_current_epoch(); // VF-B4: a restart before this insert never reached it.
        (snapshot_of(rt), rt.projection())
    };
    state.commit_backend();
    let _ = app.emit(events::PROJECTION_UPDATED, &projection);
    recents::record(&app, Path::new(&path));
    // Rebuild geometry from the loaded (all-Dirty) timeline, once the worker can
    // actually serve it (readiness-gated — see `schedule_initial_regen`).
    schedule_initial_regen(&state).await;
    Ok(snapshot)
}

/// **Start-screen lane**: opens a NEW document whose first (and only) feature is an
/// `ImportStep` of `path` (`CadClient.importStep`). The in-editor "Import STEP…"
/// lane is [`insert_step`], which appends to the document already open.
///
/// Responds exactly like [`open_document`] — a [`DocumentSnapshotDto`] plus a
/// `projection-updated` emit and a from-0 regen request — because the frontend
/// treats it identically (`appStore.importStep` calls `enter(...)` on the result).
///
/// The probe runs BEFORE the new runtime is published, so an unreadable STEP file
/// leaves the previous screen state untouched: no document is swapped in, and the
/// error surfaces as the start screen's `importError`.
///
/// **`path` must come from [`step_file_dialog`], not `open_file_dialog`** — the
/// latter filters `.onecad`, so a user could never select a `.step` through it.
#[tauri::command]
#[tracing::instrument(skip_all, fields(path = %path), err(Display))]
pub async fn import_step(
    state: State<'_, AppState>,
    app: AppHandle,
    path: String,
) -> Result<DocumentSnapshotDto, ApiError> {
    let (engine, meshes, solver) = state.make_backend();
    // Worker round-trip FIRST, with no lock held and no runtime published (the
    // R-WP11 rule); the backend was just built, so `state.step_import()` is this
    // document's worker.
    //
    // `appStore.importStep` catches a failure here and leaves the user on their
    // current document, so both fallible steps must roll the staged backend swap back
    // — that document is still live and still needs its own worker.
    let prepared = crate::imports::prepare_import(&*state.step_import(), Path::new(&path))
        .await
        .inspect_err(|_| state.rollback_backend())?;
    let mut rt = DocumentRuntime::new_blank(engine, meshes, solver);
    rt.add_import_record(&prepared, false)
        .inspect_err(|_| state.rollback_backend())?;
    let (snapshot, projection) = {
        let mut guard = state.runtime.lock().await;
        *guard = Some(rt);
        let rt = guard.as_ref().unwrap();
        rt.adopt_current_epoch(); // VF-B4: a restart before this insert never reached it.
        (snapshot_of(rt), rt.projection())
    };
    state.commit_backend();
    let _ = app.emit(events::PROJECTION_UPDATED, &projection);
    schedule_initial_regen(&state).await;
    state.note_mutation();
    Ok(snapshot)
}

/// **In-editor lane**: appends an `ImportStep` record to the OPEN document
/// (`CadClient.insertStep`), inserted at the rollback cursor like every other
/// authored feature.
///
/// Rust owns the `.step` dialog (the webview has zero fs capability), so this takes
/// no arguments and resolves `Ok(None)` when the dialog is cancelled — the house
/// shape the frontend's `insertStep` correlates against (a null projection cancels
/// its regen awaiter). Otherwise it responds like `apply_edit_command`: the
/// pre-regen [`DocumentProjection`], with the authoritative geometry arriving on
/// the normal `document-changed` / `projection-updated` events.
#[tauri::command]
#[tracing::instrument(skip_all, err(Display))]
pub async fn insert_step(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Option<DocumentProjection>, ApiError> {
    {
        // Presence check only — the lock is NOT held across the probe below.
        let guard = state.runtime.lock().await;
        guard
            .as_ref()
            .ok_or_else(|| ApiError::NoDocument("insertStep".into()))?;
    }
    let Some(path) = pick_step_open(app.clone()).await else {
        return Ok(None); // dialog cancelled
    };
    let prepared = crate::imports::prepare_import(&*state.step_import(), Path::new(&path)).await?;
    let (outcome, projection) = {
        let mut guard = state.runtime.lock().await;
        let rt = guard
            .as_mut()
            .ok_or_else(|| ApiError::NoDocument("insertStep".into()))?;
        let outcome = rt.add_import_record(&prepared, true)?;
        (outcome, rt.projection())
    };
    let _ = app.emit(events::PROJECTION_UPDATED, &projection);
    if let Some(sched) = state.scheduler.get() {
        sched.handle(&outcome);
    }
    state.note_mutation();
    Ok(Some(projection))
}

/// Saves the open document (`CadClient` save). `path` `None` reuses the last save
/// path; an unsaved document with no path is an error (the frontend's Save action
/// then falls back to Save As). Records the saved path in the recents store.
///
/// `preview_png` is an optional base64 PNG capture of the viewport, stored as the
/// container's `preview.png` and served to the start screen by
/// [`list_recents`](crate::api::list_recents). It is **entirely optional**: a
/// frontend that does not send it (and Tauri passes `None` for an argument the
/// caller omits entirely) keeps whatever thumbnail the document already had, and a
/// malformed or oversized capture is dropped with a warning. A thumbnail never
/// fails a save.
///
/// ## Off the single-writer lock (VF-B7 / plan F1a)
///
/// The two slow parts of a save — the worker `SaveCheckpoint` round-trip and the
/// container write (JSON + deflate of every import blob) — used to run with the
/// runtime lock held, stalling every edit for their whole duration. They now run
/// unlocked, in five phases:
///
/// 1. locked: resolve the target path, admit a checkpoint
///    ([`prepare_checkpoint`](DocumentRuntime::prepare_checkpoint)), clone the engine;
/// 2. **unlocked**: `SaveCheckpoint`; re-lock briefly to
///    [`adopt_checkpoint`](DocumentRuntime::adopt_checkpoint), whose fencing recheck
///    discards artifacts an interleaved edit or worker restart invalidated;
/// 3. locked: [`build_save_payload`](DocumentRuntime::build_save_payload) + the
///    revision it describes;
/// 4. **unlocked**, under the persistence lane: the container write, then the
///    recovery-marker clear ([`autosave::write_save_payload`]);
/// 5. locked: [`mark_saved`](DocumentRuntime::mark_saved) — adopt the path, and go
///    clean only if no edit landed during the write.
///
/// Every re-lock re-checks the document identity: `close_document` / `open_document`
/// can interleave now that the lock is dropped, and the tail of a save must never be
/// applied to a different document.
#[tauri::command]
#[tracing::instrument(skip_all, fields(path = ?path), err(Display))]
pub async fn save_document(
    state: State<'_, AppState>,
    app: AppHandle,
    path: Option<String>,
    preview_png: Option<String>,
) -> Result<(), ApiError> {
    let preview_png = decode_preview_png(preview_png);
    // ── 1. Locked: target path + checkpoint admission + engine handle. ──
    let (target, document_id, ticket, engine) = {
        let guard = state.runtime.lock().await;
        let rt = guard
            .as_ref()
            .ok_or_else(|| ApiError::NoDocument("save".into()))?;
        let target: PathBuf = match path {
            Some(p) => PathBuf::from(p),
            None => rt
                .path()
                .map(Path::to_path_buf)
                .ok_or_else(|| ApiError::Io("no save path; provide one".into()))?,
        };
        (
            target,
            rt.document_uuid(),
            rt.prepare_checkpoint(),
            rt.engine_arc(),
        )
    };

    // ── 2. Unlocked: the worker round-trip. Checkpoint policy (SCHEMA §7.7) — mint a
    //    durable acceleration base of the current head so a later edit regens
    //    incrementally. Best-effort: a worker failure just skips it (Invariant 7).
    if let Some(ticket) = ticket {
        if let Ok(artifacts) = engine.save_checkpoint(ticket.step()).await {
            let mut guard = state.runtime.lock().await;
            if let Some(rt) = guard
                .as_mut()
                .filter(|rt| rt.document_uuid() == document_id)
            {
                rt.adopt_checkpoint(ticket, artifacts);
            }
        }
    }

    // ── 3. Locked: snapshot the bytes-to-be + the revision they describe. ──
    let (payload, revision) = {
        let mut guard = state.runtime.lock().await;
        let rt = guard
            .as_mut()
            .filter(|rt| rt.document_uuid() == document_id)
            .ok_or_else(|| ApiError::NoDocument("save".into()))?;
        let revision = rt.revision();
        let caches = SaveCaches {
            meshes: true,
            preview_png,
        };
        (rt.build_save_payload(save_meta(), caches), revision)
    };

    // ── 4. Unlocked, persistence lane: write + clear the crash-recovery state. ──
    let recovery_root = autosave::autosave_root(&app);
    autosave::write_save_payload(
        &state.persistence,
        &target,
        payload,
        recovery_root.as_deref(),
        document_id,
    )
    .await?;

    // ── 5. Locked: adopt the path; go clean only if nothing was edited mid-write. ──
    {
        let mut guard = state.runtime.lock().await;
        match guard
            .as_mut()
            .filter(|rt| rt.document_uuid() == document_id)
        {
            Some(rt) => rt.mark_saved(&target, revision),
            None => tracing::warn!(
                "save: the document closed while its container was being written; \
                 the file at {target:?} is complete but no live document adopts it"
            ),
        }
    }
    recents::record(&app, &target);
    Ok(())
}

/// Hard cap on a decoded viewport capture (512 KiB). A start-screen card is a few
/// hundred pixels; anything past this is not a thumbnail, and the container reader
/// caps at 2 MiB anyway ([`MAX_PREVIEW_BYTES`]) — rejecting here means we never
/// write a `preview.png` that our own reader would refuse.
///
/// [`MAX_PREVIEW_BYTES`]: onecad_core::io::container::MAX_PREVIEW_BYTES
const MAX_PREVIEW_PNG_BYTES: usize = 512 * 1024;

/// Decodes the frontend's base64 viewport capture for
/// [`save_document`](crate::api::save_document).
///
/// Accepts either bare base64 or a full `data:image/png;base64,…` URL (what
/// `canvas.toDataURL()` produces) — the standard alphabet contains no `,`, so the
/// last comma can only be the data-URL separator.
///
/// **Every failure is `None` with a warning, never an error.** A save carries the
/// user's work; a thumbnail is decoration. Refusing to persist a document because
/// its screenshot was malformed would be an absurd trade.
fn decode_preview_png(encoded: Option<String>) -> Option<Vec<u8>> {
    use base64::Engine as _;

    let raw = encoded?;
    if raw.is_empty() {
        return None;
    }
    // Bound BEFORE decoding: base64 inflates 3 bytes into 4, so anything past
    // 2× the byte cap cannot be under it and must not be allocated.
    if raw.len() > MAX_PREVIEW_PNG_BYTES * 2 {
        tracing::warn!(
            encoded_len = raw.len(),
            "save: preview capture is far over the {MAX_PREVIEW_PNG_BYTES}-byte cap — dropped"
        );
        return None;
    }
    let b64 = raw.rsplit_once(',').map_or(raw.as_str(), |(_, tail)| tail);
    let bytes = match base64::engine::general_purpose::STANDARD.decode(b64) {
        Ok(bytes) => bytes,
        Err(e) => {
            tracing::warn!(error = %e, "save: preview capture is not valid base64 — dropped");
            return None;
        }
    };
    if bytes.len() > MAX_PREVIEW_PNG_BYTES {
        tracing::warn!(
            len = bytes.len(),
            cap = MAX_PREVIEW_PNG_BYTES,
            "save: preview capture is over the cap — dropped"
        );
        return None;
    }
    Some(bytes)
}

/// Exports every body at head to a STEP file (`CadClient.exportStep`). `path`
/// `None` shows a native save dialog (`.step` filter); a cancel resolves to `None`.
/// Schema is AP214 (`"AP214IS"`); returns the written path. Rust owns the dialog
/// and the worker `ExportStep` verb (the webview has zero fs capability).
#[tauri::command]
pub async fn export_step_file(
    state: State<'_, AppState>,
    app: AppHandle,
    path: Option<String>,
) -> Result<Option<String>, ApiError> {
    let target = match path {
        Some(p) => p,
        None => match pick_step_save(app).await {
            Some(p) => p,
            None => return Ok(None), // dialog cancelled
        },
    };
    let bodies: Vec<BodyId> = {
        let guard = state.runtime.lock().await;
        let rt = guard
            .as_ref()
            .ok_or_else(|| ApiError::NoDocument("exportStep".into()))?;
        rt.head_body_ids()
    };
    let exporter = state.exporter();
    exporter.export_step(&target, &bodies, "AP214IS").await?;
    Ok(Some(target))
}

/// Exports every body at head to a binary STL file (`CadClient.exportStl`). `path`
/// `None` shows a native `.stl` save dialog; a cancel resolves to `None`. Meshed at
/// the `fine` LOD (a mesh that leaves the app wants the best tessellation). Returns
/// the written path. Rust owns the dialog + the worker `ExportStl` verb.
#[tauri::command]
pub async fn export_stl_file(
    state: State<'_, AppState>,
    app: AppHandle,
    path: Option<String>,
) -> Result<Option<String>, ApiError> {
    let target = match path {
        Some(p) => p,
        None => match pick_mesh_save(app, "STL", &["stl"]).await {
            Some(p) => p,
            None => return Ok(None), // dialog cancelled
        },
    };
    let bodies = head_bodies(&state).await?;
    state
        .exporter()
        .export_stl(&target, &bodies, /*binary=*/ true, "fine")
        .await?;
    Ok(Some(target))
}

/// Exports every body at head to an ASCII OBJ file (`CadClient.exportObj`). `path`
/// `None` shows a native `.obj` save dialog; a cancel resolves to `None`. Meshed at
/// the `fine` LOD. Returns the written path. Rust owns the dialog + the worker
/// `ExportObj` verb.
#[tauri::command]
pub async fn export_obj_file(
    state: State<'_, AppState>,
    app: AppHandle,
    path: Option<String>,
) -> Result<Option<String>, ApiError> {
    let target = match path {
        Some(p) => p,
        None => match pick_mesh_save(app, "OBJ", &["obj"]).await {
            Some(p) => p,
            None => return Ok(None), // dialog cancelled
        },
    };
    let bodies = head_bodies(&state).await?;
    state
        .exporter()
        .export_obj(&target, &bodies, "fine")
        .await?;
    Ok(Some(target))
}

/// The body ids at head for an export command (locks the runtime briefly).
async fn head_bodies(state: &State<'_, AppState>) -> Result<Vec<BodyId>, ApiError> {
    let guard = state.runtime.lock().await;
    let rt = guard
        .as_ref()
        .ok_or_else(|| ApiError::NoDocument("export".into()))?;
    Ok(rt.head_body_ids())
}

/// Closes the open document, dropping its runtime + caches. Clears the document's
/// crash-recovery state first (a clean close is not a crash).
#[tauri::command]
pub async fn close_document(state: State<'_, AppState>, app: AppHandle) -> Result<(), ApiError> {
    let closed = {
        let mut guard = state.runtime.lock().await;
        let id = guard.as_ref().map(DocumentRuntime::document_uuid);
        *guard = None;
        id
    };
    if let (Some(id), Some(root)) = (closed, autosave::autosave_root(&app)) {
        autosave::clear_recovery_state(&root, id);
    }
    // The closed document's worker can serve nothing now: retire it (this is the
    // process leak — it used to live until the app quit) and pre-warm its
    // replacement so the next open is instant. Exactly one sidecar stays alive.
    state.rewarm();
    let _ = app.emit(events::PROJECTION_UPDATED, &DocumentProjection::empty());
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Close/quit confirmation (unsaved-changes guard)
// ─────────────────────────────────────────────────────────────────────────────
//
// `crate::run`'s `on_window_event` / `RunEvent::ExitRequested` handlers prevent
// the native window-close and app-exit, then emit `events::CLOSE_REQUESTED` so
// the frontend can prompt for unsaved changes. These two commands are how the
// frontend resolves that prompt; both clear `crate::ExitGuard` so a LATER
// close/quit attempt prompts again rather than being silently swallowed by a
// stale guard.

/// Resolves a pending close/quit prompt by proceeding: clears the open document's
/// crash-recovery state and the re-entrancy guard, then exits. `AppHandle::exit`
/// re-fires `RunEvent::ExitRequested` with `code: Some(0)`, which the
/// `code.is_none()` gate in `crate::run` already lets through unprompted — so this
/// actually terminates the app rather than looping back into another prompt.
///
/// WHY the recovery clear: reaching here means the user answered the unsaved-changes
/// prompt with Save or **Don't Save**. An explicit discard is NOT a crash, but the
/// autosave marker + container survive process exit, so leaving them behind makes
/// the next launch offer the deliberately-discarded work back as "recovered" —
/// resurrecting data the user just chose to throw away. Mirrors `close_document`'s
/// clear (a clean close is likewise not a crash). Save already cleared it at that
/// document's save path; clearing again is a harmless no-op.
#[tauri::command]
pub async fn confirm_exit(app: AppHandle) {
    let document_id = {
        let state = app.state::<AppState>();
        let guard = state.runtime.lock().await;
        guard.as_ref().map(DocumentRuntime::document_uuid)
    };
    if let (Some(id), Some(root)) = (document_id, autosave::autosave_root(&app)) {
        autosave::clear_recovery_state(&root, id);
    }
    // The single funnel every real quit passes through (⌘Q and the window-close
    // button both route here via `events::CLOSE_REQUESTED`), so this is where both
    // worker slots are retired — once, and before the event loop unwinds.
    app.state::<AppState>().retire_all();
    app.state::<crate::ExitGuard>().clear();
    app.exit(0);
}

/// Resolves a pending close/quit prompt by cancelling: clears the re-entrancy
/// guard WITHOUT exiting, so the app stays open and a later close/quit attempt
/// prompts again.
#[tauri::command]
pub async fn cancel_exit(app: AppHandle) {
    app.state::<crate::ExitGuard>().clear();
}

// ─────────────────────────────────────────────────────────────────────────────
// Crash recovery (autosave; `io::recovery`)
// ─────────────────────────────────────────────────────────────────────────────

/// Scans for a crash-recovery offer at startup (`CadClient.checkRecovery`): a stale
/// session marker (its owning process is gone, per [`autosave::pid_alive`]) whose
/// autosave container survives. Stashes the offer for a later
/// [`recover_document`] decision and returns its display info; `None` when there is
/// nothing to recover.
#[tauri::command]
pub async fn check_recovery(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Option<RecoveryInfoDto>, ApiError> {
    let Some(root) = autosave::autosave_root(&app) else {
        return Ok(None);
    };
    let offers =
        scan_stale_markers(&root, autosave::pid_alive).map_err(|e| ApiError::Io(e.to_string()))?;
    let Some(offer) = offers.into_iter().next() else {
        *state.pending_recovery.lock().unwrap() = None;
        return Ok(None);
    };
    let dto = recovery_info_dto(&offer);
    *state.pending_recovery.lock().unwrap() = Some(offer);
    Ok(Some(dto))
}

/// Acts on the pending crash-recovery offer (`CadClient.recoverDocument`).
///
/// `accept == true`: opens the autosave as the current document, re-targets its
/// **real** save path (so a later Save writes the original file, not the autosave),
/// marks it dirty (unsaved recovered work), clears the stale marker, and returns the
/// snapshot. `accept == false`: discards the autosave + marker. Either way the
/// pending offer is consumed; `None` when nothing was pending.
#[tauri::command]
pub async fn recover_document(
    state: State<'_, AppState>,
    app: AppHandle,
    accept: bool,
) -> Result<Option<DocumentSnapshotDto>, ApiError> {
    let offer = state.pending_recovery.lock().unwrap().take();
    let Some(offer) = offer else {
        return Ok(None);
    };
    let root = autosave::autosave_root(&app);
    if !accept {
        if let Some(root) = &root {
            autosave::clear_recovery_state(root, offer.document_id);
        }
        return Ok(None);
    }
    // Restore: open the autosave container as the live document.
    let mut rt =
        open_runtime_over_new_backend_blocking(&state, offer.autosave_path.clone()).await?;
    rt.mark_recovered(offer.marker.opened_path.clone());
    let (snapshot, projection) = {
        let mut guard = state.runtime.lock().await;
        *guard = Some(rt);
        let rt = guard.as_ref().unwrap();
        rt.adopt_current_epoch(); // VF-B4: a restart before this insert never reached it.
        (snapshot_of(rt), rt.projection())
    };
    state.commit_backend();
    // Consume the marker (the autosave is superseded on the next save/close). The
    // autosave file itself is kept so a re-crash before the next tick still recovers.
    if let Some(root) = &root {
        let _ = onecad_core::io::recovery::remove_marker(root, offer.document_id);
    }
    let _ = app.emit(events::PROJECTION_UPDATED, &projection);
    // Rebuild geometry from the recovered (all-Dirty) timeline, once the worker
    // can actually serve it (readiness-gated — see `schedule_initial_regen`).
    schedule_initial_regen(&state).await;
    Ok(Some(snapshot))
}

/// Maps a [`RecoveryOffer`] to the start-screen DTO (`modifiedMs` from the autosave
/// file's mtime).
fn recovery_info_dto(offer: &RecoveryOffer) -> RecoveryInfoDto {
    RecoveryInfoDto {
        original_path: offer
            .marker
            .opened_path
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned()),
        autosave_path: offer.autosave_path.to_string_lossy().into_owned(),
        modified_ms: file_mtime_ms(&offer.autosave_path),
    }
}

/// A file's last-modified time in Unix-epoch milliseconds (`0` if unavailable).
fn file_mtime_ms(p: &Path) -> u64 {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ─────────────────────────────────────────────────────────────────────────────
// Edits + queries
// ─────────────────────────────────────────────────────────────────────────────

/// Applies one [`EditCommand`] and enqueues the resulting regen. Returns the
/// (pre-regen) projection; post-regen geometry arrives via `document-changed` +
/// `projection-updated` events (projection stores are written only by events).
#[tauri::command]
#[tracing::instrument(skip_all, fields(cmd = tracing::field::Empty), err(Display))]
pub async fn apply_edit_command(
    state: State<'_, AppState>,
    app: AppHandle,
    command: EditCommand,
) -> Result<DocumentProjection, ApiError> {
    let summary = edit_command_summary(&command);
    // Recorded (not a span-attribute expression) so the digest is built ONCE.
    tracing::Span::current().record("cmd", summary.as_str());
    tracing::info!("apply_edit_command: {summary}");
    let (outcome, projection) = {
        let mut guard = state.runtime.lock().await;
        let rt = guard
            .as_mut()
            .ok_or_else(|| ApiError::NoDocument("apply".into()))?;
        let outcome = rt.apply(command).inspect_err(|e| {
            tracing::warn!("apply_edit_command: {summary} REJECTED: {e}");
        })?;
        (outcome, rt.projection())
    };
    tracing::info!(
        "apply_edit_command: {summary} applied rev={} appliedOps={}/{}",
        projection.revision,
        projection.applied_ops,
        projection.total_ops
    );
    let _ = app.emit(events::PROJECTION_UPDATED, &projection);
    if let Some(sched) = state.scheduler.get() {
        sched.handle(&outcome);
    }
    state.note_mutation();
    Ok(projection)
}

/// One-line `EditCommand` digest for tracing: the serde `cmd` tag plus the ids
/// that identify the target (never the full payload — sketch ops are huge).
fn edit_command_summary(command: &EditCommand) -> String {
    match command {
        EditCommand::AddOperation { record, at_cursor } => format!(
            "addOperation name={} record={} atCursor={at_cursor}",
            record.name, record.record_id
        ),
        EditCommand::UpdateOperationParams { record, .. } => {
            format!("updateOperationParams record={record}")
        }
        EditCommand::RemoveOperation { record } => format!("removeOperation record={record}"),
        other => serde_json::to_value(other)
            .ok()
            .and_then(|v| v.get("cmd").and_then(|c| c.as_str()).map(String::from))
            .unwrap_or_else(|| "unknown".into()),
    }
}

/// Undoes the last committed edit (`CadClient.undo`).
///
/// The follow-up regen is whatever [`DocumentRuntime::undo`] named — a
/// [`RegenRequest::RevertToEnd`] over the reverted step's real dirty floor, not the
/// blind full-history `ToEnd { from: 0 }` this used to enqueue (HISTORY-HARDEN H8).
/// The request is minted by the runtime precisely so this lane cannot hand the H6a
/// edit-scoped veto a `from > 0` that came from a revert.
#[tauri::command]
pub async fn undo(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<DocumentProjection, ApiError> {
    let (revert, projection) = {
        let mut guard = state.runtime.lock().await;
        let rt = guard
            .as_mut()
            .ok_or_else(|| ApiError::NoDocument("undo".into()))?;
        (rt.undo(), rt.projection())
    };
    let _ = app.emit(events::PROJECTION_UPDATED, &projection);
    if let Some(revert) = revert {
        if let (Some(sched), Some(request)) = (state.scheduler.get(), revert.regen) {
            sched.request(request);
        }
        state.note_mutation();
    }
    Ok(projection)
}

/// Redoes the last undone edit (`CadClient.redo`). Same H8 dirty-floor lane as
/// [`undo`].
#[tauri::command]
pub async fn redo(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<DocumentProjection, ApiError> {
    let (revert, projection) = {
        let mut guard = state.runtime.lock().await;
        let rt = guard
            .as_mut()
            .ok_or_else(|| ApiError::NoDocument("redo".into()))?;
        (rt.redo()?, rt.projection())
    };
    let _ = app.emit(events::PROJECTION_UPDATED, &projection);
    if let Some(revert) = revert {
        if let (Some(sched), Some(request)) = (state.scheduler.get(), revert.regen) {
            sched.request(request);
        }
        state.note_mutation();
    }
    Ok(projection)
}

/// The current document projection (empty when nothing is open).
#[tauri::command]
pub async fn get_projection(state: State<'_, AppState>) -> Result<DocumentProjection, ApiError> {
    let guard = state.runtime.lock().await;
    Ok(guard
        .as_ref()
        .map_or_else(DocumentProjection::empty, DocumentRuntime::projection))
}

/// Fetches a body's MESH1 blob as a zero-copy `ArrayBuffer` (pull model).
/// `generation` `None` ⇒ the latest snapshot. A miss yields an empty response.
#[tauri::command]
pub async fn get_mesh(
    state: State<'_, AppState>,
    body_id: String,
    lod: String,
    generation: Option<u64>,
) -> Result<tauri::ipc::Response, ApiError> {
    let body = BodyId::from_str(&body_id)
        .map_err(|e| ApiError::InvalidCommand(format!("bad bodyId {body_id:?}: {e}")))?;
    let lod = lod_from_str(&lod);
    let bytes = {
        let mut guard = state.runtime.lock().await;
        let rt = guard
            .as_mut()
            .ok_or_else(|| ApiError::NoDocument("getMesh".into()))?;
        rt.get_mesh(body, lod, generation).await
    };
    // MESH1 travels verbatim; a miss is an empty buffer (frontend keeps its mesh).
    let data = bytes.map(|a| a.as_ref().clone()).unwrap_or_default();
    Ok(tauri::ipc::Response::new(data))
}

// ─────────────────────────────────────────────────────────────────────────────
// Sketch solver lane (SCHEMA §7.4) — mirrors the frontend `localSolver` seam
// ─────────────────────────────────────────────────────────────────────────────

/// Enters sketch mode: syncs the sketch to the worker solver lane and returns the
/// live session + real dof/status (`CadClient.enterSketch`; the F-WP9 swap target).
#[tauri::command]
#[tracing::instrument(skip_all, fields(sketchId = %sketch_id), err(Display))]
pub async fn enter_sketch(
    state: State<'_, AppState>,
    app: AppHandle,
    sketch_id: String,
) -> Result<SketchSessionDto, ApiError> {
    let id = parse_sketch_id(&sketch_id)?;
    let (session, projection) = {
        let mut guard = state.runtime.lock().await;
        let rt = guard
            .as_mut()
            .ok_or_else(|| ApiError::NoDocument("enterSketch".into()))?;
        let session = rt.enter_sketch(id).await?;
        (session, rt.projection())
    };
    let _ = app.emit(events::PROJECTION_UPDATED, &projection);
    Ok(session)
}

/// Reads a sketch's current geometry as a static snapshot — no solver call, no
/// session state, no events (`CadClient.getSketch`; the model-mode display-layer
/// read). Returns the same [`SketchSessionDto`] shape as [`enter_sketch`], with
/// `dof`/`status` from the last solver-lane solve (or the "never solved" default).
#[tauri::command]
pub async fn get_sketch(
    state: State<'_, AppState>,
    sketch_id: String,
) -> Result<SketchSessionDto, ApiError> {
    let id = parse_sketch_id(&sketch_id)?;
    let guard = state.runtime.lock().await;
    let rt = guard
        .as_ref()
        .ok_or_else(|| ApiError::NoDocument("getSketch".into()))?;
    Ok(rt.get_sketch(id)?)
}

/// `Some(recordId)` iff a new placement gesture on `bodyId` may **fold into** an
/// existing `TransformBody` record instead of appending a new one
/// (`CadClient.canFoldTransform`; SCHEMA §7.3 cumulative-placement rule).
///
/// A pure, read-only lineage query — no session state, no events. It lives in the
/// core (not the frontend) because the frontend has no lineage: it sees a picked
/// body and a flat history list, never "which record last touched this body".
#[tauri::command]
pub async fn can_fold_transform(
    state: State<'_, AppState>,
    body_id: String,
) -> Result<Option<String>, ApiError> {
    let id = BodyId::from_str(&body_id)
        .map_err(|e| ApiError::InvalidCommand(format!("bad bodyId `{body_id}`: {e}")))?;
    let guard = state.runtime.lock().await;
    let rt = guard
        .as_ref()
        .ok_or_else(|| ApiError::NoDocument("canFoldTransform".into()))?;
    Ok(rt.can_fold_transform(id).map(|r| r.to_string()))
}

/// Rebuilds the closed regions of a persisted sketch without opening, finishing,
/// squashing or otherwise mutating its edit session.
#[tauri::command]
pub async fn get_sketch_regions(
    state: State<'_, AppState>,
    sketch_id: String,
) -> Result<FinishSketchDto, ApiError> {
    let id = parse_sketch_id(&sketch_id)?;
    let prepared = {
        let guard = state.runtime.lock().await;
        let rt = guard
            .as_ref()
            .ok_or_else(|| ApiError::NoDocument("getSketchRegions".into()))?;
        rt.prepare_sketch_regions(id)?
    };
    Ok(prepared.drive().await?)
}

/// Applies sketch edits (add/move/delete entities+constraints) then re-solves for
/// live dof/status (`CadClient.sketchUpsert`).
#[tauri::command]
pub async fn sketch_upsert(
    state: State<'_, AppState>,
    app: AppHandle,
    sketch_id: String,
    ops: Vec<SketchEditOp>,
) -> Result<SketchUpsertDto, ApiError> {
    let id = parse_sketch_id(&sketch_id)?;
    let (result, outcome, projection) = {
        let mut guard = state.runtime.lock().await;
        let rt = guard
            .as_mut()
            .ok_or_else(|| ApiError::NoDocument("sketchUpsert".into()))?;
        let (result, outcome) = rt.sketch_upsert_with_outcome(id, ops).await?;
        (result, outcome, rt.projection())
    };
    let _ = app.emit(events::SKETCH_SOLVED, &result);
    let _ = app.emit(events::PROJECTION_UPDATED, &projection);
    if let Some(outcome) = outcome {
        if let Some(sched) = state.scheduler.get() {
            sched.handle(&outcome);
        }
        state.note_mutation();
    }
    Ok(result)
}

/// What the pointer grabbed (`BeginGesture.drag`; SCHEMA §7.4) — `{kind, entity,
/// role?, grab?}`. Validated STRICTLY into a [`wire::GestureTarget`]: an unknown
/// `kind` or `role`, or an unparseable `entity`, is refused here rather than
/// forwarded, because §7.4 forbids degrading an unrecognized target to a point
/// drag (it would move a handle the user never grabbed).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GestureTargetArgs {
    pub kind: String,
    pub entity: String,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub grab: Option<[f64; 2]>,
}

/// The five §7.4 `drag.role` tokens, case-folded. Mapping (rather than
/// forwarding) the client string is what makes the role a `&'static str` on the
/// wire — a typo fails at this boundary instead of becoming `REF_UNRESOLVED`
/// three layers down.
fn parse_gesture_role(role: &str) -> Option<&'static str> {
    match role.to_ascii_lowercase().as_str() {
        "start" => Some("start"),
        "end" => Some("end"),
        "center" => Some("center"),
        "p0" => Some("p0"),
        "p1" => Some("p1"),
        _ => None,
    }
}

impl GestureTargetArgs {
    fn validate(self) -> Result<wire::GestureTarget, ApiError> {
        let kind = wire::DragKind::parse(&self.kind).ok_or_else(|| {
            ApiError::InvalidCommand(format!(
                "beginGesture: unknown drag kind {:?} (point|arcEnd|radius|entityBody)",
                self.kind
            ))
        })?;
        let entity = EntityId::from_str(&self.entity).map_err(|e| {
            ApiError::InvalidCommand(format!("beginGesture: bad entity {:?}: {e}", self.entity))
        })?;
        let role = match self.role.as_deref().filter(|r| !r.is_empty()) {
            None => None,
            Some(r) => Some(parse_gesture_role(r).ok_or_else(|| {
                ApiError::InvalidCommand(format!(
                    "beginGesture: unknown drag role {r:?} (start|end|center|p0|p1)"
                ))
            })?),
        };
        if kind == wire::DragKind::ArcEnd && !matches!(role, Some("start") | Some("end")) {
            return Err(ApiError::InvalidCommand(
                "beginGesture: arcEnd requires role start|end".into(),
            ));
        }
        if let Some(grab) = self.grab {
            if !grab.iter().all(|v| v.is_finite()) {
                return Err(ApiError::InvalidCommand(format!(
                    "beginGesture: non-finite grab {grab:?}"
                )));
            }
        }
        Ok(wire::GestureTarget {
            kind,
            entity,
            role,
            grab: self.grab,
        })
    }
}

/// Opens a drag gesture (`BeginGesture`; SCHEMA §7.4).
///
/// `target` is OPTIONAL and additive: absent ⇒ the plain point drag on
/// `drag_point`, whose request stays byte-identical to the pre-SP-2 form, so every
/// existing caller is unchanged.
#[tauri::command]
pub async fn begin_gesture(
    state: State<'_, AppState>,
    sketch_id: String,
    drag_point: String,
    target: Option<GestureTargetArgs>,
) -> Result<BeginGestureDto, ApiError> {
    let id = parse_sketch_id(&sketch_id)?;
    let point = EntityId::from_str(&drag_point)
        .map_err(|e| ApiError::InvalidCommand(format!("bad dragPoint {drag_point:?}: {e}")))?;
    let target = match target {
        Some(t) => t.validate()?,
        None => wire::GestureTarget::point(point),
    };
    let mut guard = state.runtime.lock().await;
    let rt = guard
        .as_mut()
        .ok_or_else(|| ApiError::NoDocument("beginGesture".into()))?;
    Ok(rt.begin_gesture(id, point, target).await?)
}

/// One latest-wins incremental drag solve (`SolveDrag`; preview only).
#[tauri::command]
pub async fn solve_drag(
    state: State<'_, AppState>,
    target: [f64; 2],
) -> Result<DragSolveDto, ApiError> {
    let mut guard = state.runtime.lock().await;
    let rt = guard
        .as_mut()
        .ok_or_else(|| ApiError::NoDocument("solveDrag".into()))?;
    Ok(rt.solve_drag(target).await?)
}

/// Pointer-up: final exact solve committed as ONE undo command (`EndGesture`).
#[tauri::command]
pub async fn end_gesture(
    state: State<'_, AppState>,
    app: AppHandle,
    final_target: Option<[f64; 2]>,
) -> Result<SketchUpsertDto, ApiError> {
    let (result, outcome, projection) = {
        let mut guard = state.runtime.lock().await;
        let rt = guard
            .as_mut()
            .ok_or_else(|| ApiError::NoDocument("endGesture".into()))?;
        let (result, outcome) = rt.end_gesture_with_outcome(final_target).await?;
        (result, outcome, rt.projection())
    };
    let _ = app.emit(events::SKETCH_SOLVED, &result);
    let _ = app.emit(events::PROJECTION_UPDATED, &projection);
    if let Some(sched) = state.scheduler.get() {
        sched.handle(&outcome);
    }
    state.note_mutation();
    Ok(result)
}

/// Exits sketch mode / cancels an in-flight gesture without committing.
#[tauri::command]
pub async fn cancel_sketch(state: State<'_, AppState>, sketch_id: String) -> Result<(), ApiError> {
    let id = parse_sketch_id(&sketch_id)?;
    tracing::info!("cancel_sketch: sketch={id} (squash-only exit — NO timeline record minted)");
    let mut guard = state.runtime.lock().await;
    let rt = guard
        .as_mut()
        .ok_or_else(|| ApiError::NoDocument("cancelSketch".into()))?;
    rt.cancel_sketch(id).await?;
    Ok(())
}

/// Computes the closed profile regions for extrude/revolve selection + preview fill
/// (`finishSketch` → `SketchRegions`).
#[tauri::command]
#[tracing::instrument(skip_all, fields(sketchId = %sketch_id), err(Display))]
pub async fn finish_sketch(
    state: State<'_, AppState>,
    app: AppHandle,
    sketch_id: String,
) -> Result<FinishSketchDto, ApiError> {
    let id = parse_sketch_id(&sketch_id)?;
    let (dto, outcome, projection) = {
        let mut guard = state.runtime.lock().await;
        let rt = guard
            .as_mut()
            .ok_or_else(|| ApiError::NoDocument("finishSketch".into()))?;
        let (dto, outcome) = rt.finish_sketch_with_outcome(id).await?;
        (dto, outcome, rt.projection())
    };
    tracing::info!(
        "finish_sketch: sketch={id} regions={} recordOutcome={}",
        dto.regions.len(),
        outcome.is_some()
    );
    // The finish may have appended/refreshed the sketch's timeline record: the
    // new feature row must reach the frontend and the regen scheduler must see
    // the outcome, or the record stays an unregenerated draft.
    let _ = app.emit(events::PROJECTION_UPDATED, &projection);
    if let Some(outcome) = outcome {
        if let Some(sched) = state.scheduler.get() {
            sched.handle(&outcome);
        }
        state.note_mutation();
    }
    Ok(dto)
}

// ─────────────────────────────────────────────────────────────────────────────
// Element identity (SCHEMA §7.5) — pick → promote
// ─────────────────────────────────────────────────────────────────────────────

/// One pick to promote (`{topoKey, anchor?}`).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PickInput {
    pub topo_key: String,
    #[serde(default)]
    pub anchor: Option<AnchorIntent>,
}

/// One ref to dry-run-resolve (`{refId, primary?, intent?, anchor?}`).
#[derive(Debug, Deserialize)]
pub struct ResolveRefInput {
    #[serde(rename = "refId")]
    pub ref_id: String,
    #[serde(flatten)]
    pub element: ElementRef,
}

/// Promotes snapshot-scoped TopoKey picks to persistent, Rust-minted `ElementId`s
/// (`AcquireElementIds`; SCHEMA §7.5) — the pick→promote surface for M2.
#[tauri::command]
pub async fn promote_selection(
    state: State<'_, AppState>,
    app: AppHandle,
    snapshot_id: u64,
    body_id: String,
    picks: Vec<PickInput>,
) -> Result<Vec<PromotedElementDto>, ApiError> {
    let body = wire::parse_body_id(&body_id).map_err(ApiError::InvalidCommand)?;
    let picks: Vec<(TopoKey, Option<AnchorIntent>)> = picks
        .into_iter()
        .map(|p| (TopoKey::new(p.topo_key), p.anchor))
        .collect();
    let (ids, projection) = {
        let mut guard = state.runtime.lock().await;
        let rt = guard
            .as_mut()
            .ok_or_else(|| ApiError::NoDocument("promoteSelection".into()))?;
        let ids = rt
            .promote_selection(SnapshotId(snapshot_id), body, picks)
            .await?;
        (ids, rt.projection())
    };
    let _ = app.emit(events::PROJECTION_UPDATED, &projection);
    Ok(ids)
}

/// Drag-time preview: run ONE candidate op against a throwaway copy of the
/// worker's head and return the resulting MESH1 blobs (SCHEMA §7.6 `PreviewOp`).
///
/// This replaces a client-side stand-in. The "exact" mesh shown during an extrude
/// drag used to be synthesized in JavaScript by the same function the MOCK client
/// uses, so a Cut preview never subtracted and Fillet/Shell/Revolve had no preview
/// at all — the commit could produce something the preview had never shown.
///
/// Nothing is committed: the worker runs the op on a copy and drops it, so the
/// head bodies, history hash, snapshot id, revision and epoch are untouched and
/// the preview is invisible to fencing. The runtime lock is taken only to confirm
/// a document is open, never across the worker round-trip.
///
/// `op` is the core `{opType, params}` shape used by AddOperation, optionally
/// carrying a stable `opId` that commit reuses as its RecordId. Rust deserializes
/// it to [`Operation`] and performs all worker-wire lowering; raw worker params
/// are never accepted. `expected_snapshot_id` rejects stale drag candidates.
#[tauri::command]
pub async fn preview_op(
    state: State<'_, AppState>,
    op: serde_json::Value,
    sketch_id: Option<String>,
    expected_snapshot_id: Option<u64>,
    lod: Option<String>,
) -> Result<crate::dto::PreviewResultDto, ApiError> {
    {
        let guard = state.runtime.lock().await;
        guard
            .as_ref()
            .ok_or_else(|| ApiError::NoDocument("previewOp".into()))?;
    }
    let lod = match lod.as_deref() {
        Some("fine") => onecad_core::regen::Lod::Fine,
        Some("medium") => onecad_core::regen::Lod::Medium,
        _ => onecad_core::regen::Lod::Coarse,
    };
    let (operation, op_id) = parse_preview_operation(op)?;
    let op_id_log = op_id.clone();
    let result = state
        .preview()
        .preview_op(
            operation,
            op_id,
            sketch_id,
            expected_snapshot_id.map(SnapshotId),
            lod,
        )
        .await;
    match result {
        Err(onecad_core::regen::EngineError::OpFailed {
            code: onecad_core::regen::OpFailureCode::StalePreview,
            message,
            ..
        }) => {
            // `trace`, not `debug`: a stale candidate is the EXPECTED outcome of a
            // drag frame that raced a publish — at debug it would drown the default
            // filter's `onecad_lib=debug` lane once per frame.
            tracing::trace!("preview_op: op={op_id_log} STALE: {message}");
            Err(ApiError::StalePreview(message))
        }
        Err(error) => {
            tracing::warn!("preview_op: op={op_id_log} FAILED: {error}");
            Err(error.into())
        }
        Ok(preview) => Ok(preview),
    }
}

fn parse_preview_operation(mut raw: serde_json::Value) -> Result<(Operation, String), ApiError> {
    let object = raw
        .as_object_mut()
        .ok_or_else(|| ApiError::InvalidCommand("preview operation must be an object".into()))?;
    let op_id = match object.remove("opId") {
        // Compatibility with W3 callers predating stable frontend preview ids.
        // Keep the fallback deterministic across drag frames while preserving
        // the body_<RecordId> identity shape required by NewBody lowering.
        None => RecordId::from_uuid(uuid::Uuid::nil()).to_string(),
        Some(serde_json::Value::String(id)) => RecordId::from_str(&id)
            .map(|record| record.to_string())
            .map_err(|e| ApiError::InvalidCommand(format!("bad preview opId {id:?}: {e}")))?,
        Some(_) => {
            return Err(ApiError::InvalidCommand(
                "preview opId must be a RecordId string".into(),
            ));
        }
    };
    let operation = serde_json::from_value(raw)
        .map_err(|e| ApiError::InvalidCommand(format!("bad preview operation: {e}")))?;
    Ok((operation, op_id))
}

/// Resolve a picked FACE to the sketch plane a sketch placed on it would freeze
/// (MODEL-OPS W2 sketch-on-face).
///
/// Rust owns identity, so the plane is derived HERE rather than from the
/// frontend's tessellated triangle normal: `QueryElement` (SCHEMA §7.5) returns
/// the kernel's own descriptor, and for a planar face its bounding-box centre
/// lies on the plane, so `{center, normal}` defines it exactly. The in-plane axes
/// come from the deterministic, lock-tested
/// [`plane_from_point_normal`](onecad_core::sketch::plane_from_point_normal) rule —
/// the frame is frozen with the sketch, so a non-deterministic basis would rotate
/// existing sketches on reopen.
///
/// FAILS LOUDLY for a non-planar face rather than approximating one: a cylinder's
/// descriptor normal is not a plane normal, and silently sketching on a made-up
/// plane is exactly the class of "silent wrong bind" this codebase refuses.
/// Worker IO runs OUTSIDE the runtime lock (the lock is taken only to read the
/// head snapshot id).
///
/// `topo_key` walks the SAME two-rung read ladder `element_info` does (see its
/// doc comment for the full rationale): a just-promoted face id has not yet
/// been CONSUMED by any operation, so the worker's on-demand element-map
/// partition has no entry for it and an ElementId-only lookup fails loudly —
/// exactly the flow `SketchController.tryEnterOnSelectedFace` drives (select a
/// face, sketch on it immediately). The topoKey rung resolves against the body
/// shape directly, so it is tried FIRST; the elementId is the fallback.
#[tauri::command]
pub async fn face_sketch_plane(
    state: State<'_, AppState>,
    snapshot_id: u64,
    body_id: String,
    element_id: String,
    topo_key: Option<String>,
) -> Result<crate::dto::SketchPlaneDto, ApiError> {
    let body = wire::parse_body_id(&body_id).map_err(ApiError::InvalidCommand)?;
    {
        // Presence check only — the runtime lock is NOT held across the worker
        // round-trip below (the R-WP11 rule; a held lock makes fencing inert).
        let guard = state.runtime.lock().await;
        guard
            .as_ref()
            .ok_or_else(|| ApiError::NoDocument("faceSketchPlane".into()))?;
    }
    // The caller supplies the snapshot its pick was made against — the same
    // contract `promoteSelection` uses, so a TopoKey/ElementId resolves against
    // the exact snapshot the mesh was tessellated at (Invariant 4).
    let snapshot = SnapshotId(snapshot_id);
    let query = state.element_query();
    let mut info = None;
    if let Some(topo_key) = topo_key.as_deref().filter(|k| !k.is_empty()) {
        info = query
            .query_element_by_topo_key(snapshot, body, topo_key)
            .await?;
    }
    if info.is_none() {
        info = query.query_element(snapshot, body, &element_id).await?;
    }
    let info = info.ok_or_else(|| {
        ApiError::InvalidCommand(format!(
            "faceSketchPlane: element {element_id} is not present in the current snapshot"
        ))
    })?;

    if info.kind != "face" {
        return Err(ApiError::InvalidCommand(format!(
            "faceSketchPlane: element {element_id} is a {}, not a face",
            info.kind
        )));
    }
    // `GeomAbs_Plane` is OCCT's FIRST surface-type enumerator, i.e. 0.
    const GEOM_ABS_PLANE: i64 = 0;
    if info.surface_type != GEOM_ABS_PLANE || !info.has_normal {
        return Err(ApiError::InvalidCommand(
            "faceSketchPlane: only a planar face can host a sketch".into(),
        ));
    }

    let plane = onecad_core::sketch::plane_from_point_normal(
        onecad_core::math::Vec3::new_unchecked(info.center[0], info.center[1], info.center[2]),
        onecad_core::math::Vec3::new_unchecked(info.normal[0], info.normal[1], info.normal[2]),
    );
    Ok(plane.into())
}

/// The addressing rung that actually resolved a picked face, kept so the SECOND
/// projection round-trip re-uses it verbatim instead of re-walking the ladder.
///
/// Re-laddering between the two calls would be a real hazard, not a nicety: the
/// two rungs can resolve to DIFFERENT faces (a stale `topoKey` and a live
/// `elementId` need not agree), and the handshake's whole premise is that both
/// round-trips describe the same face.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ResolvedFaceAddress {
    ElementId(String),
    TopoKey(String),
}

impl ResolvedFaceAddress {
    fn as_wire(&self) -> wire::FaceAddress<'_> {
        match self {
            Self::ElementId(id) => wire::FaceAddress::ElementId(id),
            Self::TopoKey(key) => wire::FaceAddress::TopoKey(key),
        }
    }
}

/// Creates a sketch ON a picked planar face, seeded with that face's projected
/// boundary as locked reference geometry (SKETCH-ON-FACE W1b; SCHEMA §7.6
/// `ProjectFaceBoundary`).
///
/// This is the AUTHORING sibling of [`face_sketch_plane`], which stays the
/// frontend's read-only pre-flight (does this face host a sketch, and where?).
/// Here the backend additionally asks the kernel for the face's boundary and
/// mints it into the new sketch as `referenceLocked` entities pinned by `Fixed`
/// constraints — the geometry a user snaps a profile to.
///
/// # The two-round-trip plane handshake (SCHEMA §7.6)
///
/// 1. `frameOnly` — the kernel's own `gp_Pln` origin + orientation-corrected
///    normal. This is why the verb exists rather than reusing `QueryElement`: a
///    descriptor `center` is an axis-aligned **bbox** centre and sits OFF the
///    plane for a tilted face, so a sketch built on it would extrude a sliver.
/// 2. Rust builds the deterministic, lock-tested basis
///    ([`plane_from_point_normal`](onecad_core::sketch::plane_from_point_normal)) —
///    Rust owns the frame, and the frame is frozen with the sketch, so a
///    non-deterministic basis would rotate stored sketches on reopen.
/// 3. The real projection, in THAT basis, over `coplanarBody` scope so a face
///    split into coplanar patches projects as one outline. Its echoed `exact` is
///    compared against round-trip 1 as a **tripwire**: a mismatch means the head
///    moved between the calls, and the command refuses rather than committing a
///    sketch whose boundary belongs to a different face.
///
/// # Identity + ordering
///
/// * `sketch_id` is the FRONTEND-minted uuid and becomes the document `SketchId`
///   **verbatim**. The frontend keys its projection store by that id, so minting
///   a different one here would strand the store's entry.
/// * `topo_key` walks the same two-rung read ladder [`face_sketch_plane`] and
///   [`element_info`] do — topoKey FIRST, because a just-promoted, never-consumed
///   `ElementId` is genuinely absent from the worker's on-demand element-map
///   partition (see [`element_info`]'s doc comment).
/// * Both worker round-trips run OUTSIDE the runtime lock; the lock is taken only
///   for the presence check and for the single [`EditCommand::AddSketch`] apply.
/// * An unresolvable face, a non-planar face, or a malformed response leaves the
///   document **completely untouched** — nothing is applied until the projection
///   has landed and translated.
///
/// A sketch created here has entities in the DOCUMENT immediately, but NOT a
/// `Sketch` timeline record: the regen planner resolves an extrude's profile only
/// from such a record, and minting one is `finish_sketch`'s job
/// (EXTRUDE-COMMIT-FIX). Enter then finish before extruding off this sketch.
#[tauri::command]
#[tracing::instrument(
    skip_all,
    fields(sketchId = %sketch_id, bodyId = %body_id, snapshot = snapshot_id),
    err(Display)
)]
pub async fn add_sketch_on_face(
    state: State<'_, AppState>,
    snapshot_id: u64,
    body_id: String,
    element_id: String,
    topo_key: Option<String>,
    sketch_id: String,
    name: String,
) -> Result<SketchOnFaceDto, ApiError> {
    let body = wire::parse_body_id(&body_id).map_err(ApiError::InvalidCommand)?;
    // Adopted VERBATIM (see the doc comment): backend SketchId == frontend id.
    let sketch_id = parse_sketch_id(&sketch_id)?;
    let topo_key = topo_key.unwrap_or_default();
    if element_id.is_empty() && topo_key.is_empty() {
        return Err(ApiError::InvalidCommand(
            "addSketchOnFace: one of elementId / topoKey is required".into(),
        ));
    }
    {
        // Presence check only — the runtime lock is NOT held across the worker
        // round-trips below (the R-WP11 rule; a held lock makes fencing inert).
        let guard = state.runtime.lock().await;
        guard
            .as_ref()
            .ok_or_else(|| ApiError::NoDocument("addSketchOnFace".into()))?;
    }
    let snapshot = SnapshotId(snapshot_id);
    let projector = state.face_projection();

    // ── Round-trip 1: the kernel-exact frame, via the topoKey-first ladder ────
    let mut resolved: Option<(ResolvedFaceAddress, onecad_core::sketch::FaceFrame)> = None;
    if !topo_key.is_empty() {
        let address = ResolvedFaceAddress::TopoKey(topo_key.clone());
        if let Some(frame) = projector
            .project_face_boundary_frame(snapshot, body, address.as_wire())
            .await?
        {
            resolved = Some((address, frame));
        }
    }
    if resolved.is_none() && !element_id.is_empty() {
        let address = ResolvedFaceAddress::ElementId(element_id.clone());
        if let Some(frame) = projector
            .project_face_boundary_frame(snapshot, body, address.as_wire())
            .await?
        {
            resolved = Some((address, frame));
        }
    }
    let (address, frame) = resolved.ok_or_else(|| {
        ApiError::InvalidCommand(format!(
            "addSketchOnFace: face (elementId {element_id:?}, topoKey {topo_key:?}) on body \
             {body_id} is not present in snapshot {snapshot_id}"
        ))
    })?;

    // ── The frame Rust owns, frozen with the sketch ──────────────────────────
    let plane = onecad_core::sketch::plane_from_point_normal(frame.origin, frame.normal);

    // ── Round-trip 2: the real projection in that basis ──────────────────────
    let payload = projector
        .project_face_boundary(
            snapshot,
            body,
            address.as_wire(),
            &plane,
            // A model face is routinely split into coplanar patches by unrelated
            // features; projecting only the seed patch would seed a sketch with a
            // partial outline that silently forms the wrong region.
            wire::ProjectionScope::CoplanarBody,
        )
        .await?
        .ok_or_else(|| {
            ApiError::Internal(format!(
                "addSketchOnFace: face on body {body_id} resolved for the frame query but \
                 vanished before the projection — the head moved mid-handshake"
            ))
        })?;

    // TRIPWIRE (SCHEMA §7.6): the echoed `exact` must still describe the frame the
    // sketch's basis was built from. It is not a fence, so it cannot prevent a
    // concurrent edit — it can only refuse to commit a boundary that no longer
    // belongs to the plane it is expressed in.
    const EXACT_EPS: f64 = 1e-9;
    if !payload.exact.approx_eq(&frame, EXACT_EPS) {
        return Err(ApiError::Internal(format!(
            "addSketchOnFace: the projection echoed a different exact frame than the \
             frameOnly query (frameOnly {:?}/{:?}, projection {:?}/{:?}) — refusing to commit a \
             boundary expressed in a stale basis",
            frame.origin, frame.normal, payload.exact.origin, payload.exact.normal
        )));
    }

    // ── Translate + build (pure; no worker, no lock) ─────────────────────────
    let (entities, constraints) = onecad_core::sketch::projected_sketch_content(
        &payload,
        &mut EntityId::new,
        &mut ConstraintId::new,
    );
    let attachment = onecad_core::sketch::SketchAttachment::HostFace {
        face: ElementRef {
            // Only a real, non-empty ElementId is a binding; an empty string
            // would be a fabricated identity the resolution ladder would chase.
            primary: (!element_id.is_empty()).then(|| PrimaryRef {
                body,
                element: ElementId::new(element_id.clone()),
                kind: ElementKind::Face,
                extra: Default::default(),
            }),
            // No `intent`: the descriptor is worker-owned evidence and capturing
            // it would cost a third round-trip this command has no other use for.
            intent: None,
            anchor: Some(AnchorIntent {
                // The kernel-exact plane origin — a point genuinely ON the face,
                // unlike a bbox centre.
                world_point: frame.origin,
                surface_uv: None,
                local_frame: None,
                adjacency_hint: None,
                extra: Default::default(),
            }),
            extra: Default::default(),
        },
        projected_boundary_version: 1,
    };
    let mut sketch = onecad_core::sketch::Sketch::new(sketch_id, name, attachment);
    sketch.plane = plane;
    let (entity_count, constraint_count) = (entities.len(), constraints.len());
    for entity in entities {
        sketch
            .add_entity(entity)
            .map_err(|e| ApiError::Internal(format!("addSketchOnFace: projected entity: {e}")))?;
    }
    for constraint in constraints {
        sketch.add_constraint(constraint).map_err(|e| {
            ApiError::Internal(format!("addSketchOnFace: projected constraint: {e}"))
        })?;
    }

    // ── The single locked write ──────────────────────────────────────────────
    let (outcome, projection) = {
        let mut guard = state.runtime.lock().await;
        let rt = guard
            .as_mut()
            .ok_or_else(|| ApiError::NoDocument("addSketchOnFace".into()))?;
        let outcome = rt
            .apply(EditCommand::AddSketch { sketch })
            .inspect_err(|e| {
                tracing::warn!("add_sketch_on_face: sketch={sketch_id} REJECTED: {e}")
            })?;
        (outcome, rt.projection())
    };
    tracing::info!(
        "add_sketch_on_face: sketch={sketch_id} body={body_id} faces={} entities={entity_count} \
         constraints={constraint_count} closed={}",
        payload.face_count,
        payload.has_closed_boundary
    );
    // The AppHandle comes from the set-once `AppState` slot rather than a command
    // parameter: `AppHandle` is generic over the runtime, and taking it here would
    // pin this command to `Wry` and make it uncallable from a `mock_app` test (the
    // only way to exercise it against the real worker). Absent ⇒ no webview yet.
    if let Some(app) = state.app.get() {
        let _ = app.emit(events::PROJECTION_UPDATED, &projection);
    }
    if let Some(sched) = state.scheduler.get() {
        sched.handle(&outcome);
    }
    state.note_mutation();

    Ok(SketchOnFaceDto {
        sketch_id: sketch_id.to_string(),
        plane: plane.into(),
        entity_count: entity_count.try_into().unwrap_or(u32::MAX),
        constraint_count: constraint_count.try_into().unwrap_or(u32::MAX),
        face_count: payload.face_count,
        has_closed_boundary: payload.has_closed_boundary,
        projected_boundary_version: 1,
    })
}

/// Read one element's geometric evidence (`QueryElement`; SCHEMA §7.5) —
/// MEASURE V1a's only backend verb.
///
/// A pure READ: it touches no history, mints nothing, and publishes nothing, so
/// measuring can never enter the undo stack. `magnitude` is the kernel's EXACT
/// quantity (face area / edge arc length — `BRepGProp`), not a value re-derived
/// from the tessellation the viewport happens to be showing.
///
/// Returns `Ok(None)` — not an error — when the element is absent from the
/// snapshot: a stale pick after an edit is an ordinary, expected outcome ("that
/// edge is gone"), and the caller simply drops it. Everything else (no document,
/// a malformed body id, a worker failure) still fails LOUDLY. Worker IO runs
/// OUTSIDE the runtime lock; the lock is taken only for the presence check.
///
/// # The two-rung read ladder
///
/// A caller may hold an `elementId`, a `topoKey`, or both, and NEITHER alone
/// answers every case:
///
/// * `elementId` is the durable identity, but the worker's element-map partition
///   mints entries **on demand** — only when an OPERATION resolves the element as
///   an input. `AcquireElementIds` returns evidence and RUST mints the id; the
///   worker is never told. So a just-promoted, not-yet-used id is genuinely
///   absent from the partition and this rung reports `None`.
/// * `topoKey` resolves against the body shape itself, so it answers for a fresh
///   pick — but it is snapshot-scoped ordinal evidence, and when a mesh carries
///   minted ElementIds in its id table the frontend's `topoKey` field actually
///   holds an ElementId (`Picker.ts`), which this rung cannot resolve.
///
/// So the topoKey is tried FIRST (the fresh-pick case, one round-trip) and the
/// elementId is the fallback. Both rungs are exact lookups — neither guesses, so
/// a miss on one can never bind the other to the wrong thing. Supplying neither
/// is a caller bug and is rejected rather than silently answered `None`.
#[tauri::command]
pub async fn element_info(
    state: State<'_, AppState>,
    snapshot_id: u64,
    body_id: String,
    element_id: String,
    topo_key: Option<String>,
) -> Result<Option<crate::dto::ElementInfoDto>, ApiError> {
    let body = wire::parse_body_id(&body_id).map_err(ApiError::InvalidCommand)?;
    let topo_key = topo_key.unwrap_or_default();
    if element_id.is_empty() && topo_key.is_empty() {
        return Err(ApiError::InvalidCommand(
            "elementInfo: one of elementId / topoKey is required".into(),
        ));
    }
    {
        // Presence check only — the runtime lock is NOT held across the worker
        // round-trip below (the R-WP11 rule; a held lock makes fencing inert).
        let guard = state.runtime.lock().await;
        guard
            .as_ref()
            .ok_or_else(|| ApiError::NoDocument("elementInfo".into()))?;
    }
    // The caller supplies the snapshot its pick was made against — the same
    // contract `promoteSelection` / `faceSketchPlane` use, so the lookup resolves
    // against the exact snapshot the mesh was tessellated at (Invariant 4).
    let snapshot = SnapshotId(snapshot_id);
    let query = state.element_query();
    if !topo_key.is_empty() {
        if let Some(info) = query
            .query_element_by_topo_key(snapshot, body, &topo_key)
            .await?
        {
            return Ok(Some(info));
        }
    }
    if element_id.is_empty() {
        return Ok(None);
    }
    query
        .query_element(snapshot, body, &element_id)
        .await
        .map_err(Into::into)
}

/// One body's exact mass properties (`QueryMassProperties`; SCHEMA §7.5; WP-C1).
///
/// A pure READ, like [`element_info`]: no fence, no history, no minting, nothing
/// published — measuring can never enter the undo stack. Worker IO runs OUTSIDE
/// the runtime lock (the R-WP11 rule); the lock is taken only for the
/// document-presence check.
///
/// No snapshot argument, deliberately. A `BodyId` is durable across snapshots
/// (Invariant 1) while a `TopoKey` is not, so unlike [`element_info`] there is
/// nothing here for a snapshot stamp to disambiguate — the answer is always
/// "this body, as the head has it now", which is what a live measurement panel
/// wants.
///
/// An unknown body fails LOUDLY (`REF_UNRESOLVED` from the worker) rather than
/// resolving to `None`. A missing body is not the ordinary stale-pick outcome
/// that `element_info` absorbs: there is no partial reading to fall back to, and
/// a silent empty answer would look identical to a body with no volume.
#[tauri::command]
pub async fn query_mass_properties(
    state: State<'_, AppState>,
    body_id: String,
) -> Result<crate::dto::MassPropertiesDto, ApiError> {
    let body = wire::parse_body_id(&body_id).map_err(ApiError::InvalidCommand)?;
    {
        // Presence check only — the lock is NOT held across the worker round-trip.
        let guard = state.runtime.lock().await;
        guard
            .as_ref()
            .ok_or_else(|| ApiError::NoDocument("queryMassProperties".into()))?;
    }
    state
        .element_query()
        .query_mass_properties(body, body_id)
        .await
        .map_err(Into::into)
}

/// One picked face for [`prepare_offset_face`] — `{bodyId?, topoKey?, elementId?}`
/// (SCHEMA §7.6 `pickedFaces[]`).
///
/// The two addressing rungs are mutually exclusive on the wire (the worker
/// branches on the PRESENCE of `elementId`), so a pick carrying both is rejected
/// here rather than letting a stale id shadow a good topoKey.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OffsetFacePickInput {
    #[serde(default)]
    pub body_id: Option<String>,
    #[serde(default)]
    pub topo_key: Option<String>,
    #[serde(default)]
    pub element_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EdgeOpPickInput {
    #[serde(default)]
    pub body_id: Option<String>,
    #[serde(default)]
    pub topo_key: Option<String>,
    #[serde(default)]
    pub element_id: Option<String>,
}

fn edge_op_mode(value: &str) -> Result<wire::EdgeOpMode, ApiError> {
    match value {
        "Fillet" => Ok(wire::EdgeOpMode::Fillet),
        "Chamfer" => Ok(wire::EdgeOpMode::Chamfer),
        _ => Err(ApiError::InvalidCommand(
            "prepareEdgeOp: unknown mode".into(),
        )),
    }
}

fn edge_op_bodies(picks: &[EdgeOpPickInput]) -> Result<Vec<Option<BodyId>>, ApiError> {
    picks
        .iter()
        .enumerate()
        .map(|(index, pick)| {
            let element = pick.element_id.as_deref().filter(|value| !value.is_empty());
            let topo = pick.topo_key.as_deref().filter(|value| !value.is_empty());
            if element.is_some() == topo.is_some() {
                return Err(ApiError::InvalidCommand(format!(
                    "prepareEdgeOp: pick {index} must carry exactly one address"
                )));
            }
            let body = pick
                .body_id
                .as_deref()
                .filter(|value| !value.is_empty())
                .map(wire::parse_body_id)
                .transpose()
                .map_err(ApiError::InvalidCommand)?;
            if topo.is_some() && body.is_none() {
                return Err(ApiError::InvalidCommand(format!(
                    "prepareEdgeOp: pick {index} topoKey has no bodyId"
                )));
            }
            Ok(body)
        })
        .collect()
}

fn attach_edge_identities(
    prepared: &mut crate::dto::PrepareEdgeOpDto,
    promoted: &[PromotedElementDto],
) -> Result<(), ApiError> {
    for edge in &mut prepared.edges {
        let identity = promoted
            .iter()
            .find(|item| item.topo_key == edge.topo_key)
            .filter(|item| item.kind == "edge")
            .ok_or_else(|| {
                ApiError::Internal("prepared edge promotion returned mismatched identity".into())
            })?;
        edge.element_id = Some(identity.element_id.clone());
        edge.body_id = Some(identity.body_id.clone());
        edge.kind = Some(identity.kind.clone());
    }
    Ok(())
}

fn wire_edge_op_picks<'a>(
    picks: &'a [EdgeOpPickInput],
    bodies: &'a [Option<BodyId>],
) -> Vec<wire::EdgeOpPick<'a>> {
    picks
        .iter()
        .zip(bodies)
        .map(|(pick, body)| wire::EdgeOpPick {
            body: *body,
            address: match pick.element_id.as_deref().filter(|value| !value.is_empty()) {
                Some(id) => wire::FaceAddress::ElementId(id),
                None => wire::FaceAddress::TopoKey(pick.topo_key.as_deref().unwrap_or_default()),
            },
        })
        .collect()
}

async fn promote_prepared_edges(
    state: &AppState,
    prepared: &mut crate::dto::PrepareEdgeOpDto,
) -> Result<DocumentProjection, ApiError> {
    let body = wire::parse_body_id(&prepared.target_body_id).map_err(ApiError::Internal)?;
    let picks = prepared
        .edges
        .iter()
        .map(|edge| (TopoKey::new(edge.topo_key.clone()), None))
        .collect();
    let mut guard = state.runtime.lock().await;
    let runtime = guard
        .as_mut()
        .ok_or_else(|| ApiError::NoDocument("prepareEdgeOp".into()))?;
    let promoted = runtime
        .promote_selection(SnapshotId(prepared.snapshot_id), body, picks)
        .await?;
    attach_edge_identities(prepared, &promoted)?;
    Ok(runtime.projection())
}

/// The read-only first half of the OffsetFace authoring transaction
/// (`PrepareOffsetFace`; SCHEMA §7.6): the G1 tangent closure over the picked
/// faces, the `Total` opposite candidate, and the `currentDims` that seed an
/// absolute distance type.
///
/// A pure READ, like [`element_info`]: it neither fences nor prepares, accepts,
/// discards — nor, critically, MINTS. The response carries snapshot-scoped
/// TopoKeys and descriptor/anchor EVIDENCE only; Rust promotes that evidence and
/// mints the `ElementId`s through [`promote_selection`] (Invariant 2), and the
/// caller then authors the final params and runs one exact [`preview_op`] with
/// them before committing. Worker IO runs OUTSIDE the runtime lock (the R-WP11
/// rule); the lock is taken only for the document-presence check.
///
/// A REFUSAL (`crossBody`, `chainMismatch`, `unsupportedSurface`, …) comes back
/// as a successful result carrying
/// [`refusal`](crate::dto::PrepareOffsetFaceDto::refusal) — the tool needs to
/// explain *why* it cannot offset, and an error would erase the reason.
#[tauri::command]
#[tracing::instrument(
    skip_all,
    fields(snapshot = snapshot_id, picks = picked_faces.len()),
    err(Display)
)]
pub async fn prepare_offset_face(
    state: State<'_, AppState>,
    snapshot_id: u64,
    picked_faces: Vec<OffsetFacePickInput>,
    chain_tangent_faces: bool,
    distance_type: Option<String>,
) -> Result<crate::dto::PrepareOffsetFaceDto, ApiError> {
    if picked_faces.is_empty() {
        return Err(ApiError::InvalidCommand(
            "prepareOffsetFace: at least one picked face is required".into(),
        ));
    }
    // Parsed through the CORE enum so the command can never send a spelling the
    // op's own `distanceType` param would reject.
    let distance_type: onecad_core::document::record::OffsetDistanceType =
        match distance_type.as_deref() {
            None => Default::default(),
            Some(s) => {
                serde_json::from_value(serde_json::Value::String(s.to_string())).map_err(|_| {
                    ApiError::InvalidCommand(format!(
                        "prepareOffsetFace: unknown distanceType {s:?} \
                         (Offset | Total | Radius | Diameter)"
                    ))
                })?
            }
        };
    // Bodies + address rungs are resolved BEFORE the round-trip so a malformed
    // pick is a caller error, never a half-sent request.
    let mut bodies: Vec<Option<BodyId>> = Vec::with_capacity(picked_faces.len());
    for (i, p) in picked_faces.iter().enumerate() {
        let element = p.element_id.as_deref().filter(|s| !s.is_empty());
        let topo = p.topo_key.as_deref().filter(|s| !s.is_empty());
        match (element, topo) {
            (None, None) => {
                return Err(ApiError::InvalidCommand(format!(
                    "prepareOffsetFace: pick {i} carries neither elementId nor topoKey"
                )))
            }
            (Some(_), Some(_)) => {
                return Err(ApiError::InvalidCommand(format!(
                    "prepareOffsetFace: pick {i} carries BOTH elementId and topoKey — the two \
                     rungs are mutually exclusive (SCHEMA §7.6)"
                )))
            }
            _ => {}
        }
        let body = match p.body_id.as_deref().filter(|s| !s.is_empty()) {
            Some(b) => Some(wire::parse_body_id(b).map_err(ApiError::InvalidCommand)?),
            // The `elementId` rung resolves its own body from the partition entry;
            // the `topoKey` rung has nothing to resolve against without one.
            None if topo.is_some() => {
                return Err(ApiError::InvalidCommand(format!(
                    "prepareOffsetFace: pick {i} addresses a topoKey but carries no bodyId"
                )))
            }
            None => None,
        };
        bodies.push(body);
    }
    let picks: Vec<wire::OffsetFacePick<'_>> = picked_faces
        .iter()
        .zip(&bodies)
        .map(|(p, body)| {
            let address = match p.element_id.as_deref().filter(|s| !s.is_empty()) {
                Some(id) => wire::FaceAddress::ElementId(id),
                None => wire::FaceAddress::TopoKey(p.topo_key.as_deref().unwrap_or_default()),
            };
            wire::OffsetFacePick {
                body: *body,
                address,
            }
        })
        .collect();
    {
        // Presence check only — the lock is NOT held across the worker round-trip.
        let guard = state.runtime.lock().await;
        guard
            .as_ref()
            .ok_or_else(|| ApiError::NoDocument("prepareOffsetFace".into()))?;
    }
    state
        .face_projection()
        .prepare_offset_face(
            SnapshotId(snapshot_id),
            &picks,
            chain_tangent_faces,
            distance_type,
        )
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn prepare_edge_op(
    state: State<'_, AppState>,
    app: AppHandle,
    snapshot_id: u64,
    mode: String,
    picked_edges: Vec<EdgeOpPickInput>,
    chain_tangent_edges: bool,
) -> Result<crate::dto::PrepareEdgeOpDto, ApiError> {
    if picked_edges.is_empty() {
        return Err(ApiError::InvalidCommand(
            "prepareEdgeOp: at least one picked edge is required".into(),
        ));
    }
    let mode = edge_op_mode(&mode)?;
    let bodies = edge_op_bodies(&picked_edges)?;
    let picks = wire_edge_op_picks(&picked_edges, &bodies);
    {
        let guard = state.runtime.lock().await;
        guard
            .as_ref()
            .ok_or_else(|| ApiError::NoDocument("prepareEdgeOp".into()))?;
    }
    let mut prepared = state
        .face_projection()
        .prepare_edge_op(SnapshotId(snapshot_id), mode, &picks, chain_tangent_edges)
        .await
        .map_err(ApiError::from)?;
    if prepared.refusal.is_some() {
        return Ok(prepared);
    }
    let projection = promote_prepared_edges(state.inner(), &mut prepared).await?;
    let _ = app.emit(events::PROJECTION_UPDATED, &projection);
    Ok(prepared)
}

/// Dry-run ladder resolution for repair dialogs (`ResolveRefs`; SCHEMA §7.5) —
/// binds nothing.
#[tauri::command]
pub async fn resolve_refs(
    state: State<'_, AppState>,
    snapshot_id: u64,
    refs: Vec<ResolveRefInput>,
) -> Result<Vec<ResolveRefDto>, ApiError> {
    let req = ResolveRequest {
        snapshot_id: SnapshotId(snapshot_id),
        refs: refs
            .into_iter()
            .map(|r| ResolveRef {
                ref_id: r.ref_id,
                element: r.element,
            })
            .collect(),
    };
    let resolutions = {
        let guard = state.runtime.lock().await;
        let rt = guard
            .as_ref()
            .ok_or_else(|| ApiError::NoDocument("resolveRefs".into()))?;
        rt.resolve_refs(req).await?
    };
    Ok(resolutions
        .into_iter()
        .map(ResolveRefDto::from_resolution)
        .collect())
}

/// The stored params of an operation record (read-only; `CadClient.getOperationParams`)
/// as the `EditCommand` `op.params` serde JSON. A parametric scalar re-edit (revolve
/// angle / shell thickness / fillet radius) fetches these on arm and deep-merges the
/// changed scalar on commit, so it preserves the op's non-scalar inputs (axis /
/// openFaces / edges) the projection does not expose — a whole-params replace would
/// otherwise silently clobber them.
#[tauri::command]
pub async fn get_operation_params(
    state: State<'_, AppState>,
    record_id: String,
) -> Result<serde_json::Value, ApiError> {
    let record = RecordId::from_str(&record_id)
        .map_err(|e| ApiError::InvalidCommand(format!("bad recordId {record_id:?}: {e}")))?;
    let guard = state.runtime.lock().await;
    let rt = guard
        .as_ref()
        .ok_or_else(|| ApiError::NoDocument("getOperationParams".into()))?;
    rt.operation_params(record).ok_or_else(|| {
        ApiError::InvalidCommand(format!(
            "no params for record {record_id} (unknown or opaque)"
        ))
    })
}

/// A feature's upstream/downstream transitive closures (read-only; H10 dependency
/// view — `CadClient.featureDependencies`). Consumed by the delete/suppress
/// confirmations (dependent counts) and the inspector's "Depends on / Used by"
/// section. A suppressed record is still a valid query target — suppression is a
/// node flag on the graph, not removal from it.
#[tauri::command]
pub async fn feature_dependencies(
    state: State<'_, AppState>,
    record_id: String,
) -> Result<FeatureDependenciesDto, ApiError> {
    let record = RecordId::from_str(&record_id)
        .map_err(|e| ApiError::InvalidCommand(format!("bad recordId {record_id:?}: {e}")))?;
    let guard = state.runtime.lock().await;
    let rt = guard
        .as_ref()
        .ok_or_else(|| ApiError::NoDocument("featureDependencies".into()))?;
    rt.feature_dependencies(record)
        .ok_or_else(|| ApiError::InvalidCommand(format!("unknown record {record_id}")))
}

fn parse_sketch_id(s: &str) -> Result<SketchId, ApiError> {
    SketchId::from_str(s).map_err(|e| ApiError::InvalidCommand(format!("bad sketchId {s:?}: {e}")))
}

// ─────────────────────────────────────────────────────────────────────────────
// Worker crash circuit
// ─────────────────────────────────────────────────────────────────────────────

/// Clears the worker crash-circuit breaker (SCHEMA §8 poison / F3) and returns how
/// many poison keys were forgotten.
///
/// A poison key is `(historyPrefixHash-through-op, opRecordId, occtFingerprint)`, so
/// the ordinary fix — editing the crashing op's parameters — already mints a new key
/// and heals the circuit by itself. This exists for what the key cannot see: an
/// input file repaired outside the document, or a worker rebuilt in place without a
/// fingerprint bump. No document needs to be open (the circuit belongs to the
/// worker, not the document).
#[tauri::command]
pub async fn clear_worker_circuit(state: State<'_, AppState>) -> Result<usize, ApiError> {
    Ok(state.circuit().clear_circuit())
}

// ─────────────────────────────────────────────────────────────────────────────
// Start screen + native dialogs (Rust-side; webview has zero fs/dialog cap)
// ─────────────────────────────────────────────────────────────────────────────

/// Recent projects for the start screen, read from the persisted recents store at
/// `<app_config_dir>/recents.json` (a missing file ⇒ empty). Written on every
/// successful open/save by [`recents::record`].
///
/// Each card's `thumbnail` is that project's container `preview.png` as a `data:`
/// URL. Those are up to ten small file reads, so they run in ONE `spawn_blocking`
/// rather than on the async runtime — and if that task somehow fails, the listing
/// still returns, just without pictures.
#[tauri::command]
pub async fn list_recents(app: AppHandle) -> Result<Vec<RecentProjectDto>, ApiError> {
    let entries = recents::list(&app);
    let with_previews = entries.clone();
    match tokio::task::spawn_blocking(move || recents::with_thumbnails(with_previews)).await {
        Ok(list) => Ok(list),
        Err(e) => {
            tracing::warn!(error = %e, "recents: thumbnail read task failed — listing without previews");
            Ok(entries)
        }
    }
}

/// Renames a recent project's `.onecad` file on disk and its `recents.json` entry
/// (start-screen card menu). See [`recents::rename`] for the exact rules (name
/// validation, collision refusal).
#[tauri::command]
#[tracing::instrument(skip_all, fields(path = %path, new_name = %new_name), err(Display))]
pub async fn rename_recent_project(
    app: AppHandle,
    path: String,
    new_name: String,
) -> Result<(), ApiError> {
    recents::rename(&app, Path::new(&path), &new_name)?;
    Ok(())
}

/// Moves a recent project's `.onecad` file to the OS trash and drops its
/// `recents.json` entry (start-screen card menu). See [`recents::remove`].
#[tauri::command]
#[tracing::instrument(skip_all, fields(path = %path), err(Display))]
pub async fn delete_recent_project(app: AppHandle, path: String) -> Result<(), ApiError> {
    recents::remove(&app, Path::new(&path))
}

/// Reveals a recent project's `.onecad` file in the OS file manager (start-screen
/// card menu). See [`recents::reveal`].
#[tauri::command]
#[tracing::instrument(skip_all, fields(path = %path), err(Display))]
pub async fn reveal_in_file_manager(path: String) -> Result<(), ApiError> {
    recents::reveal(Path::new(&path))
}

/// Shows a native open dialog (Rust owns the dialog; `tauri-plugin-dialog` Rust
/// API). Resolves to the chosen path or `None` if cancelled.
#[tauri::command]
pub async fn open_file_dialog(app: AppHandle) -> Result<Option<String>, ApiError> {
    Ok(pick_file(app, false).await)
}

/// Shows a native save dialog. Resolves to the chosen path or `None`.
#[tauri::command]
pub async fn save_file_dialog(app: AppHandle) -> Result<Option<String>, ApiError> {
    Ok(pick_file(app, true).await)
}

/// Shows a native **STEP** open dialog (`.step`/`.stp`) for the start-screen import
/// lane, resolving to the chosen path or `None` on cancel.
///
/// A separate command from [`open_file_dialog`] on purpose: that one filters
/// `.onecad`, and widening its filter would let the plain Open action offer files
/// [`open_document`] cannot read. The start-screen `importStep` flow must pick its
/// path HERE — see the note on [`import_step`]. (The in-editor lane needs no
/// command: [`insert_step`] runs its own dialog.)
#[tauri::command]
pub async fn step_file_dialog(app: AppHandle) -> Result<Option<String>, ApiError> {
    Ok(pick_step_open(app).await)
}

async fn pick_file(app: AppHandle, save: bool) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    let dialog = app.dialog().file().add_filter("OneCAD", &["onecad"]);
    let cb = move |file: Option<tauri_plugin_dialog::FilePath>| {
        let _ = tx.send(file);
    };
    if save {
        dialog.save_file(cb);
    } else {
        dialog.pick_file(cb);
    }
    rx.await
        .ok()
        .flatten()
        .and_then(|f| f.into_path().ok())
        .map(|p| p.to_string_lossy().into_owned())
}

/// Shows a native STEP save dialog (`.step`/`.stp` filter). Resolves to the chosen
/// path or `None` on cancel. Mirrors [`pick_file`] but with the STEP filter.
async fn pick_step_save(app: AppHandle) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter("STEP", &["step", "stp"])
        .save_file(move |file: Option<tauri_plugin_dialog::FilePath>| {
            let _ = tx.send(file);
        });
    rx.await
        .ok()
        .flatten()
        .and_then(|f| f.into_path().ok())
        .map(|p| p.to_string_lossy().into_owned())
}

/// Shows a native STEP **open** dialog (`.step`/`.stp` filter) for the in-editor
/// import lane. Resolves to the chosen path or `None` on cancel. Mirrors
/// [`pick_step_save`] with `pick_file` instead of `save_file`.
async fn pick_step_open(app: AppHandle) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter("STEP", &["step", "stp"])
        .pick_file(move |file: Option<tauri_plugin_dialog::FilePath>| {
            let _ = tx.send(file);
        });
    rx.await
        .ok()
        .flatten()
        .and_then(|f| f.into_path().ok())
        .map(|p| p.to_string_lossy().into_owned())
}

/// Shows a native mesh save dialog with a `label`/`extensions` filter (`.stl` /
/// `.obj`). Resolves to the chosen path or `None` on cancel. Mirrors
/// [`pick_step_save`] with the caller's filter (M5a mesh export).
async fn pick_mesh_save(app: AppHandle, label: &str, extensions: &[&str]) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().add_filter(label, extensions).save_file(
        move |file: Option<tauri_plugin_dialog::FilePath>| {
            let _ = tx.send(file);
        },
    );
    rx.await
        .ok()
        .flatten()
        .and_then(|f| f.into_path().ok())
        .map(|p| p.to_string_lossy().into_owned())
}

// ─────────────────────────────────────────────────────────────────────────────
// Frontend log bridge (DEV-OBSERVABILITY wave R/F)
// ─────────────────────────────────────────────────────────────────────────────

/// One frontend log event, as batched by `src/debug/logSink.ts`. The shape is
/// FROZEN with that module (camelCase over the wire, both sides land in one wave).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeLogEvent {
    /// `Date.now()` at emit (epoch ms, fractional).
    pub ts: f64,
    /// `performance.now()` at emit (ms since page load) — the monotonic ordering key.
    pub t_mono: f64,
    /// Per-session monotonic sequence (gap ⇒ the ring dropped events).
    pub seq: u64,
    /// Reserved for a future non-`fe` producer; the target is always `fe` today.
    pub lane: String,
    /// `error` / `warn` / `info` / anything else ⇒ debug.
    pub level: String,
    /// Tag taxonomy (`ipc`, `hint`, `fsm`, `vp`, …).
    pub tag: String,
    pub msg: String,
    #[serde(default)]
    pub ctx: Option<serde_json::Value>,
}

/// Max serialized `ctx` bytes re-emitted per event — a frontend context object is
/// capped at the source too, this is the belt-and-braces bound on the log line.
const FE_CTX_CAP: usize = 2048;

/// Re-emits batched frontend log events into the Rust `tracing` stream under target
/// `fe`, so ONE `logs/dev.jsonl` carries both lanes in wall-clock order.
///
/// Deliberately infallible and NOT instrumented: it is the logging path, so it must
/// never fail a caller, never recurse, and never add a span of its own to every
/// batch. Malformed content is capped/stringified, never rejected.
#[tauri::command]
pub fn log_event(events: Vec<FeLogEvent>) {
    for event in events {
        let ctx = event
            .ctx
            .as_ref()
            .map(|v| cap_str(v.to_string(), FE_CTX_CAP));
        emit_fe_event(&event, ctx.as_deref());
    }
}

/// Truncates `s` to at most `max` bytes on a char boundary, marking the cut.
fn cap_str(mut s: String, max: usize) -> String {
    if s.len() > max {
        let mut end = max;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
        s.push('…');
    }
    s
}

fn emit_fe_event(event: &FeLogEvent, ctx: Option<&str>) {
    // The level is runtime data but `tracing` levels are compile-time, so the four
    // arms are spelled out (one macro body, four call sites).
    macro_rules! fe {
        ($level:ident) => {
            tracing::$level!(
                target: "fe",
                seq = event.seq,
                tag = %event.tag,
                feTs = event.ts,
                tMono = event.t_mono,
                ctx = ctx,
                "{}",
                event.msg
            )
        };
    }
    match event.level.as_str() {
        "error" => fe!(error),
        "warn" => fe!(warn),
        "info" => fe!(info),
        _ => fe!(debug),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Shared helpers (also used by the regen driver in `crate::run`)
// ─────────────────────────────────────────────────────────────────────────────

/// Emits the post-regen events: `document-changed` (pull-model body refs) when a
/// snapshot published, the refreshed `projection-updated`, `regen-finished`
/// (`{revision, outcome}`) at the end of **every** regen so the frontend
/// correlation resolves promptly without the 8 s fallback (F-WP8 flag 3), and —
/// on a **published** regen — `needs-repair` (`{revision, items}`) so the repair
/// banner appears (items non-empty) or is dropped (items empty ⇒ repairs cleared;
/// M4a). A superseded/failed/no-op regen leaves the live repair state unchanged, so
/// no `needs-repair` is emitted for those.
///
/// **Not a logging site.** The `regen:` postmortem line + the per-failed-step warns
/// live in [`DocumentRuntime::finish_regen`], which runs on EVERY regen path
/// (including the inline `run_regen` used by tests and the headless CLI); logging
/// here as well would double every line for the app lane only.
pub fn emit_regen_events(app: &AppHandle, report: &RegenReport, projection: &DocumentProjection) {
    if let Some(change) = report.document_change() {
        let _ = app.emit(events::DOCUMENT_CHANGED, change);
    }
    let _ = app.emit(events::PROJECTION_UPDATED, projection);
    let _ = app.emit(
        events::REGEN_FINISHED,
        crate::dto::RegenFinished {
            revision: report.revision,
            source_revision: report.source_revision,
            outcome: report.outcome_str().to_string(),
            message: report.failure_message(),
            diagnostics: report.diagnostics.clone(),
            // Finding 1: a published-but-partially-failed regen carries per-record
            // failure + created/modified-body maps so a correlated apply resolves its
            // own commit precisely (never mistaking sibling republishes for success).
            failed_steps: report.failed_steps.clone(),
            affected_bodies: report.affected_bodies.clone(),
        },
    );
    if report.published() {
        let _ = app.emit(
            events::NEEDS_REPAIR,
            crate::dto::NeedsRepairEvent {
                revision: report.revision,
                items: report.needs_repair.clone(),
            },
        );
    }
}

fn snapshot_of(rt: &DocumentRuntime) -> DocumentSnapshotDto {
    DocumentSnapshotDto {
        document_id: rt.document_id(),
        title: rt.title().to_string(),
    }
}

/// Provenance metadata for a save. The pure core never reads the wall clock, so
/// the app supplies the timestamps here.
fn save_meta() -> SaveMeta {
    let now = now_rfc3339();
    SaveMeta {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        occt_fingerprint: None,
        created: now.clone(),
        modified: now,
    }
}

/// The current UTC time as an RFC-3339 string (`YYYY-MM-DDThh:mm:ssZ`).
fn now_rfc3339() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    rfc3339_from_secs(secs)
}

/// An RFC-3339 string (`YYYY-MM-DDThh:mm:ssZ`) for `secs` since the Unix epoch,
/// computed without a calendar dependency (Howard Hinnant's civil-date algorithm).
/// Shared with the recents store (last-opened timestamps).
pub(crate) fn rfc3339_from_secs(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (hh, mm, ss) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let (y, mon, d) = civil_from_days(days);
    format!("{y:04}-{mon:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

/// `(year, month, day)` from a Unix day count (days since 1970-01-01).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_from_days_epoch_and_a_known_date() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        // 2026-07-17 is day 20651 since the Unix epoch.
        assert_eq!(civil_from_days(20_651), (2026, 7, 17));
    }

    #[test]
    fn now_rfc3339_is_well_formed() {
        let s = now_rfc3339();
        assert_eq!(s.len(), 20, "YYYY-MM-DDThh:mm:ssZ");
        assert!(s.ends_with('Z') && s.contains('T'));
    }

    #[test]
    fn gesture_target_args_validate_strictly() {
        let entity = EntityId(uuid::Uuid::from_u128(0x41));
        let parse = |v: serde_json::Value| {
            serde_json::from_value::<GestureTargetArgs>(v)
                .expect("deserializes")
                .validate()
        };

        // Full arcEnd target: kind + entity + role + grab all survive.
        let ok = parse(serde_json::json!({
            "kind": "arcEnd", "entity": entity.to_string(), "role": "End", "grab": [1.5, -2.0]
        }))
        .expect("valid arcEnd");
        assert_eq!(
            ok,
            wire::GestureTarget {
                kind: wire::DragKind::ArcEnd,
                entity,
                role: Some("end"), // case-folded onto the §7.4 token
                grab: Some([1.5, -2.0]),
            }
        );
        // Optional members really are optional.
        let body = parse(serde_json::json!({ "kind": "entityBody", "entity": entity.to_string() }))
            .expect("valid entityBody");
        assert_eq!(body.role, None);
        assert_eq!(body.grab, None);

        // Every rejection: unknown kind (NEVER degraded to point), bad entity,
        // unknown role, arcEnd without a usable role, non-finite grab.
        for bad in [
            serde_json::json!({ "kind": "scale", "entity": entity.to_string() }),
            serde_json::json!({ "kind": "Point", "entity": entity.to_string() }),
            serde_json::json!({ "kind": "radius", "entity": "not-a-uuid" }),
            serde_json::json!({ "kind": "point", "entity": entity.to_string(), "role": "middle" }),
            serde_json::json!({ "kind": "arcEnd", "entity": entity.to_string() }),
            serde_json::json!({ "kind": "arcEnd", "entity": entity.to_string(), "role": "center" }),
        ] {
            let err = parse(bad.clone()).expect_err(&format!("{bad} must be refused"));
            assert!(
                matches!(err, ApiError::InvalidCommand(_)),
                "{bad} → {err:?} should be an invalid-command error"
            );
        }

        // A non-finite `grab` cannot even be spelled in JSON (it stringifies to
        // null and fails at deserialization), so this guard is asserted against a
        // directly-constructed value. It matters because the worker's `read_grab`
        // treats a non-finite grab as ABSENT — which silently turns a body drag
        // into an anchor teleport instead of failing.
        let err = GestureTargetArgs {
            kind: "entityBody".into(),
            entity: entity.to_string(),
            role: None,
            grab: Some([f64::NAN, 0.0]),
        }
        .validate()
        .expect_err("a non-finite grab is refused");
        assert!(matches!(err, ApiError::InvalidCommand(_)), "{err:?}");
    }

    #[test]
    fn preview_operation_is_typed_and_preserves_the_draft_id() {
        let op_id = uuid::Uuid::from_u128(0xCAFE).to_string();
        let sketch_id = uuid::Uuid::from_u128(0xBEEF).to_string();
        let raw = serde_json::json!({
            "opType": "Extrude",
            "opId": op_id,
            "params": {
                "profile": { "sketchId": sketch_id, "regionId": "r_second" },
                "distance": { "value": 5.0 },
                "draftAngleDeg": { "value": 0.0 },
                "extrudeMode": "Blind",
                "booleanMode": "NewBody",
                "twoDirections": false,
                "extrudeMode2": "Blind",
                "distance2": { "value": 0.0 }
            }
        });
        let (operation, parsed_id) = parse_preview_operation(raw).unwrap();
        assert_eq!(parsed_id, op_id);
        let Operation::Known(onecad_core::document::record::KnownOperation::Extrude(params)) =
            operation
        else {
            panic!("expected typed Extrude");
        };
        assert_eq!(
            params.profile.unwrap().region.as_str(),
            "r_second",
            "exact selected region survives typed deserialization"
        );
    }

    #[test]
    fn malformed_known_preview_operation_is_rejected() {
        let raw = serde_json::json!({
            "opType": "Extrude",
            "params": { "distance": "five" }
        });
        assert!(matches!(
            parse_preview_operation(raw),
            Err(ApiError::InvalidCommand(_))
        ));

        let bad_id = serde_json::json!({ "opType": "FutureOp", "opId": "not-a-record-id" });
        assert!(matches!(
            parse_preview_operation(bad_id),
            Err(ApiError::InvalidCommand(_))
        ));
    }

    #[test]
    fn preview_operation_without_id_uses_a_canonical_stable_fallback() {
        let raw = serde_json::json!({ "opType": "FutureOp", "params": {} });
        let (_, first) = parse_preview_operation(raw.clone()).unwrap();
        let (_, second) = parse_preview_operation(raw).unwrap();
        assert_eq!(first, second);
        assert_eq!(RecordId::from_str(&first).unwrap().to_string(), first);
    }

    // ─────────────────────────────────────────────────────────────────────
    // `spawn_gated_regen` — the cold-worker regen race fix
    // ─────────────────────────────────────────────────────────────────────
    //
    // Neither scripted readiness type touches a real worker: `Never` sleeps out
    // the timeout and reports not-ready (mirrors `PendingBackend`); `Already`
    // resolves instantly (mirrors an adopted, already-`Ready` `WorkerManager`
    // under `AppState::prewarm`). The scheduler side is a REAL
    // `onecad_core::regen::RegenScheduler` over a driver that just echoes the
    // request onto a channel — enough to observe "the request fired", without
    // a `DocumentRuntime` in the loop.

    mod readiness_gate {
        use std::sync::{Arc, OnceLock};
        use std::time::Duration;

        use tokio::sync::mpsc::unbounded_channel;

        use onecad_core::regen::{Outcome, RegenDirective, RegenRequest, RegenScheduler};

        use crate::state::SharedScheduler;
        use crate::worker::WorkerReadiness;

        use super::super::spawn_gated_regen;

        /// Never reports ready; sleeps out the caller's timeout first so the
        /// gate's "fire anyway on timeout" behavior is genuinely exercised
        /// rather than racing a same-tick return.
        struct Never;
        #[async_trait::async_trait]
        impl WorkerReadiness for Never {
            async fn wait_ready(&self, timeout: Duration) -> bool {
                tokio::time::sleep(timeout).await;
                false
            }
        }

        /// Readiness the test opens by hand, so the window BEFORE it resolves —
        /// the one the cold-worker race lived in — is directly observable.
        /// `notify_one` stores a permit, so releasing the latch before the gate
        /// gets to await it is not a lost wakeup.
        struct Latch(Arc<tokio::sync::Notify>);
        #[async_trait::async_trait]
        impl WorkerReadiness for Latch {
            async fn wait_ready(&self, timeout: Duration) -> bool {
                tokio::time::timeout(timeout, self.0.notified())
                    .await
                    .is_ok()
            }
        }

        /// Reports ready immediately (the `AppState::prewarm` common case).
        struct Already;
        #[async_trait::async_trait]
        impl WorkerReadiness for Already {
            async fn wait_ready(&self, _timeout: Duration) -> bool {
                true
            }
        }

        /// A live `RegenScheduler` whose driver echoes every request's `request`
        /// onto an unbounded channel and reports `NoOp` (no `DocumentRuntime`
        /// needed to observe "the request fired").
        fn recording_scheduler() -> (
            SharedScheduler,
            tokio::sync::mpsc::UnboundedReceiver<RegenRequest>,
        ) {
            let (tx, rx) = unbounded_channel();
            let driver = move |directive: RegenDirective| {
                let tx = tx.clone();
                async move {
                    let _ = tx.send(directive.request);
                    Outcome::NoOp
                }
            };
            let (scheduler, handle) = RegenScheduler::new(driver);
            tokio::spawn(scheduler.run());
            let shared: SharedScheduler = Arc::new(OnceLock::new());
            shared.set(handle).expect("freshly-constructed OnceLock");
            (shared, rx)
        }

        /// The load-bearing half of the fix: while the worker is still connecting,
        /// NOTHING reaches the scheduler. Firing into that window is what produced
        /// "worker not connected" on every open of a project with geometry — a
        /// from-0 plan has no retry path, so that first failure was terminal.
        #[tokio::test(flavor = "multi_thread")]
        async fn withholds_the_request_until_readiness_resolves() {
            let (sched, mut rx) = recording_scheduler();
            let latch = Arc::new(tokio::sync::Notify::new());
            spawn_gated_regen(
                Arc::new(Latch(latch.clone())),
                sched.clone(),
                Duration::from_secs(30),
                || async { true },
            );

            assert!(
                tokio::time::timeout(Duration::from_millis(150), rx.recv())
                    .await
                    .is_err(),
                "no regen request may reach the scheduler before readiness resolves"
            );

            latch.notify_one();
            let got = tokio::time::timeout(Duration::from_secs(2), rx.recv())
                .await
                .expect("the request fires as soon as readiness resolves")
                .expect("scheduler channel stayed open");
            assert!(
                matches!(got, RegenRequest::ToEnd { from: 0 }),
                "got {got:?}"
            );
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn fires_the_request_after_timeout_when_never_ready() {
            let (sched, mut rx) = recording_scheduler();
            // `sched` is kept alive here for the rest of the test — mirroring
            // `AppState` retaining its OWN `SharedScheduler` reference in
            // production. Moving the only strong reference into the spawned
            // task would drop the `SchedulerHandle`'s sender the instant that
            // task finishes, racing (and sometimes beating) the scheduler's own
            // poll of the in-flight driver future.
            spawn_gated_regen(
                Arc::new(Never),
                sched.clone(),
                Duration::from_millis(20),
                || async { true },
            );

            let got = tokio::time::timeout(Duration::from_secs(5), rx.recv())
                .await
                .expect("the request must still fire once the timeout elapses")
                .expect("scheduler channel stayed open");
            assert!(
                matches!(got, RegenRequest::ToEnd { from: 0 }),
                "got {got:?}"
            );
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn fires_promptly_when_already_ready() {
            let (sched, mut rx) = recording_scheduler();
            // A generous timeout the fast path must never actually wait out.
            // `sched` stays alive for the same reason as the sibling test above.
            spawn_gated_regen(
                Arc::new(Already),
                sched.clone(),
                Duration::from_secs(30),
                || async { true },
            );

            let got = tokio::time::timeout(Duration::from_secs(2), rx.recv())
                .await
                .expect("an already-ready readiness must not wait anywhere near the timeout")
                .expect("scheduler channel stayed open");
            assert!(
                matches!(got, RegenRequest::ToEnd { from: 0 }),
                "got {got:?}"
            );
        }

        /// A gate whose document was replaced while it waited must NOT fire:
        /// the scheduler is global, so a stale from-0 request would land on the
        /// NEW document — racing its own still-connecting worker (the exact
        /// failure this gate exists to prevent) or superseding its active regen.
        #[tokio::test(flavor = "multi_thread")]
        async fn skips_when_the_document_was_replaced_while_waiting() {
            let (sched, mut rx) = recording_scheduler();
            spawn_gated_regen(
                Arc::new(Already),
                sched.clone(),
                Duration::from_secs(30),
                || async { false }, // the close/open-during-wait shape
            );

            assert!(
                tokio::time::timeout(Duration::from_millis(200), rx.recv())
                    .await
                    .is_err(),
                "a stale gate must never reach the scheduler"
            );
        }

        /// `PendingBackend` — the production no-worker fallback — must itself
        /// report not-ready immediately rather than hanging the caller's timeout
        /// budget (it's a distinct behavior from `Never` above, which sleeps
        /// deliberately to exercise the gate's timeout path).
        #[tokio::test]
        async fn pending_backend_reports_not_ready_without_waiting() {
            let result = tokio::time::timeout(
                Duration::from_millis(200),
                crate::worker::PendingBackend.wait_ready(Duration::from_secs(30)),
            )
            .await;
            assert_eq!(result, Ok(false), "PendingBackend never becomes ready");
        }
    }
}
