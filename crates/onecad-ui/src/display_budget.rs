//! Process-wide display memory of one window.
//!
//! Every tab of a window draws through the window's single `Renderer`: only the active
//! document's targets, IBL and geometry are on the GPU, and the renderer charges each
//! replacement (resize, rotation, scene or option change) with its old and new resources
//! against `RenderLimits::max_gpu_bytes`. Parked tabs keep only disposable caches: the
//! ⌃Tab switcher thumbnail (GPU) and a prepared scene not yet uploaded (CPU, re-prepared
//! on activation). The aggregate splits into the renderer's allowance, reserved for the
//! active document, and what parked caches may hold; parked caches are evicted least
//! recently active first, so the sum of both never exceeds the aggregate. On Apple
//! silicon GPU and CPU share one memory pool, so both count against one figure.
use crate::app::App;
use onecad_render::{RenderLimits, RenderProfile};

const MIB: u64 = 1 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayBudget {
    /// Everything display-related one window may hold at a replacement peak.
    pub aggregate: u64,
    /// The renderer's allowance (`max_gpu_bytes`), reserved for the active document.
    pub active: u64,
}

impl DisplayBudget {
    /// Desktop keeps the renderer's 512 MiB and adds the parked share on top; Mobile
    /// fits both into 512 MiB.
    pub fn for_profile(profile: RenderProfile) -> Self {
        let parked = 64 * MIB;
        let active = match profile {
            RenderProfile::Desktop => RenderLimits::default().max_gpu_bytes,
            RenderProfile::Mobile => 512 * MIB - parked,
        };
        Self {
            aggregate: active + parked,
            active,
        }
    }

    pub fn parked(&self) -> u64 {
        self.aggregate.saturating_sub(self.active)
    }

    pub fn render_limits(&self) -> RenderLimits {
        RenderLimits {
            max_cpu_prepared_bytes: crate::budget::MAX_PREPARED_BYTES,
            max_upload_bytes: crate::budget::MAX_PREPARED_BYTES,
            max_gpu_bytes: self.active,
            ..Default::default()
        }
    }
}

/// One accounting of the window's display memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ledger {
    /// Targets + IBL + geometry the renderer holds now.
    pub resident: u64,
    /// The renderer's last checked replacement: new plus old resources.
    pub peak: u64,
    /// Caches parked tabs hold.
    pub parked: u64,
}

impl Ledger {
    pub fn peak_total(&self) -> u64 {
        self.peak.max(self.resident) + self.parked
    }
}

/// What a memory warning did.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MemoryRelief {
    /// Parked-tab cache bytes released.
    pub released: u64,
    /// The new scene scale ceiling, when the active document was over its allowance.
    pub lowered_scale: Option<f64>,
}

fn thumbnail_bytes(doc: &crate::document_editor::DocumentEditor) -> u64 {
    doc.thumbnail.as_ref().map_or(0, |thumbnail| {
        let size = thumbnail._source.size();
        u64::from(size.width) * u64::from(size.height) * 4
    })
}

fn prepared_bytes(doc: &crate::document_editor::DocumentEditor) -> u64 {
    doc.display.as_ref().map_or(0, |display| {
        display
            .prepared
            .lock()
            .map_or(0, |prepared| prepared.as_ref().map_or(0, |p| p.cpu_bytes()))
    })
}

fn parked_bytes(doc: &crate::document_editor::DocumentEditor) -> u64 {
    thumbnail_bytes(doc) + prepared_bytes(doc)
}

impl App {
    pub fn display_ledger(&self) -> Ledger {
        let estimate = self.renderer.memory_estimate();
        Ledger {
            resident: estimate.geometry_bytes + estimate.target_bytes + estimate.ibl_bytes,
            peak: estimate.peak_gpu_bytes,
            parked: self.tabs.parked.iter().map(parked_bytes).sum(),
        }
    }

    /// Evicts parked caches, least recently active tab first, until they fit their share.
    pub(crate) fn enforce_parked_budget(&mut self) {
        let mut order: Vec<usize> = (0..self.tabs.parked.len()).collect();
        order.sort_by_key(|index| self.tabs.parked[*index].last_active);
        for index in order {
            if self.display_ledger().parked <= self.display_budget.parked() {
                return;
            }
            self.release_parked_caches_of(index);
        }
    }

    /// Drops one parked tab's thumbnail and unuploaded prepared scene; returns the bytes.
    fn release_parked_caches_of(&mut self, index: usize) -> u64 {
        let doc = &mut self.tabs.parked[index];
        let bytes = parked_bytes(doc);
        if let Some(display) = &doc.display
            && let Ok(mut prepared) = display.prepared.lock()
        {
            // Activation re-prepares a consumed scene under the same identity.
            prepared.take();
        }
        let thumbnail = doc.thumbnail.take();
        self.release_thumbnail(thumbnail);
        bytes
    }

    /// Memory pressure: every parked tab's caches go; they are rebuilt on demand.
    pub fn release_parked_caches(&mut self) -> u64 {
        (0..self.tabs.parked.len())
            .map(|index| self.release_parked_caches_of(index))
            .sum()
    }

    /// Memory warning: releases parked caches, then lowers the scene scale one step when
    /// the active document could no longer replace its targets within its allowance.
    pub fn relieve_memory_pressure(&mut self) -> MemoryRelief {
        let released = self.release_parked_caches();
        let ledger = self.display_ledger();
        let replacement = self
            .renderer
            .target_bytes(self.renderer.size())
            .unwrap_or(0);
        let over = ledger.resident.saturating_add(replacement) > self.display_budget.active;
        let lowered_scale = over
            .then(|| self.renderer.profile().lower_scale(self.scene_scale))
            .flatten();
        if let Some(scale) = lowered_scale {
            self.render_scale_limit = scale;
            self.draw_dirty = true;
        }
        MemoryRelief {
            released,
            lowered_scale,
        }
    }

    /// The host stops drawing (backgrounded): fences the last frame, waits for it (at most
    /// 1 s) and releases the renderer's targets. Scene geometry, camera and document state
    /// stay; the next frame recreates the targets. `false` when the GPU was still busy.
    pub fn release_render_targets(&mut self, ctx: &egui::Context) -> bool {
        crate::input::fence_previous_frame(self, ctx);
        if !self.renderer.has_targets() {
            return true;
        }
        let _ = self.gpu.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(1)),
        });
        let idle = self.renderer.poll_idle().unwrap_or(false)
            && self
                .sampling_done
                .load(std::sync::atomic::Ordering::Acquire);
        if !idle || self.renderer.resize([0, 0]).is_err() {
            return false;
        }
        crate::viewport::replace_texture(self);
        self.doc.view = None;
        self.draw_dirty = true;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_keeps_the_renderer_allowance_and_mobile_fits_512_mib() {
        let desktop = DisplayBudget::for_profile(RenderProfile::Desktop);
        assert_eq!(desktop.active, 512 * MIB);
        assert_eq!(desktop.render_limits().max_gpu_bytes, 512 * MIB);
        assert_eq!(desktop.parked(), 64 * MIB);
        let mobile = DisplayBudget::for_profile(RenderProfile::Mobile);
        assert_eq!(mobile.aggregate, 512 * MIB);
        assert_eq!(mobile.active + mobile.parked(), mobile.aggregate);
        assert_eq!(
            mobile.render_limits().max_total_bytes,
            RenderLimits::default().max_total_bytes
        );
    }

    #[test]
    fn the_peak_total_counts_the_larger_of_peak_and_resident() {
        let ledger = Ledger {
            resident: 10,
            peak: 25,
            parked: 3,
        };
        assert_eq!(ledger.peak_total(), 28);
        assert_eq!(Ledger { peak: 0, ..ledger }.peak_total(), 13);
    }
}
