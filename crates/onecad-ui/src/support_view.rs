//! Presentation uses the same immutable support frame as sketch projection.
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
    let colors = [app.theme.axis_x, app.theme.axis_y, app.theme.axis_z];
    let length = (view.camera().offset.length() * 0.16).clamp(0.01, 1.0e8);
    let (origin, directions) = if let Some(support) = grid(app) {
        let named = support.origin == DVec3::ZERO
            && [SketchPlane::xy(), SketchPlane::xz(), SketchPlane::yz()]
                .iter()
                .any(|plane| frame(*plane) == support);
        let directions = [support.x_axis, support.y_axis]
            .into_iter()
            .enumerate()
            .map(|(index, axis)| {
                let world = USER_AXES
                    .iter()
                    .position(|direction| axis.dot(*direction) > 1.0 - 1e-10);
                let (label, color) = if named {
                    let world = world.unwrap_or(index);
                    (["X", "Y", "Z"][world], colors[world])
                } else {
                    (["local X", "local Y"][index], colors[index])
                };
                (axis, label, color)
            })
            .collect::<Vec<_>>();
        (support.origin, directions)
    } else {
        (
            DVec3::ZERO,
            USER_AXES
                .into_iter()
                .enumerate()
                .map(|(index, axis)| (axis, ["X", "Y", "Z"][index], colors[index]))
                .collect(),
        )
    };
    let painter = painter.with_clip_rect(rect);
    for (axis, label, color) in directions {
        if let (Some(start), Some(end)) =
            (view.project(origin), view.project(origin + axis * length))
        {
            let point = |p: onecad_scene::ProjectedPoint| {
                rect.min + egui::vec2(p.logical.x as f32, p.logical.y as f32)
            };
            let start = point(start);
            let end = point(end);
            if start.distance(end) < 8.0 {
                continue;
            }
            painter.arrow(start, end - start, egui::Stroke::new(1.5, color));
            painter.text(
                end + egui::vec2(4.0, -4.0),
                egui::Align2::LEFT_BOTTOM,
                label,
                egui::FontId::proportional(12.0),
                color,
            );
            painter.circle_filled(start, 2.5, color);
        }
    }
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
