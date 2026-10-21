//! One host frame. Order (the CAD fence contract of `onecad_ui::App::frame`):
//! assemble `RawInput` → `Editor::frame` (raw-input routing fences the previous
//! frame's CAD sampling, then logic and ui; the viewport submits its CAD pass) →
//! upload egui textures → tessellate → acquire → encode the egui pass → submit →
//! present → free textures. Exactly one egui pass is submitted per frame, and it
//! is submitted before the next frame's routing registers its fence.
use crate::ffi::Insets;
use egui_wgpu::{RenderState, ScreenDescriptor};

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
) -> bool {
    let ppp = output.pixels_per_point;
    let jobs = ctx.tessellate(output.shapes, ppp);
    let mut textures = output.textures_delta;
    let mut renderer = gpu.renderer.write();
    for (id, deltas) in &textures.set {
        for delta in deltas {
            renderer.update_texture(&gpu.device, &gpu.queue, *id, delta);
        }
    }
    textures.set.clear();
    let presented = surface.is_some_and(|surface| draw(gpu, surface, &mut renderer, &jobs, ppp));
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
) -> bool {
    let (frame, suboptimal) = match surface.acquire(gpu) {
        crate::gpu::Acquired::Frame(frame, suboptimal) => (frame, suboptimal),
        crate::gpu::Acquired::Skip => return false,
    };
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
    // eframe's default clear colour; the editor paints every pixel over it.
    let [r, g, b, a] =
        egui::Color32::from_rgba_unmultiplied(12, 12, 12, 180).to_normalized_gamma_f32();
    let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("onecad-ios egui pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &view,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color {
                    r: r.into(),
                    g: g.into(),
                    b: b.into(),
                    a: a.into(),
                }),
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
    gpu.queue
        .submit(user.into_iter().chain(std::iter::once(encoder.finish())));
    gpu.queue.present(frame);
    if suboptimal {
        surface.reconfigure(gpu);
    }
    true
}

impl crate::app::IosApp {
    /// Runs one frame; refused while backgrounded, terminated or detached.
    pub fn frame(&mut self, time: f64) -> crate::app::Outcome<crate::ffi::FrameResult> {
        use crate::app::{Failure, Phase};
        use crate::ffi::Status;
        if matches!(self.phase, Phase::Background | Phase::Terminated) {
            return Err(Failure::new(Status::State, "no frames in the background"));
        }
        if !time.is_finite() {
            return Err(Failure::new(
                Status::InvalidArgument,
                "time_s is not finite",
            ));
        }
        let view = self
            .view
            .ok_or_else(|| Failure::new(Status::State, "no surface is attached"))?;
        let editor = self
            .editor
            .as_mut()
            .ok_or_else(|| Failure::new(Status::State, "the editor has stopped"))?;
        let drained = self.input.drain();
        if drained.cancelled {
            onecad_ui::host::cancel_contact(&self.ctx);
        }
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
        let output = editor.frame(&self.ctx, raw);
        self.wake.set_in_frame(false);
        Ok(self.finish(output, drained.again))
    }

    fn finish(&mut self, mut output: egui::FullOutput, again: bool) -> crate::ffi::FrameResult {
        use crate::ffi;
        let platform = std::mem::take(&mut output.platform_output);
        let delay = output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .map_or(std::time::Duration::MAX, |viewport| viewport.repaint_delay);
        let presented = paint(&self.gpu, self.surface.as_mut(), &self.ctx, output);
        // A skipped drawable (outdated, lost, timeout) leaves a stale or wrong-size
        // image on screen; draw again rather than wait for the next input.
        let retry = !presented && self.surface.is_some();
        let mut result = ffi::FrameResult {
            struct_size: std::mem::size_of::<ffi::FrameResult>() as u32,
            repaint_after_s: if again || retry { 0.0 } else { seconds(delay) },
            ..Default::default()
        };
        if result.repaint_after_s == 0.0 {
            result.flags |= ffi::FRAME_REDRAW_REQUESTED;
        }
        if presented {
            result.flags |= ffi::FRAME_PRESENTED;
        }
        if let Some(ime) = &platform.ime {
            result.flags |= ffi::FRAME_TEXT_INPUT_ACTIVE;
            result.text_cursor_x = ime.cursor_rect.min.x;
            result.text_cursor_y = ime.cursor_rect.min.y;
            result.text_cursor_h = ime.cursor_rect.height();
            let numeric = self
                .editor
                .as_ref()
                .is_some_and(|e| e.numeric_entry(&self.ctx));
            result.text_input_kind = if numeric {
                ffi::TEXT_NUMERIC_EXPRESSION
            } else {
                ffi::TEXT_GENERAL
            };
        }
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

fn seconds(delay: std::time::Duration) -> f64 {
    if delay == std::time::Duration::MAX {
        -1.0
    } else {
        delay.as_secs_f64()
    }
}
