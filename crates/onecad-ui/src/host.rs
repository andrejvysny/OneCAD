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
        self.app
            .document_request(crate::document_flow::Request::Save { save_as: true });
        if self.app.doc.documents.running {
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
