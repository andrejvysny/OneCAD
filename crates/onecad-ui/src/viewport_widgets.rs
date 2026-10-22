//! Camera controls and authoring overlays share the captured CAD view.
use crate::{
    app::App,
    navigation_cube::{self, CubeAction},
};
use onecad_scene::{Camera, DVec3, Projection, ViewSnapshot};

#[path = "pointer_policy.rs"]
mod pointer_policy;
pub use pointer_policy::{
    PointerKind, allows_pointer, capture_canvas, edge_tolerance, gesture_allowed, over_control,
    set_pointer_kind,
};

/// Region the cube is laid out in: its top-right corner is the ViewCluster anchor, which
/// moves with the Inspector.
fn cube_host(app: &App, ui: &egui::Ui, rect: egui::Rect) -> egui::Rect {
    let chrome = crate::layout::chrome(ui.ctx().content_rect(), &app.ui);
    let chrome = if app
        .doc
        .interaction
        .tool
        .as_ref()
        .is_some_and(|tool| !tool.editing_existing)
        && !app.sketch_active()
    {
        chrome.with_feature_card()
    } else {
        chrome
    };
    let anchor = chrome.view_cluster;
    egui::Rect::from_min_max(
        egui::pos2(rect.left(), anchor.y),
        egui::pos2(anchor.x, rect.bottom().max(anchor.y)),
    )
}

pub fn owns_pointer(app: &App, ui: &egui::Ui, rect: egui::Rect) -> bool {
    if app.doc.transform_gizmo.gizmo.active() || app.doc.extrude_handle.active() {
        return true;
    }
    let Some(pointer) = ui.input(|input| input.pointer.hover_pos()) else {
        return false;
    };
    owns_position(app, ui, rect, pointer)
}

pub(crate) fn owns_position(
    app: &App,
    ui: &egui::Ui,
    rect: egui::Rect,
    pointer: egui::Pos2,
) -> bool {
    allows_pointer(ui, rect, pointer)
        && (navigation_cube::rect(cube_host(app, ui, rect)).contains(pointer)
            || app.doc.view.as_ref().is_some_and(|view| {
                crate::transform_ui::owns_pointer(app, view, rect, pointer)
                    || crate::extrude_handle::owns_pointer(app, view, rect, pointer)
            }))
}

pub fn show(app: &mut App, ui: &mut egui::Ui, rect: egui::Rect, view: &ViewSnapshot) -> bool {
    let cube = ui
        .add_enabled_ui(
            !app.doc.transform_gizmo.gizmo.active() && !app.doc.extrude_handle.active(),
            |ui| navigation_cube::show(ui, cube_host(app, ui, rect), view, &app.theme),
        )
        .inner;
    for action in cube.actions {
        apply_cube(app, view, action);
        app.draw_dirty = true;
        ui.ctx().request_repaint();
    }
    let gizmo = if cube.owns_pointer {
        false
    } else {
        crate::transform_ui::show(app, ui, view, rect)
    };
    let handle = if cube.owns_pointer || gizmo {
        false
    } else {
        crate::extrude_handle::show(app, ui, view, rect)
    };
    cube.owns_pointer || gizmo || handle
}

fn apply_cube(app: &mut App, view: &ViewSnapshot, action: CubeAction) {
    match action {
        CubeAction::SetDirection(direction) => {
            if let Some(direction) = direction.try_normalize() {
                app.doc.camera.offset = direction * app.doc.camera.offset.length();
                app.doc.camera.roll_radians = 0.0;
            }
        }
        CubeAction::Orbit(delta) => {
            crate::camera_navigation::apply(
                &mut app.doc.camera,
                view.viewport(),
                view.identity().scene,
                None,
                crate::navigation::Motion::Orbit(delta),
            );
        }
        CubeAction::QuarterOrbit { axis, radians } => {
            rotate_camera(&mut app.doc.camera, axis, radians)
        }
        CubeAction::Roll(angle) => {
            app.doc.camera.roll_radians =
                (app.doc.camera.roll_radians + angle).rem_euclid(std::f64::consts::TAU);
        }
        CubeAction::Home => {
            app.doc.camera = Camera::default();
            app.doc.fit_pending = true;
        }
        CubeAction::Fit => app.doc.fit_pending = true,
        CubeAction::ToggleProjection => {
            app.doc.camera.projection = if app.doc.camera.projection == Projection::Perspective {
                Projection::Orthographic
            } else {
                Projection::Perspective
            };
        }
    }
}

fn rotate_camera(camera: &mut Camera, axis: DVec3, radians: f64) {
    let (Some(axis), Ok(basis)) = (axis.try_normalize(), camera.basis()) else {
        return;
    };
    if !radians.is_finite() {
        return;
    }
    let rotation = glam::DQuat::from_axis_angle(axis, radians);
    let desired_right = rotation * basis.right;
    let mut candidate = *camera;
    candidate.offset = rotation * camera.offset;
    candidate.pole_right = desired_right;
    candidate.roll_radians = 0.0;
    let Ok(base) = candidate.basis() else {
        return;
    };
    candidate.roll_radians = desired_right
        .dot(base.up)
        .atan2(desired_right.dot(base.right));
    if candidate.basis().is_ok() {
        *camera = candidate;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quarter_turn_keeps_view_frame_through_poles() {
        let mut camera = Camera {
            roll_radians: 0.7,
            ..Default::default()
        };
        let basis = camera.basis().unwrap();
        let old = camera;
        let angle = std::f64::consts::FRAC_PI_2;
        rotate_camera(&mut camera, basis.right, angle);
        let rotation = glam::DQuat::from_axis_angle(basis.right, angle);
        assert!((camera.offset - rotation * old.offset).length() < 1e-10);
        assert!((camera.basis().unwrap().right - rotation * basis.right).length() < 1e-10);
        assert_eq!(camera.target, old.target);
    }
}
