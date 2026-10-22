//! The bridge side of `docs/ios/INPUT.md`: the router's per-batch context from
//! the previous frame and the editor, and the Pencil accessory policy (IP19).
use crate::app::{Failure, IosApp, Outcome};
use crate::ffi::{self, Status, TouchSample};
use onecad_interaction::touch::{
    Point, RouteContext,
    accessory::{self, AccessoryAction, AccessoryEvent, Preference},
};

impl IosApp {
    /// Routes one batch (`onecad_ios_input_submit` and test-hook replay alike):
    /// `over_control` and `drag_authoring` come from the last completed frame,
    /// `pencil_only_drawing` from `onecad_ios_capabilities_set`. `Ok(true)` when
    /// queued movement was coalesced.
    pub(crate) fn submit_touches(
        &mut self,
        samples: &[TouchSample],
        generation: u64,
    ) -> Outcome<bool> {
        let ctx = &self.ctx;
        let over_control = |point: Point| {
            onecad_ui::host::over_control(ctx, egui::pos2(point.x as f32, point.y as f32))
        };
        let route = RouteContext {
            pencil_only_drawing: self.capabilities.prefers_pencil_only_drawing != 0,
            drag_authoring: self
                .editor
                .as_ref()
                .is_some_and(onecad_ui::host::Editor::drag_authoring),
            over_control: &over_control,
        };
        let merged = self
            .input
            .submit(samples, generation, &route)
            .map_err(|error| Failure::new(Status::InvalidArgument, error.to_string()))?;
        self.ctx.request_repaint();
        Ok(merged)
    }

    /// Barrel double tap and squeeze: the user's preferred action decides
    /// (INPUT.md accessory table). They never author; the action runs at the
    /// start of the next frame, where the editor's tool gates apply.
    pub(crate) fn pencil_event(&mut self, event: &ffi::PencilEvent) -> Outcome {
        let last_pencil = self
            .input
            .pencil_position()
            .map(|at| Point::new(f64::from(at.x), f64::from(at.y)));
        let anchors = Anchors {
            hover_capable: self.capabilities.flags & ffi::CAP_HOVER != 0,
            last_pencil,
            center: self.canvas_center(),
        };
        match accessory_action(event, &anchors)? {
            AccessoryAction::Nothing | AccessoryAction::SystemShortcut => {}
            action => {
                self.accessory = Some(action);
                self.ctx.request_repaint();
            }
        }
        Ok(())
    }

    fn canvas_center(&self) -> Point {
        self.view.map_or(Point::new(0.0, 0.0), |view| {
            let scale = f64::from(view.scale);
            Point::new(
                f64::from(view.size_px[0]) / scale / 2.0,
                f64::from(view.size_px[1]) / scale / 2.0,
            )
        })
    }

    /// Applies the queued accessory action before the frame's editor pass.
    pub(crate) fn apply_accessory(&mut self) {
        let (Some(action), Some(editor)) = (self.accessory.take(), self.editor.as_mut()) else {
            return;
        };
        match action {
            AccessoryAction::TogglePreviousTool => {
                editor.toggle_previous_sketch_tool();
            }
            AccessoryAction::OpenPalette(at) => {
                editor.open_tool_palette(egui::pos2(at.x as f32, at.y as f32));
            }
            AccessoryAction::Nothing | AccessoryAction::SystemShortcut => {}
        }
    }
}

/// Where a palette may open, besides the event's own hover location.
struct Anchors {
    /// `ONECAD_IOS_CAP_HOVER`: unavailable capabilities stay unavailable.
    hover_capable: bool,
    last_pencil: Option<Point>,
    center: Point,
}

/// The accessory policy for one event; `ERR_INVALID_ARGUMENT` for values the
/// header does not define.
fn accessory_action(event: &ffi::PencilEvent, anchors: &Anchors) -> Outcome<AccessoryAction> {
    let invalid = |what: &str| Failure::new(Status::InvalidArgument, what.to_owned());
    let kind = AccessoryEvent::from_abi(event.action).ok_or_else(|| invalid("pencil action"))?;
    let preference =
        Preference::from_abi(event.preferred).ok_or_else(|| invalid("preferred action"))?;
    let completed = completed(kind, event.phase).ok_or_else(|| invalid("pencil phase"))?;
    let hovering = anchors.hover_capable && event.present & ffi::HAS_LOCATION != 0;
    let hover = hovering.then(|| Point::new(event.x_pt, event.y_pt));
    let anchor = accessory::palette_anchor(hover, anchors.last_pencil, anchors.center);
    Ok(accessory::action(kind, preference, completed, anchor))
}

/// Whether the event finished (a barrel tap reports only its end; a squeeze
/// acts on its end); `None` for a phase the header does not allow.
fn completed(kind: AccessoryEvent, phase: u32) -> Option<bool> {
    match (kind, phase) {
        (_, ffi::PHASE_ENDED) => Some(true),
        (AccessoryEvent::Squeeze, ffi::PHASE_BEGAN | ffi::PHASE_MOVED | ffi::PHASE_CANCELLED) => {
            Some(false)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BARREL: u32 = 0;
    const SQUEEZE: u32 = 1;
    const SWITCH_PREVIOUS: u32 = 2;
    const CONTEXTUAL_PALETTE: u32 = 5;

    fn event(action: u32, preferred: u32, phase: u32, located: bool) -> ffi::PencilEvent {
        ffi::PencilEvent {
            struct_size: std::mem::size_of::<ffi::PencilEvent>() as u32,
            action,
            preferred,
            phase,
            present: if located { ffi::HAS_LOCATION } else { 0 },
            x_pt: 10.0,
            y_pt: 20.0,
            hover_z: 0.0,
            altitude_rad: 0.0,
            azimuth_rad: 0.0,
            roll_rad: 0.0,
            timestamp_s: 1.0,
        }
    }

    fn anchors(hover_capable: bool, last_pencil: Option<Point>) -> Anchors {
        Anchors {
            hover_capable,
            last_pencil,
            center: Point::new(400.0, 300.0),
        }
    }

    fn palette(event: &ffi::PencilEvent, anchors: &Anchors) -> Point {
        match accessory_action(event, anchors) {
            Ok(AccessoryAction::OpenPalette(at)) => at,
            other => panic!("expected a palette, got {:?}", other.map_err(|f| f.message)),
        }
    }

    #[test]
    fn the_palette_opens_at_hover_only_with_the_hover_capability_then_falls_back() {
        let located = event(SQUEEZE, CONTEXTUAL_PALETTE, ffi::PHASE_ENDED, true);
        let last = Some(Point::new(50.0, 60.0));
        assert_eq!(
            palette(&located, &anchors(true, last)),
            Point::new(10.0, 20.0)
        );
        assert_eq!(
            palette(&located, &anchors(false, last)),
            Point::new(50.0, 60.0),
            "no hover capability: the location is not a hover pose"
        );
        let unlocated = event(BARREL, CONTEXTUAL_PALETTE, ffi::PHASE_ENDED, false);
        assert_eq!(
            palette(&unlocated, &anchors(true, last)),
            Point::new(50.0, 60.0)
        );
        assert_eq!(
            palette(&unlocated, &anchors(true, None)),
            Point::new(400.0, 300.0),
            "no Pencil seen yet: the canvas centre"
        );
    }

    #[test]
    fn undefined_values_are_invalid_and_unfinished_events_do_nothing() {
        let any = anchors(true, None);
        let status =
            |event: ffi::PencilEvent| accessory_action(&event, &any).err().map(|f| f.status);
        let invalid = Some(Status::InvalidArgument);
        assert_eq!(
            status(event(BARREL, SWITCH_PREVIOUS, ffi::PHASE_BEGAN, false)),
            invalid
        );
        assert_eq!(
            status(event(7, SWITCH_PREVIOUS, ffi::PHASE_ENDED, false)),
            invalid
        );
        assert_eq!(status(event(BARREL, 99, ffi::PHASE_ENDED, false)), invalid);
        let began = event(SQUEEZE, SWITCH_PREVIOUS, ffi::PHASE_BEGAN, false);
        assert_eq!(
            accessory_action(&began, &any).ok(),
            Some(AccessoryAction::Nothing)
        );
        let tapped = event(BARREL, SWITCH_PREVIOUS, ffi::PHASE_ENDED, false);
        assert_eq!(
            accessory_action(&tapped, &any).ok(),
            Some(AccessoryAction::TogglePreviousTool)
        );
    }

    #[test]
    fn only_an_ended_event_acts_and_a_tap_reports_only_its_end() {
        use AccessoryEvent::{BarrelDoubleTap, Squeeze};
        assert_eq!(completed(BarrelDoubleTap, ffi::PHASE_ENDED), Some(true));
        assert_eq!(completed(Squeeze, ffi::PHASE_ENDED), Some(true));
        for phase in [ffi::PHASE_BEGAN, ffi::PHASE_MOVED, ffi::PHASE_CANCELLED] {
            assert_eq!(completed(Squeeze, phase), Some(false));
            assert_eq!(completed(BarrelDoubleTap, phase), None);
        }
        assert_eq!(completed(Squeeze, ffi::PHASE_HOVER_MOVED), None);
    }
}
