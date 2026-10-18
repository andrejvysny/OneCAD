use super::*;
use solidcraft_render::{AnalysisColors, AnalysisMode};

/// Validated draw settings; geometry and identity always remain OneCAD-owned.
#[derive(Clone)]
pub struct SolidCraftOptions {
    pub section: [f32; 4],
    pub cap: [f32; 4],
    pub analysis: Option<AnalysisMode>,
    pub outline: bool,
    pub hidden_edges: bool,
    pub opacity: f32,
    pub ssaa: f32,
    pub surface: Option<solidcraft_render::surface::OpaqueSurface>,
    pub images: Vec<view::GpuImage>,
    pub access: Option<view::AccessMap>,
}
impl Default for SolidCraftOptions {
    fn default() -> Self {
        Self {
            section: [0.0; 4],
            cap: [0.7, 0.55, 0.3, 1.0],
            analysis: None,
            outline: true,
            hidden_edges: false,
            opacity: 1.0,
            ssaa: 1.0,
            surface: None,
            images: Vec::new(),
            access: None,
        }
    }
}
impl SolidCraftOptions {
    pub(super) fn validate(
        &self,
        limits: RenderLimits,
        device: &wgpu::Device,
    ) -> Result<u64, RenderError> {
        if !self.section.iter().chain(&self.cap).all(|n| n.is_finite())
            || !self.cap.iter().all(|n| (0.0..=1.0).contains(n))
            || !self.opacity.is_finite()
            || !(0.0..=1.0).contains(&self.opacity)
            || !self.ssaa.is_finite()
            || !(1.0..=2.0).contains(&self.ssaa)
        {
            return Err(RenderError::Mesh("invalid donor display options"));
        }
        let bytes = add(
            self.validate_images(limits, device)?,
            self.validate_access(limits, device)?,
        )?;
        let analysis = view::analysis_uniform(self.analysis, self.access.as_ref());
        if !analysis.iter().all(|v| v.is_finite()) {
            return Err(RenderError::Mesh("invalid donor analysis"));
        }
        if matches!(self.analysis, Some(AnalysisMode::Access { .. })) && self.access.is_none() {
            return Err(RenderError::Mesh(
                "accessibility analysis requires a prepared access map",
            ));
        }
        Ok(bytes)
    }
    fn validate_images(
        &self,
        limits: RenderLimits,
        device: &wgpu::Device,
    ) -> Result<u64, RenderError> {
        if self.images.len() > limits.max_bodies {
            return Err(RenderError::Budget("reference image count"));
        }
        let mut bytes = 0;
        for (index, image) in self.images.iter().enumerate() {
            limits.validate_size(image.size, device)?;
            let pixels = mul(mul(u64::from(image.size[0]), u64::from(image.size[1]))?, 4)?;
            if image.size.contains(&0)
                || image.rgba.len() as u64 != pixels
                || !image.opacity.is_finite()
                || !(0.0..=1.0).contains(&image.opacity)
                || !image.corners.iter().flatten().all(|v| v.is_finite())
                || self.images[..index].iter().any(|old| old.id == image.id)
            {
                return Err(RenderError::Mesh("invalid donor reference image"));
            }
            bytes = add(bytes, add(pixels, 144)?)?;
        }
        Ok(bytes)
    }
    fn validate_access(
        &self,
        limits: RenderLimits,
        device: &wgpu::Device,
    ) -> Result<u64, RenderError> {
        let mut bytes = 0;
        if let Some(access) = &self.access {
            limits.validate_size([access.size; 2], device)?;
            let pixels = mul(u64::from(access.size), u64::from(access.size))?;
            if access.version == 0
                || access.size == 0
                || access.depth.len() as u64 != pixels
                || !access.scale.is_finite()
                || access.scale <= 0.0
                || !access.u.iter().chain(&access.v).all(|v| v.is_finite())
                || !access
                    .depth
                    .iter()
                    .all(|v| v.is_finite() || *v == f32::NEG_INFINITY)
            {
                return Err(RenderError::Mesh("invalid donor access map"));
            }
            bytes = add(bytes, mul(pixels, 4)?)?;
        }
        Ok(bytes)
    }
}
impl SolidCraftViewport {
    pub fn options(&self) -> &SolidCraftOptions {
        &self.options
    }
    pub fn set_options(&mut self, options: SolidCraftOptions) -> Result<(), RenderError> {
        self.require_idle()?;
        let image_bytes = self.options_payload_bytes(&options)?;
        self.limits.validate_size(
            self.size
                .map(|v| (v as f64 * f64::from(options.ssaa)).ceil() as u32),
            &self.device,
        )?;
        let estimate = self.check_options_budget(image_bytes, options.ssaa)?;
        let model = self
            .scene
            .as_ref()
            .map(|s| model(s, &options))
            .transpose()?;
        let preview = self.preview.as_ref().map(preview).transpose()?;
        if let Some(model) = model {
            self.put_model(model)?;
        }
        if let Some(preview) = preview {
            self.set_slot(false, preview)?;
        }
        self.access_highwater_bytes = self.access_highwater_bytes.max(
            options
                .access
                .as_ref()
                .map_or(0, |access| access.depth.len() as u64 * 4),
        );
        self.options = options;
        self.estimate = estimate;
        self.refresh_accounting()
    }
    fn check_options_budget(
        &self,
        image_bytes: u64,
        ssaa: f32,
    ) -> Result<MemoryEstimate, RenderError> {
        let geometry = add(
            self.scene.as_ref().map_or(0, |s| s.bytes),
            self.preview.as_ref().map_or(0, |s| s.bytes),
        )?;
        let target = supersample_bytes(self.size, ssaa)?.max(self.estimate.target_bytes);
        self.limits.check(
            MemoryEstimate {
                geometry_bytes: add(add(mul(geometry, 3)?, image_bytes)?, self.highlight_bytes)?,
                cpu_prepared_bytes: add(
                    add(mul(geometry, 4)?, image_bytes)?,
                    add(self.highlight_bytes, self.retired_highlight_bytes)?,
                )?,
                resident_metadata_bytes: add(
                    add(mul(geometry, 2)?, image_bytes)?,
                    self.highlight_bytes,
                )?,
                upload_bytes: add(geometry, image_bytes)?,
                target_bytes: target,
                source_bytes: self.estimate.source_bytes,
                ..Default::default()
            },
            self.estimate,
        )
    }
}
pub(super) fn supersample_bytes(size: [u32; 2], factor: f32) -> Result<u64, RenderError> {
    let base = super::target_bytes(size)?;
    if factor == 1.0 {
        return Ok(base);
    }
    let size = size.map(|v| (v as f64 * f64::from(factor)).ceil() as u64);
    add(base, mul(mul(size[0], size[1])?, 52)?)
}

pub(super) fn empty() -> Frame {
    let slot = || Arc::new(Mutex::new(Some(GpuScene::default())));
    Frame {
        product: false,
        key: 0,
        slot: slot(),
        sketch_key: 0,
        sketch_slot: slot(),
        hl_key: 0,
        hl_slot: slot(),
        pv_key: 0,
        pv_slot: slot(),
        view_proj: [[0.0; 4]; 4],
        back: [0.0, 0.0, 1.0],
        size_px: [1.0; 2],
        clip: [0.0; 4],
        cap: [0.7, 0.5, 0.3, 1.0],
        images: Vec::new(),
        analysis: [0.0; 8],
        analysis_colors: view::analysis_colors(&AnalysisColors::DEFAULT),
        access: None,
        outline: true,
        eye: [0.0; 4],
        ssaa: 1.0,
    }
}
pub(super) fn update(
    frame: &mut Frame,
    view: &ViewSnapshot,
    size: [u32; 2],
    options: &SolidCraftOptions,
) -> Result<(), RenderError> {
    let viewport = view.viewport();
    if [
        (viewport.width * viewport.scale_factor).round() as u32,
        (viewport.height * viewport.scale_factor).round() as u32,
    ] != size
    {
        return Err(RenderError::CaptureMismatch);
    }
    let matrix = view.view_projection().as_mat4().to_cols_array_2d();
    let eye = view.eye().as_vec3().to_array();
    let back = view.basis().back.as_vec3().to_array();
    if !matrix
        .iter()
        .flatten()
        .chain(&eye)
        .chain(&back)
        .all(|n| n.is_finite())
    {
        return Err(RenderError::Mesh("unrepresentable donor camera"));
    }
    frame.view_proj = matrix;
    frame.back = back;
    frame.eye = [
        eye[0],
        eye[1],
        eye[2],
        if view.camera().projection == onecad_scene::Projection::Perspective {
            1.0
        } else {
            0.0
        },
    ];
    frame.size_px = size.map(|v| v as f32);
    frame.clip = options.section;
    frame.cap = options.cap;
    frame.outline = options.outline;
    frame.ssaa = options.ssaa;
    frame.analysis = view::analysis_uniform(options.analysis, options.access.as_ref());
    frame.access.clone_from(&options.access);
    frame.images.clone_from(&options.images);
    Ok(())
}
pub(super) fn model(
    scene: &PreparedSolidCraftScene,
    options: &SolidCraftOptions,
) -> Result<GpuScene, RenderError> {
    let mut out = GpuScene::default();
    grid(&mut out, scene.bounds);
    for body in &scene.bodies {
        out.tris
            .try_reserve_exact(body.scene.tris.len())
            .map_err(|_| RenderError::Budget("donor model"))?;
        out.lines
            .try_reserve_exact(body.scene.lines.len())
            .map_err(|_| RenderError::Budget("donor edges"))?;
        out.tris.extend_from_slice(&body.scene.tris);
        out.lines.extend_from_slice(&body.scene.lines);
    }
    if options.hidden_edges {
        out.hidden = out.lines.clone();
    }
    if options.opacity < 1.0 {
        for vertex in out.tris.chunks_exact_mut(view::TRI_SIZE) {
            vertex[27] = (options.opacity * 255.0).round() as u8;
        }
        out.glass = std::mem::take(&mut out.tris);
    } else if let Some(surface) = options.surface {
        out.pbr
            .try_reserve_exact(out.tris.len() / view::TRI_SIZE * view::PBR_SIZE)
            .map_err(|_| RenderError::Budget("donor PBR"))?;
        let tris = std::mem::take(&mut out.tris);
        for vertex in tris.chunks_exact(view::TRI_SIZE) {
            let mut values = [0.0; 6];
            for (value, bytes) in values.iter_mut().zip(vertex.chunks_exact(4)) {
                *value = f32::from_le_bytes(
                    bytes
                        .try_into()
                        .map_err(|_| RenderError::Mesh("donor vertex"))?,
                );
            }
            out.pbr_tri(
                [values[0], values[1], values[2]],
                [values[3], values[4], values[5]],
                surface,
            );
        }
    }
    Ok(out)
}

pub(super) fn preview(scene: &PreparedSolidCraftScene) -> Result<GpuScene, RenderError> {
    let mut gpu = model(scene, &SolidCraftOptions::default())?;
    for vertex in gpu.tris.chunks_exact_mut(view::TRI_SIZE) {
        vertex[24..28].copy_from_slice(&[75, 170, 235, 170]);
    }
    gpu.trans = std::mem::take(&mut gpu.tris);
    gpu.under.clear();
    gpu.guide_lines.clear();
    Ok(gpu)
}

fn grid(scene: &mut GpuScene, bounds: Option<onecad_scene::Bounds>) {
    let extent = bounds
        .map_or(100.0, |b| (b.max - b.min).max_element().max(1.0))
        .min(f64::from(f32::MAX) / 1000.0);
    let step = 10_f64.powf((extent / 10.0).log10().floor());
    let center = bounds.map_or(onecad_scene::DVec3::ZERO, |b| (b.min + b.max) * 0.5);
    if center.abs().max_element() > f64::from(f32::MAX) / 4.0 {
        return;
    }
    let cx = (center.x / step).round() * step;
    let cy = (center.y / step).round() * step;
    let span = step * 50.0;
    for index in -50..=50 {
        let color = if index % 10 == 0 {
            [80, 88, 102, 130]
        } else {
            [62, 70, 85, 75]
        };
        let x = (cx + f64::from(index) * step) as f32;
        let y = (cy + f64::from(index) * step) as f32;
        scene.under_line(
            [x, (cy - span) as f32, 0.0],
            [x, (cy + span) as f32, 0.0],
            color,
            1.0,
        );
        scene.under_line(
            [(cx - span) as f32, y, 0.0],
            [(cx + span) as f32, y, 0.0],
            color,
            1.0,
        );
    }
    scene.guide_line(
        [(cx - span) as f32, 0.0, 0.0],
        [(cx + span) as f32, 0.0, 0.0],
        [190, 65, 65, 190],
        1.0,
    );
    scene.guide_line(
        [0.0, (cy - span) as f32, 0.0],
        [0.0, (cy + span) as f32, 0.0],
        [70, 160, 90, 190],
        1.0,
    );
}
