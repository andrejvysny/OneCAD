use super::*;
use crate::{highlights::HighlightTone, HighlightKind, PreparationBudget, PreparedHighlights};
impl SolidCraftViewport {
    pub fn highlight_preparation_budget(&self) -> PreparationBudget {
        PreparationBudget {
            limits: self.limits,
            resident: self.estimate,
            max_buffer: self.device.limits().max_buffer_size,
        }
    }
    pub fn replace_highlights(
        &mut self,
        highlights: PreparedHighlights,
    ) -> Result<(), RenderError> {
        self.require_idle()?;
        highlights.validate_scene(self.scene_identity())?;
        if highlights.len() > self.limits.max_highlights {
            return Err(RenderError::Budget("highlight count"));
        }
        let bytes = self.checked_highlight_bytes(&highlights)?;
        let estimate = self.check_highlight_budget(bytes)?;
        let scene = self.scene.as_ref().ok_or(RenderError::CaptureMismatch)?;
        let gpu = build_highlights(scene, &highlights)?;
        self.set_slot(true, gpu)?;
        self.retired_highlight_bytes = self.retired_highlight_bytes.max(self.highlight_bytes);
        self.highlight_bytes = bytes;
        self.estimate = estimate;
        self.refresh_accounting()
    }
    fn checked_highlight_bytes(&self, highlights: &PreparedHighlights) -> Result<u64, RenderError> {
        let scene = self.scene.as_ref().ok_or(RenderError::CaptureMismatch)?;
        let mut bytes = 0;
        for draw in &highlights.draws {
            let body = scene
                .bodies
                .get(draw.body_index)
                .filter(|b| b.id == draw.body_id)
                .ok_or(RenderError::Highlight("unknown body"))?;
            draw.validate(
                body.revision,
                (body.scene.tris.len() / view::TRI_SIZE) as u32,
                (body.scene.lines.len() / view::LINE_SIZE) as u32,
            )?;
            let stride = if draw.kind == HighlightKind::Face {
                view::TRI_SIZE
            } else {
                view::LINE_SIZE
            };
            bytes = add(bytes, mul(u64::from(draw.count), stride as u64)?)?;
        }
        Ok(bytes)
    }
    fn check_highlight_budget(&self, bytes: u64) -> Result<MemoryEstimate, RenderError> {
        self.limits.check(
            MemoryEstimate {
                cpu_prepared_bytes: add(
                    add(
                        mul(
                            add(
                                self.scene.as_ref().map_or(0, |s| s.bytes),
                                self.preview.as_ref().map_or(0, |s| s.bytes),
                            )?,
                            4,
                        )?,
                        self.options_payload_bytes(&self.options)?,
                    )?,
                    add(
                        bytes,
                        add(self.highlight_bytes, self.retired_highlight_bytes)?,
                    )?,
                )?,
                geometry_bytes: add(self.estimate.geometry_bytes - self.highlight_bytes, bytes)?,
                resident_metadata_bytes: add(
                    self.estimate.resident_metadata_bytes - self.highlight_bytes,
                    bytes,
                )?,
                upload_bytes: bytes,
                ..self.estimate
            },
            MemoryEstimate {
                geometry_bytes: self.highlight_bytes,
                ..Default::default()
            },
        )
    }
    pub fn clear_highlights(&mut self) -> Result<(), RenderError> {
        self.require_idle()?;
        self.set_slot(true, GpuScene::default())?;
        self.retired_highlight_bytes = self.retired_highlight_bytes.max(self.highlight_bytes);
        self.highlight_bytes = 0;
        self.refresh_accounting()
    }
    pub fn set_body_style(&mut self, id: BodyId, style: BodyStyle) -> bool {
        if self.require_idle().is_err() {
            return false;
        }
        let Some(scene) = &mut self.scene else {
            return false;
        };
        let Some(index) = scene.body_indices.get(&id).copied() else {
            return false;
        };
        let Some(body) = scene.bodies.get_mut(index) else {
            return false;
        };
        if body.selected == style.selected {
            return true;
        }
        let old = body.selected;
        body.selected = style.selected;
        let color = if style.selected {
            [75, 155, 230, 255]
        } else {
            [168, 179, 194, 255]
        };
        for vertex in body.scene.tris.chunks_exact_mut(view::TRI_SIZE) {
            vertex[24..28].copy_from_slice(&color);
        }
        let model = match frame::model(scene, &self.options) {
            Ok(model) => model,
            Err(_) => {
                if let Some(body) = scene.bodies.get_mut(index) {
                    body.selected = old;
                    let color = if old {
                        [75, 155, 230, 255]
                    } else {
                        [168, 179, 194, 255]
                    };
                    for vertex in body.scene.tris.chunks_exact_mut(view::TRI_SIZE) {
                        vertex[24..28].copy_from_slice(&color);
                    }
                }
                return false;
            }
        };
        self.put_model(model).is_ok()
    }
}
fn append(
    out: &mut GpuScene,
    body: &GpuScene,
    kind: HighlightKind,
    first: u32,
    count: u32,
    color: [u8; 4],
) -> Result<(), RenderError> {
    let stride = if kind == HighlightKind::Face {
        view::TRI_SIZE
    } else {
        view::LINE_SIZE
    };
    let source = if kind == HighlightKind::Face {
        &body.tris
    } else {
        &body.lines
    };
    let range = first as usize * stride..(first as usize + count as usize) * stride;
    let data = source
        .get(range)
        .ok_or(RenderError::Highlight("draw range"))?;
    let dest = if kind == HighlightKind::Face {
        &mut out.trans
    } else {
        &mut out.overlay
    };
    dest.try_reserve_exact(data.len())
        .map_err(|_| RenderError::Budget("donor highlights"))?;
    for vertex in data.chunks_exact(stride) {
        let start = dest.len();
        dest.extend_from_slice(vertex);
        dest[start + 24..start + 28].copy_from_slice(&color);
        if kind == HighlightKind::Edge {
            dest[start + 28..start + 32].copy_from_slice(&2.5_f32.to_le_bytes());
        }
    }
    Ok(())
}

fn build_highlights(
    scene: &PreparedSolidCraftScene,
    highlights: &PreparedHighlights,
) -> Result<GpuScene, RenderError> {
    let mut gpu = GpuScene::default();
    for draw in &highlights.draws {
        let body = scene
            .bodies
            .get(draw.body_index)
            .ok_or(RenderError::Highlight("unknown body"))?;
        let color = if draw.tone == HighlightTone::Selected {
            [45, 155, 240, 130]
        } else {
            [245, 180, 65, 110]
        };
        append(
            &mut gpu,
            &body.scene,
            draw.kind,
            draw.first,
            draw.count,
            color,
        )?;
    }
    Ok(gpu)
}
