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
        let invalid = |what: &str| Failure::new(Status::InvalidArgument, what.to_owned());
        let kind =
            AccessoryEvent::from_abi(event.action).ok_or_else(|| invalid("pencil action"))?;
        let preference =
            Preference::from_abi(event.preferred).ok_or_else(|| invalid("preferred action"))?;
        let completed = completed(kind, event.phase).ok_or_else(|| invalid("pencil phase"))?;
        // Unavailable capabilities stay unavailable: no hover pose without hover.
        let hovering =
            self.capabilities.flags & ffi::CAP_HOVER != 0 && event.present & ffi::HAS_LOCATION != 0;
        let hover = hovering.then(|| Point::new(event.x_pt, event.y_pt));
        let last_pencil = self
            .input
            .pencil_position()
            .map(|at| Point::new(f64::from(at.x), f64::from(at.y)));
        let anchor = accessory::palette_anchor(hover, last_pencil, self.canvas_center());
        match accessory::action(kind, preference, completed, anchor) {
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
