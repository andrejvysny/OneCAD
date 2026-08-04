//! OneCAD Tauri application shell (crate `onecad`, lib `onecad_lib`).
//!
//! Thin host around [`onecad_core`]: owns the webview, the geometry-backend seam,
//! and the command/event surface. All filesystem/dialog IO happens in Rust (the
//! webview has zero shell/fs capabilities; capabilities stay `core:default`).
//!
//! Layers:
//! * [`document_runtime`] — the per-document single writer (all domain logic).
//! * [`state`] — [`AppState`](state::AppState): the managed runtime + scheduler
//!   handle + backend factory.
//! * [`api`] — thin `#[tauri::command]` wrappers that delegate to the runtime.
//! * [`worker`] — the [`GeometryEngine`](onecad_core::regen::GeometryEngine) +
//!   [`MeshProvider`](worker::MeshProvider) seam R-WP11 plugs the real sidecar
//!   into, plus the D1 [`AdoptingEngine`](worker::AdoptingEngine).
//! * [`dto`]/[`events`] — the camelCase projection DTOs + event channel names.

pub mod api;
pub mod autosave;
pub mod document_runtime;
pub mod dto;
pub mod error;
pub mod events;
pub mod export;
pub mod imports;
pub mod logging;
pub mod mesh_cache;
pub mod recents;
pub mod state;
pub mod worker;

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager, RunEvent, WindowEvent};
use tokio::sync::{watch, Mutex};

use onecad_core::regen::{Outcome, RegenDirective, RegenScheduler};

use crate::document_runtime::{DocumentRuntime, RegenReport};
use crate::dto::DocumentProjection;
use crate::state::AppState;

/// Re-entrancy guard for the close/quit confirmation prompt (unsaved-changes
/// guard). Shared by the native window-close button
/// ([`WindowEvent::CloseRequested`]) and the app-level exit request
/// ([`RunEvent::ExitRequested`], e.g. ⌘Q) so only the FIRST of either emits
/// [`events::CLOSE_REQUESTED`] while a frontend prompt is already pending —
/// repeated clicks/chords just keep blocking the close/exit without piling up
/// duplicate prompts. `api::confirm_exit` / `api::cancel_exit` clear it once the
/// frontend resolves the prompt.
///
/// The claim is only kept when the prompt request actually goes out: if there is
/// no main window, or the emit fails, the claimer releases it and lets the
/// close/exit proceed — a latched guard nobody can resolve would make the app
/// permanently unclosable.
#[derive(Default)]
pub struct ExitGuard(AtomicBool);

impl ExitGuard {
    /// Attempts to claim the guard (false → true). Returns whether THIS call
    /// claimed it — only the claimer should emit [`events::CLOSE_REQUESTED`].
    fn begin(&self) -> bool {
        self.0
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    /// Releases the guard so a future close/exit attempt prompts again.
    pub fn clear(&self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// The boxed future the regen driver hands the scheduler (the driver closure is a
/// distinct type per call, so it is type-erased for the scheduler's `select!`).
pub type BoxFut = Pin<Box<dyn Future<Output = Outcome> + Send>>;

/// A post-regen completion sink: called once per finished regen with the
/// [`RegenReport`] + refreshed [`DocumentProjection`]. The production wrapper
/// forwards to the Tauri event bus ([`api::emit_regen_events`], which needs an
/// [`AppHandle`]); tests inject a plain channel sink so a worker-backed regen can
/// be asserted through the **real** scheduler/driver wiring without a webview
/// (Codex MINOR-10 — the driver seam must not require an `AppHandle`).
pub type RegenEmitter = Arc<dyn Fn(&RegenReport, &DocumentProjection) + Send + Sync>;

/// Builds the app-layer regen driver over an explicit completion [`RegenEmitter`]
/// (plan "app layer runs the executor").
///
/// **Fencing goes live (R-WP11):** the driver holds the single-writer lock only
/// for the two short phases that mutate the document — phase 1
/// ([`begin_regen`](DocumentRuntime::begin_regen): compile the plan + clone a
/// scratch session) and phase 3 ([`finish_regen`](DocumentRuntime::finish_regen):
/// commit or supersede). The slow worker IO (phase 2,
/// [`PreparedRegen::drive`](crate::document_runtime::PreparedRegen::drive)) runs
/// with the lock **released**, so an edit can land during it, advance the fencing
/// tokens, and supersede the stale prepare via the executor's revision gate.
/// Debounce/coalesce/preview-priority stay in the [`RegenScheduler`] (policy only).
pub fn regen_driver_with_emitter(
    runtime: Arc<Mutex<Option<DocumentRuntime>>>,
    emit: RegenEmitter,
    autosave_tick: Arc<watch::Sender<u64>>,
) -> impl Fn(RegenDirective) -> BoxFut + Send + Sync + 'static {
    move |directive: RegenDirective| {
        let runtime = runtime.clone();
        let emit = emit.clone();
        let autosave_tick = autosave_tick.clone();
        Box::pin(async move {
            // Phase 1 (locked): compile the plan + clone the scratch session. An EMPTY
            // plan (a no-op-producing request — a rolled-back append, an already-current
            // timeline) still emits exactly ONE NoOp completion so the frontend awaiter's
            // `regen-finished{noop}` anti-hang terminal fires in production (finding 4):
            // returning silently here would strand the awaiter until its 8 s fallback.
            let prepared = {
                let mut guard = runtime.lock().await;
                let Some(rt) = guard.as_mut() else {
                    return Outcome::NoOp; // document closed while the job was queued.
                };
                match rt.begin_regen(directive.request) {
                    Some(prepared) => prepared,
                    None => {
                        let report = rt.noop_report();
                        let projection = rt.projection();
                        drop(guard);
                        emit(&report, &projection);
                        return Outcome::NoOp;
                    }
                }
            };
            // Phase 2 (UNLOCKED): drive the worker; concurrent edits may supersede.
            let driven = prepared.drive(directive.cancel).await;
            // Phase 3 (locked): commit iff still current, then emit events.
            let (report, projection) = {
                let mut guard = runtime.lock().await;
                let Some(rt) = guard.as_mut() else {
                    return Outcome::NoOp; // document closed during the worker IO.
                };
                let report = rt.finish_regen(driven);
                let projection = rt.projection();
                (report, projection)
            };
            emit(&report, &projection);
            // A published regen produced new geometry outputs worth autosaving
            // (the debounce coalesces the edit-tick + this publish-tick).
            if report.published() {
                autosave_tick.send_modify(|v| *v = v.wrapping_add(1));
            }
            report.outcome
        })
    }
}

/// The production regen driver: a thin [`regen_driver_with_emitter`] wrapper whose
/// emitter forwards each completion to the Tauri event bus
/// ([`api::emit_regen_events`]). ZERO behavior change from the pre-seam driver.
fn make_regen_driver(
    runtime: Arc<Mutex<Option<DocumentRuntime>>>,
    app: AppHandle,
    autosave_tick: Arc<watch::Sender<u64>>,
) -> impl Fn(RegenDirective) -> BoxFut + Send + Sync + 'static {
    let emit: RegenEmitter = Arc::new(
        move |report: &RegenReport, projection: &DocumentProjection| {
            api::emit_regen_events(&app, report, projection);
        },
    );
    regen_driver_with_emitter(runtime, emit, autosave_tick)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Logs go to stderr + `logs/dev.jsonl`; stdout is reserved for worker OCW1
    // frames downstream. `logging::init` is called from HERE ONLY — no test or CLI
    // may enable the file lane (see the `logging` module docs).
    let mut log_guard = logging::init();
    logging::install_panic_hook();

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .manage(ExitGuard::default())
        // Native window-close button (macOS traffic light / OS chrome): prevent the
        // immediate close and — the FIRST time while no prompt is already pending —
        // ask the frontend via CLOSE_REQUESTED instead. The frontend proceeds
        // through `confirm_exit` (see below), never through the OS's own close. The
        // close is prevented ONLY while something can still resolve the guard.
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let guard = window.state::<ExitGuard>();
                if !guard.begin() {
                    api.prevent_close(); // a prompt is already pending — keep blocking
                    return;
                }
                // SELF-HEALING: only prevent the close once the prompt request is
                // actually out. A failed emit means nothing can ever resolve the
                // guard, and a latched guard makes the window permanently
                // unclosable — release it and let the OS close proceed instead.
                match window.emit(events::CLOSE_REQUESTED, ()) {
                    Ok(()) => api.prevent_close(),
                    Err(e) => {
                        tracing::warn!("close-requested emit failed ({e}); allowing close");
                        guard.clear();
                    }
                }
            }
        })
        .setup(|app| {
            // Reclaim import-blob temp directories a previous crash left behind
            // (`ImportWorkspace::drop` never ran). Best-effort, mtime-only, no
            // daemon — see `crate::imports`.
            imports::sweep_stale_workspaces(imports::STALE_WORKSPACE_AGE);
            // Spawn the single regen scheduler over the shared runtime + app handle.
            let state = app.state::<AppState>();
            // Publish the app handle so the backend factory's worker-status
            // forwarder can emit events (the factory is built before this exists).
            let _ = state.app.set(app.handle().clone());
            let driver = make_regen_driver(
                state.runtime.clone(),
                app.handle().clone(),
                state.autosave_tick.clone(),
            );
            let (scheduler, handle) = RegenScheduler::new(driver);
            tauri::async_runtime::spawn(scheduler.run());
            let _ = state.scheduler.set(handle);
            // Spawn the debounced autosave driver over the shared runtime + app-data
            // root. Each autosave emits `events::AUTOSAVE {path, atMs}`. A headless /
            // pathless environment (no app-data dir) simply skips autosave.
            if let Some(app_data) = autosave::autosave_root(app.handle()) {
                let runtime = state.runtime.clone();
                let lane = state.persistence.clone();
                let tick = state.autosave_tick.subscribe();
                let emitter = app.handle().clone();
                tauri::async_runtime::spawn(autosave::run(
                    runtime,
                    app_data,
                    lane,
                    tick,
                    autosave::AUTOSAVE_DEBOUNCE,
                    move |ev| {
                        let _ = emitter.emit(events::AUTOSAVE, &ev);
                    },
                ));
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            api::new_document,
            api::open_document,
            api::import_step,
            api::insert_step,
            api::save_document,
            api::export_step_file,
            api::export_stl_file,
            api::export_obj_file,
            api::close_document,
            api::check_recovery,
            api::recover_document,
            api::apply_edit_command,
            api::get_operation_params,
            api::undo,
            api::redo,
            api::get_projection,
            api::get_mesh,
            api::enter_sketch,
            api::get_sketch,
            api::can_fold_transform,
            api::get_sketch_regions,
            api::sketch_upsert,
            api::begin_gesture,
            api::solve_drag,
            api::end_gesture,
            api::cancel_sketch,
            api::finish_sketch,
            api::promote_selection,
            api::face_sketch_plane,
            api::add_sketch_on_face,
            api::element_info,
            api::query_mass_properties,
            api::preview_op,
            api::resolve_refs,
            api::clear_worker_circuit,
            api::list_recents,
            api::open_file_dialog,
            api::step_file_dialog,
            api::save_file_dialog,
            api::confirm_exit,
            api::cancel_exit,
            api::log_event,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(move |handle, event| {
        // Normal termination: drop the non-blocking JSONL writer's guard so the
        // buffered tail is flushed before the process goes away. A SIGINT (Ctrl-C on
        // `tauri dev`) never reaches this arm — at most one buffer of `dev.jsonl`
        // tail is lost there, which is why the stderr layer stays unbuffered.
        // Matched FIRST and by a binding-free pattern, so `event` is not moved.
        if let RunEvent::Exit = event {
            drop(log_guard.take());
        }
        // App-level exit request (⌘Q, or the OS default "quit after last window
        // closes"): `code` is `None` for a user-triggered request and `Some(_)`
        // for a PROGRAMMATIC one (`AppHandle::exit` / `restart`). `api::confirm_exit`
        // calls `app.exit(0)`, which re-fires this event with `code: Some(0)` — the
        // `code.is_none()` gate below lets that one through unprompted, so a
        // confirmed close/quit actually terminates the app instead of looping.
        if let RunEvent::ExitRequested { api, code, .. } = event {
            if code.is_none() {
                let guard = handle.state::<ExitGuard>();
                if !guard.begin() {
                    api.prevent_exit(); // a prompt is already pending — keep blocking
                    return;
                }
                // SELF-HEALING (mirrors `on_window_event`): with no main window, or
                // a failed emit, NOTHING can ever resolve this guard. Latching it
                // would leave the app unquittable, so release it and let the exit
                // through rather than block forever on a prompt nobody can answer.
                match handle
                    .get_webview_window("main")
                    .map(|w| w.emit(events::CLOSE_REQUESTED, ()))
                {
                    Some(Ok(())) => api.prevent_exit(),
                    Some(Err(e)) => {
                        tracing::warn!("close-requested emit failed ({e}); allowing exit");
                        guard.clear();
                    }
                    None => {
                        tracing::warn!("no main window to prompt for exit; allowing exit");
                        guard.clear();
                    }
                }
            }
        }
    });
}
