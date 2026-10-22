//! Device-class quality and memory choices, fixed when the renderer is built (pipelines and
//! the prefiltered environment are created once).
//!
//! `Desktop` is the original quality. `Mobile` trades screen-space ambient occlusion, IBL
//! resolution and permanently allocated Xray targets for target memory, and renders the scene
//! image below native resolution on large drawables (see [`RenderProfile::scene_scale`]).
//! Both keep 4x MSAA: without `TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES` wgpu guarantees only
//! 1 and 4 samples, and single sampling aliases the CAD edges the image exists to show.

/// Quality and memory profile of one [`crate::Renderer`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RenderProfile {
    #[default]
    Desktop,
    Mobile,
}

/// Prefiltered environment cube edges; the radiance cube keeps a full mip chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct IblSizes {
    pub env: u32,
    pub spec: u32,
}

/// Lowest scene image scale relative to native pixels: below it, edges and silhouettes
/// blur more than the smaller target is worth.
pub const MIN_SCENE_SCALE: f64 = 0.5;
/// Granularity of [`RenderProfile::lower_scale`] (memory-pressure steps).
pub const SCENE_SCALE_STEP: f64 = 0.125;
/// Mobile scene image cap: at 52 B/px (see `limits::quality_target_bytes`) 3 Mpx keeps one
/// target set near 158 MiB, so a same-size replacement overlap fits the mobile allowance with
/// geometry headroom.
pub const MOBILE_MAX_SCENE_PIXELS: u64 = 3_000_000;

impl RenderProfile {
    /// MSAA samples of the colour and depth attachments (both profiles; see module docs).
    pub const fn msaa_samples(self) -> u32 {
        crate::targets::SAMPLES
    }

    /// Ground-truth ambient occlusion (prepass normals, GTAO and its blur).
    pub const fn ambient_occlusion(self) -> bool {
        matches!(self, Self::Desktop)
    }

    /// Xray accumulation targets (and the depth they test against) exist only while Xray is
    /// shown; switching into or out of Xray replaces the targets.
    pub const fn lazy_transparency(self) -> bool {
        matches!(self, Self::Mobile)
    }

    /// Whether 2x supersampling may be selected.
    pub const fn supersampling(self) -> bool {
        matches!(self, Self::Desktop)
    }

    /// Scene images above this many physical pixels render at a reduced scale.
    pub const fn max_scene_pixels(self) -> Option<u64> {
        match self {
            Self::Desktop => None,
            Self::Mobile => Some(MOBILE_MAX_SCENE_PIXELS),
        }
    }

    pub(crate) const fn ibl(self) -> IblSizes {
        match self {
            Self::Desktop => IblSizes {
                env: 512,
                spec: 256,
            },
            Self::Mobile => IblSizes {
                env: 256,
                spec: 128,
            },
        }
    }

    /// Scene image scale relative to native pixels for a viewport of `native_pixels`
    /// physical pixels: 1 unless the profile caps the pixel count or `limit` (a memory
    /// pressure ceiling) is lower, never below [`MIN_SCENE_SCALE`]. Continuous, so the
    /// image tracks resizes without steps.
    pub fn scene_scale(self, native_pixels: f64, limit: f64) -> f64 {
        let cap = match self.max_scene_pixels() {
            Some(cap) if native_pixels > cap as f64 => (cap as f64 / native_pixels).sqrt(),
            _ => 1.0,
        };
        let limit = if limit.is_finite() { limit } else { 1.0 };
        cap.min(limit).clamp(MIN_SCENE_SCALE, 1.0)
    }

    /// The next [`SCENE_SCALE_STEP`] multiple strictly below `current`, not below
    /// [`MIN_SCENE_SCALE`]; `None` once `current` is at the minimum.
    pub fn lower_scale(self, current: f64) -> Option<f64> {
        if !current.is_finite() || current <= MIN_SCENE_SCALE + 1e-9 {
            return None;
        }
        let below = ((current - 1e-9) / SCENE_SCALE_STEP).floor() * SCENE_SCALE_STEP;
        Some(below.clamp(MIN_SCENE_SCALE, 1.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_keeps_native_scale_and_full_quality() {
        let desktop = RenderProfile::Desktop;
        assert_eq!(desktop.scene_scale(2732.0 * 2048.0, 1.0), 1.0);
        assert!(desktop.ambient_occlusion() && desktop.supersampling());
        assert!(!desktop.lazy_transparency());
        assert_eq!(
            desktop.ibl(),
            IblSizes {
                env: 512,
                spec: 256
            }
        );
        assert_eq!(desktop.msaa_samples(), 4);
    }

    #[test]
    fn mobile_scale_is_continuous_capped_and_clamped() {
        let mobile = RenderProfile::Mobile;
        assert_eq!(mobile.scene_scale(2266.0 * 1488.0 * 0.5, 1.0), 1.0);
        for size in [[2732.0, 2048.0], [2420.0, 1668.0], [2266.0, 1488.0]] {
            let pixels: f64 = size[0] * size[1];
            let scale = mobile.scene_scale(pixels, 1.0);
            assert!((MIN_SCENE_SCALE..1.0).contains(&scale), "{size:?} {scale}");
            assert!((pixels * scale * scale - 3e6).abs() < 1.0, "{size:?}");
        }
        assert_eq!(mobile.scene_scale(40e6, 1.0), MIN_SCENE_SCALE);
        assert_eq!(mobile.scene_scale(1e6, 0.75), 0.75);
        assert_eq!(mobile.scene_scale(1e6, 0.1), MIN_SCENE_SCALE);
        assert_eq!(mobile.scene_scale(0.0, 1.0), 1.0);
        assert_eq!(mobile.scene_scale(f64::NAN, f64::NAN), 1.0);
    }

    #[test]
    fn pressure_steps_go_strictly_down_to_the_minimum() {
        let mobile = RenderProfile::Mobile;
        assert_eq!(mobile.lower_scale(1.0), Some(0.875));
        assert_eq!(mobile.lower_scale(0.875), Some(0.75));
        assert_eq!(mobile.lower_scale(0.73), Some(0.625));
        assert_eq!(mobile.lower_scale(0.55), Some(MIN_SCENE_SCALE));
        assert_eq!(mobile.lower_scale(MIN_SCENE_SCALE), None);
        assert_eq!(mobile.lower_scale(f64::NAN), None);
    }
}
