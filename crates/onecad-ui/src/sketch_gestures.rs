//! Native pointer gestures keep checked preview geometry separate from authority.
mod pointer;
use crate::app::App;
use onecad_interaction::sketch::{
    SketchAction, SketchSessionKey,
    gesture::{GestureAction, GestureEffect, GestureTag},
};

pub(crate) struct Followup {
    key: SketchSessionKey,
    action: SketchAction,
}

impl App {
    pub(crate) fn gesture_dispatch(&mut self, action: GestureAction) {
        if self.doc.bridge.shared.input_trace.enabled() && !matches!(action, GestureAction::Sync) {
            crate::input_trace::app(
                self,
                "gesture.dispatch",
                crate::input_trace::actions::gesture(&action),
            );
        }
        let before = self.doc.sketch_gesture.error().map(str::to_owned);
        let effects = self.doc.sketch_gesture.dispatch(action, &self.doc.sketch);
        for effect in effects {
            let tag = effect_tag(&effect).clone();
            if matches!(effect, GestureEffect::Begin { .. }) {
                self.doc.gesture_expected = self
                    .doc
                    .display
                    .as_ref()
                    .map(|display| display.context.clone());
            }
            let result = self
                .doc
                .gesture_expected
                .clone()
                .ok_or_else(|| "Gesture requires an accepted document capture".to_owned())
                .and_then(|expected| self.doc.bridge.submit_gesture(effect, expected));
            if let Err(message) = result {
                self.gesture_dispatch(GestureAction::Failed { tag, message });
            }
        }
        if !self.doc.sketch_gesture.busy() {
            self.doc.gesture_expected = None;
        }
        if let Some(error) = self
            .doc
            .sketch_gesture
            .error()
            .filter(|error| before.as_deref() != Some(*error))
        {
            self.doc.status = error.to_owned();
            self.doc.diagnostics.record(
                crate::diagnostics::Source::Sketch,
                crate::diagnostics::State::Rejected,
                error,
            );
        }
    }

    pub(crate) fn cancel_sketch_gesture(&mut self) {
        self.doc.armed_sketch_gesture = None;
        if let Some(tag) = self.doc.sketch_gesture.tag().cloned() {
            self.gesture_dispatch(GestureAction::Cancel { tag });
        }
    }

    pub(crate) fn sync_sketch_gesture(&mut self) {
        let capture_changed = self.doc.sketch_gesture.tag().is_some_and(|tag| {
            self.doc.view.as_ref().map(|view| view.identity()) != Some(tag.scope.capture)
        });
        if !self.viewport_ready() || !self.doc.bridge.valid() || self.busy() || capture_changed {
            self.cancel_sketch_gesture();
        } else {
            self.gesture_dispatch(GestureAction::Sync);
        }
    }

    pub(crate) fn defer_for_sketch_gesture(
        &mut self,
        action: SketchAction,
    ) -> Option<SketchAction> {
        if !self.doc.sketch_gesture.busy() {
            return Some(action);
        }
        match action {
            SketchAction::Escape => self.cancel_sketch_gesture(),
            action @ (SketchAction::SetTool(_)
            | SketchAction::Finish
            | SketchAction::Discard
            | SketchAction::History(_)) => {
                if let Some(tag) = self.doc.sketch_gesture.tag() {
                    self.doc.gesture_followup = Some(Followup {
                        key: tag.scope.key.clone(),
                        action,
                    });
                }
                self.cancel_sketch_gesture();
            }
            action @ (SketchAction::ContextChanged(_) | SketchAction::AcceptView { .. }) => {
                return Some(action);
            }
            action if presentation_reply(&action) => return Some(action),
            _ => {
                self.doc.status = "Finish or cancel the drag before editing the sketch".into();
            }
        }
        None
    }

    pub(crate) fn gesture_delivery(&mut self, delivery: crate::gesture_services::Delivery) {
        if delivery.accepted {
            self.sync_document_state();
        }
        let current = self.doc.sketch_gesture.tag().is_some_and(|tag| {
            tag.gesture == delivery.tag.gesture && tag.scope == delivery.tag.scope
        });
        for action in delivery.actions {
            self.gesture_dispatch(action);
        }
        if let Some(display) = delivery.display
            && self.doc.bridge.shared.accept_display(&display)
        {
            self.adopt_display(display);
        }
        if delivery.accepted
            && let Some(display) = self.doc.bridge.take_display()
            && self.doc.bridge.shared.accept_display(&display)
        {
            self.adopt_display(display);
        }
        let accepted_without_scene = delivery.accepted
            && delivery.moved
            && delivery.accepted_revision.is_some_and(|revision| {
                self.doc.document_state.as_ref().is_some_and(|state| {
                    state.runtime_session == delivery.tag.scope.document.runtime_session
                        && state.revision >= revision
                }) && self.doc.display.as_ref().is_none_or(|display| {
                    display.context.runtime_session != delivery.tag.scope.document.runtime_session
                        || display.context.revision.0 < revision
                })
            });
        if accepted_without_scene {
            self.doc.bridge.shared.invalidate(Some(
                "Drag committed; final scene is unavailable. Save remains available.".into(),
            ));
            self.doc.view = None;
            self.doc.armed_sketch_gesture = None;
            if let Some(session) = self
                .doc
                .sketch
                .session
                .as_mut()
                .filter(|session| session.key == delivery.tag.scope.key)
            {
                session.accepted_capture = None;
                session.selection_dragging = false;
                session.phase = onecad_interaction::sketch::SketchPhase::NeedsRefresh;
                session.status = onecad_interaction::sketch::SketchStatus::Refused(
                    "Drag committed; refresh the accepted scene before editing".into(),
                );
            }
        }
        if let Some(error) = delivery.error.filter(|_| current) {
            self.doc.status = if accepted_without_scene {
                format!("Drag committed; accepted document was preserved. {error}")
            } else {
                error
            };
            self.doc.diagnostics.record(
                crate::diagnostics::Source::Sketch,
                crate::diagnostics::State::Failed,
                &self.doc.status,
            );
        }
        self.replay_gesture_followup();
        crate::input_trace::app(
            self,
            "gesture.delivered",
            serde_json::json!({"accepted": delivery.accepted, "moved": delivery.moved, "revision": delivery.accepted_revision}),
        );
    }

    pub(crate) fn replay_gesture_followup(&mut self) {
        let Some(followup) = self.doc.gesture_followup.as_ref() else {
            return;
        };
        if self
            .doc
            .sketch
            .session
            .as_ref()
            .is_none_or(|session| session.key != followup.key)
        {
            self.doc.gesture_followup = None;
            return;
        }
        if !self.doc.sketch_gesture.busy()
            && !self.busy()
            && self.doc.sketch.session.as_ref().is_some_and(|session| {
                session.phase == onecad_interaction::sketch::SketchPhase::Active
            })
            && let Some(followup) = self.doc.gesture_followup.take()
        {
            self.sketch_dispatch(followup.action);
        }
    }
}

fn presentation_reply(action: &SketchAction) -> bool {
    matches!(
        action,
        SketchAction::Created { .. }
            | SketchAction::Entered { .. }
            | SketchAction::Refreshed { .. }
            | SketchAction::Solved { .. }
            | SketchAction::BatchAccepted { .. }
            | SketchAction::Failed { .. }
    )
}

fn effect_tag(effect: &GestureEffect) -> &GestureTag {
    match effect {
        GestureEffect::Begin { tag, .. }
        | GestureEffect::Update { tag, .. }
        | GestureEffect::End { tag, .. }
        | GestureEffect::Cancel { tag, .. } => tag,
    }
}

pub(crate) fn route_raw(app: &mut App, ctx: &egui::Context, raw: &egui::RawInput) {
    app.sync_sketch_gesture();
    if !raw.focused
        || raw
            .events
            .iter()
            .any(|event| matches!(event, egui::Event::PointerGone))
        || crate::input::ui_owns_keyboard(&app.ui, app.doc.sketch_entry.open, ctx)
    {
        app.cancel_sketch_gesture();
        crate::sketch_view::cancel_selection_pointer(ctx);
    }
}

pub(crate) use pointer::{Armed, delivered_inside, viewport};
