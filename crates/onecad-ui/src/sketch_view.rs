//! Sketch overlay and screen-space picks over one immutable scene capture.
//! This module never changes a sketch, runs a solver, or allocates entity ids.

use crate::theme::Theme;
use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke};
use onecad_core::{
    ids::EntityId,
    math::Vec2,
    sketch::{Sketch, SketchEntity, SketchPlane},
};
use onecad_interaction::sketch::{DraftPrimitive, SketchPhase, SketchSession};
use onecad_scene::{DVec3, ViewSnapshot};
use std::f64::consts::{PI, TAU};

pub const DEFAULT_PICK_RADIUS: f32 = 8.0;
const MAX_CURVE_SEGMENTS: usize = 512;

pub(crate) fn paint_selected(
    painter: &Painter,
    rect: Rect,
    view: &ViewSnapshot,
    sketch: &Sketch,
    color: Color32,
) {
    let plane = sketch.effective_plane();
    let painter = painter.with_clip_rect(rect);
    for entity in sketch.entities() {
        if let SketchEntity::Point { at, .. } = entity {
            if let Some(screen) = project(rect, view, plane, *at) {
                painter.circle_filled(screen, 3.0, color);
            }
        } else if let Some(points) = entity_points(sketch, entity, rect, view, plane) {
            paint_path(
                &painter,
                rect,
                view,
                plane,
                &points,
                Stroke::new(1.5, color),
            );
        }
    }
}

fn segment_distance(point: Pos2, a: Pos2, b: Pos2) -> f32 {
    let edge = b - a;
    let length = edge.length_sq();
    if length == 0.0 {
        return point.distance(a);
    }
    let t = ((point - a).dot(edge) / length).clamp(0.0, 1.0);
    point.distance(a + edge * t)
}

fn matches_rect(rect: Rect, view: &ViewSnapshot) -> bool {
    rect.is_finite()
        && rect.is_positive()
        && (f64::from(rect.width()) - view.viewport().width).abs() < 0.001
        && (f64::from(rect.height()) - view.viewport().height).abs() < 0.001
}

fn project(rect: Rect, view: &ViewSnapshot, plane: SketchPlane, point: Vec2) -> Option<Pos2> {
    let world = plane.to_world(point);
    let projected = view.project(DVec3::new(world.x, world.y, world.z))?;
    let position = rect.min + egui::vec2(projected.logical.x as f32, projected.logical.y as f32);
    (position.x.is_finite() && position.y.is_finite()).then_some(position)
}

fn paint_path(
    painter: &Painter,
    rect: Rect,
    view: &ViewSnapshot,
    plane: SketchPlane,
    points: &[Vec2],
    stroke: Stroke,
) {
    for pair in points.windows(2) {
        if let (Some(a), Some(b)) = (
            project(rect, view, plane, pair[0]),
            project(rect, view, plane, pair[1]),
        ) {
            painter.line_segment([a, b], stroke);
        }
    }
}

fn point(sketch: &Sketch, id: EntityId) -> Option<Vec2> {
    match sketch.get_entity(id) {
        Some(SketchEntity::Point { at, .. }) => Some(*at),
        _ => None,
    }
}

fn entity_points(
    sketch: &Sketch,
    entity: &SketchEntity,
    rect: Rect,
    view: &ViewSnapshot,
    plane: SketchPlane,
) -> Option<Vec<Vec2>> {
    let curve = match entity {
        SketchEntity::Point { .. } => return None,
        SketchEntity::Line { start, end, .. } => {
            return Some(vec![point(sketch, *start)?, point(sketch, *end)?]);
        }
        SketchEntity::Circle { center, radius, .. } => {
            Curve::circle(point(sketch, *center)?, *radius)
        }
        SketchEntity::Arc {
            center,
            radius,
            start_angle,
            end_angle,
            ..
        } => Curve::arc(point(sketch, *center)?, *radius, *start_angle, *end_angle),
        SketchEntity::Ellipse {
            center,
            major_r,
            minor_r,
            rotation,
            ..
        } => Curve {
            center: point(sketch, *center)?,
            axes: [*major_r, *minor_r],
            rotation: *rotation,
            start: 0.0,
            sweep: TAU,
        },
    };
    curve.sample(rect, view, plane)
}

fn draft_points(
    draft: &DraftPrimitive,
    rect: Rect,
    view: &ViewSnapshot,
    plane: SketchPlane,
) -> Vec<Vec2> {
    match draft {
        DraftPrimitive::Point { at } => vec![*at],
        DraftPrimitive::Line { start, end } => vec![*start, *end],
        DraftPrimitive::Rectangle {
            first: a,
            opposite: b,
        } => vec![
            *a,
            Vec2::new_unchecked(b.x, a.y),
            *b,
            Vec2::new_unchecked(a.x, b.y),
            *a,
        ],
        DraftPrimitive::Circle { center, radius } => Curve::circle(*center, *radius)
            .sample(rect, view, plane)
            .unwrap_or_default(),
        DraftPrimitive::Arc { arc } => {
            Curve::arc(arc.center, arc.radius, arc.start_angle, arc.end_angle)
                .sample(rect, view, plane)
                .unwrap_or_default()
        }
        DraftPrimitive::Slot {
            center_start,
            center_end,
            radius,
        } => {
            slot_points(*center_start, *center_end, *radius, rect, view, plane).unwrap_or_default()
        }
    }
}

fn slot_points(
    start: Vec2,
    end: Vec2,
    radius: f64,
    rect: Rect,
    view: &ViewSnapshot,
    plane: SketchPlane,
) -> Option<Vec<Vec2>> {
    let length = (end.x - start.x).hypot(end.y - start.y);
    if !length.is_finite() || length <= 0.0 || !radius.is_finite() || radius <= 0.0 {
        return None;
    }
    let angle = (end.y - start.y).atan2(end.x - start.x) + PI * 0.5;
    let mut points = Curve::arc(start, radius, angle, angle + PI).sample(rect, view, plane)?;
    points.extend(Curve::arc(end, radius, angle + PI, angle + TAU).sample(rect, view, plane)?);
    points.push(*points.first()?);
    Some(points)
}

struct Curve {
    center: Vec2,
    axes: [f64; 2],
    rotation: f64,
    start: f64,
    sweep: f64,
}

impl Curve {
    fn arc(center: Vec2, radius: f64, start: f64, end: f64) -> Self {
        let delta = end - start;
        Self {
            center,
            axes: [radius, radius],
            rotation: 0.0,
            start,
            sweep: if delta.abs() >= TAU {
                TAU
            } else {
                delta.rem_euclid(TAU)
            },
        }
    }

    fn circle(center: Vec2, radius: f64) -> Self {
        Self {
            center,
            axes: [radius, radius],
            rotation: 0.0,
            start: 0.0,
            sweep: TAU,
        }
    }

    fn at(&self, angle: f64) -> Option<Vec2> {
        let (sin, cos) = angle.sin_cos();
        let (sr, cr) = self.rotation.sin_cos();
        Vec2::new(
            self.center.x + self.axes[0] * cos * cr - self.axes[1] * sin * sr,
            self.center.y + self.axes[0] * cos * sr + self.axes[1] * sin * cr,
        )
    }

    fn sample(&self, rect: Rect, view: &ViewSnapshot, plane: SketchPlane) -> Option<Vec<Vec2>> {
        if self.axes.iter().any(|r| !r.is_finite() || *r <= 0.0)
            || !self.rotation.is_finite()
            || !self.start.is_finite()
            || !self.sweep.is_finite()
        {
            return None;
        }
        let center = project(rect, view, plane, self.center);
        let radius = [0.0, PI / 2.0, PI, PI * 1.5]
            .into_iter()
            .filter_map(|angle| {
                center
                    .zip(project(rect, view, plane, self.at(angle)?))
                    .map(|(c, p)| c.distance(p))
            })
            .fold(0.0_f32, f32::max);
        // Screen-size detail heuristic; perspective error is not bounded here.
        let full = (PI * (f64::from(radius) / 0.5).sqrt())
            .ceil()
            .clamp(16.0, MAX_CURVE_SEGMENTS as f64) as usize;
        let count = ((full as f64 * self.sweep / TAU).ceil() as usize).clamp(1, MAX_CURVE_SEGMENTS);
        (0..=count)
            .map(|i| self.at(self.start + self.sweep * i as f64 / count as f64))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use onecad_core::{ids::SketchId, sketch::WorldPlane};
    use onecad_scene::{Camera, Projection, SceneIdentity, Viewport};
    use uuid::Uuid;

    fn setup() -> (Sketch, Rect, ViewSnapshot) {
        let sketch = Sketch::on_world_plane(SketchId(Uuid::from_u128(1)), "Sketch", WorldPlane::XY);
        let rect = Rect::from_min_size(egui::pos2(50.0, 80.0), egui::vec2(800.0, 600.0));
        let view = ViewSnapshot::new(
            SceneIdentity {
                session: 1,
                scene: 1,
            },
            1,
            Camera {
                offset: DVec3::new(0.0, 0.0, 100.0),
                projection: Projection::Orthographic,
                ..Default::default()
            },
            Viewport {
                width: 800.0,
                height: 600.0,
                scale_factor: 2.0,
            },
        )
        .unwrap();
        (sketch, rect, view)
    }

    #[test]
    fn point_handle_tier_wins_over_exact_curve_body() {
        let (mut sketch, rect, view) = setup();
        let id = |n| EntityId(Uuid::from_u128(n));
        for (n, x, y) in [(2, -20.0, 0.0), (3, 20.0, 0.0), (4, 0.0, 1.0)] {
            sketch
                .add_entity(SketchEntity::point(
                    id(n),
                    Vec2::new(x, y).unwrap(),
                    false,
                    false,
                ))
                .unwrap();
        }
        sketch
            .add_entity(SketchEntity::line(id(5), id(2), id(3), false))
            .unwrap();
        let position = project(rect, &view, sketch.plane, Vec2::new(0.0, 0.0).unwrap()).unwrap();
        assert_eq!(
            pick_sketch(&sketch, sketch.plane, rect, &view, position, 8.0),
            PickResult::Unique(onecad_interaction::sketch::SketchTarget::Point {
                entity: id(4),
                role: onecad_core::sketch::CurvePosition::Arbitrary
            })
        );
    }

    #[test]
    fn pick_reach_stays_in_logical_pixels_at_two_zoom_levels() {
        let (mut sketch, rect, view) = setup();
        let center = EntityId(Uuid::from_u128(2));
        let circle = EntityId(Uuid::from_u128(3));
        sketch
            .add_entity(SketchEntity::point(
                center,
                Vec2::new(0.0, 0.0).unwrap(),
                false,
                false,
            ))
            .unwrap();
        sketch
            .add_entity(SketchEntity::circle(circle, center, 10.0, false).unwrap())
            .unwrap();
        for scale in [1.0, 4.0] {
            let mut camera = view.camera();
            camera.offset *= scale;
            let view =
                ViewSnapshot::new(view.identity().scene, 2, camera, view.viewport()).unwrap();
            let edge = project(rect, &view, sketch.plane, Vec2::new(10.0, 0.0).unwrap()).unwrap();
            let origin = project(rect, &view, sketch.plane, Vec2::new(0.0, 0.0).unwrap()).unwrap();
            let outward = (edge - origin).normalized();
            assert_eq!(
                pick_sketch(
                    &sketch,
                    sketch.plane,
                    rect,
                    &view,
                    edge + outward * 7.9,
                    8.0
                ),
                PickResult::Unique(onecad_interaction::sketch::SketchTarget::Entity(circle))
            );
            assert_eq!(
                pick_sketch(
                    &sketch,
                    sketch.plane,
                    rect,
                    &view,
                    edge + outward * 8.1,
                    8.0
                ),
                PickResult::None
            );
        }
    }

    #[test]
    fn shifted_canvas_uses_same_viewport_matrix_and_rejects_old_size() {
        let (sketch, rect, view) = setup();
        let zero = project(rect, &view, sketch.plane, Vec2::new(0.0, 0.0).unwrap()).unwrap();
        assert_eq!(zero, rect.center());
        assert!(matches_rect(rect, &view));
        assert!(!matches_rect(rect.expand(1.0), &view));
    }
}

mod paint;
pub use paint::paint;
pub(crate) use paint::{hover_target, paint_draft};
mod selection;
#[cfg(test)]
use selection::pick_sketch;
pub use selection::{PickResult, marquee_targets, pick_entity};
mod selection_input;
pub(crate) use selection_input::cancel_pointer as cancel_selection_pointer;
pub use selection_input::selection_input;

mod target_chooser;
pub use target_chooser::{show_target_chooser, target_name};
