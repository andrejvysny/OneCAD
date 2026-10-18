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
pub mod library;
#[cfg(desktop)]
pub mod menu;
#[cfg(feature = "tauri-e2e")]
mod tauri_e2e;

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{Emitter, Manager, RunEvent, WindowEvent};

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

pub use onecad_app::regen_driver::*;
pub use onecad_app::{
    autosave, document_runtime, dto, error, events, export, export_threemf, imports,
    library_ingest, library_seed, library_seed_templates, logging, mesh_cache, recents,
    sketch_projection, state, worker,
};
mod host;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Logs go to stderr + `logs/dev.jsonl`; stdout is reserved for worker OCW1
    // frames downstream. `logging::init` is called from HERE ONLY — no test or CLI
    // may enable the file lane (see the `logging` module docs).
    let mut log_guard = logging::init();
    logging::install_panic_hook();

    let builder = tauri::Builder::default().plugin(tauri_plugin_dialog::init());
    #[cfg(feature = "tauri-e2e")]
    let builder = builder
        .plugin(tauri_plugin_wdio::init())
        .plugin(tauri_plugin_wdio_webdriver::init());

    let mut context = tauri::generate_context!();
    // tauri-utils 2.9.3 emits Vec<u8> for this array-valued configuration field.
    if context.config().identifier == "com.andrejvysny.onecad.nativeqa" {
        for window in &mut context.config_mut().app.windows {
            window.data_store_identifier = Some([
                164, 19, 209, 62, 77, 87, 74, 132, 156, 40, 98, 22, 35, 16, 19, 1,
            ]);
        }
    }
    let app = builder
        .manage(AppState::default())
        .manage(ExitGuard::default())
        // Native window-close button (macOS traffic light / OS chrome): prevent the
        // immediate close and — the FIRST time while no prompt is already pending —
        // ask the frontend via CLOSE_REQUESTED instead. The frontend proceeds
        // through `confirm_exit` (see below), never through the OS's own close. The
        // close is prevented ONLY while something can still resolve the guard.
        .on_window_event(|window, event| {
            // Losing focus is the cheapest reliable "the user stepped away" signal
            // there is, and stepping away is exactly when the machine gets closed,
            // suspended or killed. Flush immediately rather than waiting out the
            // debounce. Throttled by `claim_blur_flush`; a clean document writes
            // nothing (the dirty gate in `autosave_current`).
            if let WindowEvent::Focused(false) = event {
                let app = window.app_handle().clone();
                tauri::async_runtime::spawn(async move {
                    let state = app.state::<AppState>();
                    if !state.claim_blur_flush() {
                        return;
                    }
                    let Some(app_data) = autosave::autosave_root(&host::context(&app)) else {
                        return;
                    };
                    if let Some(ev) = autosave::autosave_current_reporting(
                        &state.runtime,
                        &app_data,
                        &state.persistence,
                    )
                    .await
                    {
                        let _ = app.emit(events::AUTOSAVE, &ev);
                    }
                });
                return;
            }
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
            // The native menu (WP-U1 / D-3). Installed HERE because `setup` runs on
            // the main thread, which is the only thread AppKit lets a menu be built
            // on. A failure must not abort startup: without a menu the ⌘-chords in
            // `useShortcuts` still reach every one of the same verbs, so the app
            // degrades rather than refuses to open.
            #[cfg(desktop)]
            if let Err(e) = menu::install(app.handle()) {
                tracing::warn!("native menu install failed ({e}); keyboard chords still apply");
            }
            let state = app.state::<AppState>();
            tauri::async_runtime::block_on(async { state.start(host::context(app.handle())) })
                .map_err(std::io::Error::other)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            api::new_document,
            api::open_document,
            api::import_step,
            api::insert_step,
            api::import_project,
            api::save_document,
            api::export_step_file,
            api::export_stl_file,
            api::export_obj_file,
            api::export_3mf_file,
            api::close_document,
            api::check_recovery,
            api::recover_document,
            api::apply_edit_command,
            api::apply_operation,
            api::rollback_failed_operation,
            api::repair_mate_axis,
            api::get_operation_params,
            api::feature_dependencies,
            api::undo,
            api::get_module_state,
            api::set_module_state,
            api::list_document_modules,
            api::list_variables,
            api::upsert_variable,
            api::upsert_variable_expr,
            api::rename_variable,
            api::evaluate_expression,
            api::remove_variable,
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
            api::cancel_gesture,
            api::cancel_sketch,
            api::finish_sketch,
            api::promote_selection,
            api::face_sketch_plane,
            api::add_sketch_on_face,
            api::project_to_sketch,
            api::update_projection,
            api::detach_projection,
            api::element_info,
            api::query_mass_properties,
            api::classify_element,
            api::query_body_topology,
            library::list_library_components,
            library::reindex_library,
            library::resolve_component_source,
            library::place_component,
            library::set_component_params,
            library::replace_component,
            library::save_as_component,
            library::list_templates,
            library::save_as_template,
            library::new_from_template,
            library::component_preview_mesh,
            library::component_upgrade_available,
            library::detach_component,
            api::prepare_offset_face,
            api::prepare_edge_op,
            api::analyze_edge_op_range,
            api::preview_op,
            api::resolve_refs,
            api::clear_worker_circuit,
            api::list_recents,
            api::rename_recent_project,
            api::delete_recent_project,
            api::reveal_in_file_manager,
            api::import_file_dialog,
            api::open_file_dialog,
            api::step_file_dialog,
            api::pick_component_files,
            api::ingest_components,
            api::save_file_dialog,
            api::confirm_exit,
            api::cancel_exit,
            api::log_event,
            api::mesh_render_completed,
            #[cfg(feature = "tauri-e2e")]
            tauri_e2e::composition_status,
            #[cfg(feature = "tauri-e2e")]
            tauri_e2e::agent_identity,
            #[cfg(feature = "tauri-e2e")]
            tauri_e2e::agent_status,
        ])
        .build(context)
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
