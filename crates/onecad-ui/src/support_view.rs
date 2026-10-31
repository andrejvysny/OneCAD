//! Presentation uses the same immutable support frame as sketch projection.

use crate::app::App;
use onecad_core::sketch::SketchPlane;
use onecad_render::{AxesFrame, AxisColor, AxisLine};
use onecad_scene::DVec3;

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
        app.doc.sketch.session.as_ref().map(|session| session.plane)
    } else {
        app.doc
            .sketch_entry
            .plane
            .filter(|_| app.doc.sketch_entry.open)
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

/// Origin triad for the renderer, which depth-tests it against the bodies.
pub(crate) fn axes(app: &App) -> AxesFrame {
    // Axes follow the kernel's world frame, matching the navigation cube.
    let world = [
        (DVec3::X, AxisColor::X),
        (DVec3::Y, AxisColor::Y),
        (DVec3::Z, AxisColor::Z),
    ];
    let Some(support) = grid(app) else {
        return frame_of(DVec3::ZERO, world);
    };
    let named = support.origin == DVec3::ZERO
        && [SketchPlane::xy(), SketchPlane::xz(), SketchPlane::yz()]
            .iter()
            .any(|plane| frame(*plane) == support);
    if named {
        let normal = support.x_axis.cross(support.y_axis);
        return frame_of(
            support.origin,
            world
                .into_iter()
                .filter(|(axis, _)| axis.dot(normal).abs() < 1e-10),
        );
    }
    frame_of(
        support.origin,
        [
            (support.x_axis, AxisColor::X),
            (support.y_axis, AxisColor::Y),
        ],
    )
}

fn frame_of(origin: DVec3, lines: impl IntoIterator<Item = (DVec3, AxisColor)>) -> AxesFrame {
    let mut frame = AxesFrame {
        origin,
        lines: [None; 3],
    };
    for (slot, (direction, color)) in frame.lines.iter_mut().zip(lines) {
        *slot = Some(AxisLine { direction, color });
    }
    frame
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
    #[test]
    fn world_triad_without_support_plane() {
        let frame = frame_of(
            DVec3::ZERO,
            [(DVec3::X, AxisColor::X), (DVec3::Z, AxisColor::Z)],
        );
        assert_eq!(frame.lines[0].unwrap().color, AxisColor::X);
        assert_eq!(frame.lines[1].unwrap().direction, DVec3::Z);
        assert!(frame.lines[2].is_none());
    }
}
