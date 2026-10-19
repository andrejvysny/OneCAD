use crate::{limits, MemoryEstimate, RenderError, RenderOptions, Renderer};

pub(crate) struct RetainedOutput {
    pub _texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub bytes: u64,
}

impl RenderError {
    pub fn recoverable_target_refusal(&self) -> bool {
        matches!(self, Self::Budget(_) | Self::Allocation(_))
    }
}

impl Renderer {
    /// Retry a target replacement after ALL host sampling and CAD fences have drained.
    /// Retains the previous output image, but retires intermediate attachments before
    /// allocating. A failed retry leaves that image available; no new capture is drawable.
    pub fn resize_reclaiming(&mut self, size: [u32; 2]) -> Result<(), RenderError> {
        self.require_idle()?;
        self.reclaim_targets(size, self.options)?;
        self.replace_targets(size, self.options)
    }

    /// Same retirement contract as `resize_reclaiming`; options publish only on success.
    pub fn set_options_reclaiming(&mut self, options: RenderOptions) -> Result<(), RenderError> {
        self.validate_options(options)?;
        if options.section != self.options.section {
            return Err(RenderError::CaptureMismatch);
        }
        self.require_idle()?;
        let size = self.size;
        self.reclaim_targets(size, options)?;
        self.replace_targets(size, options)?;
        self.options = options;
        Ok(())
    }

    fn reclaim_targets(
        &mut self,
        size: [u32; 2],
        options: RenderOptions,
    ) -> Result<(), RenderError> {
        self.limits
            .validate_size(options.render_size(size)?, &self.device)?;
        let bytes = self.retained_output.as_ref().map_or_else(
            || {
                if self.targets.is_some() {
                    u64::from(self.size[0]) * u64::from(self.size[1]) * 4
                } else {
                    0
                }
            },
            |output| output.bytes,
        );
        let next = MemoryEstimate {
            target_bytes: limits::quality_target_bytes(size, options)?,
            cpu_prepared_bytes: 0,
            upload_bytes: 0,
            ..self.estimate
        };
        self.limits.check(
            next,
            MemoryEstimate {
                target_bytes: bytes,
                ..Default::default()
            },
        )?;
        if let Some(targets) = self.targets.take() {
            self.retained_output = Some(RetainedOutput {
                _texture: targets._output,
                view: targets.output_view,
                bytes,
            });
            self.estimate.target_bytes = bytes;
        }
        Ok(())
    }
}
