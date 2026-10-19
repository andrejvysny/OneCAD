use super::*;

pub(super) fn checked(baseline: &Sketch, preview: &Sketch) -> bool {
    baseline.id == preview.id
        && baseline.plane == preview.plane
        && baseline.effective_plane() == preview.effective_plane()
        && baseline.attachment == preview.attachment
        && baseline.projections == preview.projections
        && baseline.constraints() == preview.constraints()
        && baseline.entities().len() == preview.entities().len()
        && super::super::validate_sketch_plane(preview.effective_plane())
        && baseline.entities().iter().all(|entity| {
            preview
                .get_entity(entity.id())
                .is_some_and(|candidate| same_entity(entity, candidate))
        })
}

fn same_entity(baseline: &SketchEntity, preview: &SketchEntity) -> bool {
    if baseline.is_reference_locked() {
        return baseline == preview;
    }
    if baseline.is_construction() != preview.is_construction() || preview.is_reference_locked() {
        return false;
    }
    match (baseline, preview) {
        (SketchEntity::Point { .. }, SketchEntity::Point { at, .. }) => at.is_finite(),
        (
            SketchEntity::Line {
                start: a, end: b, ..
            },
            SketchEntity::Line { start, end, .. },
        ) => a == start && b == end,
        (SketchEntity::Circle { center: a, .. }, SketchEntity::Circle { center, radius, .. }) => {
            a == center && positive(*radius)
        }
        (
            SketchEntity::Arc { center: a, .. },
            SketchEntity::Arc {
                center,
                radius,
                start_angle,
                end_angle,
                ..
            },
        ) => a == center && positive(*radius) && start_angle.is_finite() && end_angle.is_finite(),
        (
            SketchEntity::Ellipse { center: a, .. },
            SketchEntity::Ellipse {
                center,
                major_r,
                minor_r,
                rotation,
                ..
            },
        ) => a == center && positive(*major_r) && positive(*minor_r) && rotation.is_finite(),
        _ => false,
    }
}

fn positive(value: f64) -> bool {
    value.is_finite() && value > 0.0
}
