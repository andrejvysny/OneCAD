//! The editor as seen by a host that owns its own window, GPU and event loop (the
//! iPadOS bridge). It is the same `App` eframe runs on desktop; only construction
//! and the frame call differ, and the editor's internals stay crate-private.
use crate::{app::App, qa::QaPaths};
use onecad_app::host::DialogPort;
use std::sync::Arc;

pub use crate::display_budget::{DisplayBudget, Ledger, MemoryRelief};
pub use crate::viewport_widgets::PointerKind;

pub struct Editor {
    app: App,
}

impl Editor {
    /// `gpu` must be the device the host presents with: the CAD renderer shares it.
    pub fn new(
        ctx: &egui::Context,
        gpu: egui_wgpu::RenderState,
        paths: QaPaths,
        dialogs: Arc<dyn DialogPort>,
        backend: onecad_app::worker::WorkerBackend,
        profile: onecad_render::RenderProfile,
    ) -> Result<Self, String> {
        App::new_with(ctx, gpu, paths, dialogs, backend, profile).map(|app| Self { app })
    }

    /// See [`App::frame`]: submit and present the output before the next call.
    pub fn frame(&mut self, ctx: &egui::Context, raw: egui::RawInput) -> egui::FullOutput {
        self.app.frame(ctx, raw)
    }

    /// Drains service messages outside a frame (shutdown pumping).
    pub fn poll(&mut self, ctx: &egui::Context) {
        self.app.poll(ctx);
    }

    /// Asks every open document's autosave to write its pending recovery now;
    /// the returned flush reports when every one has finished.
    pub fn flush_recovery(&self) -> RecoveryFlush {
        RecoveryFlush(self.app.request_recovery_flush())
    }

    /// Memory warning: drops parked tabs' caches (rebuilt on demand) and lowers the scene
    /// scale one step when the active document is over its display allowance.
    pub fn relieve_memory_pressure(&mut self) -> MemoryRelief {
        self.app.relieve_memory_pressure()
    }

    /// Backgrounded: releases the renderer's targets after the last frame finished; the
    /// next frame recreates them. `false` when the GPU was still busy after 1 s.
    pub fn release_render_targets(&mut self, ctx: &egui::Context) -> bool {
        self.app.release_render_targets(ctx)
    }

    /// The scene image scale relative to native pixels the viewport last used.
    pub fn scene_scale(&self) -> f64 {
        self.app.scene_scale
    }

    /// Whether the renderer holds targets (released in the background).
    pub fn has_render_targets(&self) -> bool {
        self.app.renderer.has_targets()
    }

    /// One accounting of the window's display memory.
    pub fn display_ledger(&self) -> (Ledger, DisplayBudget) {
        (self.app.display_ledger(), self.app.display_budget)
    }

    /// Starts shutting every document host down; see [`Editor::stopped`].
    pub fn stop(&mut self) {
        self.app.stop();
    }

    /// Every document host has stopped after [`Editor::stop`].
    pub fn stopped(&self) -> bool {
        self.app.may_close
    }

    /// Drops the editor. Its service runtime thread then finishes its actors and
    /// stops; the returned exit observes that (`None` if already taken).
    pub fn close(self) -> Option<RuntimeExit> {
        let exit = self.app.tabs.runtime().take_exit().map(RuntimeExit);
        drop(self);
        exit
    }

    /// Files written by Export since the last call, for the host to publish.
    pub fn take_exported(&mut self) -> Vec<std::path::PathBuf> {
        #[cfg(not(feature = "desktop"))]
        {
            std::mem::take(&mut self.app.exported)
        }
        #[cfg(feature = "desktop")]
        {
            Vec::new()
        }
    }

    /// Opens a file the system handed to the app, like File › Open Recent: the same
    /// busy/tool gates apply and a refusal is reported in the status bar.
    pub fn open_path(&mut self, path: std::path::PathBuf) {
        self.app
            .document_request(crate::document_flow::Request::Open(Some(path)));
    }

    /// INPUT.md `drag_authoring`: see [`drag_authoring`].
    pub fn drag_authoring(&self) -> bool {
        drag_authoring(&self.app)
    }

    /// Pencil accessory "switch to previous tool": `false` when nothing changed.
    pub fn toggle_previous_sketch_tool(&mut self) -> bool {
        crate::tool_palette::toggle_previous(&mut self.app)
    }

    /// Pencil accessory "show palette": the adaptive tool palette opens at `at`
    /// (points) on the next frame.
    pub fn open_tool_palette(&mut self, at: egui::Pos2) {
        crate::tool_palette::open(&mut self.app, at);
    }

    /// The active document's application state (acceptance probes only).
    /// `None` until its document host has started, and after it stopped.
    #[cfg(feature = "semantic-probe")]
    pub fn document_access(&self) -> Option<DocumentAccess> {
        self.app.doc.bridge.shared.probe.lock().ok()?.clone()
    }

    /// Document tabs that hold a document (acceptance probes only).
    #[cfg(feature = "semantic-probe")]
    pub fn open_tab_count(&self) -> usize {
        self.app
            .tab_summaries()
            .iter()
            .filter(|tab| tab.open)
            .count()
    }

    /// Starts the active document's File › Save As (acceptance probes only):
    /// the document flow, services and `DialogPort` a user's Save As goes
    /// through. `Err` is the status line when the editor refused to start it.
    #[cfg(feature = "semantic-probe")]
    pub fn request_save_as(&mut self) -> Result<(), String> {
        use crate::diagnostics::{FileAction, Source, State};
        self.app
            .document_request(crate::document_flow::Request::Save { save_as: true });
        // `running` alone may be another file action's job, which refused this
        // Save; only a Save that just went pending is ours.
        let pending = matches!(
            self.app
                .doc
                .diagnostics
                .latest(Source::File(FileAction::Save)),
            Some((State::Pending, _))
        );
        if self.app.doc.documents.running && pending {
            Ok(())
        } else {
            Err(self.app.doc.status.clone())
        }
    }

    /// The last Save the active document reported (acceptance probes only):
    /// `None` while one runs or none finished; else its status line, `Ok` with
    /// whether the document went clean when the file was written.
    #[cfg(feature = "semantic-probe")]
    pub fn save_outcome(&self) -> Option<Result<(bool, String), String>> {
        use crate::diagnostics::{FileAction, Source, State};
        if self.app.doc.documents.running {
            return None;
        }
        let latest = self
            .app
            .doc
            .diagnostics
            .latest(Source::File(FileAction::Save));
        match latest? {
            (State::Pending, _) => None,
            (State::Succeeded, message) => Some(Ok((true, message))),
            // Saved, but an edit landed during the write (`saved.clean` false).
            (State::Warning, message) => Some(Ok((false, message))),
            (_, message) => Some(Err(message)),
        }
    }

    /// A dimension or numeric-expression editor owns the keyboard.
    pub fn numeric_entry(&self, ctx: &egui::Context) -> bool {
        crate::sketch_numeric::editor_open(ctx) || crate::constraint_authoring::editor_open(ctx)
    }
}

/// A dragged finger can author because a sketch creation tool is armed, a
/// sketch handle drag runs, or a model tool shows on-canvas handles (extrude
/// distance, move/rotate triad). Otherwise, including model tools without
/// handles, fingers orbit.
pub(crate) fn drag_authoring(app: &App) -> bool {
    let sketch_tool = app.doc.sketch.session.as_ref().is_some_and(|session| {
        app.sketch_active() && session.tool != onecad_interaction::sketch::SketchTool::Select
    });
    let handles = !app.sketch_active()
        && (app.doc.extrude_handle.active()
            || app.doc.transform_gizmo.gizmo.active()
            || crate::extrude_handle::available(app)
            || crate::transform_ui::triad(app).is_some());
    sketch_tool || app.doc.sketch_gesture.busy() || handles
}

/// Cancels a touch contact whose press egui has already seen, without leaving
/// the active tool: egui forgets the press and any drag, so no release, click or
/// drag-stop follows. Call it between frames; the next frame should carry
/// `PointerGone`, which the editor's canvas gestures treat as a cancellation.
pub fn cancel_contact(ctx: &egui::Context) {
    ctx.input_mut_for(egui::ViewportId::ROOT, |input| {
        input.pointer = Default::default();
    });
    ctx.stop_dragging();
}

/// INPUT.md `over_control` between frames: `pos` (points) is over chrome or
/// a control rather than the bare CAD canvas of the last completed pass.
pub fn over_control(ctx: &egui::Context, pos: egui::Pos2) -> bool {
    crate::viewport_widgets::over_control(ctx, pos)
}

/// Records which input kind presses next, so picking uses its edge aperture
/// (a fingertip covers far more than the 8 points a mouse needs). Set it
/// before the frame that carries the press; desktop never calls it.
pub fn set_pointer_kind(ctx: &egui::Context, kind: PointerKind) {
    crate::viewport_widgets::set_pointer_kind(ctx, kind);
}

/// Whether the press the next frame carries is a Pencil tip double tap
/// (`tap_count` 2), which picks like a double click (face to body). Set it with
/// every press; desktop never calls it.
pub fn set_pencil_double_tap(ctx: &egui::Context, double: bool) {
    crate::viewport_widgets::set_pencil_double_tap(ctx, double);
}

/// Recovery flushes requested by [`Editor::flush_recovery`].
pub struct RecoveryFlush(Vec<(Arc<crate::bridge::Shared>, u64)>);

impl RecoveryFlush {
    /// Every document's actor has finished a flush since the request.
    pub fn done(&self) -> bool {
        self.0.iter().all(|(shared, before)| {
            shared
                .recovery_flushes
                .load(std::sync::atomic::Ordering::Acquire)
                > *before
        })
    }
}

/// The editor's service runtime thread, observed after [`Editor::close`].
pub struct RuntimeExit(std::sync::mpsc::Receiver<()>);

impl RuntimeExit {
    /// Waits at most `timeout`; `true` once the runtime thread has exited.
    pub fn wait(&self, timeout: std::time::Duration) -> bool {
        matches!(
            self.0.recv_timeout(timeout),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
        )
    }
}

/// One document host's services, as the acceptance probes of the iPadOS
/// bridge see them: the same `AppState` and host context the editor's
/// document actor calls, and the runtime that actor runs on.
#[cfg(feature = "semantic-probe")]
#[derive(Clone)]
pub struct DocumentAccess {
    pub state: Arc<onecad_app::state::AppState>,
    pub host: onecad_app::host::HostContext,
    /// The sandbox Documents directory saves and exports are confined to.
    pub documents: std::path::PathBuf,
    pub runtime: tokio::runtime::Handle,
}
