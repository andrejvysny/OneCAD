//! Cached donor batches remain charged until their replacement frame has completed.
use super::*;
impl SolidCraftViewport {
    pub(super) fn options_payload_bytes(
        &self,
        options: &SolidCraftOptions,
    ) -> Result<u64, RenderError> {
        let actual = options
            .access
            .as_ref()
            .map_or(0, |access| access.depth.len() as u64 * 4);
        add(
            options.validate(self.limits, &self.device)?,
            self.access_highwater_bytes.saturating_sub(actual),
        )
    }
    pub(super) fn refresh_accounting(&mut self) -> Result<(), RenderError> {
        let geometry = add(
            self.scene.as_ref().map_or(0, |s| s.bytes),
            self.preview.as_ref().map_or(0, |s| s.bytes),
        )?;
        let retired = add(self.retired_model_bytes, self.retired_preview_bytes)?;
        let images = self.options_payload_bytes(&self.options)?;
        let estimate = MemoryEstimate {
            source_bytes: add(
                self.scene.as_ref().map_or(0, |s| s.source_bytes),
                self.preview.as_ref().map_or(0, |s| s.source_bytes),
            )?,
            cpu_prepared_bytes: add(
                add(mul(geometry, 4)?, images)?,
                add(self.highlight_bytes, self.retired_highlight_bytes)?,
            )?,
            resident_metadata_bytes: add(
                add(
                    add(mul(add(geometry, retired)?, 2)?, images)?,
                    self.highlight_bytes,
                )?,
                self.retired_highlight_bytes,
            )?,
            geometry_bytes: add(
                add(
                    add(mul(add(geometry, retired)?, 3)?, images)?,
                    self.highlight_bytes,
                )?,
                self.retired_highlight_bytes,
            )?,
            upload_bytes: 0,
            target_bytes: self.estimate.target_bytes,
            ..Default::default()
        };
        self.estimate = self.limits.check(estimate, MemoryEstimate::default())?;
        Ok(())
    }
}
