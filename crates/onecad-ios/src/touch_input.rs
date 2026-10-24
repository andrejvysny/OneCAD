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
    /// Controls and drag availability come from the last completed frame.
    /// Canvas authoring always requires Pencil or an external pointer; the
    /// system drawing preference cannot enable finger authoring. `Ok(true)`
    /// means queued movement was coalesced.
    pub(crate) fn submit_touches(
        &mut self,
        samples: &[TouchSample],
        generation: u64,
    ) -> Outcome<bool> {
        if let Some(editor) = &self.editor {
            self.input
                .set_prediction_context(editor.presentation_context(&self.ctx));
        }
        let ctx = &self.ctx;
        let over_control = |point: Point| {
            onecad_ui::host::over_control(ctx, egui::pos2(point.x as f32, point.y as f32))
        };
        let route = route_context(
            self.editor
                .as_ref()
                .is_some_and(onecad_ui::host::Editor::drag_authoring),
            &over_control,
        );
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
        let action = capability_action(
            event,
            &anchors,
            self.capabilities_published.then_some(&self.capabilities),
        )?;
        if event.phase == ffi::PHASE_CANCELLED {
            self.accessory.clear();
        }
        if let Some(editor) = &self.editor {
            self.accessory.submit(
                event,
                action,
                editor.presentation_context(&self.ctx),
                self.input.contact_context(),
            );
            self.ctx.request_repaint();
        }
        Ok(())
    }

    pub(crate) fn set_capabilities(&mut self, caps: ffi::Capabilities) -> Outcome {
        validate_capabilities(&caps)?;
        self.capabilities = caps;
        self.capabilities_published = true;
        self.accessory.clear();
        self.ctx.request_repaint();
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
        if !self.opens.is_empty() {
            self.accessory.clear();
            return;
        }
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        let context = editor.presentation_context(&self.ctx);
        let Some(action) = self.accessory.take(&context, self.input.contact_context()) else {
            return;
        };
        let changed = match action {
            AccessoryAction::TogglePreviousTool => editor.toggle_previous_sketch_tool(),
            AccessoryAction::OpenPalette(at) => {
                editor.open_tool_palette(egui::pos2(at.x as f32, at.y as f32))
            }
            AccessoryAction::Nothing | AccessoryAction::SystemShortcut => false,
        };
        if let Some(request) = crate::pencil_accessory::feedback(changed, self.capabilities.flags) {
            self.host.push(request, None);
        }
    }
}

fn route_context<'a>(
    drag_authoring: bool,
    over_control: &'a dyn Fn(Point) -> bool,
) -> RouteContext<'a> {
    RouteContext {
        pencil_only_drawing: true,
        drag_authoring,
        over_control,
    }
}

fn validate_capabilities(caps: &ffi::Capabilities) -> Outcome {
    if caps.flags & !0xff != 0
        || caps.prefers_pencil_only_drawing > 1
        || Preference::from_abi(caps.preferred_tap_action).is_none()
        || Preference::from_abi(caps.preferred_squeeze_action).is_none()
    {
        return Err(Failure::new(Status::InvalidArgument, "pencil capabilities"));
    }
    Ok(())
}

fn capability_action(
    event: &ffi::PencilEvent,
    anchors: &Anchors,
    caps: Option<&ffi::Capabilities>,
) -> Outcome<AccessoryAction> {
    // Validate the event even when a published preference supersedes its fallback.
    accessory_action(event, anchors)?;
    let Some(caps) = caps else {
        return accessory_action(event, anchors);
    };
    let mut event = *event;
    let (flag, preference) = match event.action {
        0 => (ffi::CAP_BARREL_TAP, caps.preferred_tap_action),
        _ => (ffi::CAP_SQUEEZE, caps.preferred_squeeze_action),
    };
    if caps.flags & flag == 0 {
        return Ok(AccessoryAction::Nothing);
    }
    event.preferred = preference;
    accessory_action(&event, anchors)
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
    if !event.timestamp_s.is_finite() || event.timestamp_s < 0.0 || event.present & !0x5e != 0 {
        return Err(Failure::new(
            Status::InvalidArgument,
            "pencil timestamp or pose flags",
        ));
    }
    for (flag, values) in [
        (ffi::HAS_LOCATION, &[event.x_pt, event.y_pt][..]),
        (1 << 1, &[event.altitude_rad][..]),
        (1 << 2, &[event.azimuth_rad][..]),
        (1 << 3, &[event.roll_rad][..]),
        (1 << 4, &[event.hover_z][..]),
    ] {
        if event.present & flag != 0 && values.iter().any(|value| !value.is_finite()) {
            return Err(Failure::new(Status::InvalidArgument, "pencil pose"));
        }
    }
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

    #[test]
    fn published_preferences_override_event_and_unavailable_actions_are_suppressed() {
        let event = event(BARREL, SWITCH_PREVIOUS, ffi::PHASE_ENDED, false);
        let anchors = anchors(false, None);
        assert_eq!(
            capability_action(&event, &anchors, None).unwrap(),
            AccessoryAction::TogglePreviousTool
        );
        let mut caps = ffi::Capabilities {
            flags: ffi::CAP_BARREL_TAP,
            preferred_tap_action: 0,
            preferred_squeeze_action: CONTEXTUAL_PALETTE,
            ..Default::default()
        };
        assert_eq!(
            capability_action(&event, &anchors, Some(&caps)).unwrap(),
            AccessoryAction::Nothing
        );
        caps.preferred_tap_action = CONTEXTUAL_PALETTE;
        assert_eq!(
            capability_action(&event, &anchors, Some(&caps)).unwrap(),
            AccessoryAction::OpenPalette(anchors.center)
        );
        caps.flags = 0;
        assert_eq!(
            capability_action(&event, &anchors, Some(&caps)).unwrap(),
            AccessoryAction::Nothing
        );
        let squeeze = event_for_squeeze();
        caps.flags = ffi::CAP_SQUEEZE;
        assert_eq!(
            capability_action(&squeeze, &anchors, Some(&caps)).unwrap(),
            AccessoryAction::OpenPalette(anchors.center)
        );
        assert_eq!(
            caps.flags & ffi::CAP_HAPTICS,
            0,
            "no pressure/hover/haptics fabricated"
        );
    }

    fn event_for_squeeze() -> ffi::PencilEvent {
        event(SQUEEZE, 0, ffi::PHASE_ENDED, false)
    }

    #[test]
    fn invalid_capabilities_and_superseded_event_enums_are_refused() {
        for changed in 0..4 {
            let mut caps = ffi::Capabilities::default();
            match changed {
                0 => caps.flags = 1 << 31,
                1 => caps.preferred_tap_action = 7,
                2 => caps.preferred_squeeze_action = 7,
                _ => caps.prefers_pencil_only_drawing = 2,
            }
            assert!(validate_capabilities(&caps).is_err());
        }
        let mut event = event(BARREL, 99, ffi::PHASE_ENDED, false);
        assert!(
            capability_action(
                &event,
                &anchors(false, None),
                Some(&ffi::Capabilities::default())
            )
            .is_err()
        );
        event.preferred = 0;
        event.timestamp_s = f64::NAN;
        assert!(accessory_action(&event, &anchors(false, None)).is_err());
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

#[cfg(test)]
#[path = "touch_input_routing_tests.rs"]
mod routing_tests;
