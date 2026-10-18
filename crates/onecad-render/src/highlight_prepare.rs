use crate::{
    highlights::{highlight_budget, HighlightDraw, HighlightTone},
    HighlightKind, HighlightTarget, PreparationBudget, PreparedHighlights, RenderError,
};
use onecad_protocol::mesh::MeshView;
use onecad_scene::SceneSnapshot;

impl PreparedHighlights {
    /// Resolve topology on a service thread against the same immutable scene used for picking.
    /// Unknown, ambiguous, empty and stale targets fail; selected targets suppress hover.
    pub fn new(
        scene: &SceneSnapshot,
        selected: &[HighlightTarget<'_>],
        hover: Option<HighlightTarget<'_>>,
        budget: PreparationBudget,
    ) -> Result<Self, RenderError> {
        let count = selected
            .len()
            .checked_add(usize::from(hover.is_some()))
            .ok_or(RenderError::Budget("highlight count"))?;
        if count > budget.limits.max_highlights {
            return Err(RenderError::Budget("highlight count"));
        }
        let metadata =
            crate::limits::mul(count as u64, std::mem::size_of::<HighlightDraw>() as u64)?;
        let gpu_bytes = if count == 0 {
            0
        } else {
            2 * std::mem::size_of::<crate::FrameU>() as u64
        };
        highlight_budget(budget, metadata, gpu_bytes)?;
        let mut draws = Vec::new();
        draws
            .try_reserve_exact(count)
            .map_err(|_| RenderError::Budget("highlight preparation"))?;
        for target in selected {
            let draw = resolve(scene, *target, HighlightTone::Selected)?;
            if !draws
                .iter()
                .any(|old: &HighlightDraw| old.same_target(&draw))
            {
                draws.push(draw);
            }
        }
        if let Some(target) = hover {
            let draw = resolve(scene, target, HighlightTone::Hover)?;
            if !draws.iter().any(|old| old.same_target(&draw)) {
                draws.push(draw);
            }
        }
        Ok(Self {
            scene: scene.identity(),
            draws,
        })
    }
}

fn resolve(
    scene: &SceneSnapshot,
    target: HighlightTarget<'_>,
    tone: HighlightTone,
) -> Result<HighlightDraw, RenderError> {
    if scene.identity() != target.scene {
        return Err(RenderError::CaptureMismatch);
    }
    let (body_index, body) = scene
        .bodies()
        .iter()
        .enumerate()
        .find(|(_, body)| body.body_id() == target.body_id)
        .ok_or(RenderError::Highlight("unknown body"))?;
    if body.mesh_revision() != target.mesh_revision {
        return Err(RenderError::CaptureMismatch);
    }
    let mesh = body.mesh()?;
    let index = find_topology(&mesh, target.kind, target.topology_id)?;
    let (first, count) = match target.kind {
        HighlightKind::Face => face_indices(&mesh, index)?,
        HighlightKind::Edge => edge_segments(&mesh, index)?,
    };
    Ok(HighlightDraw {
        body_id: target.body_id,
        body_index,
        mesh_revision: target.mesh_revision,
        kind: target.kind,
        first,
        count,
        tone,
    })
}

fn find_topology(mesh: &MeshView<'_>, kind: HighlightKind, id: &str) -> Result<usize, RenderError> {
    let count = match kind {
        HighlightKind::Face => mesh.header().face_count,
        HighlightKind::Edge => mesh.header().edge_count,
    };
    let mut found = None;
    for index in 0..count as usize {
        let value = match kind {
            HighlightKind::Face => mesh.face_id(index),
            HighlightKind::Edge => mesh.edge_id(index),
        };
        if value == Some(id) {
            if found.is_some() {
                return Err(RenderError::Highlight("ambiguous topology ID"));
            }
            found = Some(index);
        }
    }
    found.ok_or(RenderError::Highlight("unknown topology ID"))
}

pub(crate) fn face_indices(mesh: &MeshView<'_>, face: usize) -> Result<(u32, u32), RenderError> {
    let span = mesh
        .face_range(face)
        .ok_or(RenderError::Highlight("face span"))?;
    let end = span
        .first
        .checked_add(span.count)
        .ok_or(RenderError::Highlight("face span overflow"))?;
    if span.count == 0 || end > mesh.header().triangle_count {
        return Err(RenderError::Highlight("face span bounds"));
    }
    let first = span
        .first
        .checked_mul(3)
        .ok_or(RenderError::Highlight("index count overflow"))?;
    let count = span
        .count
        .checked_mul(3)
        .ok_or(RenderError::Highlight("index count overflow"))?;
    first
        .checked_add(count)
        .ok_or(RenderError::Highlight("index count overflow"))?;
    Ok((first, count))
}

pub(crate) fn edge_segments(mesh: &MeshView<'_>, edge: usize) -> Result<(u32, u32), RenderError> {
    let mut first = 0u32;
    for index in 0..=edge {
        let span = mesh
            .edge_range(index)
            .ok_or(RenderError::Highlight("edge span"))?;
        let count = span.count.saturating_sub(1);
        if index == edge {
            if count == 0 {
                return Err(RenderError::Highlight("empty edge span"));
            }
            first
                .checked_add(count)
                .ok_or(RenderError::Highlight("segment count overflow"))?;
            return Ok((first, count));
        }
        first = first
            .checked_add(count)
            .ok_or(RenderError::Highlight("segment count overflow"))?;
    }
    Err(RenderError::Highlight("edge span"))
}
