//! GPU publication of synthetic section contours.
use crate::{limits::add, PreparedSectionContours, RenderError, Renderer, SectionContourKey};
use wgpu::util::DeviceExt;

pub(crate) struct GpuSectionContours {
    buffer: wgpu::Buffer,
    count: u32,
    key: SectionContourKey,
}
impl GpuSectionContours {
    fn bytes(&self) -> u64 {
        self.buffer.size()
    }
}
impl Renderer {
    pub fn replace_prepared_scene_with_section_contours(
        &mut self,
        scene: crate::PreparedScene,
        contours: PreparedSectionContours,
    ) -> Result<(), RenderError> {
        self.require_idle()?;
        crate::options::validate_section_bodies(self.options, scene.body_count())?;
        let (identity, bounds, source) = (scene.identity, scene.bounds, scene.source_bytes);
        let estimate = self.scene_contour_estimate(&scene, &contours, None)?;
        let (bodies, _) = self.upload_prepared(scene, false)?;
        let contours = self.upload_section_contours(contours);
        self.publish_scene(bodies, estimate, identity, bounds, source);
        self.section_contours[0] = Some(contours);
        Ok(())
    }
    pub fn replace_prepared_preview_with_section_contours(
        &mut self,
        identity: crate::PreviewIdentity,
        scene: crate::PreparedScene,
        contours: PreparedSectionContours,
    ) -> Result<(), RenderError> {
        self.require_idle()?;
        if Some(identity.base) != self.scene_identity
            || scene.identity != identity.base
            || self
                .latest_preview_generation
                .is_some_and(|last| identity.generation <= last)
        {
            return Err(RenderError::CaptureMismatch);
        }
        let source = scene.source_bytes;
        let estimate = self.scene_contour_estimate(&scene, &contours, Some(identity))?;
        let (bodies, _) = self.upload_prepared(scene, true)?;
        let contours = self.upload_section_contours(contours);
        self.publish_preview(bodies, estimate, identity, source);
        self.section_contours[1] = Some(contours);
        Ok(())
    }
    fn scene_contour_estimate(
        &self,
        scene: &crate::PreparedScene,
        contours: &PreparedSectionContours,
        preview: Option<crate::PreviewIdentity>,
    ) -> Result<crate::MemoryEstimate, RenderError> {
        if contours.key().scene != scene.identity
            || contours.key().preview != preview
            || contours.key().section != self.options.section.map(f32::to_bits)
        {
            return Err(RenderError::CaptureMismatch);
        }
        let bytes = (contours.segments().len() as u64 * 24).max(64);
        if bytes > self.device.limits().max_buffer_size
            || contours.segments().len() > u32::MAX as usize
        {
            return Err(RenderError::Budget("section buffer"));
        }
        let incoming = self.scene_contour_memory(scene, contours, preview.is_some(), bytes)?;
        self.limits.check(
            incoming,
            crate::MemoryEstimate {
                geometry_bytes: self.estimate.geometry_bytes,
                source_bytes: self.estimate.source_bytes,
                resident_metadata_bytes: self.estimate.resident_metadata_bytes,
                ..Default::default()
            },
        )
    }
    fn scene_contour_memory(
        &self,
        scene: &crate::PreparedScene,
        contours: &PreparedSectionContours,
        preview: bool,
        bytes: u64,
    ) -> Result<crate::MemoryEstimate, RenderError> {
        let (retained, source, count, metadata) = if preview {
            (
                self.committed
                    .iter()
                    .map(crate::mesh::gpu_bytes)
                    .sum::<u64>()
                    + self.highlight_gpu_bytes()
                    + self.section_contour_gpu_bytes(false),
                self.committed_source_bytes,
                self.committed.len(),
                self.highlight_metadata_bytes() + self.section_contour_metadata_bytes(false),
            )
        } else {
            (0, 0, 0, 0)
        };
        Ok(crate::MemoryEstimate {
            source_bytes: add(scene.source_bytes, source)?,
            geometry_bytes: add(
                add(add(scene.gpu_bytes, retained)?, bytes)?,
                std::mem::size_of::<crate::FrameU>() as u64,
            )?,
            cpu_prepared_bytes: add(scene.cpu_bytes, contours.cpu_bytes())?,
            upload_bytes: add(scene.gpu_bytes, bytes)?,
            resident_metadata_bytes: (scene.body_count() + count) as u64
                * std::mem::size_of::<crate::mesh::GpuBody>() as u64
                + std::mem::size_of::<GpuSectionContours>() as u64
                + metadata,
            target_bytes: self.estimate.target_bytes,
            ibl_bytes: self.estimate.ibl_bytes,
            peak_total_bytes: 0,
        })
    }
    /// Publish a section plane and its synthetic contours together. Rejection preserves
    /// the prior options and buffers; all replacement overlap is checked before upload.
    pub fn set_options_with_section_contours(
        &mut self,
        options: crate::RenderOptions,
        committed: PreparedSectionContours,
        preview: Option<PreparedSectionContours>,
    ) -> Result<(), RenderError> {
        self.require_idle()?;
        self.validate_options(options)?;
        let plane = options.section.map(f32::to_bits);
        if Some(committed.key.scene) != self.scene_identity
            || committed.key.preview.is_some()
            || committed.key.section != plane
            || preview.as_ref().map(|p| p.key.preview) != self.preview_identity.map(Some)
            || preview
                .as_ref()
                .is_some_and(|p| Some(p.key.scene) != self.scene_identity || p.key.section != plane)
        {
            return Err(RenderError::CaptureMismatch);
        }
        let prepared = [Some(committed), preview];
        let estimate = self.contour_replacement_estimate(options, &prepared)?;
        let uploaded = prepared.map(|p| p.map(|p| self.upload_section_contours(p)));
        self.apply_options(options)?;
        self.section_contours = uploaded;
        self.estimate = estimate;
        Ok(())
    }
    fn contour_replacement_estimate(
        &self,
        options: crate::RenderOptions,
        prepared: &[Option<PreparedSectionContours>; 2],
    ) -> Result<crate::MemoryEstimate, RenderError> {
        let bytes = prepared.iter().flatten().try_fold(0, |total, p| {
            add(total, (p.segments().len() as u64 * 24).max(64))
        })?;
        let cpu = prepared
            .iter()
            .flatten()
            .try_fold(0, |total, p| add(total, p.cpu_bytes()))?;
        let old = self
            .section_contours
            .iter()
            .flatten()
            .map(GpuSectionContours::bytes)
            .sum::<u64>();
        let estimate = self.limits.check(
            crate::MemoryEstimate {
                geometry_bytes: add(self.estimate.geometry_bytes.saturating_sub(old), bytes)?,
                resident_metadata_bytes: self.estimate.resident_metadata_bytes
                    - self.section_contour_metadata_bytes(false)
                    - self.section_contour_metadata_bytes(true)
                    + prepared.iter().flatten().count() as u64
                        * std::mem::size_of::<GpuSectionContours>() as u64,
                target_bytes: crate::limits::quality_target_bytes(self.size, options)?,
                cpu_prepared_bytes: cpu,
                upload_bytes: bytes,
                ..self.estimate
            },
            crate::MemoryEstimate {
                geometry_bytes: old,
                resident_metadata_bytes: self.section_contour_metadata_bytes(false)
                    + self.section_contour_metadata_bytes(true),
                target_bytes: if options.ssaa != self.options.ssaa {
                    self.estimate.target_bytes
                } else {
                    0
                },
                ..Default::default()
            },
        )?;
        for p in prepared.iter().flatten() {
            if (p.segments().len() as u64 * 24).max(64) > self.device.limits().max_buffer_size
                || p.segments().len() > u32::MAX as usize
            {
                return Err(RenderError::Budget("section buffer"));
            }
        }
        Ok(estimate)
    }
    fn upload_section_contours(&self, prepared: PreparedSectionContours) -> GpuSectionContours {
        let data = bytemuck::cast_slice(prepared.segments());
        let buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("synthetic section contours"),
                contents: if data.is_empty() { &[0; 64] } else { data },
                usage: wgpu::BufferUsages::VERTEX,
            });
        GpuSectionContours {
            buffer,
            count: prepared.segments().len() as u32,
            key: prepared.key,
        }
    }
    pub(crate) fn section_contour_gpu_bytes(&self, preview: bool) -> u64 {
        self.section_contours[usize::from(preview)]
            .as_ref()
            .map_or(0, GpuSectionContours::bytes)
    }
    pub(crate) fn section_contour_metadata_bytes(&self, preview: bool) -> u64 {
        if self.section_contours[usize::from(preview)].is_some() {
            std::mem::size_of::<GpuSectionContours>() as u64
        } else {
            0
        }
    }
    pub(crate) fn clear_preview_section_contours(&mut self) {
        let bytes = self.section_contour_gpu_bytes(true);
        self.estimate.resident_metadata_bytes -= self.section_contour_metadata_bytes(true);
        self.section_contours[1] = None;
        self.estimate.geometry_bytes = self.estimate.geometry_bytes.saturating_sub(bytes);
    }
    pub fn replace_prepared_section_contours(
        &mut self,
        prepared: PreparedSectionContours,
    ) -> Result<(), RenderError> {
        self.require_idle()?;
        let key = prepared.key();
        if Some(key.scene) != self.scene_identity
            || key.section != self.options.section.map(f32::to_bits)
            || (key.preview.is_some() && key.preview != self.preview_identity)
        {
            return Err(RenderError::CaptureMismatch);
        }
        let index = usize::from(key.preview.is_some());
        let old = self.section_contours[index]
            .as_ref()
            .map_or(0, GpuSectionContours::bytes);
        let size = (prepared.segments().len() as u64 * 24).max(64);
        let next = crate::MemoryEstimate {
            geometry_bytes: add(self.estimate.geometry_bytes.saturating_sub(old), size)?,
            resident_metadata_bytes: self.estimate.resident_metadata_bytes
                - self.section_contour_metadata_bytes(key.preview.is_some())
                + std::mem::size_of::<GpuSectionContours>() as u64,
            cpu_prepared_bytes: prepared.cpu_bytes(),
            upload_bytes: size,
            ..self.estimate
        };
        let estimate = self.limits.check(
            next,
            crate::MemoryEstimate {
                geometry_bytes: old,
                resident_metadata_bytes: self.section_contour_metadata_bytes(key.preview.is_some()),
                ..Default::default()
            },
        )?;
        if size > self.device.limits().max_buffer_size
            || prepared.segments().len() > u32::MAX as usize
        {
            return Err(RenderError::Budget("section buffer"));
        }
        self.section_contours[index] = Some(self.upload_section_contours(prepared));
        self.estimate = estimate;
        Ok(())
    }
    pub(crate) fn clear_section_contours(&mut self) {
        let bytes = self
            .section_contours
            .iter()
            .flatten()
            .map(GpuSectionContours::bytes)
            .sum::<u64>();
        self.estimate.resident_metadata_bytes -=
            self.section_contour_metadata_bytes(false) + self.section_contour_metadata_bytes(true);
        self.section_contours = [None, None];
        self.estimate.geometry_bytes = self.estimate.geometry_bytes.saturating_sub(bytes);
    }
    pub(crate) fn draw_section_contours(&self, pass: &mut wgpu::RenderPass<'_>) {
        if self.options.display_mode == crate::DisplayMode::Shaded {
            return;
        }
        pass.set_pipeline(&self.pipelines.edge);
        pass.set_bind_group(0, &self.frame_bind, &[]);
        for contours in self.section_contours.iter().flatten() {
            if Some(contours.key.scene) == self.scene_identity
                && contours.key.section == self.options.section.map(f32::to_bits)
                && (contours.key.preview.is_none() || contours.key.preview == self.preview_identity)
            {
                pass.set_vertex_buffer(0, contours.buffer.slice(..));
                pass.draw(0..4, 0..contours.count);
            }
        }
    }
}
