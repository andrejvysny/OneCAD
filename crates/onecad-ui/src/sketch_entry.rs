//! Sketch support choices retain cancellation ownership until admission completes.
mod picking;

use crate::{app::App, bridge::Command, sketch_services::Request};
use onecad_app::sketch_candidate::SketchCandidateControl;
use onecad_core::sketch::SketchPlane;
use onecad_core::{ids::SketchId, sketch::WorldPlane};
use onecad_interaction::DocumentContext;
use onecad_scene::CaptureIdentity;
use std::sync::{Arc, Mutex, mpsc};

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Candidate {
    Named(WorldPlane),
    Face(onecad_core::ids::BodyId, String),
}

#[derive(Clone, Debug)]
struct Preview {
    candidate: Candidate,
    capture: CaptureIdentity,
    receiver: Arc<Mutex<mpsc::Receiver<Result<SketchPlane, String>>>>,
}

#[derive(Clone, Debug, Default)]
pub struct Entry {
    pub open: bool,
    pub pending: Option<SketchCandidateControl>,
    context: Option<DocumentContext>,
    candidate: Option<Candidate>,
    capture: Option<CaptureIdentity>,
    pub(crate) plane: Option<SketchPlane>,
    query: Option<Preview>,
    message: Option<String>,
    accept: Option<(Candidate, CaptureIdentity)>,
}

impl Entry {
    pub fn dismiss(&mut self) {
        self.candidate = None;
        self.plane = None;
        self.query = None;
        self.accept = None;
        if let Some(control) = &self.pending {
            control.cancel();
        } else {
            self.open = false;
        }
    }
}

impl App {
    pub fn choose_sketch_support(&mut self) {
        if self.viewport_ready() && !self.busy() && !self.sketch_active() {
            self.ui.sketch_entry.open = true;
            self.ui.sketch_entry.context = self.interaction.context.clone();
            // A face selected beforehand already names the support; skip the chooser.
            if let Some((body, identity)) = self
                .display
                .as_ref()
                .and_then(|display| self.topology.selected_face(display.scene.identity()))
            {
                self.ui.sketch_entry.plane = None;
                self.ui.sketch_entry.candidate = None;
                self.new_sketch_on_face(body, identity);
            }
        }
    }

    fn new_sketch_on_face(&mut self, body: onecad_core::ids::BodyId, identity: String) {
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

pub(crate) fn viewport(
    app: &mut App,
    ui: &egui::Ui,
    rect: egui::Rect,
    response: &egui::Response,
    view: &onecad_scene::ViewSnapshot,
    display: &crate::bridge::Display,
) -> bool {
    if !app.ui.sketch_entry.open {
        return false;
    }
    if app.ui.sketch_entry.pending.is_some() {
        return true;
    }
    let candidate = pointer_candidate(app, ui, rect, response, view, display);
    let entry = &mut app.ui.sketch_entry;
    app.draw_dirty |= update_candidate(entry, candidate, view.identity());
    let previous_plane = entry.plane;
    poll_preview(entry, view.identity());
    app.draw_dirty |= previous_plane != entry.plane;
    start_preview(app, view.identity());
    admit_clicked(app);
    true
}

fn pointer_candidate(
    app: &mut App,
    ui: &egui::Ui,
    rect: egui::Rect,
    response: &egui::Response,
    view: &onecad_scene::ViewSnapshot,
    display: &crate::bridge::Display,
) -> Option<Candidate> {
    let position = ui.input(|input| input.pointer.hover_pos()).filter(|point| {
        pointer_allowed(app, ui, rect, *point)
            && !ui.input(|input| input.pointer.secondary_down() || input.pointer.middle_down())
    });
    let logical = |point: egui::Pos2| {
        onecad_scene::DVec2::new(
            f64::from(point.x - rect.min.x),
            f64::from(point.y - rect.min.y),
        )
    };
    let mut candidate = picking::candidate_at(
        ui.painter(),
        rect,
        view,
        display,
        position.map(logical),
        app,
    );
    if response.clicked()
        && let Some(point) = response
            .interact_pointer_pos()
            .filter(|point| pointer_allowed(app, ui, rect, *point))
    {
        candidate =
            picking::candidate_at(ui.painter(), rect, view, display, Some(logical(point)), app);
        app.ui.sketch_entry.accept = candidate
            .clone()
            .map(|candidate| (candidate, view.identity()));
    }
    retain_clicked(&mut app.ui.sketch_entry, candidate, view.identity())
}

fn retain_clicked(
    entry: &mut Entry,
    candidate: Option<Candidate>,
    capture: CaptureIdentity,
) -> Option<Candidate> {
    if entry
        .accept
        .as_ref()
        .is_some_and(|(_, accepted_capture)| *accepted_capture != capture)
    {
        entry.accept = None;
    }
    entry
        .accept
        .as_ref()
        .map(|(accepted, _)| accepted.clone())
        .or(candidate)
}

fn pointer_allowed(app: &App, ui: &egui::Ui, rect: egui::Rect, point: egui::Pos2) -> bool {
    crate::viewport_widgets::allows_pointer(ui, rect, point)
        && !crate::viewport_widgets::owns_position(app, ui, rect, point)
}

fn update_candidate(
    entry: &mut Entry,
    candidate: Option<Candidate>,
    capture: CaptureIdentity,
) -> bool {
    if entry.candidate == candidate && entry.capture == Some(capture) {
        return false;
    }
    entry.candidate = candidate;
    entry.capture = Some(capture);
    entry.plane = match entry.candidate {
        Some(Candidate::Named(plane)) => Some(plane.plane()),
        _ => None,
    };
    entry.message = None;
    true
}

fn admit_clicked(app: &mut App) {
    if app.ui.sketch_entry.accept.is_none() || app.busy() {
        return;
    }
    let entry = &mut app.ui.sketch_entry;
    match entry.candidate.clone() {
        Some(Candidate::Named(plane)) => {
            entry.open = false;
            entry.plane = None;
            entry.accept = None;
            app.new_sketch_on_plane(plane);
        }
        Some(Candidate::Face(body, identity)) if entry.plane.is_some() => {
            entry.accept = None;
            app.new_sketch_on_face(body, identity);
        }
        _ => {}
    }
}

fn poll_preview(entry: &mut Entry, capture: CaptureIdentity) {
    let reply = entry
        .query
        .as_ref()
        .and_then(|query| query.receiver.lock().ok()?.try_recv().ok());
    if let Some(result) = reply {
        let Some(query) = entry.query.take() else {
            return;
        };
        if entry.candidate.as_ref() == Some(&query.candidate) && query.capture == capture {
            match result {
                Ok(plane) => entry.plane = Some(plane),
                Err(message) => {
                    entry.message = Some(message);
                    entry.accept = None;
                }
            }
        }
    }
}

fn start_preview(app: &mut App, capture: CaptureIdentity) {
    let entry = &mut app.ui.sketch_entry;
    if entry.query.is_some() || entry.plane.is_some() || entry.message.is_some() {
        return;
    }
    let Some(Candidate::Face(body, identity)) = entry.candidate.clone() else {
        return;
    };
    let Some(context) = entry.context.clone() else {
        return;
    };
    let (sender, receiver) = mpsc::channel();
    let request = Request::FacePreview {
        context,
        body,
        identity: identity.clone(),
        sender,
    };
    match app.bridge.submit(Command::Sketch(Box::new(request))) {
        Ok(()) => {
            entry.query = Some(Preview {
                candidate: Candidate::Face(body, identity),
                capture,
                receiver: Arc::new(Mutex::new(receiver)),
            })
        }
        Err(message) => entry.message = Some(message),
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
    if app.ui.sketch_entry.context != app.interaction.context {
        app.ui.sketch_entry.dismiss();
        return;
    }
    if app.ui.sketch_entry.pending.is_some() && app.ui.sketch_entry.message.is_none() {
        return;
    }
    egui::Area::new(egui::Id::unique("sketch-support"))
        .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 100.0))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| instruction_row(app, ui));
                if let Some(message) = &app.ui.sketch_entry.message {
                    ui.small(message);
                }
            });
        });
}

fn instruction_row(app: &mut App, ui: &mut egui::Ui) {
    let entry = &app.ui.sketch_entry;
    ui.label(if entry.pending.is_some() {
        "Opening sketch…"
    } else if entry.query.is_some() {
        "Checking exact planar support…"
    } else {
        "Choose XY, XZ, YZ or a planar face"
    });
    if app.ui.sketch_entry.pending.is_none()
        && let Some((body, identity)) = app
            .display
            .as_ref()
            .and_then(|display| app.topology.selected_face(display.scene.identity()))
        && ui.small_button("Selected face").clicked()
    {
        app.new_sketch_on_face(body, identity);
    }
    if ui.button("Cancel").clicked() {
        app.ui.sketch_entry.dismiss();
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
mod tests;
