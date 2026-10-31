//! Paints the winning snap's inference guides and their type tags.
use super::project;
use crate::sketch_snapping::{GuideShape, ResolvedSnap, SnapGuide};
use crate::theme::Theme;
use egui::{FontId, Painter, Pos2, Rect, Shape, Stroke, Vec2 as ScreenVec};
use onecad_core::{math::Vec2, sketch::SketchPlane};
use onecad_scene::ViewSnapshot;

const DASH: f32 = 6.0;
const GAP: f32 = 4.0;
const ALONG_HALF_LENGTH: f32 = 48.0;
const TAG_OFFSET: f32 = 18.0;
const TAG_NUDGE: f32 = 14.0;

pub(super) struct Scene<'a> {
    pub painter: &'a Painter,
    pub rect: Rect,
    pub view: &'a ViewSnapshot,
    pub plane: SketchPlane,
    pub theme: &'a Theme,
}

pub(super) fn paint(scene: &Scene<'_>, resolved: &ResolvedSnap, position: Pos2) {
    let mut placed: Option<Rect> = None;
    for guide in resolved.guides.iter().flatten() {
        let Some(toward) = draw_guide(scene, resolved, guide, position) else {
            continue;
        };
        placed = Some(tag(scene, guide, position, toward, placed));
    }
}

/// Draws the guide and returns the unit screen direction from the snap toward its source.
fn draw_guide(
    scene: &Scene<'_>,
    resolved: &ResolvedSnap,
    guide: &SnapGuide,
    position: Pos2,
) -> Option<ScreenVec> {
    let point = resolved.snap.point;
    let stroke = Stroke::new(1.25, scene.theme.sketch_inference);
    match guide.shape {
        GuideShape::From(source) => {
            let start = project(scene.rect, scene.view, scene.plane, source)?;
            if guide.persisted {
                scene.painter.line_segment([start, position], stroke);
                scene
                    .painter
                    .circle_filled(start, 3.0, scene.theme.sketch_inference);
            } else {
                let line = ray_to_rect(start, position, scene.rect).unwrap_or([start, position]);
                dashed(scene.painter, &line, stroke);
            }
            Some(unit(start - position))
        }
        GuideShape::Along(dir) => {
            let ahead = Vec2::new_unchecked(point.x + dir.x, point.y + dir.y);
            let ahead = project(scene.rect, scene.view, scene.plane, ahead)?;
            let along = unit(ahead - position);
            let line = [
                position - along * ALONG_HALF_LENGTH,
                position + along * ALONG_HALF_LENGTH,
            ];
            dashed(scene.painter, &line, stroke);
            Some(along)
        }
        GuideShape::Arc {
            center,
            radius,
            from_angle,
            forward,
        } => {
            let angle = (point.y - center.y).atan2(point.x - center.x);
            let sweep = if forward {
                (angle - from_angle).rem_euclid(std::f64::consts::TAU)
            } else {
                -(from_angle - angle).rem_euclid(std::f64::consts::TAU)
            };
            let steps = ((sweep.abs() / (std::f64::consts::TAU / 64.0)).ceil() as usize).max(8);
            let on_circle = |a: f64| {
                let at =
                    Vec2::new_unchecked(center.x + radius * a.cos(), center.y + radius * a.sin());
                project(scene.rect, scene.view, scene.plane, at)
            };
            let points: Vec<Pos2> = (0..=steps)
                .filter_map(|i| on_circle(from_angle + sweep * i as f64 / steps as f64))
                .collect();
            if guide.persisted {
                scene.painter.add(Shape::line(points, stroke));
            } else {
                dashed(scene.painter, &points, stroke);
            }
            Some(unit(on_circle(from_angle)? - position))
        }
    }
}

fn dashed(painter: &Painter, points: &[Pos2], stroke: Stroke) {
    painter.extend(Shape::dashed_line(points, stroke, DASH, GAP));
}

fn unit(v: ScreenVec) -> ScreenVec {
    if v.length() < 1e-3 || !v.is_finite() {
        ScreenVec::X
    } else {
        v.normalized()
    }
}

/// Places a bare glyph tag near the snap, away from the viewport edge and any earlier tag.
fn tag(
    scene: &Scene<'_>,
    guide: &SnapGuide,
    position: Pos2,
    toward: ScreenVec,
    earlier: Option<Rect>,
) -> Rect {
    let label = guide.kind.label();
    let font = FontId::proportional(11.0);
    let galley =
        scene
            .painter
            .layout_no_wrap(label.clone(), font.clone(), scene.theme.sketch_inference);
    let at =
        |sign: f32| Rect::from_center_size(position + toward * (TAG_OFFSET * sign), galley.size());
    let mut bounds = at(1.0);
    if !scene.rect.contains_rect(bounds) {
        bounds = at(-1.0);
    }
    if earlier.is_some_and(|other| other.expand(2.0).intersects(bounds)) {
        let perp = egui::vec2(-toward.y, toward.x);
        bounds = bounds.translate(perp * TAG_NUDGE);
    }
    // Without a badge, a surface-coloured halo keeps the glyph legible over geometry.
    let halo = scene
        .painter
        .layout_no_wrap(label, font, scene.theme.surface);
    for offset in [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0)] {
        scene.painter.galley(
            bounds.min + egui::vec2(offset.0, offset.1),
            halo.clone(),
            scene.theme.surface,
        );
    }
    scene
        .painter
        .galley(bounds.min, galley, scene.theme.sketch_inference);
    bounds
}

/// Ray from `origin` through `through`, clipped to the viewport rect edge.
fn ray_to_rect(origin: Pos2, through: Pos2, rect: Rect) -> Option<[Pos2; 2]> {
    let dir = through - origin;
    if dir.length() < 1.0 || !rect.contains(origin) {
        return None;
    }
    let axis = |o: f32, d: f32, lo: f32, hi: f32| {
        if d > 0.0 {
            (hi - o) / d
        } else if d < 0.0 {
            (lo - o) / d
        } else {
            f32::INFINITY
        }
    };
    let t = axis(origin.x, dir.x, rect.min.x, rect.max.x)
        .min(axis(origin.y, dir.y, rect.min.y, rect.max.y));
    t.is_finite().then(|| [origin, origin + dir * t.max(1.0)])
}
