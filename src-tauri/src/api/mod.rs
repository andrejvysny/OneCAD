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
use crate::document_runtime::{DocumentRuntime, RegenReport};
use crate::dto::{
    BeginGestureDto, DocumentProjection, DocumentSnapshotDto, DragSolveDto, FinishSketchDto,
    PromotedElementDto, RecentProjectDto, RecoveryInfoDto, ResolveRefDto, SketchOnFaceDto,
    SketchSessionDto, SketchUpsertDto,
};
use crate::error::ApiError;
use crate::events;
use crate::recents;
use crate::state::AppState;
use crate::worker::{lod_from_str, wire};

// ─────────────────────────────────────────────────────────────────────────────
// Lifecycle
// ─────────────────────────────────────────────────────────────────────────────

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
        (snapshot_of(rt), rt.projection())
    };
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
    let (engine, meshes, solver) = state.make_backend();
    let rt = DocumentRuntime::open(Path::new(&path), engine, meshes, solver)?;
    let (snapshot, projection) = {
        let mut guard = state.runtime.lock().await;
        *guard = Some(rt);
        let rt = guard.as_ref().unwrap();
        (snapshot_of(rt), rt.projection())
    };
    let _ = app.emit(events::PROJECTION_UPDATED, &projection);
    recents::record(&app, Path::new(&path));
    // Rebuild geometry from the loaded (all-Dirty) timeline.
    if let Some(sched) = state.scheduler.get() {
        sched.request(RegenRequest::ToEnd { from: 0 });
    }
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
    let prepared = crate::imports::prepare_import(&*state.step_import(), Path::new(&path)).await?;
    let mut rt = DocumentRuntime::new_blank(engine, meshes, solver);
    rt.add_import_record(&prepared, false)?;
    let (snapshot, projection) = {
        let mut guard = state.runtime.lock().await;
        *guard = Some(rt);
        let rt = guard.as_ref().unwrap();
        (snapshot_of(rt), rt.projection())
    };
    let _ = app.emit(events::PROJECTION_UPDATED, &projection);
    if let Some(sched) = state.scheduler.get() {
        sched.request(RegenRequest::ToEnd { from: 0 });
    }
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
#[tauri::command]
#[tracing::instrument(skip_all, fields(path = ?path), err(Display))]
pub async fn save_document(
    state: State<'_, AppState>,
    app: AppHandle,
    path: Option<String>,
) -> Result<(), ApiError> {
    let (target, document_id): (PathBuf, onecad_core::ids::DocumentId) = {
        let mut guard = state.runtime.lock().await;
        let rt = guard
            .as_mut()
            .ok_or_else(|| ApiError::NoDocument("save".into()))?;
        let target: PathBuf = match path {
            Some(p) => PathBuf::from(p),
            None => rt
                .path()
                .map(Path::to_path_buf)
                .ok_or_else(|| ApiError::Io("no save path; provide one".into()))?,
        };
        // Checkpoint policy (SCHEMA §7.7): mint a durable acceleration base of the
        // current head before persisting, so a reopen/edit can regen incrementally.
        rt.take_checkpoint_at_head().await;
        rt.save(&target, save_meta())?;
        (target, rt.document_uuid())
    };
    // A clean save supersedes any crash-recovery state for this document.
    if let Some(root) = autosave::autosave_root(&app) {
        autosave::clear_recovery_state(&root, document_id);
    }
    recents::record(&app, &target);
    Ok(())
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
    let (engine, meshes, solver) = state.make_backend();
    let mut rt = DocumentRuntime::open(&offer.autosave_path, engine, meshes, solver)?;
    rt.mark_recovered(offer.marker.opened_path.clone());
    let (snapshot, projection) = {
        let mut guard = state.runtime.lock().await;
        *guard = Some(rt);
        let rt = guard.as_ref().unwrap();
        (snapshot_of(rt), rt.projection())
    };
    // Consume the marker (the autosave is superseded on the next save/close). The
    // autosave file itself is kept so a re-crash before the next tick still recovers.
    if let Some(root) = &root {
        let _ = onecad_core::io::recovery::remove_marker(root, offer.document_id);
    }
    let _ = app.emit(events::PROJECTION_UPDATED, &projection);
    // Rebuild geometry from the recovered (all-Dirty) timeline.
    if let Some(sched) = state.scheduler.get() {
        sched.request(RegenRequest::ToEnd { from: 0 });
    }
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
#[tauri::command]
pub async fn undo(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<DocumentProjection, ApiError> {
    let (changed, projection) = {
        let mut guard = state.runtime.lock().await;
        let rt = guard
            .as_mut()
            .ok_or_else(|| ApiError::NoDocument("undo".into()))?;
        (rt.undo(), rt.projection())
    };
    let _ = app.emit(events::PROJECTION_UPDATED, &projection);
    if changed {
        if let Some(sched) = state.scheduler.get() {
            sched.request(RegenRequest::ToEnd { from: 0 });
        }
        state.note_mutation();
    }
    Ok(projection)
}

/// Redoes the last undone edit (`CadClient.redo`).
#[tauri::command]
pub async fn redo(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<DocumentProjection, ApiError> {
    let (changed, projection) = {
        let mut guard = state.runtime.lock().await;
        let rt = guard
            .as_mut()
            .ok_or_else(|| ApiError::NoDocument("redo".into()))?;
        (rt.redo()?, rt.projection())
    };
    let _ = app.emit(events::PROJECTION_UPDATED, &projection);
    if changed {
        if let Some(sched) = state.scheduler.get() {
            sched.request(RegenRequest::ToEnd { from: 0 });
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

/// Opens a drag gesture on a point (`BeginGesture`; SCHEMA §7.4).
#[tauri::command]
pub async fn begin_gesture(
    state: State<'_, AppState>,
    sketch_id: String,
    drag_point: String,
) -> Result<BeginGestureDto, ApiError> {
    let id = parse_sketch_id(&sketch_id)?;
    let point = EntityId::from_str(&drag_point)
        .map_err(|e| ApiError::InvalidCommand(format!("bad dragPoint {drag_point:?}: {e}")))?;
    let mut guard = state.runtime.lock().await;
    let rt = guard
        .as_mut()
        .ok_or_else(|| ApiError::NoDocument("beginGesture".into()))?;
    Ok(rt.begin_gesture(id, point).await?)
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

fn parse_sketch_id(s: &str) -> Result<SketchId, ApiError> {
    SketchId::from_str(s).map_err(|e| ApiError::InvalidCommand(format!("bad sketchId {s:?}: {e}")))
}

// ─────────────────────────────────────────────────────────────────────────────
// Start screen + native dialogs (Rust-side; webview has zero fs/dialog cap)
// ─────────────────────────────────────────────────────────────────────────────

/// Recent projects for the start screen, read from the persisted recents store at
/// `<app_config_dir>/recents.json` (a missing file ⇒ empty). Written on every
/// successful open/save by [`recents::record`].
#[tauri::command]
pub async fn list_recents(app: AppHandle) -> Result<Vec<RecentProjectDto>, ApiError> {
    Ok(recents::list(&app))
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
}
