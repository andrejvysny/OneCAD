//! Sketch chrome and input emit typed actions; solved geometry stays application-owned.
use crate::{
    app::App,
    bridge::Command,
    sketch_services::{Request, effect_tag},
};
use onecad_core::{
    ids::SketchId,
    sketch::{Sketch, SketchAttachment, WorldPlane},
};
use onecad_interaction::sketch::{SketchAction, SketchPhase, SketchStatus, SketchTool};
use onecad_scene::ViewSnapshot;

impl App {
    pub fn sketch_active(&self) -> bool {
        self.doc
            .sketch
            .session
            .as_ref()
            .is_some_and(|session| session.phase != SketchPhase::Closed)
    }
    pub fn sketch_dispatch(&mut self, action: SketchAction) {
        let align_entry = matches!(
            action,
            SketchAction::Begin { .. } | SketchAction::BeginCreated { .. }
        );
        let was_active = self.sketch_active();
        if matches!(action, SketchAction::BeginCreated { .. }) {
            self.doc.sketch_entry = Default::default();
        }
        if self.doc.bridge.shared.input_trace.enabled()
            && !matches!(
                action,
                SketchAction::AcceptView { .. } | SketchAction::Move(_)
            )
        {
            crate::input_trace::app(
                self,
                "sketch.dispatch",
                crate::input_trace::actions::sketch(&action),
            );
        }
        if matches!(action, SketchAction::Escape)
            && let Some(armed) = self.doc.armed_sketch_gesture.as_mut()
        {
            armed.cancel();
            return;
        }
        if matches!(
            action,
            SketchAction::SetTool(_)
                | SketchAction::Finish
                | SketchAction::Discard
                | SketchAction::History(_)
        ) {
            self.doc.armed_sketch_gesture = None;
        }
        let Some(action) = self.defer_for_sketch_gesture(action) else {
            return;
        };
        if self.doc.documents.blocks_sketch_authoring(&action) {
            self.doc.command_status = true;
            self.doc.status = "Finish the file action before editing the sketch".into();
            crate::input_trace::app(
                self,
                "sketch.refused",
                serde_json::json!({"reason": self.doc.status}),
            );
            return;
        }
        if let Some(report) = crate::profile_diagnostics::Report::observe(&self.doc.sketch, &action)
        {
            self.doc.diagnostics.sketch_visit(report.key());
            self.doc.diagnostics.profile = Some(report);
            // The inspector is window chrome: a background document never opens it.
            if !self.parked {
                self.ui.inspector_open = true;
                self.ui.inspector_tab = crate::ui_state::InspectorTab::Properties;
            }
        }
        let tool_before = self.doc.sketch.session.as_ref().map(|s| s.tool);
        for effect in self.doc.sketch.dispatch(action) {
            let tag = effect_tag(&effect).clone();
            if let Err(message) = self.submit_sketch_effect(effect) {
                self.sketch_dispatch(SketchAction::Failed { tag, message });
            }
        }
        let tool_after = self.doc.sketch.session.as_ref().map(|s| s.tool);
        if tool_before.is_some() && tool_after.is_some() && tool_before != tool_after {
            self.ui.previous_sketch_tool = tool_before;
        }
        if align_entry && let Some(session) = &self.doc.sketch.session {
            self.doc.modeling_camera.get_or_insert(self.doc.camera);
            crate::sketch_entry::align_camera(&mut self.doc.camera, session.plane);
            self.draw_dirty = true;
        }
        if was_active
            && !self.sketch_active()
            && self.doc.sketch.session.as_ref().is_some_and(|session| {
                matches!(
                    session.status,
                    SketchStatus::Finished { .. }
                        | SketchStatus::Discarded(_)
                        | SketchStatus::FinishUncertain(_)
                )
            })
            && let Some(camera) = self.doc.modeling_camera.take()
        {
            self.doc.camera = camera;
            self.draw_dirty = true;
        }
        self.sync_sketch_gesture();
        self.replay_gesture_followup();
        if let Some(session) = &self.doc.sketch.session {
            if self
                .doc
                .document_state
                .as_ref()
                .is_some_and(|state| state.runtime_session != session.document.runtime_session)
            {
                return;
            }
            self.doc.diagnostics.sketch_visit(&session.key);
            self.doc.status = match &session.status {
                SketchStatus::Starting => "Opening sketch…".into(),
                SketchStatus::Entered(_) | SketchStatus::Solved(_) => {
                    crate::sketch_feedback::summary(&session.status).label
                }
                SketchStatus::Pending => "Solving sketch…".into(),
                SketchStatus::Accepted => {
                    "Sketch batch accepted; waiting for solved snapshot".into()
                }
                SketchStatus::Refused(message) | SketchStatus::Failed { message, .. } => {
                    message.clone()
                }
                SketchStatus::Finished {
                    kept_reason,
                    result,
                } => crate::sketch_feedback::finished(kept_reason.as_deref(), result).0,
                SketchStatus::Discarded(_) => "Sketch edits discarded".into(),
                SketchStatus::FinishUncertain(message) => message.clone(),
            };
            use crate::diagnostics::{Source, State};
            let state = match &session.status {
                SketchStatus::Starting | SketchStatus::Pending | SketchStatus::Accepted => {
                    State::Pending
                }
                SketchStatus::Refused(_) => State::Rejected,
                SketchStatus::Failed { .. } => State::Failed,
                SketchStatus::Finished {
                    kept_reason,
                    result,
                } => crate::sketch_feedback::finished(kept_reason.as_deref(), result).1,
                SketchStatus::Discarded(_) => State::Cancelled,
                SketchStatus::FinishUncertain(_) => State::Warning,
                SketchStatus::Entered(_) | SketchStatus::Solved(_)
                    if matches!(
                        crate::sketch_feedback::summary(&session.status).assessment,
                        crate::sketch_feedback::Assessment::Unavailable
                            | crate::sketch_feedback::Assessment::Conflicting
                    ) =>
                {
                    State::Warning
                }
                _ => State::Succeeded,
            };
            self.doc
                .diagnostics
                .record(Source::Sketch, state, &self.doc.status);
            if matches!(
                session.status,
                SketchStatus::Refused(_) | SketchStatus::Failed { .. }
            ) {
                crate::input_trace::app(
                    self,
                    "sketch.refused",
                    serde_json::json!({"reason": crate::input_trace::bounded_id(&self.doc.status)}),
                );
            }
        }
        if !self
            .doc
            .diagnostics
            .profile
            .as_ref()
            .is_some_and(|report| report.is_current(&self.doc.sketch))
            && let Some(display) = self.doc.display.as_ref()
            && let Some(report) =
                crate::profile_diagnostics::Report::from_display(&self.doc.sketch, display)
        {
            self.doc.diagnostics.profile = Some(report);
            self.ui.inspector_open = true;
            self.ui.inspector_tab = crate::ui_state::InspectorTab::Properties;
        }
    }
    #[cfg(test)]
    pub fn new_sketch(&mut self) {
        self.new_sketch_on_plane(WorldPlane::XY);
    }
    pub fn new_sketch_on_plane(&mut self, plane: WorldPlane) {
        if !self.viewport_ready() || crate::viewport_selection::sectioned(self) {
            self.doc.status = "Turn off section display before editing geometry".into();
            return;
        }
        let Some(context) = self.doc.interaction.context.clone() else {
            return;
        };
        self.dispatch(onecad_interaction::Action::Cancel);
        let sketch = Sketch::new(SketchId::new(), "Sketch", SketchAttachment::World { plane });
        self.sketch_dispatch(SketchAction::Begin {
            sketch: Box::new(sketch),
            create: true,
            context,
        });
    }
    pub fn edit_sketch(&mut self, id: SketchId) {
        if !self.viewport_ready() || crate::viewport_selection::sectioned(self) {
            self.doc.status = "Turn off section display before editing geometry".into();
            return;
        }
        let Some(context) = self.doc.interaction.context.clone() else {
            return;
        };
        self.dispatch(onecad_interaction::Action::Cancel);
        self.submit(Command::Sketch(Box::new(Request::Load { id, context })));
    }
}

/// Sketch tools are clickable only on an active, bridge-valid session.
pub fn tools_enabled(app: &App) -> bool {
    app.viewport_ready() && !app.busy() && !app.doc.sketch_gesture.busy() && can_finish(app)
}
fn can_finish(app: &App) -> bool {
    app.doc
        .sketch
        .session
        .as_ref()
        .is_some_and(|session| session.phase == SketchPhase::Active && app.doc.bridge.valid())
}
/// Finish and Discard stay available on a faulted session so it can be left.
pub fn can_exit(app: &App) -> bool {
    can_finish(app) || session_faulted(app)
}
fn session_faulted(app: &App) -> bool {
    app.doc
        .sketch
        .session
        .as_ref()
        .is_some_and(|session| session.phase == SketchPhase::Faulted)
}

pub fn viewport(
    app: &mut App,
    ui: &egui::Ui,
    rect: egui::Rect,
    response: &egui::Response,
    view: &ViewSnapshot,
) {
    let Some(session) = app.doc.sketch.session.as_ref() else {
        crate::sketch_creation::clear_candidate(ui.ctx());
        return;
    };
    if session.phase == SketchPhase::Closed {
        crate::sketch_creation::clear_candidate(ui.ctx());
        return;
    }
    if session.phase != SketchPhase::Active || (!app.viewport_ready() || !app.doc.bridge.valid()) {
        crate::sketch_creation::clear_candidate(ui.ctx());
        return;
    }
    let key = session.key.clone();
    let document = session.document.clone();
    app.sketch_dispatch(SketchAction::AcceptView {
        key,
        document,
        capture: view.identity(),
    });
    let Some(session) = app.doc.sketch.session.as_ref() else {
        return;
    };
    if session.tool == SketchTool::PenStroke {
        crate::sketch_creation::clear_candidate(ui.ctx());
        crate::sketch_pen_stroke::viewport(app, ui, rect, view);
        return;
    }
    crate::sketch_pen_stroke::clear(ui.ctx());
    if session.tool == SketchTool::Select {
        crate::sketch_creation::clear_candidate(ui.ctx());
        if crate::sketch_gestures::viewport(app, ui, rect, response, view) {
            return;
        }
        let Some(session) = app.doc.sketch.session.as_ref() else {
            return;
        };
        match crate::sketch_view::selection_input(ui, rect, response, view, session) {
            Ok(Some(action)) => app.sketch_dispatch(action),
            Ok(None) => {}
            Err(message) => {
                app.sketch_dispatch(SketchAction::SelectionDrag {
                    key: session.key.clone(),
                    document: session.document.clone(),
                    capture: view.identity(),
                    active: false,
                });
                app.doc.status = message.clone();
                app.doc.command_status = true;
                app.doc.diagnostics.record(
                    crate::diagnostics::Source::Sketch,
                    crate::diagnostics::State::Rejected,
                    &message,
                );
            }
        }
        return;
    }
    crate::sketch_creation::viewport(app, ui, rect, response, view);
}
