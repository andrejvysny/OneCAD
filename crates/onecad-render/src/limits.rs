use crate::{frame::FrameU, RenderError};

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
    if size.contains(&0) {
        return Ok(0);
    }
    // RGBA8 resolved+MSAA4, Depth32 MSAA4+prepass, RGBA16 normal, 3 R16 AO.
    add(
        mul(mul(u64::from(size[0]), u64::from(size[1]))?, 54)?,
        (1024 * 1024 * 8) + (256 * 256 * 4 * 3),
    )
}

pub(crate) fn quality_target_bytes(
    size: [u32; 2],
    options: crate::RenderOptions,
) -> Result<u64, RenderError> {
    if size.contains(&0) {
        return Ok(0);
    }
    let render = options.render_size(size)?;
    // 8 bytes per depth/stencil texel is conservative for both negotiated formats.
    let attachments = add(
        mul(mul(u64::from(render[0]), u64::from(render[1]))?, 74)?,
        (1024 * 1024 * 8) + (256 * 256 * 4 * 3),
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

pub(crate) fn ibl_bytes() -> u64 {
    let cube = |size: u64, levels: u32| {
        (0..levels)
            .map(|level| (size >> level).max(1).pow(2) * 6 * 8)
            .sum::<u64>()
    };
    // Includes the temporary radiance cube retained until IBL commands finish.
    cube(512, 10) + cube(256, 6) + cube(32, 1) + 128 * 128 * 4
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
        if add(gpu, overlap)? > self.max_gpu_bytes {
            return Err(RenderError::Budget("GPU replacement overlap"));
        }
        let cpu = add(
            add(next.source_bytes, next.cpu_prepared_bytes)?,
            next.upload_bytes,
        )?;
        let metadata = add(next.resident_metadata_bytes, old.resident_metadata_bytes)?;
        next.peak_total_bytes = add(
            add(add(gpu, overlap)?, add(cpu, old.source_bytes)?)?,
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

pub(crate) fn uniform_bytes(bodies: usize) -> Result<u64, RenderError> {
    mul(bodies as u64 + 1, std::mem::size_of::<FrameU>() as u64)
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
            100 * 200 * 54 + 1024 * 1024 * 8 + 256 * 256 * 4 * 3
        );
        assert_eq!(target_bytes([0, u32::MAX]).unwrap(), 0);
        assert!(mul(u64::MAX, 2).is_err());
        assert!(add(u64::MAX, 1).is_err());
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
