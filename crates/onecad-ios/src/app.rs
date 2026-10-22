//! The process's one bridge instance: egui context, the shared editor (which owns
//! the service runtime and document hosts), the GPU, the host surface and the
//! queues that carry UIKit input in and host requests out.
use crate::a11y::Tree;
use crate::ffi::{self, Status};
use crate::frame::View;
use crate::gpu::{self, Surface};
use crate::host::{Dialogs, Queue};
use crate::input::Input;
use crate::wake::Gate;
use onecad_app::worker::{EmbeddedKernelConfig, WorkerBackend};
use onecad_ui::{
    HostLayout, QaPaths,
    host::{Editor, RuntimeExit},
};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Scene image pixel cap: at the renderer's 94 B/px target estimate a full
/// 13-inch iPad drawable (5.7 Mpx) exceeds its 512 MiB GPU budget, while 3 Mpx
/// keeps targets near 270 MiB and leaves room for a replacement overlap.
const MOBILE_RENDER_PIXEL_CAP: f64 = 3_000_000.0;

/// Upper bound on flushing autosave and stopping the document hosts (terminate
/// and destroy).
pub const SHUTDOWN_GRACE: Duration = Duration::from_secs(3);
/// Upper bound on destroy waiting for the service runtime thread to exit after
/// the editor is dropped.
pub const RUNTIME_EXIT_GRACE: Duration = Duration::from_secs(3);
/// System-provided files waiting for the next frame.
const OPENS_MAX: usize = 16;

/// A failed call: the status the ABI returns and the `last_error` text.
#[derive(Debug)]
pub struct Failure {
    pub status: Status,
    pub message: String,
}

impl Failure {
    pub fn new(status: Status, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

pub type Outcome<T = ()> = Result<T, Failure>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Active,
    Inactive,
    Background,
    Terminated,
}

pub struct IosApp {
    pub(crate) ctx: egui::Context,
    pub(crate) editor: Option<Editor>,
    pub(crate) gpu: egui_wgpu::RenderState,
    pub(crate) surface: Option<Surface>,
    pub(crate) view: Option<View>,
    pub(crate) shown_document: crate::frame::ShownDocument,
    pub(crate) input: Input,
    pub(crate) a11y: Tree,
    pub(crate) host: Arc<Queue>,
    pub(crate) wake: Arc<Gate>,
    pub(crate) phase: Phase,
    pub(crate) theme: Option<egui::Theme>,
    pub(crate) keyboard_overlap: f32,
    pub(crate) capabilities: ffi::Capabilities,
    /// Strings of the request last returned by `onecad_ios_host_request_next`.
    pub(crate) request_strings: Vec<std::ffi::CString>,
    /// `onecad_ios_open_path` files, opened at the start of the next frame.
    pub(crate) opens: Vec<PathBuf>,
    /// The latest Pencil accessory action, applied at the start of the next frame.
    pub(crate) accessory: Option<onecad_interaction::touch::accessory::AccessoryAction>,
    /// Run id, pending results and injected faults of the semantic test hooks.
    #[cfg(feature = "test-hooks")]
    pub(crate) semantic: crate::test_hooks_semantic::State,
    _logging: Box<dyn std::any::Any>,
}

impl IosApp {
    pub fn create(layout: &HostLayout, wake: Arc<Gate>) -> Outcome<Self> {
        let logging: Box<dyn std::any::Any> = Box::new(onecad_app::logging::init());
        let internal = |message: String| Failure::new(Status::Internal, message);
        let paths =
            QaPaths::sandbox(layout).map_err(|e| Failure::new(Status::InvalidArgument, e))?;
        let ctx = egui::Context::default();
        let gate = wake.clone();
        ctx.set_request_repaint_callback(move |_| gate.wake());
        ctx.enable_accesskit();
        let gpu = gpu::create_render_state().map_err(internal)?;
        let host = Arc::new(Queue::new(ctx.clone()));
        let dialogs = Arc::new(Dialogs(host.clone()));
        // A journal or save fault a crash left armed must not outlive that launch.
        #[cfg(feature = "test-hooks")]
        crate::test_hooks_semantic::repair_journal(&layout.app_support);
        #[cfg(feature = "test-hooks")]
        crate::test_hooks_save::repair_save_fault(&layout.app_support);
        let backend =
            WorkerBackend::Embedded(EmbeddedKernelConfig::for_app_support(&layout.app_support));
        let mut editor =
            Editor::new(&ctx, gpu.clone(), paths, dialogs, backend).map_err(internal)?;
        editor.set_render_pixel_cap(Some(MOBILE_RENDER_PIXEL_CAP));
        Ok(Self {
            ctx,
            editor: Some(editor),
            gpu,
            surface: None,
            view: None,
            shown_document: Default::default(),
            input: Input::default(),
            a11y: Tree::default(),
            host,
            wake,
            phase: Phase::Active,
            theme: None,
            keyboard_overlap: 0.0,
            capabilities: ffi::Capabilities::default(),
            request_strings: Vec::new(),
            opens: Vec::new(),
            accessory: None,
            #[cfg(feature = "test-hooks")]
            semantic: crate::test_hooks_semantic::State::new(layout.app_support.clone()),
            _logging: logging,
        })
    }

    /// # Safety
    /// `desc.ca_metal_layer` must be a live `CAMetalLayer` the host keeps
    /// retained until `detach` or destroy.
    pub unsafe fn attach(&mut self, desc: &ffi::SurfaceDesc) -> Outcome {
        let view = view(desc, self.keyboard_overlap)?;
        if desc.ca_metal_layer.is_null() {
            return Err(Failure::new(
                Status::InvalidArgument,
                "ca_metal_layer is NULL",
            ));
        }
        if self.surface.is_some() {
            return Err(Failure::new(Status::State, "a surface is already attached"));
        }
        // SAFETY: forwarded from this function's contract.
        let surface = unsafe { Surface::new(&self.gpu, desc.ca_metal_layer, view.size_px) }
            .map_err(|e| Failure::new(Status::Internal, e))?;
        self.surface = Some(surface);
        self.view = Some(view);
        self.ctx.request_repaint();
        Ok(())
    }

    pub fn resize(&mut self, desc: &ffi::SurfaceDesc) -> Outcome {
        let view = view(desc, self.keyboard_overlap)?;
        let surface = self
            .surface
            .as_mut()
            .ok_or_else(|| Failure::new(Status::State, "no surface is attached"))?;
        if surface.size() != view.size_px {
            surface.resize(&self.gpu, view.size_px);
        }
        self.view = Some(view);
        self.ctx.request_repaint();
        Ok(())
    }

    pub fn detach(&mut self) {
        // Frames already submitted may still target the drawable; let them finish
        // before the layer can go away.
        let _ = self.gpu.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(Duration::from_secs(1)),
        });
        self.surface = None;
        self.view = None;
    }

    pub fn set_keyboard_overlap(&mut self, overlap: f32) -> Outcome {
        if !overlap.is_finite() || overlap < 0.0 {
            return Err(Failure::new(Status::InvalidArgument, "keyboard overlap"));
        }
        self.keyboard_overlap = overlap;
        if let Some(view) = &mut self.view {
            view.keyboard_overlap = overlap;
        }
        self.ctx.request_repaint();
        Ok(())
    }

    /// Input changes what the next frame shows; refused once frames are.
    pub fn accept_input(&self) -> Outcome {
        match self.phase {
            Phase::Background => Err(Failure::new(
                Status::State,
                "input is not accepted in the background",
            )),
            Phase::Terminated => Err(Failure::new(Status::State, "the app has terminated")),
            Phase::Active | Phase::Inactive => Ok(()),
        }
    }

    /// Queues a system-provided file for the next frame, where the editor's busy
    /// and tool gates apply (and GPU work such as the tab thumbnail is allowed).
    pub fn queue_open(&mut self, path: PathBuf) -> Outcome {
        if self.phase == Phase::Terminated {
            return Err(Failure::new(Status::State, "the app has terminated"));
        }
        if self.opens.len() >= OPENS_MAX {
            return Err(Failure::new(Status::Overflow, "too many pending opens"));
        }
        self.opens.push(path);
        self.ctx.request_repaint();
        Ok(())
    }

    /// Flushes autosave, then stops every document host, within `grace`.
    ///
    /// Waiting host dialogs are cancelled first: an open picker holds its
    /// document's actor inside a command, where it serves neither the flush nor
    /// the stop. The queue stays closed (terminate and destroy only).
    pub fn shutdown(&mut self, grace: Duration) {
        let deadline = Instant::now() + grace;
        self.input
            .cancel_all(onecad_interaction::touch::CancelReason::SceneInactive);
        self.host.close();
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        let flush = editor.flush_recovery();
        if !pump(editor, &self.ctx, deadline, |_| flush.done()) {
            tracing::warn!(?grace, "autosave flush unfinished at the shutdown grace");
        }
        editor.stop();
        if !pump(editor, &self.ctx, deadline, Editor::stopped) {
            tracing::warn!(
                ?grace,
                "document hosts still stopping at the shutdown grace"
            );
        }
    }

    /// Destroy: no host callback, request or GPU use survives this call. Returns
    /// the service runtime's exit when the thread outlived its grace.
    pub fn teardown(mut self) -> Option<RuntimeExit> {
        if self.phase != Phase::Terminated {
            self.shutdown(SHUTDOWN_GRACE);
        }
        self.wake.close();
        self.host.close();
        let exit = self.editor.take().and_then(Editor::close);
        self.detach();
        exit.filter(|exit| !exit.wait(RUNTIME_EXIT_GRACE))
    }
}

/// Pumps service messages until `done` or `deadline`; whether `done` held.
fn pump(
    editor: &mut Editor,
    ctx: &egui::Context,
    deadline: Instant,
    mut done: impl FnMut(&Editor) -> bool,
) -> bool {
    loop {
        if done(editor) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        editor.poll(ctx);
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn view(desc: &ffi::SurfaceDesc, keyboard_overlap: f32) -> Outcome<View> {
    let insets = desc.safe_area;
    let finite = [insets.top, insets.left, insets.bottom, insets.right]
        .iter()
        .all(|v| v.is_finite() && *v >= 0.0);
    if desc.width_px == 0
        || desc.height_px == 0
        || !desc.scale.is_finite()
        || desc.scale <= 0.0
        || !finite
    {
        return Err(Failure::new(
            Status::InvalidArgument,
            "surface needs a non-zero size, positive scale and finite insets",
        ));
    }
    Ok(View {
        size_px: [desc.width_px, desc.height_px],
        scale: desc.scale,
        safe_area: insets,
        keyboard_overlap,
    })
}
