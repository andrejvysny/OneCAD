use crate::{frame::FrameU, RenderError, RenderProfile};

/// Conservative payload estimates; driver metadata and pipeline compiler memory are outside
/// these portable estimates. Replacement peaks include old/new resources and upload staging.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MemoryEstimate {
    pub source_bytes: u64,
    pub cpu_prepared_bytes: u64,
    pub resident_metadata_bytes: u64,
    pub geometry_bytes: u64,
    pub upload_bytes: u64,
    pub target_bytes: u64,
    pub ibl_bytes: u64,
    /// GPU payload of the last checked replacement: new resources plus the old ones they
    /// replace (what `RenderLimits::max_gpu_bytes` bounds).
    pub peak_gpu_bytes: u64,
    pub peak_total_bytes: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct RenderLimits {
    pub max_bodies: usize,
    pub max_highlights: usize,
    pub max_texture_dimension: u32,
    pub max_cpu_prepared_bytes: u64,
    pub max_upload_bytes: u64,
    pub max_gpu_bytes: u64,
    pub max_total_bytes: u64,
}

impl Default for RenderLimits {
    fn default() -> Self {
        Self {
            max_bodies: 4096,
            max_highlights: 4096,
            max_texture_dimension: 4096,
            max_cpu_prepared_bytes: 256 << 20,
            max_upload_bytes: 128 << 20,
            max_gpu_bytes: 512 << 20,
            max_total_bytes: 768 << 20,
        }
    }
}

pub(crate) fn add(a: u64, b: u64) -> Result<u64, RenderError> {
    a.checked_add(b)
        .ok_or(RenderError::Budget("arithmetic overflow"))
}

pub(crate) fn mul(a: u64, b: u64) -> Result<u64, RenderError> {
    a.checked_mul(b)
        .ok_or(RenderError::Budget("arithmetic overflow"))
}

#[cfg(test)]
pub(crate) fn target_bytes(size: [u32; 2]) -> Result<u64, RenderError> {
    quality_target_bytes(
        size,
        crate::RenderOptions::default(),
        RenderProfile::Desktop,
    )
}

/// Bytes of the targets `Targets::new` allocates for `size` (exactly its attachment list).
/// Depth/stencil counts 8 bytes per texel, conservative for both negotiated formats; Xray
/// accumulation counts the FLOAT32_BLENDABLE formats (16 + 4 bytes), the larger pair.
/// Attachments a profile leaves out are 1x1 placeholders that keep bind groups valid.
pub(crate) fn quality_target_bytes(
    size: [u32; 2],
    options: crate::RenderOptions,
    profile: RenderProfile,
) -> Result<u64, RenderError> {
    if size.contains(&0) {
        return Ok(0);
    }
    let render = options.render_size(size)?;
    let ao = profile.ambient_occlusion();
    let oit = !profile.lazy_transparency() || options.display_mode == crate::DisplayMode::Xray;
    // MSAA colour (4) and depth (8) per sample, then the single-sample resolve/output (4).
    let mut per_pixel = u64::from(profile.msaa_samples()) * (4 + 8) + 4;
    let mut placeholders = 0;
    for (needed, bytes) in [(ao || oit, 8), (ao, 8 + 3 * 2), (oit, 16 + 4)] {
        if needed {
            per_pixel += bytes;
        } else {
            placeholders += bytes;
        }
    }
    let attachments = add(
        mul(mul(u64::from(render[0]), u64::from(render[1]))?, per_pixel)?,
        (1024 * 1024 * 8) + (256 * 256 * 4 * 3) + placeholders,
    )?;
    if options.ssaa == 1.0 {
        Ok(attachments)
    } else {
        add(
            attachments,
            mul(mul(u64::from(size[0]), u64::from(size[1]))?, 4)?,
        )
    }
}

pub(crate) fn ibl_bytes(profile: RenderProfile) -> u64 {
    let cube = |size: u64, levels: u32| {
        (0..levels)
            .map(|level| (size >> level).max(1).pow(2) * 6 * 8)
            .sum::<u64>()
    };
    let sizes = profile.ibl();
    let env = u64::from(sizes.env);
    // Includes the temporary radiance cube retained until IBL commands finish.
    cube(env, env.ilog2() + 1)
        + cube(u64::from(sizes.spec), crate::ibl::SPEC_MIPS)
        + cube(u64::from(crate::ibl::IRR_SIZE), 1)
        + u64::from(crate::ibl::LUT_SIZE).pow(2) * 4
}

impl RenderLimits {
    pub(crate) fn check(
        &self,
        mut next: MemoryEstimate,
        old: MemoryEstimate,
    ) -> Result<MemoryEstimate, RenderError> {
        if next.cpu_prepared_bytes > self.max_cpu_prepared_bytes {
            return Err(RenderError::Budget("CPU preparation"));
        }
        if next.upload_bytes > self.max_upload_bytes {
            return Err(RenderError::Budget("upload staging"));
        }
        let gpu = add(add(next.geometry_bytes, next.target_bytes)?, next.ibl_bytes)?;
        let overlap = add(old.geometry_bytes, old.target_bytes)?;
        next.peak_gpu_bytes = add(gpu, overlap)?;
        if next.peak_gpu_bytes > self.max_gpu_bytes {
            return Err(RenderError::Budget("GPU replacement overlap"));
        }
        let cpu = add(
            add(next.source_bytes, next.cpu_prepared_bytes)?,
            next.upload_bytes,
        )?;
        let metadata = add(next.resident_metadata_bytes, old.resident_metadata_bytes)?;
        next.peak_total_bytes = add(
            add(next.peak_gpu_bytes, add(cpu, old.source_bytes)?)?,
            metadata,
        )?;
        if next.peak_total_bytes > self.max_total_bytes {
            return Err(RenderError::Budget("total replacement overlap"));
        }
        Ok(next)
    }
    pub(crate) fn validate_size(
        &self,
        size: [u32; 2],
        device: &wgpu::Device,
    ) -> Result<(), RenderError> {
        self.validate_dimensions(size, device.limits().max_texture_dimension_2d)
    }
    fn validate_dimensions(&self, size: [u32; 2], device_max: u32) -> Result<(), RenderError> {
        let max = self.max_texture_dimension.min(device_max);
        if size.iter().any(|value| *value > max) {
            return Err(RenderError::Budget("target dimension"));
        }
        let fixed = if size.contains(&0) { 512 } else { 1024 };
        if max < fixed {
            return Err(RenderError::Budget("fixed texture dimension"));
        }
        Ok(())
    }
}

pub(crate) const fn shared_uniform_bytes() -> u64 {
    3 * std::mem::size_of::<FrameU>() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixed_ibl_and_contact_dimensions_obey_caps() {
        let limits = RenderLimits {
            max_texture_dimension: 512,
            ..Default::default()
        };
        assert!(limits.validate_dimensions([0, 0], 4096).is_ok());
        assert!(matches!(
            limits.validate_dimensions([64, 64], 4096),
            Err(RenderError::Budget("fixed texture dimension"))
        ));
        assert!(RenderLimits::default()
            .validate_dimensions([64, 64], 256)
            .is_err());
    }
    #[test]
    fn target_payload_counts_every_quality_attachment() {
        assert_eq!(
            target_bytes([100, 200]).unwrap(),
            100 * 200 * 94 + 1024 * 1024 * 8 + 256 * 256 * 4 * 3
        );
        assert_eq!(target_bytes([0, u32::MAX]).unwrap(), 0);
        assert!(mul(u64::MAX, 2).is_err());
        assert!(add(u64::MAX, 1).is_err());
    }
    const FIXED: u64 = 1024 * 1024 * 8 + 256 * 256 * 4 * 3;
    // Mobile placeholders outside Xray: prepass depth, normal + three AO, the Xray pair.
    const MOBILE_PLACEHOLDERS: u64 = 8 + (8 + 3 * 2) + (16 + 4);
    fn bytes(size: [u32; 2], profile: RenderProfile, mode: crate::DisplayMode) -> u64 {
        let options = crate::RenderOptions {
            display_mode: mode,
            ..Default::default()
        };
        quality_target_bytes(size, options, profile).unwrap()
    }
    #[test]
    fn profile_target_bytes_at_ipad_drawables() {
        use crate::DisplayMode::{ShadedEdges, Xray};
        use RenderProfile::{Desktop, Mobile};
        for size in [[2732, 2048], [2420, 1668], [2266, 1488], [2048, 2732]] {
            let pixels = u64::from(size[0]) * u64::from(size[1]);
            assert_eq!(bytes(size, Desktop, ShadedEdges), pixels * 94 + FIXED);
            assert_eq!(bytes(size, Desktop, Xray), pixels * 94 + FIXED);
            assert_eq!(
                bytes(size, Mobile, ShadedEdges),
                pixels * 52 + FIXED + MOBILE_PLACEHOLDERS
            );
            assert_eq!(bytes(size, Mobile, Xray), pixels * 80 + FIXED + 8 + 3 * 2);
        }
        // The 13-inch drawable: Desktop targets at native plus IBL exceed the 512 MiB GPU
        // budget before any geometry; Mobile at its scene cap fits a same-size overlap and
        // leaves 64 MiB of a 448 MiB allowance for geometry.
        let desktop = bytes([2732, 2048], Desktop, ShadedEdges);
        assert!(desktop + ibl_bytes(Desktop) > 512 << 20);
        let scale = Mobile.scene_scale(2732.0 * 2048.0, 1.0);
        let size = [2732, 2048].map(|v| (f64::from(v) * scale).round() as u32);
        assert!(2 * bytes(size, Mobile, ShadedEdges) + ibl_bytes(Mobile) < (448 - 64) << 20);
    }
    #[test]
    fn profile_target_bytes_at_degenerate_sizes() {
        use crate::DisplayMode::{ShadedEdges, Xray};
        use RenderProfile::{Desktop, Mobile};
        for profile in [Desktop, Mobile] {
            for mode in [ShadedEdges, Xray] {
                assert_eq!(bytes([0, 0], profile, mode), 0);
                assert_eq!(bytes([0, 2048], profile, mode), 0);
                assert_eq!(bytes([2732, 0], profile, mode), 0);
            }
        }
        assert_eq!(bytes([1, 1], Desktop, ShadedEdges), 94 + FIXED);
        assert_eq!(
            bytes([1, 1], Mobile, ShadedEdges),
            52 + FIXED + MOBILE_PLACEHOLDERS
        );
        assert_eq!(bytes([1, 1], Mobile, Xray), 80 + FIXED + 14);
        let ssaa = crate::RenderOptions {
            ssaa: 2.0,
            ..Default::default()
        };
        assert_eq!(
            quality_target_bytes([1, 1], ssaa, Desktop).unwrap(),
            4 * 94 + FIXED + 4
        );
        assert!(quality_target_bytes([u32::MAX, 1], ssaa, Desktop).is_err());
    }
    #[test]
    fn mobile_ibl_is_about_a_quarter_of_desktop() {
        let desktop = ibl_bytes(RenderProfile::Desktop);
        assert_eq!(desktop, 16_777_200 + 4_193_280 + 49_152 + 65_536);
        assert_eq!(ibl_bytes(RenderProfile::Mobile), 5_357_296);
    }
    #[test]
    fn replacement_overlap_and_upload_staging_are_charged_before_allocation() {
        let limits = RenderLimits {
            max_gpu_bytes: 100,
            max_total_bytes: 1000,
            ..Default::default()
        };
        let next = MemoryEstimate {
            geometry_bytes: 60,
            ..Default::default()
        };
        let old = MemoryEstimate {
            geometry_bytes: 41,
            ..Default::default()
        };
        assert!(matches!(
            limits.check(next, old),
            Err(RenderError::Budget("GPU replacement overlap"))
        ));
        let limits = RenderLimits {
            max_upload_bytes: 10,
            ..Default::default()
        };
        assert!(matches!(
            limits.check(
                MemoryEstimate {
                    upload_bytes: 11,
                    ..Default::default()
                },
                MemoryEstimate::default()
            ),
            Err(RenderError::Budget("upload staging"))
        ));
    }
    #[test]
    fn source_preparation_staging_and_gpu_are_in_total_peak() {
        let limits = RenderLimits {
            max_total_bytes: 100,
            ..Default::default()
        };
        let next = MemoryEstimate {
            source_bytes: 25,
            cpu_prepared_bytes: 25,
            upload_bytes: 25,
            geometry_bytes: 26,
            ..Default::default()
        };
        assert!(matches!(
            limits.check(next, MemoryEstimate::default()),
            Err(RenderError::Budget("total replacement overlap"))
        ));
    }
}
