use super::curves::{Curve, intersections, point, vector};
use super::*;
use onecad_core::{
    ids::EntityId,
    sketch::{CurvePosition, SketchEntity},
};
use onecad_interaction::sketch::inference::{PointAlignment, SnapIntent, SnapKind};
use std::f64::consts::FRAC_PI_2;

/// Bounds the pairwise intersection search per frame.
const MAX_NEARBY_CURVES: usize = 32;

pub(super) struct Candidate {
    pub(super) snap: AcceptedSnap,
    pub(super) distance: f64,
    /// Collection order follows sketch entity order, so ties resolve reproducibly.
    order: usize,
}

impl Candidate {
    pub(super) fn score(&self) -> f64 {
        let priority = match self.snap.kind {
            SnapKind::Endpoint => 0,
            SnapKind::Origin => 1,
            SnapKind::Center => 2,
            SnapKind::Intersection => 3,
            SnapKind::Midpoint => 4,
            SnapKind::Quadrant => 5,
            SnapKind::OnCurve => 35,
            SnapKind::Guides => 50,
            SnapKind::HorizontalGuide | SnapKind::VerticalGuide => 60,
            SnapKind::Polar => 70,
            SnapKind::Grid => 80,
        };
        self.distance + f64::from(priority) * 0.35
    }
    /// Equal scores are common at shared corners and aligned points; dropping the snap
    /// there hides exactly the inference users aim for, so order them instead.
    pub(super) fn tiebreak(&self, other: &Self, sketch: &Sketch) -> std::cmp::Ordering {
        let anonymous = |candidate: &Self| candidate.snap.intent == SnapIntent::GeometryOnly;
        anonymous(self)
            .cmp(&anonymous(other))
            .then_with(|| {
                self.reference_distance(sketch)
                    .total_cmp(&other.reference_distance(sketch))
            })
            .then_with(|| self.order.cmp(&other.order))
    }
    fn reference_distance(&self, sketch: &Sketch) -> f64 {
        let distance = |target: SketchTarget| {
            target
                .point_coordinates(sketch)
                .map_or(f64::INFINITY, |at| {
                    (at.x - self.snap.point.x).hypot(at.y - self.snap.point.y)
                })
        };
        match self.snap.intent {
            SnapIntent::Guide { reference, .. } => distance(reference),
            SnapIntent::Guides {
                horizontal,
                vertical,
            } => distance(horizontal) + distance(vertical),
            SnapIntent::Quadrant { center, .. } => distance(center),
            _ => 0.0,
        }
    }
    pub(super) fn same_target(&self, previous: &AcceptedSnap) -> bool {
        self.snap.kind == previous.kind
            && self.snap.intent == previous.intent
            && (matches!(
                self.snap.kind,
                SnapKind::OnCurve
                    | SnapKind::HorizontalGuide
                    | SnapKind::VerticalGuide
                    | SnapKind::Guides
                    | SnapKind::Polar
            ) || self.snap.point == previous.point)
    }
}

struct Collector<'a, 'b> {
    request: &'a SnapRequest<'b>,
    source: SnapSource,
    candidates: Vec<Candidate>,
    order: usize,
    centers: std::collections::BTreeSet<EntityId>,
}

fn farthest<T>(items: &[T], distance: impl Fn(&T) -> f64) -> Option<usize> {
    items
        .iter()
        .enumerate()
        .max_by(|a, b| distance(a.1).total_cmp(&distance(b.1)))
        .map(|(index, _)| index)
}
impl Collector<'_, '_> {
    fn push(&mut self, value: Vec2, kind: SnapKind, intent: SnapIntent) -> Option<()> {
        let Some(screen) = project(self.request, value) else {
            return Some(());
        };
        let distance = f64::from(screen.distance(self.request.pointer));
        if distance <= self.request.settings.tolerance + self.request.settings.hysteresis {
            let snap = AcceptedSnap {
                source: self.source,
                kind,
                point: value,
                intent,
            };
            if self
                .candidates
                .iter()
                .any(|candidate| candidate.snap == snap)
            {
                return Some(());
            }
            let candidate = Candidate {
                snap,
                distance,
                order: self.order,
            };
            self.order += 1;
            if self.candidates.len() < MAX_SNAP_CANDIDATES {
                self.candidates.push(candidate);
            } else if let Some(farthest) = farthest(&self.candidates, |c| c.distance)
                && distance < self.candidates[farthest].distance
            {
                // Dense views keep the nearest targets instead of losing snapping entirely.
                self.candidates[farthest] = candidate;
            }
        }
        Some(())
    }
    fn point(&mut self, target: SketchTarget, kind: SnapKind) -> Option<()> {
        let value = target.point_coordinates(self.request.sketch)?;
        let (entity, role) = target.constraint_point(self.request.sketch)?;
        self.push(
            value,
            kind,
            SnapIntent::Coincident(SketchTarget::Point { entity, role }),
        )
    }
}

pub(super) fn collect_candidates(
    request: &SnapRequest<'_>,
    source: SnapSource,
) -> Option<Vec<Candidate>> {
    let mut collector = Collector {
        request,
        source,
        candidates: Vec::new(),
        order: 0,
        centers: request
            .sketch
            .entities()
            .iter()
            .filter_map(|entity| match entity {
                SketchEntity::Circle { center, .. }
                | SketchEntity::Arc { center, .. }
                | SketchEntity::Ellipse { center, .. } => Some(*center),
                _ => None,
            })
            .collect(),
    };
    collector.push(
        Vec2::new_unchecked(0.0, 0.0),
        SnapKind::Origin,
        SnapIntent::Origin,
    )?;
    let mut nearby: Vec<(f64, Curve)> = Vec::new();
    for entity in request.sketch.entities() {
        entity_candidates(&mut collector, entity)?;
        curve_candidates(&mut collector, entity, &mut nearby)?;
    }
    let nearby: Vec<Curve> = nearby.into_iter().map(|(_, curve)| curve).collect();
    for (index, a) in nearby.iter().enumerate() {
        for b in &nearby[index + 1..] {
            for value in intersections(*a, *b) {
                collector.push(
                    value,
                    SnapKind::Intersection,
                    SnapIntent::Intersection {
                        first: a.id(),
                        second: b.id(),
                    },
                )?;
            }
        }
    }
    guides_and_grid(&mut collector)?;
    Some(collector.candidates)
}

fn curve_candidates(
    collector: &mut Collector<'_, '_>,
    entity: &SketchEntity,
    nearby: &mut Vec<(f64, Curve)>,
) -> Option<()> {
    let request = collector.request;
    let Some(curve) = Curve::from_entity(request.sketch, entity) else {
        return Some(());
    };
    let Some(value) = curve.nearest(request) else {
        return Some(());
    };
    let Some(screen) = project(request, value) else {
        return Some(());
    };
    let distance = f64::from(screen.distance(request.pointer));
    if distance <= request.settings.tolerance + request.settings.hysteresis {
        if nearby.len() < MAX_NEARBY_CURVES {
            nearby.push((distance, curve));
        } else if let Some(farthest) = farthest(nearby, |(distance, _)| *distance)
            && distance < nearby[farthest].0
        {
            nearby[farthest] = (distance, curve);
        }
        collector.push(value, SnapKind::OnCurve, SnapIntent::OnCurve(curve.id()))?;
    }
    Some(())
}

fn target(entity: EntityId, role: CurvePosition) -> SketchTarget {
    SketchTarget::Point { entity, role }
}

fn entity_candidates(collector: &mut Collector<'_, '_>, entity: &SketchEntity) -> Option<()> {
    match entity {
        SketchEntity::Point { id, .. } => {
            if !collector.centers.contains(id) {
                collector.point(target(*id, CurvePosition::Arbitrary), SnapKind::Endpoint)?;
            }
        }
        SketchEntity::Line { id, .. } => {
            collector.point(target(*id, CurvePosition::Start), SnapKind::Endpoint)?;
            collector.point(target(*id, CurvePosition::End), SnapKind::Endpoint)?;
            let a =
                target(*id, CurvePosition::Start).point_coordinates(collector.request.sketch)?;
            let b = target(*id, CurvePosition::End).point_coordinates(collector.request.sketch)?;
            collector.push(
                Vec2::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0)?,
                SnapKind::Midpoint,
                SnapIntent::Midpoint(*id),
            )?;
        }
        SketchEntity::Circle { id, radius, .. } | SketchEntity::Arc { id, radius, .. } => {
            circle_candidates(collector, *id, *radius)?
        }
        SketchEntity::Ellipse { id, .. } => {
            collector.point(target(*id, CurvePosition::Center), SnapKind::Center)?;
        }
    }
    Some(())
}

fn circle_candidates(collector: &mut Collector<'_, '_>, id: EntityId, radius: f64) -> Option<()> {
    let center = target(id, CurvePosition::Center);
    collector.point(center, SnapKind::Center)?;
    if matches!(
        collector.request.sketch.get_entity(id),
        Some(SketchEntity::Arc { .. })
    ) {
        collector.point(target(id, CurvePosition::Start), SnapKind::Endpoint)?;
        collector.point(target(id, CurvePosition::End), SnapKind::Endpoint)?;
    }
    let at = center.point_coordinates(collector.request.sketch)?;
    let curve = Curve::from_entity(
        collector.request.sketch,
        collector.request.sketch.get_entity(id)?,
    )?;
    for quadrant in 0..4 {
        let angle = f64::from(quadrant) * FRAC_PI_2;
        let value = Vec2::new(at.x + radius * angle.cos(), at.y + radius * angle.sin())?;
        if !curve.contains(vector(value)) {
            continue;
        }
        collector.push(
            value,
            SnapKind::Quadrant,
            SnapIntent::Quadrant {
                curve: id,
                center,
                alignment: if quadrant % 2 == 0 {
                    PointAlignment::Horizontal
                } else {
                    PointAlignment::Vertical
                },
            },
        )?;
    }
    Some(())
}

#[path = "guides.rs"]
mod guides;
use guides::guides_and_grid;
