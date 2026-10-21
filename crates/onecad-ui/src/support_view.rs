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

pub(crate) fn axes(app: &App, painter: &egui::Painter, rect: egui::Rect, view: &ViewSnapshot) {
    let (origin, directions) = axis_directions(app);
    let painter = painter.with_clip_rect(rect);
    for (axis, color) in directions {
        if let Some(ray) = rays::project_ray(view, origin, axis) {
            paint_axis(&painter, rect, ray, color);
        }
    }
}

fn axis_directions(app: &App) -> (DVec3, Vec<(DVec3, egui::Color32)>) {
    // Axes follow the kernel's world frame, matching the navigation cube.
    let world = [DVec3::X, DVec3::Y, DVec3::Z].into_iter().zip([
        app.theme.axis_x,
        app.theme.axis_y,
        app.theme.axis_z,
    ]);
    let Some(support) = grid(app) else {
        return (DVec3::ZERO, world.collect());
    };
    let named = support.origin == DVec3::ZERO
        && [SketchPlane::xy(), SketchPlane::xz(), SketchPlane::yz()]
            .iter()
            .any(|plane| frame(*plane) == support);
    if named {
        let normal = support.x_axis.cross(support.y_axis);
        return (
            support.origin,
            world
                .filter(|(axis, _)| axis.dot(normal).abs() < 1e-10)
                .collect(),
        );
    }
    let colors = [app.theme.axis_x, app.theme.axis_y];
    (
        support.origin,
        [support.x_axis, support.y_axis]
            .into_iter()
            .zip(colors)
            .collect(),
    )
}

fn paint_axis(
    painter: &egui::Painter,
    rect: egui::Rect,
    ray: rays::ProjectedRay,
    color: egui::Color32,
) {
    let point = |p: onecad_scene::DVec2| rect.min + egui::vec2(p.x as f32, p.y as f32);
    painter.line_segment(
        [point(ray.start), point(ray.end)],
        egui::Stroke::new(1.25, color.linear_multiply(0.75)),
    );
    if let Some(origin) = ray.origin {
        painter.circle_filled(point(origin), 2.5, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // The frozen sketch-plane bases map principal planes onto these world directions.
    const USER_AXES: [DVec3; 3] = [DVec3::Y, DVec3::NEG_X, DVec3::Z];
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
