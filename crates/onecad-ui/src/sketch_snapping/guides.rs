use super::*;

const POLAR_MIN_RADIUS: f32 = 24.0;
/// Matches the renderer, which fades minor grid lines once cells shrink below ~8 px.
const MIN_VISIBLE_GRID_CELL: f32 = 8.0;

fn visible_step(request: &SnapRequest<'_>, raw: Vec2, minor: f64) -> f64 {
    let cell = Vec2::new(raw.x + minor, raw.y)
        .and_then(|next| Some(project(request, raw)?.distance(project(request, next)?)));
    if cell.is_some_and(|cell| cell < MIN_VISIBLE_GRID_CELL) {
        minor * 10.0
    } else {
        minor
    }
}

pub(super) fn guides_and_grid(collector: &mut Collector<'_, '_>) -> Option<()> {
    let request = collector.request;
    let logical = onecad_scene::DVec2::new(
        f64::from(request.pointer.x - request.rect.min.x),
        f64::from(request.pointer.y - request.rect.min.y),
    );
    let world = onecad_interaction::sketch::intersect_sketch_plane(
        request.plane,
        request.view.ray(logical).ok()?,
    )?;
    let raw = onecad_interaction::sketch::sketch_plane_coordinates(request.plane, world)?;
    if let Some(step) = request
        .settings
        .grid_step
        .filter(|step| step.is_finite() && *step > 0.0)
    {
        let step = visible_step(request, raw, step);
        collector.push(
            Vec2::new((raw.x / step).round() * step, (raw.y / step).round() * step)?,
            SnapKind::Grid,
            SnapIntent::GeometryOnly,
        )?;
    }
    if let Some((anchor, reference)) = request.anchor {
        add_guides(collector, raw, anchor, reference)?;
        polar(collector, raw, anchor)?;
    }
    for entity in request.sketch.entities() {
        let roles: &[CurvePosition] = match entity {
            SketchEntity::Point { .. } => &[CurvePosition::Arbitrary],
            SketchEntity::Arc { .. } => &[CurvePosition::Start, CurvePosition::End],
            _ => &[],
        };
        for role in roles {
            let reference = target(entity.id(), *role);
            let at = reference.point_coordinates(request.sketch)?;
            if project(request, at).is_none_or(|screen| !request.rect.contains(screen)) {
                continue;
            }
            add_guides(collector, raw, at, Some(reference))?;
        }
    }
    if let (Some(horizontal), Some(vertical)) = (
        best_guide(collector, SnapKind::HorizontalGuide),
        best_guide(collector, SnapKind::VerticalGuide),
    ) {
        let (
            SnapIntent::Guide {
                reference: horizontal_ref,
                ..
            },
            SnapIntent::Guide {
                reference: vertical_ref,
                ..
            },
        ) = (horizontal.intent, vertical.intent)
        else {
            return Some(());
        };
        collector.push(
            Vec2::new(vertical.point.x, horizontal.point.y)?,
            SnapKind::Guides,
            SnapIntent::Guides {
                horizontal: horizontal_ref,
                vertical: vertical_ref,
            },
        )?;
    }
    Some(())
}

fn add_guides(
    collector: &mut Collector<'_, '_>,
    raw: Vec2,
    at: Vec2,
    reference: Option<SketchTarget>,
) -> Option<()> {
    for (value, kind, alignment) in [
        (
            Vec2::new(raw.x, at.y)?,
            SnapKind::HorizontalGuide,
            PointAlignment::Horizontal,
        ),
        (
            Vec2::new(at.x, raw.y)?,
            SnapKind::VerticalGuide,
            PointAlignment::Vertical,
        ),
    ] {
        let intent = reference.map_or(SnapIntent::GeometryOnly, |reference| SnapIntent::Guide {
            reference,
            alignment,
        });
        collector.push(value, kind, intent)?;
    }
    Some(())
}

fn best_guide(collector: &Collector<'_, '_>, kind: SnapKind) -> Option<AcceptedSnap> {
    let sketch = collector.request.sketch;
    collector
        .candidates
        .iter()
        .filter(|candidate| candidate.snap.kind == kind)
        .min_by(|a, b| super::super::ranked(a, b, a.distance, b.distance, sketch))
        .map(|best| best.snap)
}

fn polar(collector: &mut Collector<'_, '_>, raw: Vec2, anchor: Vec2) -> Option<()> {
    if let Some(step) = collector
        .request
        .settings
        .polar_step_degrees
        .filter(|step| step.is_finite() && (1.0..=180.0).contains(step))
    {
        // Near the anchor every direction lies within the aperture of some polar step.
        let request = collector.request;
        if let (Some(from), Some(to)) = (project(request, anchor), project(request, raw))
            && from.distance(to) < POLAR_MIN_RADIUS
        {
            return Some(());
        }
        let delta = vector(raw) - vector(anchor);
        let radians = step.to_radians();
        let angle = (delta.y.atan2(delta.x) / radians).round() * radians;
        collector.push(
            point(
                vector(anchor)
                    + onecad_scene::DVec2::new(angle.cos(), angle.sin()) * delta.length(),
            )?,
            SnapKind::Polar,
            SnapIntent::GeometryOnly,
        )?;
    }
    Some(())
}
