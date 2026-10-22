//! The editor as seen by a host that owns its own window, GPU and event loop (the
//! iPadOS bridge). It is the same `App` eframe runs on desktop; only construction
//! and the frame call differ, and the editor's internals stay crate-private.
use crate::{app::App, qa::QaPaths};
use onecad_app::host::DialogPort;
use std::sync::Arc;

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
    ) -> Result<Self, String> {
        App::new_with(ctx, gpu, paths, dialogs, backend).map(|app| Self { app })
    }

    /// Caps the viewport image at `pixels` physical pixels (see
    /// [`App::render_pixel_cap`]); `None` renders at the display scale.
    pub fn set_render_pixel_cap(&mut self, pixels: Option<f64>) {
        self.app.render_pixel_cap = pixels;
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

    /// Memory pressure: drops caches that are rebuilt on demand.
    pub fn release_caches(&mut self) {
        self.app.release_parked_thumbnails();
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

    /// A dimension or numeric-expression editor owns the keyboard.
    pub fn numeric_entry(&self, ctx: &egui::Context) -> bool {
        crate::sketch_numeric::editor_open(ctx) || crate::constraint_authoring::editor_open(ctx)
    }
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
