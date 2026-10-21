//! Presentation uses the same immutable support frame as sketch projection.
mod rays;

use crate::app::App;
use onecad_core::sketch::SketchPlane;
use onecad_scene::{DVec3, ViewSnapshot};

pub(crate) fn grid_visible(ctx: &egui::Context) -> bool {
    ctx.data(|data| {
        data.get_temp(egui::Id::unique("sketch-grid-visible"))
            .unwrap_or(true)
    })
}

pub(crate) fn set_grid_visible(ctx: &egui::Context, visible: bool) {
    ctx.data_mut(|data| data.insert_temp(egui::Id::unique("sketch-grid-visible"), visible));
    ctx.request_repaint();
}

pub(crate) fn grid(app: &App) -> Option<onecad_render::GridFrame> {
    let plane = if app.sketch_active() {
        app.sketch.session.as_ref().map(|session| session.plane)
    } else {
        app.ui
            .sketch_entry
            .plane
            .filter(|_| app.ui.sketch_entry.open)
    }?;
    Some(frame(plane))
}

fn frame(plane: SketchPlane) -> onecad_render::GridFrame {
    let vector = |v: onecad_core::math::Vec3| DVec3::new(v.x, v.y, v.z);
    onecad_render::GridFrame {
        origin: vector(plane.origin),
        x_axis: vector(plane.x_axis),
        y_axis: vector(plane.y_axis),
    }
}

// These are user axes, not the kernel's conventional geometric axes.
const USER_AXES: [DVec3; 3] = [DVec3::Y, DVec3::NEG_X, DVec3::Z];

pub(crate) fn axes(app: &App, painter: &egui::Painter, rect: egui::Rect, view: &ViewSnapshot) {
    let (origin, directions) = axis_directions(app);
    let painter = painter.with_clip_rect(rect);
    for (axis, label, color) in directions {
        if let Some(ray) = rays::project_ray(view, origin, axis) {
            paint_axis(&painter, rect, ray, label, color);
        }
    }
}

fn axis_directions(app: &App) -> (DVec3, Vec<(DVec3, &'static str, egui::Color32)>) {
    let colors = [app.theme.axis_x, app.theme.axis_y, app.theme.axis_z];
    let Some(support) = grid(app) else {
        return (
            DVec3::ZERO,
            USER_AXES
                .into_iter()
                .enumerate()
                .map(|(index, axis)| (axis, ["X", "Y", "Z"][index], colors[index]))
                .collect(),
        );
    };
    let named = support.origin == DVec3::ZERO
        && [SketchPlane::xy(), SketchPlane::xz(), SketchPlane::yz()]
            .iter()
            .any(|plane| frame(*plane) == support);
    let directions = [support.x_axis, support.y_axis]
        .into_iter()
        .enumerate()
        .map(|(index, axis)| {
            let (label, color) = if named {
                let world = USER_AXES
                    .iter()
                    .position(|direction| axis.dot(*direction) > 1.0 - 1e-10)
                    .unwrap_or(index);
                (["X", "Y", "Z"][world], colors[world])
            } else {
                (["local X", "local Y"][index], colors[index])
            };
            (axis, label, color)
        })
        .collect();
    (support.origin, directions)
}

fn paint_axis(
    painter: &egui::Painter,
    rect: egui::Rect,
    ray: rays::ProjectedRay,
    label: &str,
    color: egui::Color32,
) {
    let point = |p: onecad_scene::DVec2| rect.min + egui::vec2(p.x as f32, p.y as f32);
    let (start, end) = (point(ray.start), point(ray.end));
    painter.line_segment(
        [start, end],
        egui::Stroke::new(1.25, color.linear_multiply(0.75)),
    );
    if let Some(origin) = ray.origin {
        painter.circle_filled(point(origin), 2.5, color);
    }
    let delta = end - start;
    let length = delta.length();
    let label_position = if length >= 8.0 {
        let direction = delta / length;
        let cue_start = start + direction * 28.0_f32.min(length * 0.2);
        let cue = direction * 18.0_f32.min(length * 0.2);
        painter.arrow(cue_start, cue, egui::Stroke::new(1.5, color));
        cue_start + cue + direction.rot90() * 12.0
    } else {
        start + egui::vec2(14.0, -14.0)
    };
    let galley = painter.layout_no_wrap(label.into(), egui::FontId::proportional(12.0), color);
    if rect.width() < galley.size().x + 8.0 || rect.height() < galley.size().y + 8.0 {
        return;
    }
    let half = galley.size() * 0.5;
    let position = label_position.clamp(
        rect.min + half + egui::vec2(4.0, 4.0),
        rect.max - half - egui::vec2(4.0, 4.0),
    );
    painter.galley(position - half, galley, color);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn user_axes_match_frozen_principal_bases() {
        for (plane, indices) in [
            (SketchPlane::xy(), [0, 1]),
            (SketchPlane::xz(), [0, 2]),
            (SketchPlane::yz(), [1, 2]),
        ] {
            let frame = frame(plane);
            assert_eq!(frame.x_axis, USER_AXES[indices[0]]);
            assert_eq!(frame.y_axis, USER_AXES[indices[1]]);
        }
    }
    #[test]
    fn offset_oblique_grid_uses_exact_support_coordinates() {
        let plane = onecad_core::sketch::plane_from_point_normal(
            onecad_core::math::Vec3::new_unchecked(12.0, -8.0, 30.0),
            onecad_core::math::Vec3::new_unchecked(0.6, 0.0, 0.8),
        );
        let frame = frame(plane);
        let world = frame.origin + frame.x_axis * 17.0 + frame.y_axis * -4.0;
        let expected = plane.to_world(onecad_core::math::Vec2::new_unchecked(17.0, -4.0));
        assert!((world - DVec3::new(expected.x, expected.y, expected.z)).length() < 1e-12);
    }
}
