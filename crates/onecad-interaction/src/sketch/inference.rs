//! Only the winning, capture-fenced relation may become a sketch constraint.
use super::SketchTarget;
use onecad_core::{
    ids::{ConstraintId, EntityId, SketchId},
    math::Vec2,
    sketch::{Constraint, CurvePosition, Sketch, SketchEntity},
};
use onecad_scene::CaptureIdentity;

mod destination;
#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapKind {
    Endpoint,
    Midpoint,
    Center,
    Origin,
    Quadrant,
    Intersection,
    OnCurve,
    HorizontalGuide,
    VerticalGuide,
    Guides,
    Polar,
    Grid,
}

impl SnapKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Endpoint => "Endpoint",
            Self::Midpoint => "Midpoint",
            Self::Center => "Center",
            Self::Origin => "Origin",
            Self::Quadrant => "Quadrant",
            Self::Intersection => "Intersection",
            Self::OnCurve => "On curve",
            Self::HorizontalGuide => "Horizontal",
            Self::VerticalGuide => "Vertical",
            Self::Guides => "Horizontal / Vertical",
            Self::Polar => "Polar",
            Self::Grid => "Grid",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointAlignment {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapIntent {
    GeometryOnly,
    Origin,
    Coincident(SketchTarget),
    Midpoint(EntityId),
    OnCurve(EntityId),
    Intersection {
        first: EntityId,
        second: EntityId,
    },
    Quadrant {
        curve: EntityId,
        center: SketchTarget,
        alignment: PointAlignment,
    },
    Guide {
        reference: SketchTarget,
        alignment: PointAlignment,
    },
    Guides {
        horizontal: SketchTarget,
        vertical: SketchTarget,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SnapSource {
    pub sketch: SketchId,
    pub capture: CaptureIdentity,
    pub generation: u64,
    pub geometry_fingerprint: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AcceptedSnap {
    pub source: SnapSource,
    pub kind: SnapKind,
    pub point: Vec2,
    pub intent: SnapIntent,
}

/// `point` names the fresh point that the tool will add in the same batch.
pub fn persist_snap(
    sketch: &Sketch,
    accepted: &AcceptedSnap,
    current: SnapSource,
    point: EntityId,
    next_id: &mut impl FnMut() -> ConstraintId,
) -> Result<Vec<Constraint>, &'static str> {
    persist_reference(
        sketch,
        accepted,
        current,
        (point, CurvePosition::Arbitrary),
        next_id,
    )
}

/// Preserve an analytic endpoint role without creating a separate endpoint entity.
pub fn persist_snap_to(
    sketch: &Sketch,
    accepted: &AcceptedSnap,
    current: SnapSource,
    destination: SketchTarget,
    additions: &[SketchEntity],
    next_id: &mut impl FnMut() -> ConstraintId,
) -> Result<Vec<Constraint>, &'static str> {
    let point = destination::resolve(sketch, destination, additions, accepted.point)?;
    persist_reference(sketch, accepted, current, point, next_id)
}

fn persist_reference(
    sketch: &Sketch,
    accepted: &AcceptedSnap,
    current: SnapSource,
    point: (EntityId, CurvePosition),
    next_id: &mut impl FnMut() -> ConstraintId,
) -> Result<Vec<Constraint>, &'static str> {
    if accepted.source != current || current.sketch != sketch.id || !accepted.point.is_finite() {
        return Err("Snap intent belongs to a stale capture or source generation");
    }
    if sketch.contains_entity(point.0) {
        return Err("Snap placement requires a fresh point identity");
    }
    let constraints = build_constraints(sketch, accepted.intent, point, next_id)?;
    let mut ids = std::collections::BTreeSet::new();
    for constraint in &constraints {
        if sketch.get_constraint(constraint.id()).is_some() || !ids.insert(constraint.id()) {
            return Err("Snap constraint identities must be fresh and unique");
        }
    }
    Ok(constraints)
}

fn point_ref(
    sketch: &Sketch,
    target: SketchTarget,
) -> Result<(EntityId, CurvePosition), &'static str> {
    target
        .constraint_point(sketch)
        .ok_or("Snap point reference is no longer available")
}

fn curve(sketch: &Sketch, id: EntityId) -> Result<EntityId, &'static str> {
    matches!(
        sketch.get_entity(id),
        Some(SketchEntity::Line { .. } | SketchEntity::Circle { .. } | SketchEntity::Arc { .. })
    )
    .then_some(id)
    .ok_or("Snap curve reference is unsupported or unavailable")
}

fn on_curve(point: (EntityId, CurvePosition), curve: EntityId, id: ConstraintId) -> Constraint {
    Constraint::OnCurve {
        id,
        point: point.0,
        point_position: point.1,
        curve,
        position: CurvePosition::Arbitrary,
    }
}

fn alignment(
    point: (EntityId, CurvePosition),
    reference: (EntityId, CurvePosition),
    axis: PointAlignment,
    id: ConstraintId,
) -> Constraint {
    let (point2, point2_position) = reference;
    match axis {
        PointAlignment::Horizontal => Constraint::HorizontalPoints {
            id,
            point1: point.0,
            point1_position: point.1,
            point2,
            point2_position,
        },
        PointAlignment::Vertical => Constraint::VerticalPoints {
            id,
            point1: point.0,
            point1_position: point.1,
            point2,
            point2_position,
        },
    }
}

fn build_constraints(
    sketch: &Sketch,
    intent: SnapIntent,
    point: (EntityId, CurvePosition),
    next: &mut impl FnMut() -> ConstraintId,
) -> Result<Vec<Constraint>, &'static str> {
    Ok(match intent {
        SnapIntent::GeometryOnly => vec![],
        SnapIntent::Origin => vec![Constraint::Fixed {
            id: next(),
            point: point.0,
            point_position: point.1,
            at: Vec2::new_unchecked(0.0, 0.0),
        }],
        SnapIntent::Coincident(target) => {
            let (point2, point2_position) = point_ref(sketch, target)?;
            vec![Constraint::Coincident {
                id: next(),
                point1: point.0,
                point1_position: point.1,
                point2,
                point2_position,
            }]
        }
        SnapIntent::Midpoint(line) => {
            if !matches!(sketch.get_entity(line), Some(SketchEntity::Line { .. })) {
                return Err("Midpoint snap requires a current line");
            }
            vec![Constraint::Midpoint {
                id: next(),
                point: point.0,
                point_position: point.1,
                line,
            }]
        }
        SnapIntent::OnCurve(id) => vec![on_curve(point, curve(sketch, id)?, next())],
        SnapIntent::Intersection { first, second } => {
            if first == second {
                return Err("Intersection snap requires two distinct curves");
            }
            vec![
                on_curve(point, curve(sketch, first)?, next()),
                on_curve(point, curve(sketch, second)?, next()),
            ]
        }
        SnapIntent::Quadrant {
            curve: owner,
            center,
            alignment: axis,
        } => {
            let reference = point_ref(sketch, center)?;
            if center
                != (SketchTarget::Point {
                    entity: owner,
                    role: CurvePosition::Center,
                })
            {
                return Err("Quadrant center must belong to its curve");
            }
            vec![
                on_curve(point, curve(sketch, owner)?, next()),
                alignment(point, reference, axis, next()),
            ]
        }
        SnapIntent::Guide {
            reference,
            alignment: axis,
        } => vec![alignment(
            point,
            point_ref(sketch, reference)?,
            axis,
            next(),
        )],
        SnapIntent::Guides {
            horizontal,
            vertical,
        } => vec![
            alignment(
                point,
                point_ref(sketch, horizontal)?,
                PointAlignment::Horizontal,
                next(),
            ),
            alignment(
                point,
                point_ref(sketch, vertical)?,
                PointAlignment::Vertical,
                next(),
            ),
        ],
    })
}

pub fn source_for(sketch: &Sketch, capture: CaptureIdentity, generation: u64) -> SnapSource {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    struct HashWriter<'a>(&'a mut std::collections::hash_map::DefaultHasher);
    impl std::io::Write for HashWriter<'_> {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.write(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    if serde_json::to_writer(HashWriter(&mut hasher), sketch).is_err() {
        0_u8.hash(&mut hasher);
    }
    SnapSource {
        sketch: sketch.id,
        capture,
        generation,
        geometry_fingerprint: hasher.finish(),
    }
}
