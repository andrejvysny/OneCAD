//! Transform triad adapted from SolidCraft; caller owns model previews and commits.

use crate::theme::Theme;
use egui::{Pos2, Rect};
use onecad_scene::{DVec3, ViewSnapshot};

#[path = "gizmo_geometry.rs"]
mod geometry;
#[path = "gizmo_paint.rs"]
mod paint;
use geometry::{Geometry, State};
pub(crate) use geometry::{along, project};
pub(crate) use paint::arrow;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Handle {
    Axis(usize),
    Plane(usize),
    Ring(usize),
    Ball,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Change {
    Translate(DVec3),
    Rotate { axis: usize, angle: f64 },
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Drag {
    pub handle: Handle,
    pub change: Change,
    pub started: bool,
    pub done: bool,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Triad {
    pub center: DVec3,
    pub translate: bool,
    pub rotate: bool,
}
#[derive(Default)]
pub struct GizmoResponse {
    pub drag: Option<Drag>,
    pub owns_pointer: bool,
}
#[derive(Default)]
pub struct Gizmo {
    state: Option<State>,
}
pub const LEN_PX: f64 = 110.0;
struct Frame<'a> {
    view: &'a ViewSnapshot,
    rect: Rect,
    triad: Triad,
    step: f64,
    free: bool,
    geometry: &'a Geometry,
}

impl Gizmo {
    pub fn cancel(&mut self) {
        self.state = None;
    }
    pub fn active(&self) -> bool {
        self.state.is_some()
    }
    pub fn contains_pointer(view: &ViewSnapshot, rect: Rect, triad: Triad, pointer: Pos2) -> bool {
        rect.contains(pointer)
            && Geometry::new(view, rect, triad)
                .is_some_and(|geometry| geometry.hit(pointer).is_some())
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        view: &ViewSnapshot,
        rect: Rect,
        triad: Triad,
        step: f64,
        theme: &Theme,
    ) -> GizmoResponse {
        let geometry = match self.geometry(view, rect, triad, step) {
            Some(geometry) => geometry,
            None => return GizmoResponse::default(),
        };
        let (pointer, events, modifiers) =
            ui.input(|i| (i.pointer.hover_pos(), i.events.clone(), i.modifiers));
        if !crate::viewport_widgets::gesture_allowed(ui) {
            self.cancel();
            return GizmoResponse::default();
        }
        let hovered = pointer
            .filter(|p| crate::viewport_widgets::allows_pointer(ui, rect, *p))
            .and_then(|p| geometry.hit(p));
        let hot = self.state.as_ref().map(|s| s.handle).or(hovered);
        paint::draw(&ui.painter().with_clip_rect(rect), &geometry, hot, theme);
        if hot.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
        let events: Vec<_> = events
            .into_iter()
            .filter(|event| match event {
                egui::Event::PointerButton {
                    pos, pressed: true, ..
                } => crate::viewport_widgets::allows_pointer(ui, rect, *pos),
                _ => true,
            })
            .collect();
        let free = modifiers.alt || modifiers.ctrl || modifiers.command;
        self.process(
            &events,
            &Frame {
                view,
                rect,
                triad,
                step,
                free,
                geometry: &geometry,
            },
        )
    }
    fn geometry(
        &mut self,
        view: &ViewSnapshot,
        rect: Rect,
        triad: Triad,
        step: f64,
    ) -> Option<Geometry> {
        let valid = step.is_finite()
            && step >= 0.0
            && self
                .state
                .as_ref()
                .is_none_or(|state| state.matches(view, rect, triad));
        let geometry = valid.then(|| Geometry::new(view, rect, triad)).flatten();
        if geometry.is_none() {
            self.cancel();
        }
        geometry
    }
    fn process(&mut self, events: &[egui::Event], frame: &Frame<'_>) -> GizmoResponse {
        let mut response = GizmoResponse {
            owns_pointer: self.active(),
            ..Default::default()
        };
        for event in events {
            if matches!(
                event,
                egui::Event::PointerGone | egui::Event::WindowFocused(false)
            ) {
                self.cancel();
                response.drag = None;
                break;
            }
            self.event(event, frame, &mut response);
        }
        response
    }
    fn event(&mut self, event: &egui::Event, frame: &Frame<'_>, out: &mut GizmoResponse) {
        let Frame {
            view,
            rect,
            triad,
            step,
            free,
            geometry,
        } = *frame;
        match event {
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                ..
            } => {
                if *pressed && self.state.is_none() && rect.contains(*pos) {
                    self.state = geometry
                        .hit(*pos)
                        .and_then(|h| State::begin(view, rect, triad, h, *pos));
                    if let Some(state) = self.state.as_mut() {
                        out.drag = state.drag(*pos, step, free, true, false);
                        out.owns_pointer = true;
                    }
                } else if !pressed && let Some(state) = self.state.as_mut() {
                    // Outside releases end ownership without extrapolating a checked ray.
                    let started = out.drag.is_some_and(|drag| drag.started);
                    out.drag = state.drag(*pos, step, free, started, true);
                    out.owns_pointer = true;
                    self.cancel();
                }
            }
            egui::Event::PointerMoved(pos) => {
                if let Some(state) = self.state.as_mut() {
                    let started = out.drag.is_some_and(|drag| drag.started);
                    if let Some(drag) = state.drag(*pos, step, free, started, false) {
                        out.drag = Some(drag);
                    }
                    out.owns_pointer = true;
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
#[path = "gizmo_tests.rs"]
mod tests;
