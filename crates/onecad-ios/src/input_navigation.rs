//! Two-finger navigation recognized natively: a trackpad-style `MouseWheel` pan
//! and `Zoom` over the gesture's anchor. Its start hands the canvas fingers to
//! it through the router (an authoring finger is cancelled, an orbit ends in
//! place); while a Pencil or pointer authors, the whole gesture is ignored so
//! the camera stays put.
use super::{Event, Input, Item};
use crate::ffi;
use egui::{Pos2, TouchPhase, Vec2};

struct Step {
    anchor: Pos2,
    delta: Vec2,
    zoom: Option<f32>,
    phase: TouchPhase,
}

fn step(nav: &ffi::Navigation) -> Option<Step> {
    let phase = match nav.phase {
        ffi::PHASE_BEGAN => TouchPhase::Start,
        ffi::PHASE_MOVED => TouchPhase::Move,
        ffi::PHASE_ENDED => TouchPhase::End,
        ffi::PHASE_CANCELLED => TouchPhase::Cancel,
        _ => return None,
    };
    let values = [
        nav.anchor_x_pt,
        nav.anchor_y_pt,
        nav.pan_dx_pt,
        nav.pan_dy_pt,
    ];
    if values.iter().any(|value| !value.is_finite()) {
        return None;
    }
    let scale = nav.scale_delta;
    Some(Step {
        anchor: Pos2::new(nav.anchor_x_pt as f32, nav.anchor_y_pt as f32),
        delta: egui::vec2(nav.pan_dx_pt as f32, nav.pan_dy_pt as f32),
        zoom: (scale.is_finite() && scale > 0.0 && scale != 1.0).then_some(scale as f32),
        phase,
    })
}

impl Input {
    /// `false` for an unknown phase or non-finite values.
    pub fn navigation(&mut self, nav: &ffi::Navigation) -> bool {
        let Some(step) = step(nav) else {
            return false;
        };
        let ending = matches!(step.phase, TouchPhase::End | TouchPhase::Cancel);
        if step.phase == TouchPhase::Start {
            let (allowed, routed) = self.router.native_navigation();
            self.apply_all(routed);
            self.navigation_ignored = !allowed;
        }
        if self.navigation_ignored {
            self.navigation_ignored = !ending;
            return true;
        }
        if step.phase == TouchPhase::Move && self.merge_navigation(&step) {
            return true;
        }
        let start = self.pending.len();
        // Navigation reads the hovered viewport; a held contact keeps its own position.
        if self.pointer.is_none() {
            self.gone = super::Gone::Idle;
            self.push_navigation(Event::PointerMoved(step.anchor));
        }
        if step.phase != TouchPhase::Move || step.delta != Vec2::ZERO {
            self.navigation_wheel(step.delta, step.phase);
        }
        if let Some(zoom) = step.zoom {
            self.push_navigation(Event::Zoom(zoom));
        }
        if ending && self.pointer.is_none() {
            self.gone = super::Gone::Armed;
        }
        self.navigation_tail = (step.phase == TouchPhase::Move).then_some(start);
        true
    }

    /// Folds a move into the queued move group of the same gesture, so a burst of
    /// navigation between frames stays three events: anchor, summed pan, product
    /// of zooms.
    fn merge_navigation(&mut self, step: &Step) -> bool {
        let Some(start) = self.navigation_tail else {
            return false;
        };
        let mut panned = step.delta == Vec2::ZERO;
        let mut zoomed = step.zoom.is_none();
        for item in &mut self.pending[start..] {
            match &mut item.event {
                Event::PointerMoved(pos) => *pos = step.anchor,
                Event::MouseWheel { delta, .. } if !panned => {
                    *delta += step.delta;
                    panned = true;
                }
                Event::Zoom(zoom) if !zoomed => {
                    *zoom *= step.zoom.unwrap_or(1.0);
                    zoomed = true;
                }
                _ => {}
            }
        }
        if !panned {
            self.navigation_wheel(step.delta, TouchPhase::Move);
        }
        if let (false, Some(zoom)) = (zoomed, step.zoom) {
            self.push_navigation(Event::Zoom(zoom));
        }
        true
    }

    /// Ends a live navigation gesture (system interruption).
    pub(super) fn cancel_navigation(&mut self) {
        if self.navigating {
            self.navigation_wheel(Vec2::ZERO, TouchPhase::Cancel);
        }
        self.navigation_ignored = false;
        self.navigation_tail = None;
        self.orbit_tail = None;
    }

    fn navigation_wheel(&mut self, delta: Vec2, phase: TouchPhase) {
        self.navigating = matches!(phase, TouchPhase::Start | TouchPhase::Move);
        let modifiers = self.modifiers;
        self.push_navigation(Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta,
            phase,
            source: egui::MouseWheelSource::Trackpad,
            modifiers,
        });
    }

    /// Like `push`, but keeps the trailing move group open for merging.
    fn push_navigation(&mut self, event: Event) {
        self.orbit_tail = None;
        self.pending.push(Item {
            event,
            disposable: false,
            contact: None,
        });
    }
}
