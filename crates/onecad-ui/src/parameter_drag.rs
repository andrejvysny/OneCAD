//! Shared parameter-drag lifecycle for the Extrude handle and the Move/Rotate
//! gizmo: nothing is authored before accepted motion changes the value, the
//! reducer owns the original, and only Apply writes history.
use crate::app::App;
use egui::Pos2;
use onecad_interaction::{Action, DragPhase};

/// Pointer values already have canonical units; expressions still use application evaluation.
pub(crate) fn scalar_update(key: onecad_interaction::ParameterKey, text: String) -> Action {
    let phase = match text.parse::<f64>() {
        Ok(number) => DragPhase::Value {
            key,
            text,
            value: onecad_core::expr::Value {
                number,
                dim: key.parameter().dimension(),
            },
        },
        Err(_) => DragPhase::Update { key, text },
    };
    Action::ParameterDrag(phase)
}

/// Sketch drag rule: travel must leave a 4 px box around the press, per axis.
pub(crate) const THRESHOLD_PX: f32 = 4.0;

/// Terminal outcome of one pointer capture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// Release; `changed` is false when the value reads as the original.
    Finish { changed: bool },
    /// Escape: the first one cancels only the drag.
    Cancel,
    /// Pointer, focus, view, target or document loss.
    Lost,
}

/// Reducer side of one capture.
#[derive(Debug, Default)]
pub(crate) struct Lifecycle {
    begun: bool,
}

impl Lifecycle {
    pub(crate) fn begun(&self) -> bool {
        self.begun
    }

    /// Opens the reducer drag on the first authored value, then authors `update`.
    /// Begun only once the reducer holds the snapshot: a refused Begin (file
    /// action running) retries on the next value, so no update ever lands
    /// without an original to restore.
    pub(crate) fn author(&mut self, app: &mut App, update: Action) {
        if !self.begun {
            app.dispatch(Action::ParameterDrag(DragPhase::Begin));
            self.begun = app
                .doc
                .interaction
                .tool
                .as_ref()
                .is_some_and(|tool| tool.dragging());
        }
        app.dispatch(update);
        app.draw_dirty = true;
    }

    /// Only a changed release keeps the value; every other end restores the
    /// original exactly, expressions included.
    pub(crate) fn end(self, app: &mut App, outcome: Outcome) {
        if !self.begun || app.doc.interaction.tool.is_none() {
            return;
        }
        let phase = match outcome {
            Outcome::Finish { changed: true } => DragPhase::Commit,
            _ => DragPhase::Cancel,
        };
        app.dispatch(Action::ParameterDrag(phase));
        app.draw_dirty = true;
    }
}

pub(crate) fn beyond_threshold(press: Pos2, position: Pos2, threshold_px: f32) -> bool {
    (position.x - press.x).abs() > threshold_px || (position.y - press.y).abs() > threshold_px
}

pub(crate) fn active(app: &App) -> bool {
    app.doc.extrude_handle.active()
        || app.doc.model_handle.active()
        || app.doc.transform_gizmo.gizmo.active()
}

/// Ends captures whose release or loss never reached the viewport this pass.
pub(crate) fn settle(app: &mut App, ctx: &egui::Context) {
    crate::extrude_handle::settle(app, ctx);
    crate::model_handle::settle(app, ctx);
    crate::transform_ui::settle(app, ctx);
}

/// Restores the authored values of every open capture (Escape, tab deactivation).
pub(crate) fn abandon(app: &mut App, outcome: Outcome) {
    crate::extrude_handle::abandon(app, outcome);
    crate::model_handle::abandon(app, outcome);
    crate::transform_ui::abandon(app, outcome);
}

/// Raw stage: the first Escape during a capture cancels only the drag and is
/// consumed; a later Escape reaches the tool. An Escape delivered after the
/// capture's release or loss in the same batch belongs to the tool: the
/// viewport ends the capture at that earlier event.
pub(crate) fn route_escape(app: &mut App, raw: &mut egui::RawInput, composition: bool) {
    if composition || !active(app) {
        return;
    }
    let Some(index) = raw.events.iter().position(|event| {
        matches!(
            event,
            egui::Event::Key {
                key: egui::Key::Escape,
                pressed: true,
                ..
            }
        )
    }) else {
        return;
    };
    if raw.events[..index].iter().any(ends_capture) {
        return;
    }
    raw.events.remove(index);
    abandon(app, Outcome::Cancel);
}

fn ends_capture(event: &egui::Event) -> bool {
    matches!(
        event,
        egui::Event::PointerButton {
            button: egui::PointerButton::Primary,
            pressed: false,
            ..
        } | egui::Event::PointerGone
            | egui::Event::WindowFocused(false)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn threshold_is_strict_per_axis_like_sketch_drags() {
        let press = egui::pos2(100.0, 100.0);
        assert!(!beyond_threshold(
            press,
            press + egui::vec2(4.0, -4.0),
            THRESHOLD_PX
        ));
        assert!(!beyond_threshold(
            press,
            press + egui::vec2(3.0, 3.0),
            THRESHOLD_PX
        ));
        assert!(beyond_threshold(
            press,
            press + egui::vec2(4.01, 0.0),
            THRESHOLD_PX
        ));
        assert!(beyond_threshold(
            press,
            press + egui::vec2(0.0, -4.01),
            THRESHOLD_PX
        ));
        assert!(beyond_threshold(press, press + egui::vec2(0.1, 0.0), 0.0));
    }
}
