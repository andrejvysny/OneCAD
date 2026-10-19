use super::*;
use onecad_core::sketch::CurvePosition;
use onecad_interaction::sketch::{MAX_SELECTION_TARGETS, SketchTarget};

#[derive(Clone, Debug, PartialEq)]
pub enum PickResult {
    None,
    Unique(SketchTarget),
    Ambiguous(Vec<SketchTarget>),
}

pub fn pick_entity(
    rect: Rect,
    view: &ViewSnapshot,
    session: &SketchSession,
    position: Pos2,
    radius: f32,
) -> PickResult {
    if session.document.scene != view.identity().scene
        || session.phase != SketchPhase::Active
        || session.accepted_capture != Some(view.identity())
    {
        return PickResult::None;
    }
    pick_sketch(
        &session.authoritative,
        session.plane,
        rect,
        view,
        position,
        radius,
    )
}

pub(super) fn handles(entity: &SketchEntity) -> Vec<SketchTarget> {
    let roles: &[CurvePosition] = match entity {
        SketchEntity::Point { .. } => &[CurvePosition::Arbitrary],
        SketchEntity::Line { .. } => &[CurvePosition::Start, CurvePosition::End],
        SketchEntity::Arc { .. } => &[
            CurvePosition::Start,
            CurvePosition::End,
            CurvePosition::Center,
        ],
        SketchEntity::Circle { .. } | SketchEntity::Ellipse { .. } => &[CurvePosition::Center],
    };
    roles
        .iter()
        .map(|role| SketchTarget::Point {
            entity: entity.id(),
            role: *role,
        })
        .collect()
}

fn owned_points(sketch: &Sketch) -> std::collections::BTreeMap<EntityId, usize> {
    let mut owned = std::collections::BTreeMap::new();
    for entity in sketch.entities() {
        match entity {
            SketchEntity::Line { start, end, .. } => {
                *owned.entry(*start).or_default() += 1;
                *owned.entry(*end).or_default() += 1;
            }
            SketchEntity::Circle { center, .. }
            | SketchEntity::Arc { center, .. }
            | SketchEntity::Ellipse { center, .. } => {
                *owned.entry(*center).or_default() += 1;
            }
            SketchEntity::Point { .. } => {}
        }
    }
    owned
}

// Shared persisted points have one identity even when several curves expose handles.
fn normalized_handles(
    sketch: &Sketch,
    entity: &SketchEntity,
    owned: &std::collections::BTreeMap<EntityId, usize>,
) -> Vec<SketchTarget> {
    handles(entity)
        .into_iter()
        .map(|target| {
            let SketchTarget::Point { role, .. } = target else {
                return target;
            };
            let point = match (entity, role) {
                (SketchEntity::Line { start, .. }, CurvePosition::Start) => Some(*start),
                (SketchEntity::Line { end, .. }, CurvePosition::End) => Some(*end),
                (
                    SketchEntity::Circle { center, .. }
                    | SketchEntity::Arc { center, .. }
                    | SketchEntity::Ellipse { center, .. },
                    CurvePosition::Center,
                ) => Some(*center),
                _ => None,
            };
            if let Some(point) = point.filter(|point| {
                owned.get(point).copied().unwrap_or(0) > 1 && sketch.contains_entity(*point)
            }) {
                SketchTarget::Point {
                    entity: point,
                    role: CurvePosition::Arbitrary,
                }
            } else {
                target
            }
        })
        .collect()
}

pub(super) fn pick_sketch(
    sketch: &Sketch,
    plane: SketchPlane,
    rect: Rect,
    view: &ViewSnapshot,
    position: Pos2,
    radius: f32,
) -> PickResult {
    if !matches_rect(rect, view) || !rect.contains(position) || !radius.is_finite() || radius <= 0.0
    {
        return PickResult::None;
    }
    let mut points = Candidates::new(radius);
    let mut curves = Candidates::new(radius);
    let owned = owned_points(sketch);
    for entity in sketch.entities() {
        if matches!(entity, SketchEntity::Point { id, .. } if owned.contains_key(id)) {
            continue;
        }
        for target in normalized_handles(sketch, entity, &owned) {
            if let Some(screen) = target
                .point_coordinates(sketch)
                .and_then(|at| project(rect, view, plane, at))
            {
                points.consider(target, screen.distance(position));
            }
        }
        if let Some(path) = entity_points(sketch, entity, rect, view, plane) {
            let distance = path
                .windows(2)
                .filter_map(|pair| {
                    Some(segment_distance(
                        position,
                        project(rect, view, plane, pair[0])?,
                        project(rect, view, plane, pair[1])?,
                    ))
                })
                .fold(f32::INFINITY, f32::min);
            curves.consider(SketchTarget::Entity(entity.id()), distance);
        }
    }
    let points = points.result();
    if points != PickResult::None {
        points
    } else {
        curves.result()
    }
}

struct Candidates {
    nearest: f32,
    radius: f32,
    values: Vec<(SketchTarget, f32)>,
}

impl Candidates {
    fn new(radius: f32) -> Self {
        Self {
            nearest: f32::INFINITY,
            radius,
            values: Vec::new(),
        }
    }

    fn consider(&mut self, target: SketchTarget, distance: f32) {
        if !distance.is_finite() || distance > self.radius || distance > self.nearest + 1.0 {
            return;
        }
        if distance < self.nearest {
            self.nearest = distance;
            self.values
                .retain(|(_, current)| *current <= distance + 1.0);
            if self.values.len() > MAX_SELECTION_TARGETS {
                self.values.pop();
            }
        }
        if self.values.len() <= MAX_SELECTION_TARGETS
            && !self.values.iter().any(|(current, _)| *current == target)
        {
            self.values.push((target, distance));
        }
    }

    fn result(self) -> PickResult {
        let targets: Vec<_> = self.values.into_iter().map(|(target, _)| target).collect();
        match targets.as_slice() {
            [] => PickResult::None,
            [target] => PickResult::Unique(*target),
            _ => PickResult::Ambiguous(targets),
        }
    }
}

pub fn marquee_targets(
    rect: Rect,
    view: &ViewSnapshot,
    session: &SketchSession,
    start: Pos2,
    end: Pos2,
) -> Option<Vec<SketchTarget>> {
    if !matches_rect(rect, view)
        || session.document.scene != view.identity().scene
        || session.accepted_capture != Some(view.identity())
        || session.phase != SketchPhase::Active
        || !start.is_finite()
        || !end.is_finite()
    {
        return None;
    }
    let bounds = Rect::from_two_pos(start, end);
    let crossing = end.x < start.x;
    let mut targets = Vec::new();
    let owned = owned_points(&session.authoritative);
    for entity in session.authoritative.entities() {
        if matches!(entity, SketchEntity::Point { id, .. } if owned.contains_key(id)) {
            continue;
        }
        let path = entity_points(&session.authoritative, entity, rect, view, session.plane)
            .and_then(|points| {
                points
                    .into_iter()
                    .map(|at| project(rect, view, session.plane, at))
                    .collect::<Option<Vec<_>>>()
            });
        let selected = path.as_ref().is_some_and(|path| {
            if crossing {
                path.windows(2)
                    .any(|pair| touches(bounds, pair[0], pair[1]))
            } else {
                path.iter().all(|point| bounds.contains(*point))
            }
        });
        if selected {
            targets.push(SketchTarget::Entity(entity.id()));
        } else {
            for target in normalized_handles(&session.authoritative, entity, &owned) {
                if target
                    .point_coordinates(&session.authoritative)
                    .and_then(|at| project(rect, view, session.plane, at))
                    .is_some_and(|at| bounds.contains(at))
                    && !targets.contains(&target)
                {
                    targets.push(target);
                }
            }
        }
        if targets.len() > MAX_SELECTION_TARGETS {
            return None;
        }
    }
    Some(targets)
}

fn touches(rect: Rect, a: Pos2, b: Pos2) -> bool {
    if rect.contains(a) || rect.contains(b) {
        return true;
    }
    let delta = b - a;
    let mut enter = 0.0_f32;
    let mut exit = 1.0_f32;
    for (origin, direction, lower, upper) in [
        (a.x, delta.x, rect.left(), rect.right()),
        (a.y, delta.y, rect.top(), rect.bottom()),
    ] {
        if direction.abs() <= f32::EPSILON {
            if origin < lower || origin > upper {
                return false;
            }
        } else {
            let first = (lower - origin) / direction;
            let second = (upper - origin) / direction;
            enter = enter.max(first.min(second));
            exit = exit.min(first.max(second));
        }
    }
    enter <= exit
}

#[cfg(test)]
pub(super) mod tests;
