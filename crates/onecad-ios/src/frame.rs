//! One host frame. Order (the CAD fence contract of `onecad_ui::App::frame`):
//! acquire → assemble `RawInput` → `Editor::frame` (raw-input routing fences the previous
//! frame's CAD sampling, then logic and ui; the viewport submits its CAD pass) →
//! tessellate → upload egui textures → encode the egui pass → submit →
//! present → free textures → `Editor::after_host_submit`. The host registers
//! lifetime fences immediately after submission; input routing remains an
//! idempotent fallback. A missing drawable submits pending texture uploads only.
use crate::ffi::Insets;
use egui_wgpu::{RenderState, ScreenDescriptor};
use onecad_ui::performance::{self, Stage};

/// The document the last frame ended on, so a frame that switched documents
/// is noticed after the editor pass.
#[derive(Default)]
pub struct ShownDocument(Option<onecad_ui::host::OwnerIdentity>);

impl ShownDocument {
    /// Records `tab`; whether a previous frame ended on another one.
    pub fn switched(&mut self, owner: onecad_ui::host::OwnerIdentity) -> bool {
        self.0.replace(owner).is_some_and(|before| before != owner)
    }
}

/// What the host told us about the view, in logical points.
#[derive(Clone, Copy, Debug)]
pub struct View {
    pub size_px: [u32; 2],
    pub scale: f32,
    pub safe_area: Insets,
    pub keyboard_overlap: f32,
}

pub struct Inputs {
    pub time: f64,
    pub focused: bool,
    pub theme: Option<egui::Theme>,
    pub events: Vec<egui::Event>,
}

pub fn raw_input(view: &View, gpu: &RenderState, inputs: Inputs) -> egui::RawInput {
    let scale = view.scale;
    let size = egui::vec2(view.size_px[0] as f32, view.size_px[1] as f32) / scale;
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
    let mut raw = egui::RawInput {
        screen_rect: Some(screen),
        // The keyboard covers the bottom edge like a safe-area inset would.
        safe_area_insets: Some(egui::SafeAreaInsets(egui::epaint::MarginF32 {
            left: view.safe_area.left,
            right: view.safe_area.right,
            top: view.safe_area.top,
            bottom: view.safe_area.bottom.max(view.keyboard_overlap),
        })),
        max_texture_side: Some(gpu.device.limits().max_texture_dimension_2d as usize),
        time: Some(inputs.time),
        events: inputs.events,
        focused: inputs.focused,
        system_theme: inputs.theme,
        ..Default::default()
    };
    let viewport = raw.viewports.entry(egui::ViewportId::ROOT).or_default();
    viewport.native_pixels_per_point = Some(scale);
    viewport.inner_rect = Some(screen);
    viewport.focused = Some(inputs.focused);
    raw
}

/// Uploads textures and, when a drawable is available, draws and presents.
/// Returns whether a drawable was presented.
pub fn paint(
    gpu: &RenderState,
    surface: Option<&mut crate::gpu::Surface>,
    ctx: &egui::Context,
    output: egui::FullOutput,
    acquired: crate::gpu::Acquired,
) -> bool {
    let ppp = output.pixels_per_point;
    let frame_id = ctx.cumulative_frame_nr().saturating_sub(1);
    let jobs = {
        let _span = performance::span(Stage::Tessellate, frame_id, [0; 3]);
        ctx.tessellate(output.shapes, ppp)
    };
    let mut textures = output.textures_delta;
    let mut renderer = gpu.renderer.write();
    for (id, deltas) in &textures.set {
        for delta in deltas {
            renderer.update_texture(&gpu.device, &gpu.queue, *id, delta);
        }
    }
    textures.set.clear();
    let presented =
        surface.is_some_and(|surface| draw(gpu, surface, &mut renderer, &jobs, ppp, ctx, acquired));
    if !presented {
        // Texture uploads above still need a submission to complete (as eframe does).
        gpu.queue.submit([]);
    }
    // Taken, not borrowed: epaint's `TexturesDelta` drop asserts it was emptied.
    for id in std::mem::take(&mut textures.free) {
        renderer.free_texture(&id);
    }
    presented
}

fn draw(
    gpu: &RenderState,
    surface: &mut crate::gpu::Surface,
    renderer: &mut egui_wgpu::Renderer,
    jobs: &[egui::ClippedPrimitive],
    pixels_per_point: f32,
    ctx: &egui::Context,
    acquired: crate::gpu::Acquired,
) -> bool {
    let (frame, suboptimal) = match acquired {
        crate::gpu::Acquired::Frame(frame, suboptimal) => (frame, suboptimal),
        crate::gpu::Acquired::Skip => return false,
    };
    let frame_id = ctx.cumulative_frame_nr().saturating_sub(1);
    let clear = surface_clear_color(ctx);
    let screen = ScreenDescriptor {
        size_in_pixels: surface.size(),
        pixels_per_point,
    };
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("onecad-ios egui"),
        });
    let user = renderer.update_buffers(&gpu.device, &gpu.queue, &mut encoder, jobs, &screen);
    let view = frame
        .texture
        .create_view(&wgpu::TextureViewDescriptor::default());
    let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("onecad-ios egui pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &view,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(clear),
                store: wgpu::StoreOp::Store,
            },
            depth_slice: None,
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    // The pass is dropped before `encoder.finish`, so the encoder is not reused early.
    renderer.render(&mut pass.forget_lifetime(), jobs, &screen);
    {
        let _span = performance::span(Stage::UiSubmit, frame_id, [jobs.len() as u64, 0, 0]);
        gpu.queue
            .submit(user.into_iter().chain(std::iter::once(encoder.finish())));
    }
    if performance::enabled() {
        gpu.queue.on_submitted_work_done(move || {
            performance::record(Stage::GpuComplete, frame_id, [0; 3])
        });
    }
    gpu.queue.present(frame);
    performance::record(
        Stage::PresentRequested,
        frame_id,
        [u64::from(suboptimal), 0, 0],
    );
    if suboptimal {
        surface.reconfigure(gpu);
    }
    true
}

fn surface_clear_color(ctx: &egui::Context) -> wgpu::Color {
    // Safe areas lie outside editor content but still belong to the drawable.
    let [r, g, b, _] = onecad_widgets::theme::look(ctx)
        .theme
        .canvas
        .to_normalized_gamma_f32();
    wgpu::Color {
        r: r.into(),
        g: g.into(),
        b: b.into(),
        a: 1.0,
    }
}

fn appearance_flags(ctx: &egui::Context) -> u32 {
    if onecad_widgets::theme::look(ctx).dark {
        crate::ffi::FRAME_DARK_APPEARANCE
    } else {
        0
    }
}

#[cfg(test)]
#[path = "frame_safe_area_tests.rs"]
mod safe_area_tests;

#[cfg(test)]
#[path = "frame_text_input_tests.rs"]
mod text_input_tests;

fn text_input_output(
    result: &mut crate::ffi::FrameResult,
    ctx: &egui::Context,
    platform: &egui::PlatformOutput,
    numeric: bool,
) {
    use crate::ffi;
    // egui omits IME output for clipped fields; the native session still owns keyboard input.
    let caret = onecad_widgets::text_session::snapshot(ctx)
        .filter(|snapshot| snapshot.enabled)
        .map(|snapshot| snapshot.caret)
        .or_else(|| {
            if onecad_widgets::text_session::enabled(ctx) {
                None
            } else {
                platform.ime.as_ref().map(|ime| ime.cursor_rect)
            }
        });
    if let Some(caret) = caret {
        result.flags |= ffi::FRAME_TEXT_INPUT_ACTIVE;
        result.text_cursor_x = caret.min.x;
        result.text_cursor_y = caret.min.y;
        result.text_cursor_h = caret.height();
        result.text_input_kind = if numeric {
            ffi::TEXT_NUMERIC_EXPRESSION
        } else {
            ffi::TEXT_GENERAL
        };
    }
}

impl crate::app::IosApp {
    /// Runs one frame; refused while backgrounded, terminated or detached.
    pub fn frame(&mut self, time: f64) -> crate::app::Outcome<crate::ffi::FrameResult> {
        use crate::app::{Failure, Phase};
        use crate::ffi::Status;
        let view = admit(self.phase, self.view, time)?;
        let frame_id = self.ctx.cumulative_frame_nr();
        let _frame_span = performance::span(
            Stage::RustFrame,
            frame_id,
            [view.size_px[0].into(), view.size_px[1].into(), 0],
        );
        self.apply_accessory();
        let editor = self
            .editor
            .as_mut()
            .ok_or_else(|| Failure::new(Status::State, "the editor has stopped"))?;
        // Drawable backpressure must precede the CAD sampling-readiness poll:
        // otherwise another UI submission can keep moving the sampling fence.
        let acquired = {
            let _span = performance::span(Stage::Acquire, frame_id, [0; 3]);
            self.surface
                .as_mut()
                .map_or(crate::gpu::Acquired::Skip, |surface| {
                    surface.acquire(&self.gpu)
                })
        };
        if !self.opens.is_empty() {
            // The active document changes under any live contact.
            self.input
                .cancel_all(onecad_interaction::touch::CancelReason::DocumentSwitch);
        }
        let mut drained = self.input.drain();
        performance::record(
            Stage::InputDrain,
            frame_id,
            [drained.events.len() as u64, u64::from(drained.again), 0],
        );
        if drained.cancelled {
            onecad_ui::host::cancel_contact(&self.ctx);
        }
        if let Some(kind) = drained.pointer_kind {
            onecad_ui::host::set_pointer_kind(&self.ctx, kind);
        }
        if let Some(double) = drained.pencil_double_tap {
            onecad_ui::host::set_pencil_double_tap(&self.ctx, double);
        }
        let clicks = drained
            .activations
            .iter()
            .filter_map(|at| self.a11y.click_target(*at));
        drained.events.extend(clicks.map(crate::a11y::click));
        for path in self.opens.drain(..) {
            editor.open_path(path);
        }
        let inputs = Inputs {
            time,
            focused: self.phase == Phase::Active,
            theme: self.theme,
            events: drained.events,
        };
        let raw = raw_input(&view, &self.gpu, inputs);
        self.wake.set_in_frame(true);
        let mut output = {
            let _span = performance::span(Stage::EditorFrame, frame_id, [0; 3]);
            editor.frame(&self.ctx, raw)
        };
        self.wake.set_in_frame(false);
        let context = editor.presentation_context(&self.ctx);
        if let Some(snapshot) = self.input.prediction_snapshot() {
            if snapshot.context == context {
                crate::predictions::append(
                    &mut output,
                    &snapshot,
                    &context,
                    (snapshot.contact, self.input.contact_context().1),
                    &editor.prediction_clips(&self.ctx),
                    editor.accent_color(),
                );
            } else {
                self.input.clear_predictions();
            }
        }
        let switched = self.left_document();
        Ok(self.finish(output, drained.again || switched, acquired))
    }

    /// A pass that changed the active document (tab strip, ⌃Tab, a new tab)
    /// cancels every live contact: it belonged to the document that was left,
    /// and must not release or drag on the one now shown. The next frame
    /// forgets egui's press and carries `PointerGone`.
    fn left_document(&mut self) -> bool {
        let Some(owner) = self
            .editor
            .as_ref()
            .map(|editor| editor.presentation_context(&self.ctx).owner)
        else {
            return false;
        };
        if !self.shown_document.switched(owner) {
            return false;
        }
        self.input
            .cancel_all(onecad_interaction::touch::CancelReason::DocumentSwitch);
        true
    }

    fn finish(
        &mut self,
        mut output: egui::FullOutput,
        again: bool,
        acquired: crate::gpu::Acquired,
    ) -> crate::ffi::FrameResult {
        use crate::ffi;
        let platform = std::mem::take(&mut output.platform_output);
        let delay = output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .map_or(std::time::Duration::MAX, |viewport| viewport.repaint_delay);
        let presented = paint(
            &self.gpu,
            self.surface.as_mut(),
            &self.ctx,
            output,
            acquired,
        );
        if let Some(editor) = &mut self.editor {
            editor.after_host_submit(&self.ctx);
        }
        let (repaint_after_s, flags) = schedule(again, presented, self.surface.is_some(), delay);
        let flags = flags | appearance_flags(&self.ctx);
        let mut result = ffi::FrameResult {
            struct_size: std::mem::size_of::<ffi::FrameResult>() as u32,
            repaint_after_s,
            flags,
            ..Default::default()
        };
        let numeric = self
            .editor
            .as_ref()
            .is_some_and(|e| e.numeric_entry(&self.ctx));
        text_input_output(&mut result, &self.ctx, &platform, numeric);
        if let Some(update) = platform.accesskit_update
            && self.a11y.update(update)
        {
            result.flags |= ffi::FRAME_A11Y_CHANGED;
        }
        self.queue_outputs(platform.commands);
        if self.host.has_pending() {
            result.flags |= ffi::FRAME_HOST_REQUESTS;
        }
        result
    }

    /// Clipboard/URL commands and finished exports become host requests.
    fn queue_outputs(&mut self, commands: Vec<egui::OutputCommand>) {
        use crate::{ffi, host::Request};
        let mut requests = Vec::new();
        for command in commands {
            match command {
                egui::OutputCommand::CopyText(text) => requests.push((ffi::REQ_COPY_TEXT, text)),
                egui::OutputCommand::OpenUrl(url) => requests.push((ffi::REQ_OPEN_URL, url.url)),
                _ => {}
            }
        }
        let requests = requests.into_iter().map(|(kind, text)| Request {
            kind,
            text,
            ..Default::default()
        });
        let exported = self
            .editor
            .as_mut()
            .map(|e| e.take_exported())
            .unwrap_or_default();
        let exports = exported.into_iter().map(|path| Request {
            kind: ffi::REQ_EXPORT_FILE,
            suggested_name: path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            path: path.to_string_lossy().into_owned(),
            ..Default::default()
        });
        for request in requests.chain(exports) {
            if self.host.push(request, None).is_none() {
                tracing::warn!("host request queue full; output request dropped");
            }
        }
    }
}

/// Frames run only in the foreground with an attached (non-empty) surface.
pub(crate) fn admit(
    phase: crate::app::Phase,
    view: Option<View>,
    time: f64,
) -> crate::app::Outcome<View> {
    use crate::app::{Failure, Phase};
    use crate::ffi::Status;
    if matches!(phase, Phase::Background | Phase::Terminated) {
        return Err(Failure::new(Status::State, "no frames in the background"));
    }
    if !time.is_finite() {
        return Err(Failure::new(
            Status::InvalidArgument,
            "time_s is not finite",
        ));
    }
    view.ok_or_else(|| Failure::new(Status::State, "no surface is attached"))
}

/// `repaint_after_s` and the frame flags it implies. Rendering is on demand: an idle
/// editor (egui's repaint delay unbounded, nothing queued, drawable presented) asks for
/// no frame (-1), so the host's display link stays paused until `wake` or input.
pub(crate) fn schedule(
    again: bool,
    presented: bool,
    surface: bool,
    delay: std::time::Duration,
) -> (f64, u32) {
    use crate::ffi;
    // A skipped drawable (outdated, lost, timeout) leaves a stale or wrong-size image
    // on screen; draw again rather than wait for the next input.
    let retry = !presented && surface;
    let after = if again || retry { 0.0 } else { seconds(delay) };
    let mut flags = 0;
    if after == 0.0 {
        flags |= ffi::FRAME_REDRAW_REQUESTED;
    }
    if presented {
        flags |= ffi::FRAME_PRESENTED;
    }
    (after, flags)
}

fn seconds(delay: std::time::Duration) -> f64 {
    if delay == std::time::Duration::MAX {
        -1.0
    } else {
        delay.as_secs_f64()
    }
}

#[cfg(test)]
mod tests {
    use super::ShownDocument;

    #[test]
    fn only_a_change_of_the_shown_document_is_a_switch() {
        let owner = |tab| onecad_ui::host::OwnerIdentity {
            tab,
            editor_visible: true,
        };
        let mut shown = ShownDocument::default();
        assert!(
            !shown.switched(owner(1)),
            "the first frame switches nothing"
        );
        assert!(!shown.switched(owner(1)));
        assert!(shown.switched(owner(2)), "tab strip, ⌃Tab or a new tab");
        assert!(!shown.switched(owner(2)));
        assert!(shown.switched(owner(1)), "and back");
        let home = onecad_ui::host::OwnerIdentity {
            editor_visible: false,
            ..owner(1)
        };
        assert!(shown.switched(home), "Home leaves the same tab's editor");
        assert!(!shown.switched(home), "repeated Home is idempotent");
        assert!(shown.switched(owner(1)));
    }
}
