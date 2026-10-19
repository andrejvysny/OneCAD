use crate::{limits, RenderError, Renderer};

/// Authoritative visibility contract for the shared native CAD renderer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DisplayMode {
    Wireframe,
    Shaded,
    #[default]
    ShadedEdges,
    Xray,
}

impl DisplayMode {
    pub(crate) fn studio(self) -> bool {
        matches!(self, Self::Shaded | Self::ShadedEdges)
    }
}

/// OneCAD display settings. Colors are linear RGB; section retains n·p <= d.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderOptions {
    pub section: [f32; 4],
    pub cap: [f32; 4],
    pub display_mode: DisplayMode,
    pub xray_opacity: f32,
    pub xray_hidden_edge_opacity: f32,
    pub ssaa: f32,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            section: [0.0; 4],
            cap: [0.7, 0.55, 0.3, 1.0],
            display_mode: DisplayMode::ShadedEdges,
            xray_opacity: 0.25,
            xray_hidden_edge_opacity: 0.22,
            ssaa: 1.0,
        }
    }
}

impl RenderOptions {
    pub(crate) fn validate(self) -> Result<(), RenderError> {
        if !self.section.iter().chain(&self.cap).all(|v| v.is_finite())
            || !self.cap.iter().all(|v| (0.0..=1.0).contains(v))
            || !self.xray_opacity.is_finite()
            || !self.xray_hidden_edge_opacity.is_finite()
            || !(0.0..=1.0).contains(&self.xray_opacity)
            || !(0.0..=1.0).contains(&self.xray_hidden_edge_opacity)
            || ![1.0, 2.0].contains(&self.ssaa)
        {
            return Err(RenderError::Mesh("invalid display options"));
        }
        Ok(())
    }

    pub(crate) fn render_size(self, size: [u32; 2]) -> Result<[u32; 2], RenderError> {
        let factor = self.ssaa as u32;
        Ok([
            size[0]
                .checked_mul(factor)
                .ok_or(RenderError::Budget("target dimension"))?,
            size[1]
                .checked_mul(factor)
                .ok_or(RenderError::Budget("target dimension"))?,
        ])
    }
}

/// Host and headless tests must use the same optional depth capability negotiation.
pub fn required_device_features(available: wgpu::Features) -> wgpu::Features {
    available & (wgpu::Features::DEPTH32FLOAT_STENCIL8 | wgpu::Features::FLOAT32_BLENDABLE)
}

pub(crate) fn depth_format(device: &wgpu::Device) -> wgpu::TextureFormat {
    if device
        .features()
        .contains(wgpu::Features::DEPTH32FLOAT_STENCIL8)
    {
        wgpu::TextureFormat::Depth32FloatStencil8
    } else {
        wgpu::TextureFormat::Depth24PlusStencil8
    }
}

impl Renderer {
    pub fn options(&self) -> &RenderOptions {
        &self.options
    }

    pub fn set_options(&mut self, options: RenderOptions) -> Result<(), RenderError> {
        self.validate_options(options)?;
        if options.section != self.options.section
            && options.section[..3].iter().any(|v| *v != 0.0)
            && !self.committed.is_empty()
        {
            return Err(RenderError::CaptureMismatch);
        }
        self.apply_options(options)
    }
    pub(crate) fn validate_options(&self, options: RenderOptions) -> Result<(), RenderError> {
        options.validate()?;
        if options.display_mode == DisplayMode::Xray
            && !self
                .device
                .features()
                .contains(wgpu::Features::FLOAT32_BLENDABLE)
        {
            return Err(RenderError::Device(
                "Xray requires FLOAT32_BLENDABLE for bounded order-independent accumulation".into(),
            ));
        }
        validate_section_bodies(options, self.committed.len())?;
        validate_section_bodies(options, self.preview.len())?;
        Ok(())
    }
    pub(crate) fn apply_options(&mut self, options: RenderOptions) -> Result<(), RenderError> {
        self.require_idle()?;
        if options.ssaa != self.options.ssaa {
            self.replace_targets(self.size, options)?;
        }
        self.contact_dirty |= options.section != self.options.section
            || options.display_mode != self.options.display_mode;
        if options.section != self.options.section {
            self.clear_section_contours();
        }
        self.options = options;
        Ok(())
    }

    pub(crate) fn replace_targets(
        &mut self,
        size: [u32; 2],
        options: RenderOptions,
    ) -> Result<(), RenderError> {
        self.limits
            .validate_size(options.render_size(size)?, &self.device)?;
        let estimate = self.limits.check(
            crate::MemoryEstimate {
                target_bytes: limits::quality_target_bytes(size, options)?,
                cpu_prepared_bytes: 0,
                upload_bytes: 0,
                ..self.estimate
            },
            crate::MemoryEstimate {
                target_bytes: self.estimate.target_bytes,
                ..Default::default()
            },
        )?;
        let gpu = crate::gpu::Gpu {
            device: &self.device,
            queue: &self.queue,
        };
        // Publish only after allocation and validation succeeded. The old output stays valid.
        let targets = if size.contains(&0) {
            None
        } else {
            Some(crate::targets::Targets::try_new(
                &gpu,
                &self.pipelines,
                &self.ibl,
                size,
                options,
            )?)
        };
        self.targets = targets;
        self.retained_output = None;
        self.size = size;
        self.estimate = estimate;
        self.contact_dirty = true;
        Ok(())
    }
}

// The stencil mask uses an eight-bit winding count. Refuse unsupported overlap depth
// instead of silently wrapping 256 closed solids back to an empty section.
pub(crate) fn validate_section_bodies(
    options: RenderOptions,
    count: usize,
) -> Result<(), RenderError> {
    if options.section[..3].iter().any(|v| *v != 0.0) && count > 255 {
        return Err(RenderError::Budget("section winding body count"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_winding_limit_refuses_wrapping_without_changing_options() {
        let section = RenderOptions {
            section: [0.0, 0.0, 1.0, 0.0],
            ..Default::default()
        };
        assert!(validate_section_bodies(section, 255).is_ok());
        assert!(matches!(
            validate_section_bodies(section, 256),
            Err(RenderError::Budget("section winding body count"))
        ));
        assert!(validate_section_bodies(RenderOptions::default(), 4096).is_ok());
    }
}
