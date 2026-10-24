use crate::{frame, gpu::Gpu, highlights::HighlightTone, BodyStyle, FrameU, RenderError, Renderer};
use onecad_scene::ViewSnapshot;

/// Upload work of the last successful CAD encode. Refused or suspended frames leave it intact.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameStats {
    pub uniform_writes: u32,
    pub uniform_upload_bytes: u64,
}

pub(crate) struct FrameBinding {
    pub buffer: wgpu::Buffer,
    pub bind: wgpu::BindGroup,
}
impl FrameBinding {
    fn new(gpu: &Gpu<'_>, layout: &wgpu::BindGroupLayout, label: &str) -> Self {
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: std::mem::size_of::<FrameU>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind = gpu.bind(label, layout, &[buffer.as_entire_binding()]);
        Self { buffer, bind }
    }
}

pub(crate) struct FrameBindings {
    pub normal: FrameBinding,
    selected: FrameBinding,
    preview: FrameBinding,
}
impl FrameBindings {
    pub fn new(gpu: &Gpu<'_>, layout: &wgpu::BindGroupLayout) -> Self {
        Self {
            normal: FrameBinding::new(gpu, layout, "CAD normal frame"),
            selected: FrameBinding::new(gpu, layout, "CAD selected frame"),
            preview: FrameBinding::new(gpu, layout, "CAD preview frame"),
        }
    }
    pub fn body(&self, style: BodyStyle, preview: bool) -> &FrameBinding {
        if preview {
            &self.preview
        } else if style.selected {
            &self.selected
        } else {
            &self.normal
        }
    }
}

pub(crate) struct FrameUniforms {
    pub normal: FrameU,
    pub selected: FrameU,
    pub preview: FrameU,
    pub highlights: Option<[FrameU; 2]>,
}
impl Renderer {
    pub fn frame_stats(&self) -> FrameStats {
        self.frame_stats
    }
    pub(crate) fn frame_upload_stats(&self) -> FrameStats {
        let writes = 1
            + u32::from(self.committed.iter().any(|body| body.style.selected))
            + u32::from(!self.preview.is_empty())
            + if self.highlights.is_some() { 2 } else { 0 };
        FrameStats {
            uniform_writes: writes,
            uniform_upload_bytes: u64::from(writes) * std::mem::size_of::<FrameU>() as u64,
        }
    }
    pub(crate) fn frame_uniforms(
        &self,
        view: &ViewSnapshot,
        style: frame::RenderStyle,
    ) -> Result<FrameUniforms, RenderError> {
        let body = |selected, preview| {
            frame::uniform(
                view,
                self.committed_bounds,
                style,
                BodyStyle { selected },
                preview,
            )
            .map(|frame| frame::with_options(frame, self.options, self.profile))
        };
        let normal = body(false, false)?;
        let selected = body(true, false)?;
        let mut preview = body(false, true)?;
        preview.display[2] = frame::PREVIEW_HIDDEN_EDGE_ALPHA;
        let highlights = if self.highlights.is_some() {
            let highlight = |tone| {
                frame::highlight_uniform(normal, view, style, tone)
                    .map(|frame| frame::with_options(frame, self.options, self.profile))
            };
            Some([
                highlight(HighlightTone::Selected)?,
                highlight(HighlightTone::Hover)?,
            ])
        } else {
            None
        };
        Ok(FrameUniforms {
            normal,
            selected,
            preview,
            highlights,
        })
    }
    pub(crate) fn upload_frame_uniforms(&self, frames: &FrameUniforms) -> FrameStats {
        let mut stats = FrameStats::default();
        let mut upload = |buffer: &wgpu::Buffer, frame: &FrameU| {
            self.queue
                .write_buffer(buffer, 0, bytemuck::bytes_of(frame));
            stats.uniform_writes += 1;
            stats.uniform_upload_bytes += std::mem::size_of::<FrameU>() as u64;
        };
        upload(&self.frame_bindings.normal.buffer, &frames.normal);
        if self.committed.iter().any(|body| body.style.selected) {
            upload(&self.frame_bindings.selected.buffer, &frames.selected);
        }
        if !self.preview.is_empty() {
            upload(&self.frame_bindings.preview.buffer, &frames.preview);
        }
        if let (Some(highlights), Some(frames)) = (&self.highlights, &frames.highlights) {
            for (buffer, frame) in highlights.uniforms.iter().zip(frames) {
                upload(buffer, frame);
            }
        }
        stats
    }
}
