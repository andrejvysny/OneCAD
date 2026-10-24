//! The editor as seen by a host that owns its own window, GPU and event loop (the
//! iPadOS bridge). It is the same `App` eframe runs on desktop; only construction
//! and the frame call differ, and the editor's internals stay crate-private.
use crate::{app::App, qa::QaPaths};
use onecad_app::host::DialogPort;
use std::sync::Arc;

#[path = "host_presentation.rs"]
mod presentation;
#[cfg(all(test, feature = "desktop"))]
#[path = "host_submit_tests.rs"]
mod submit_tests;
pub use presentation::{OwnerIdentity, PresentationContext};

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
        let scope = presentation::text_scope(&self.app);
        onecad_widgets::text_session::begin_frame(ctx, scope.clone());
        if onecad_widgets::text_session::enabled(ctx) {
            self.app.composing = onecad_widgets::text_session::composing(ctx);
        }
        let output = self.app.frame(ctx, raw);
        onecad_widgets::text_session::end_frame(ctx);
        if scope != presentation::text_scope(&self.app) {
            onecad_widgets::text_session::invalidate(ctx);
        }
        output
    }

    /// Call once after the host submits this frame's UI pass (including CAD image sampling).
    /// Completion then wakes the next display refresh; the desktop input-hook fallback remains.
    pub fn after_host_submit(&mut self, ctx: &egui::Context) {
        crate::input::fence_previous_frame(&mut self.app, ctx);
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
    /// busy/tool gates apply and a refusal is reported in the status bar. A STEP
    /// file is imported into a new document, as File › Import would (the host
    /// declares STEP as a type it can open).
    pub fn open_path(&mut self, path: std::path::PathBuf) {
        use crate::document_flow::Request;
        let step = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("step") || ext.eq_ignore_ascii_case("stp"));
        self.app.document_request(if step {
            Request::Import(Some(path))
        } else {
            Request::Open(Some(path))
        });
    }

    /// The active document's tab id. A host compares it across frames to cancel
    /// live touch contacts when the user switched documents.
    pub fn active_tab(&self) -> u64 {
        self.app.doc.tab
    }

    /// Identity and viewport of the actual editor presentation, for transient host ink.
    pub fn presentation_context(&self, ctx: &egui::Context) -> PresentationContext {
        presentation::capture(&self.app, ctx)
    }

    pub fn prediction_clips(&self, ctx: &egui::Context) -> Vec<egui::Rect> {
        presentation::clips(ctx)
    }

    pub fn accent_color(&self) -> egui::Color32 {
        self.app.theme.accent
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
    pub fn open_tool_palette(&mut self, at: egui::Pos2) -> bool {
        if self.app.ui.start_open || self.app.busy() || !at.is_finite() {
            return false;
        }
        crate::tool_palette::open(&mut self.app, at)
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

    /// The active tab's editor-side state (acceptance probes only), which the
    /// document services do not hold: strip position, selection, camera,
    /// status line and whether it is busy.
    #[cfg(feature = "semantic-probe")]
    pub fn view_probe(&self) -> ViewProbe {
        let camera = &self.app.doc.camera;
        ViewProbe {
            tab_index: self.app.active_index(),
            selected: self.app.doc.selection.len(),
            camera_target: camera.target.to_array(),
            camera_offset: camera.offset.to_array(),
            status: self.app.doc.status.clone(),
            busy: self.app.busy(),
        }
    }

    /// Starts the active document's File › Save As (acceptance probes only):
    /// the document flow, services and `DialogPort` a user's Save As goes
    /// through. `Ok` is the tab that owns the save; `Err` is the status line
    /// when the editor refused to start it.
    #[cfg(feature = "semantic-probe")]
    pub fn request_save_as(&mut self) -> Result<u64, String> {
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
            Ok(self.app.doc.tab)
        } else {
            Err(self.app.doc.status.clone())
        }
    }

    /// The last Save `tab` reported (acceptance probes only), whichever tab is
    /// shown: `None` while one runs or none finished; else its status line,
    /// `Ok` with whether the document went clean when the file was written.
    #[cfg(feature = "semantic-probe")]
    pub fn save_outcome(&self, tab: u64) -> Option<Result<(bool, String), String>> {
        use crate::diagnostics::{FileAction, Source, State};
        let Some(doc) = self.app.editor(tab) else {
            return Some(Err("the document closed before its Save finished".into()));
        };
        if doc.documents.running {
            return None;
        }
        match doc.diagnostics.latest(Source::File(FileAction::Save))? {
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

/// A dragged finger can author because a sketch session is active, a
/// sketch handle drag runs, or a model tool shows on-canvas handles (extrude
/// distance, move/rotate triad). Otherwise, including model tools without
/// handles, fingers orbit.
pub(crate) fn drag_authoring(app: &App) -> bool {
    if app.ui.start_open {
        return false;
    }
    let sketch_tool = app.sketch_active() && app.doc.sketch.session.is_some();
    let handles = !app.sketch_active()
        && (app.doc.extrude_handle.active()
            || app.doc.transform_gizmo.gizmo.active()
            || app.doc.model_handle.active()
            || crate::model_handle::available(app)
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
pub struct RecoveryFlush(Vec<crate::recovery_flush::Ticket>);
pub use crate::recovery_flush::{RecoveryResult, Status as RecoveryStatus};

impl RecoveryFlush {
    pub fn results(&self) -> Vec<RecoveryResult> {
        self.0
            .iter()
            .map(crate::recovery_flush::Ticket::snapshot)
            .collect()
    }

    /// Completion includes failure; inspect results before claiming durability.
    pub fn done(&self) -> bool {
        self.results()
            .iter()
            .all(|result| result.outcome != RecoveryStatus::Pending)
    }

    pub fn finish_waiting(&self, expired: bool) {
        for ticket in &self.0 {
            ticket.finish_waiting(expired);
        }
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

/// See [`Editor::view_probe`].
#[cfg(feature = "semantic-probe")]
#[derive(Clone, Debug, PartialEq)]
pub struct ViewProbe {
    /// 0-based position of the active tab in the strip.
    pub tab_index: usize,
    pub selected: usize,
    pub camera_target: [f64; 3],
    pub camera_offset: [f64; 3],
    pub status: String,
    /// A file action, command or decision blocks the editor (tab switches too).
    pub busy: bool,
}

/// Routing, services or title-bar actions may switch owner before fields render.
pub(crate) fn refresh_text_scope(app: &App, ctx: &egui::Context) {
    onecad_widgets::text_session::refresh_scope(ctx, presentation::text_scope(app));
}
