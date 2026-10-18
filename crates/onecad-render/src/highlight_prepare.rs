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
        Self {
            scene: scene.identity(),
            draws,
            boundaries: Vec::new(),
            boundary_draws: Vec::new(),
        }
        .with_boundaries(scene, budget)
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

impl PreparedHighlights {
    fn with_boundaries(
        mut self,
        scene: &SceneSnapshot,
        budget: PreparationBudget,
    ) -> Result<Self, RenderError> {
        let (capacity, scratch, count) = boundary_capacity(scene, &self.draws)?;
        let output_bytes = crate::limits::mul(capacity, 24)?;
        let draw_bytes = crate::limits::mul(count, std::mem::size_of::<HighlightDraw>() as u64)?;
        let metadata = crate::limits::add(
            self.metadata_bytes()?,
            crate::limits::add(output_bytes, crate::limits::add(scratch, draw_bytes)?)?,
        )?;
        let gpu = crate::limits::add(
            output_bytes.max(64),
            2 * std::mem::size_of::<crate::FrameU>() as u64,
        )?;
        highlight_budget(budget, metadata, gpu)?;
        self.boundaries
            .try_reserve_exact(capacity as usize)
            .map_err(|_| RenderError::Budget("face boundaries"))?;
        self.boundary_draws
            .try_reserve_exact(count as usize)
            .map_err(|_| RenderError::Budget("face boundary draws"))?;
        for draw in self
            .draws
            .iter()
            .filter(|draw| draw.kind == HighlightKind::Face)
        {
            let mesh = scene.bodies()[draw.body_index].mesh()?;
            let first = self.boundaries.len() as u32;
            if mesh.header().section(15).is_some() {
                topology_boundaries(&mesh, draw, &mut self.boundaries)?;
            } else {
                legacy_boundaries(&mesh, draw, &mut self.boundaries)?;
            }
            let count = self.boundaries.len() as u32 - first;
            if count != 0 {
                self.boundary_draws.push(HighlightDraw {
                    first,
                    count,
                    kind: HighlightKind::Edge,
                    ..*draw
                });
            }
        }
        Ok(self)
    }
}

fn incident(mesh: &MeshView<'_>, draw: &HighlightDraw, edge: usize) -> bool {
    mesh.edge_incident_faces(edge).iter().any(|face| {
        mesh.face_range(face as usize)
            .is_some_and(|range| range.first * 3 == draw.first && range.count * 3 == draw.count)
    })
}
fn boundary_capacity(
    scene: &SceneSnapshot,
    draws: &[HighlightDraw],
) -> Result<(u64, u64, u64), RenderError> {
    let (mut capacity, mut scratch, mut count) = (0, 0, 0);
    for draw in draws.iter().filter(|draw| draw.kind == HighlightKind::Face) {
        let mesh = scene.bodies()[draw.body_index].mesh()?;
        count += 1;
        let length = if mesh.header().section(15).is_some() {
            (0..mesh.header().edge_count as usize)
                .filter(|edge| incident(&mesh, draw, *edge))
                .try_fold(0u64, |sum, edge| {
                    crate::limits::add(
                        sum,
                        u64::from(
                            mesh.edge_range(edge)
                                .ok_or(RenderError::Highlight("edge span"))?
                                .count
                                .saturating_sub(1),
                        ),
                    )
                })?
        } else {
            scratch = scratch.max(crate::limits::mul(u64::from(draw.count), 48)?);
            u64::from(draw.count)
        };
        capacity = crate::limits::add(capacity, length)?;
    }
    if capacity > u64::from(u32::MAX) {
        return Err(RenderError::Budget("face boundary count"));
    }
    Ok((capacity, scratch, count))
}
fn topology_boundaries(
    mesh: &MeshView<'_>,
    draw: &HighlightDraw,
    output: &mut Vec<[f32; 6]>,
) -> Result<(), RenderError> {
    for edge in (0..mesh.header().edge_count as usize).filter(|edge| incident(mesh, draw, *edge)) {
        let range = mesh
            .edge_range(edge)
            .ok_or(RenderError::Highlight("edge span"))?;
        for index in 1..range.count {
            let a = mesh
                .edge_point((range.first + index - 1) as usize)
                .ok_or(RenderError::Highlight("edge point"))?;
            let b = mesh
                .edge_point((range.first + index) as usize)
                .ok_or(RenderError::Highlight("edge point"))?;
            output.push([a[0], a[1], a[2], b[0], b[1], b[2]]);
        }
    }
    Ok(())
}
fn legacy_boundaries(
    mesh: &MeshView<'_>,
    draw: &HighlightDraw,
    output: &mut Vec<[f32; 6]>,
) -> Result<(), RenderError> {
    let mut edges = Vec::new();
    edges
        .try_reserve_exact(draw.count as usize)
        .map_err(|_| RenderError::Budget("face boundary scratch"))?;
    for triangle in draw.first / 3..(draw.first + draw.count) / 3 {
        let indices = mesh
            .triangle(triangle as usize)
            .ok_or(RenderError::Highlight("face triangle"))?;
        let [Some(a), Some(b), Some(c)] = indices.map(|index| mesh.position(index as usize)) else {
            return Err(RenderError::Highlight("face position"));
        };
        for (a, b) in [(a, b), (b, c), (c, a)] {
            let bits = |p: [f32; 3]| p.map(|x| if x == 0.0 { 0 } else { x.to_bits() });
            let (ka, kb) = (bits(a), bits(b));
            let (ka, kb, a, b) = if ka <= kb {
                (ka, kb, a, b)
            } else {
                (kb, ka, b, a)
            };
            edges.push((
                [ka[0], ka[1], ka[2], kb[0], kb[1], kb[2]],
                [a[0], a[1], a[2], b[0], b[1], b[2]],
            ));
        }
    }
    edges.sort_unstable_by_key(|edge| edge.0);
    let mut index = 0;
    while index < edges.len() {
        let end = edges[index..].partition_point(|edge| edge.0 == edges[index].0) + index;
        if (end - index) % 2 == 1 {
            output.push(edges[index].1);
        }
        index = end;
    }
    Ok(())
}
