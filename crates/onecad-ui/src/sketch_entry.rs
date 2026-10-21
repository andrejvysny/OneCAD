//! Sketch support choices retain cancellation ownership until admission completes.
use crate::{app::App, bridge::Command, sketch_services::Request};
use onecad_app::sketch_candidate::SketchCandidateControl;
use onecad_core::{ids::SketchId, sketch::WorldPlane};

#[derive(Clone, Debug, Default)]
pub struct Entry {
    pub open: bool,
    pub pending: Option<SketchCandidateControl>,
}

impl Entry {
    pub fn dismiss(&mut self) {
        if let Some(control) = &self.pending {
            control.cancel();
        } else {
            self.open = false;
        }
    }
}

impl App {
    fn selected_sketch_face(&self) -> Option<(onecad_core::ids::BodyId, String)> {
        let scene = self.display.as_ref()?.scene.identity();
        self.topology.selected_face(scene)
    }

    pub fn choose_sketch_support(&mut self) {
        if self.viewport_ready() && !self.busy() && !self.sketch_active() {
            self.ui.sketch_entry.open = true;
        }
    }

    fn new_sketch_on_selected_face(&mut self) {
        let Some((body, identity)) = self.selected_sketch_face() else {
            self.status = "Sketch support is stale; select the planar face again".into();
            return;
        };
        let Some(context) = self.interaction.context.clone() else {
            return;
        };
        let control = SketchCandidateControl::default();
        self.ui.sketch_entry.pending = Some(control.clone());
        self.submit(Command::Sketch(Box::new(Request::FaceEntry {
            id: SketchId::new(),
            context,
            body,
            identity,
            control,
        })));
    }
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    if !app.ui.sketch_entry.open {
        return;
    }
    if app.ui.sketch_entry.pending.is_some() && !app.busy() {
        app.ui.sketch_entry.pending = None;
        app.ui.sketch_entry.open = false;
        return;
    }
    let pending = app.ui.sketch_entry.pending.is_some();
    let mut plane = None;
    let mut face = false;
    let modal = egui::Modal::new(egui::Id::unique("sketch-support")).show(ctx, |ui| {
        ui.heading("Sketch plane");
        ui.add_enabled_ui(!pending, |ui| {
            ui.horizontal(|ui| {
                for (name, value) in [
                    ("XY", WorldPlane::XY),
                    ("XZ", WorldPlane::XZ),
                    ("YZ", WorldPlane::YZ),
                ] {
                    if ui.button(name).clicked() {
                        plane = Some(value);
                    }
                }
            });
            face = ui
                .add_enabled(
                    app.selected_sketch_face().is_some(),
                    egui::Button::new("Selected planar face"),
                )
                .clicked();
        });
        if pending {
            ui.label("Checking planar face…");
        } else {
            ui.small("Select a planar model face before opening this chooser.");
        }
        if ui.button("Cancel").clicked() {
            app.ui.sketch_entry.dismiss();
        }
    });
    if modal.should_close() {
        app.ui.sketch_entry.dismiss();
    }
    if let Some(plane) = plane {
        app.ui.sketch_entry.open = false;
        app.new_sketch_on_plane(plane);
    } else if face {
        app.new_sketch_on_selected_face();
    }
}

pub fn align_camera(camera: &mut onecad_scene::Camera, plane: onecad_core::sketch::SketchPlane) {
    let vector = |v: onecad_core::math::Vec3| onecad_scene::DVec3::new(v.x, v.y, v.z);
    camera.target = vector(plane.origin);
    camera.offset = vector(plane.normal) * camera.offset.length().max(1.0);
    camera.pole_right = vector(plane.x_axis);
    camera.roll_radians = 0.0;
    if let Ok(basis) = camera.basis() {
        let x = vector(plane.x_axis);
        camera.roll_radians = x.dot(basis.up).atan2(x.dot(basis.right));
    }
    camera.projection = onecad_scene::Projection::Orthographic;
}

#[cfg(test)]
mod tests {
    use super::*;
    use onecad_core::{math::Vec3, sketch::plane_from_point_normal};
    use onecad_scene::{Camera, DVec3, Projection};

    #[test]
    fn camera_uses_persisted_world_and_exact_face_axes() {
        let tilted = plane_from_point_normal(
            Vec3::new(12.0, -5.0, 8.0).unwrap(),
            Vec3::new(0.6, 0.0, 0.8).unwrap(),
        );
        for plane in [
            WorldPlane::XY.plane(),
            WorldPlane::XZ.plane(),
            WorldPlane::YZ.plane(),
            tilted,
        ] {
            let mut camera = Camera::default();
            let distance = camera.offset.length();
            align_camera(&mut camera, plane);
            let basis = camera.basis().unwrap();
            let vector = |v: Vec3| DVec3::new(v.x, v.y, v.z);
            assert!((basis.right - vector(plane.x_axis)).length() < 1e-12);
            assert!((basis.up - vector(plane.y_axis)).length() < 1e-12);
            assert!((basis.back - vector(plane.normal)).length() < 1e-12);
            assert_eq!(camera.target, vector(plane.origin));
            assert!((camera.offset.length() - distance).abs() < 1e-12);
            assert_eq!(camera.projection, Projection::Orthographic);
        }
    }

    #[test]
    fn entry_cancel_keeps_executing_service_owner() {
        let control = SketchCandidateControl::default();
        let mut entry = Entry {
            open: true,
            pending: Some(control.clone()),
        };
        entry.dismiss();
        assert!(entry.open && entry.pending.is_some());
        assert!(!control.cancel());
        entry.pending = None;
        entry.dismiss();
        assert!(!entry.open);
    }
}
