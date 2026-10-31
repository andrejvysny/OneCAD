//! Inference lines and their pairwise crossings.
//!
//! A cursor resting on one inference line keeps snapping to anything crossing it:
//! another guide, a curve, a polar ray, an extension or a grid line.
use super::*;
use crate::sketch_snapping::curves::{Shape, arc_sweep, point, raw_point};
use onecad_scene::DVec2;
use std::f64::consts::TAU;

/// Bounds the pairwise crossing search per frame.
const MAX_CROSSING_LINES: usize = 16;
const MAX_EXTENSIONS: usize = 8;
/// Rays start just past the curve end so an extension never re-finds its own endpoint.
const EXTENSION_START: f64 = 1e-6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::sketch_snapping) enum LocusId {
    Guide {
        reference: Option<SketchTarget>,
        alignment: PointAlignment,
    },
    Polar(i64),
    Extension {
        curve: EntityId,
        role: CurvePosition,
    },
    Curve(EntityId),
    GridX(u64),
    GridY(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Class {
    Guide(PointAlignment),
    Polar,
    Extension,
    Grid,
    Curve,
}

/// The constraint a line contributes when a snap on it is accepted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Relation {
    None,
    Align {
        reference: SketchTarget,
        alignment: PointAlignment,
    },
    OnCurve(EntityId),
}

impl Relation {
    fn intent(self) -> SnapIntent {
        match self {
            Self::None => SnapIntent::GeometryOnly,
            Self::Align {
                reference,
                alignment,
            } => SnapIntent::Guide {
                reference,
                alignment,
            },
            Self::OnCurve(curve) => SnapIntent::OnCurve(curve),
        }
    }
}

/// Persisted intent of a crossing, or `None` when the pair cannot be represented.
fn combine(a: Relation, b: Relation) -> Option<SnapIntent> {
    Some(match (a, b) {
        (Relation::None, other) | (other, Relation::None) => other.intent(),
        (
            Relation::Align {
                reference: first,
                alignment: PointAlignment::Horizontal,
            },
            Relation::Align {
                reference: second,
                alignment: PointAlignment::Vertical,
            },
        )
        | (
            Relation::Align {
                reference: second,
                alignment: PointAlignment::Vertical,
            },
            Relation::Align {
                reference: first,
                alignment: PointAlignment::Horizontal,
            },
        ) => SnapIntent::Guides {
            horizontal: first,
            vertical: second,
        },
        (
            Relation::Align {
                reference,
                alignment,
            },
            Relation::OnCurve(curve),
        )
        | (
            Relation::OnCurve(curve),
            Relation::Align {
                reference,
                alignment,
            },
        ) => SnapIntent::GuideOnCurve {
            reference,
            alignment,
            curve,
        },
        (Relation::OnCurve(first), Relation::OnCurve(second)) if first != second => {
            SnapIntent::Intersection { first, second }
        }
        _ => return None,
    })
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Locus {
    pub(super) id: LocusId,
    pub(super) class: Class,
    pub(super) shape: Shape,
    pub(super) relation: Relation,
    pub(super) guide: Option<SnapGuide>,
}

impl Locus {
    fn single_kind(&self) -> Option<SnapKind> {
        Some(match self.class {
            Class::Guide(PointAlignment::Horizontal) => SnapKind::HorizontalGuide,
            Class::Guide(PointAlignment::Vertical) => SnapKind::VerticalGuide,
            Class::Polar => SnapKind::Polar,
            Class::Extension => SnapKind::Extension,
            Class::Grid | Class::Curve => return None,
        })
    }
    fn is_virtual(&self) -> bool {
        !matches!(self.class, Class::Grid | Class::Curve)
    }
    fn same_source(&self, other: &Self) -> bool {
        matches!(
            (self.id, other.id),
            (LocusId::Guide { reference: a, .. }, LocusId::Guide { reference: b, .. }) if a == b
        )
    }
}

/// Pushes every single inference line in reach, then crossings between nearby lines.
pub(super) fn resolve_loci(
    collector: &mut Collector<'_, '_>,
    raw: Vec2,
    loci: &[Locus],
) -> Option<()> {
    let request = collector.request;
    let reach = request.settings.tolerance + request.settings.hysteresis;
    let mut near: Vec<(f64, usize)> = Vec::new();
    for (index, locus) in loci.iter().enumerate() {
        let Some(kind) = locus.single_kind() else {
            continue;
        };
        let Some(foot) = locus.shape.project(vector(raw)).and_then(point) else {
            continue;
        };
        let Some(distance) =
            project(request, foot).map(|at| f64::from(at.distance(request.pointer)))
        else {
            continue;
        };
        if distance > reach {
            continue;
        }
        collector.push_with(
            foot,
            kind,
            locus.relation.intent(),
            Extras {
                priority: None,
                loci: [Some(locus.id), None],
                guides: [locus.guide, None],
            },
        )?;
        near.push((distance, index));
    }
    near.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    let mut lines: Vec<&Locus> = Vec::new();
    for (_, index) in near {
        if lines.len() < MAX_CROSSING_LINES && !lines.iter().any(|l| l.id == loci[index].id) {
            lines.push(&loci[index]);
        }
    }
    lines.extend(loci.iter().filter(|locus| !locus.is_virtual()));
    crossings(collector, &lines)
}

fn crossings(collector: &mut Collector<'_, '_>, lines: &[&Locus]) -> Option<()> {
    let anchor = collector.request.anchor.map(|(anchor, _)| vector(anchor));
    for (index, a) in lines.iter().enumerate() {
        for b in &lines[index + 1..] {
            // Curve and grid crossings among themselves are already points of their own.
            if !(a.is_virtual() || b.is_virtual()) || a.same_source(b) {
                continue;
            }
            let Some(intent) = combine(a.relation, b.relation) else {
                continue;
            };
            let classes = [a.class, b.class];
            let (kind, priority) = if classes.contains(&Class::Curve) {
                (SnapKind::GuideIntersection, 10)
            } else if classes.contains(&Class::Grid) {
                (SnapKind::GuideIntersection, 45)
            } else if matches!(classes, [Class::Guide(_), Class::Guide(_)]) {
                (SnapKind::Guides, 20)
            } else {
                (SnapKind::GuideIntersection, 20)
            };
            for value in intersections(a.shape, b.shape) {
                // Lines from the anchor all meet there; that is not a new target.
                if anchor.is_some_and(|anchor| (vector(value) - anchor).length() < 1e-9) {
                    continue;
                }
                collector.push_with(
                    value,
                    kind,
                    intent,
                    Extras {
                        priority: Some(priority),
                        loci: [Some(a.id), Some(b.id)],
                        guides: [a.guide, b.guide],
                    },
                )?;
            }
        }
    }
    Some(())
}

/// Extensions of curves ending at acquired endpoints and at the anchor's reference.
pub(super) fn extension_loci(request: &SnapRequest<'_>, loci: &mut Vec<Locus>) {
    let sources = request
        .extensions
        .iter()
        .copied()
        .chain(request.anchor.and_then(|(_, reference)| reference));
    let mut added = 0;
    for target in sources {
        for (curve, role) in incident(request.sketch, target) {
            let id = LocusId::Extension { curve, role };
            if added >= MAX_EXTENSIONS || loci.iter().any(|locus| locus.id == id) {
                continue;
            }
            if let Some(locus) = extension(request.sketch, curve, role) {
                loci.push(locus);
                added += 1;
            }
        }
    }
}

/// Curve ends meeting at `target`: lines sharing a point entity, or a curve's own end role.
fn incident(sketch: &Sketch, target: SketchTarget) -> Vec<(EntityId, CurvePosition)> {
    let SketchTarget::Point { entity, role } = target else {
        return Vec::new();
    };
    match sketch.get_entity(entity) {
        Some(SketchEntity::Point { .. }) => sketch
            .entities()
            .iter()
            .filter_map(|curve| match curve {
                SketchEntity::Line { id, start, .. } if *start == entity => {
                    Some((*id, CurvePosition::Start))
                }
                SketchEntity::Line { id, end, .. } if *end == entity => {
                    Some((*id, CurvePosition::End))
                }
                _ => None,
            })
            .collect(),
        Some(SketchEntity::Line { .. } | SketchEntity::Arc { .. })
            if matches!(role, CurvePosition::Start | CurvePosition::End) =>
        {
            vec![(entity, role)]
        }
        _ => Vec::new(),
    }
}

fn extension(sketch: &Sketch, curve: EntityId, role: CurvePosition) -> Option<Locus> {
    let forward = role == CurvePosition::End;
    let (shape, guide) = match sketch.get_entity(curve)? {
        SketchEntity::Line { start, end, .. } => {
            let (a, b) = (raw_point(sketch, *start)?, raw_point(sketch, *end)?);
            let (origin, dir) = if forward { (b, b - a) } else { (a, a - b) };
            let shape = Shape::Line {
                origin,
                dir,
                range: (EXTENSION_START, f64::INFINITY),
            };
            (shape, GuideShape::From(point(origin)?))
        }
        SketchEntity::Arc {
            center,
            radius,
            start_angle,
            end_angle,
            ..
        } if *radius > 0.0 => arc_extension(
            raw_point(sketch, *center)?,
            *radius,
            *start_angle,
            arc_sweep(*start_angle, *end_angle),
            forward,
        )?,
        _ => return None,
    };
    Some(Locus {
        id: LocusId::Extension { curve, role },
        class: Class::Extension,
        shape,
        relation: Relation::OnCurve(curve),
        guide: Some(SnapGuide {
            kind: GuideKind::Extension,
            shape: guide,
            persisted: true,
        }),
    })
}

/// The rest of the arc's circle, drawn from the chosen end.
fn arc_extension(
    center: DVec2,
    radius: f64,
    start: f64,
    sweep: f64,
    forward: bool,
) -> Option<(Shape, GuideShape)> {
    if sweep >= TAU - 1e-10 {
        return None;
    }
    let shape = Shape::Circle {
        center,
        radius,
        start: start + sweep,
        sweep: TAU - sweep,
    };
    let guide = GuideShape::Arc {
        center: point(center)?,
        radius,
        from_angle: if forward { start + sweep } else { start },
        forward,
    };
    Some((shape, guide))
}
