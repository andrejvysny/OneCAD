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
        self.sketch
            .session
            .as_ref()
            .is_some_and(|session| session.phase != SketchPhase::Closed)
    }
    pub fn sketch_dispatch(&mut self, action: SketchAction) {
        let align_entry = matches!(
            action,
            SketchAction::Begin { .. } | SketchAction::BeginCreated { .. }
        );
        if matches!(action, SketchAction::BeginCreated { .. }) {
            self.ui.sketch_entry = Default::default();
        }
        if self.bridge.shared.input_trace.enabled()
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
            && let Some(armed) = self.armed_sketch_gesture.as_mut()
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
            self.armed_sketch_gesture = None;
        }
        let Some(action) = self.defer_for_sketch_gesture(action) else {
            return;
        };
        if self.documents.blocks_sketch_authoring(&action) {
            self.command_status = true;
            self.status = "Finish the file action before editing the sketch".into();
            crate::input_trace::app(
                self,
                "sketch.refused",
                serde_json::json!({"reason": self.status}),
            );
            return;
        }
        if let Some(report) = crate::profile_diagnostics::Report::observe(&self.sketch, &action) {
            self.diagnostics.sketch_visit(report.key());
            self.diagnostics.profile = Some(report);
            self.ui.inspector_open = true;
            self.ui.inspector_tab = crate::ui_state::InspectorTab::Properties;
        }
        for effect in self.sketch.dispatch(action) {
            let tag = effect_tag(&effect).clone();
            if let Err(message) = self.submit_sketch_effect(effect) {
                self.sketch_dispatch(SketchAction::Failed { tag, message });
            }
        }
        if align_entry && let Some(session) = &self.sketch.session {
            crate::sketch_entry::align_camera(&mut self.camera, session.plane);
            self.draw_dirty = true;
        }
        self.sync_sketch_gesture();
        self.replay_gesture_followup();
        if let Some(session) = &self.sketch.session {
            if self
                .document_state
                .as_ref()
                .is_some_and(|state| state.runtime_session != session.document.runtime_session)
            {
                return;
            }
            self.diagnostics.sketch_visit(&session.key);
            self.status = match &session.status {
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
                SketchStatus::Finished { kept_reason, .. } => {
                    kept_reason.clone().unwrap_or("Sketch finished".into())
                }
                SketchStatus::Discarded(_) => "Sketch edits discarded".into(),
            };
            use crate::diagnostics::{Source, State};
            let state = match &session.status {
                SketchStatus::Starting | SketchStatus::Pending | SketchStatus::Accepted => {
                    State::Pending
                }
                SketchStatus::Refused(_) => State::Rejected,
                SketchStatus::Failed { .. } => State::Failed,
                SketchStatus::Finished {
                    kept_reason: Some(_),
                    ..
                } => State::Warning,
                SketchStatus::Discarded(_) => State::Cancelled,
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
            self.diagnostics.record(Source::Sketch, state, &self.status);
            if matches!(
                session.status,
                SketchStatus::Refused(_) | SketchStatus::Failed { .. }
            ) {
                crate::input_trace::app(
                    self,
                    "sketch.refused",
                    serde_json::json!({"reason": crate::input_trace::bounded_id(&self.status)}),
                );
            }
        }
        if !self
            .diagnostics
            .profile
            .as_ref()
            .is_some_and(|report| report.is_current(&self.sketch))
            && let Some(display) = self.display.as_ref()
            && let Some(report) =
                crate::profile_diagnostics::Report::from_display(&self.sketch, display)
        {
            self.diagnostics.profile = Some(report);
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
            self.status = "Turn off section display before editing geometry".into();
            return;
        }
        let Some(context) = self.interaction.context.clone() else {
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
            self.status = "Turn off section display before editing geometry".into();
            return;
        }
        let Some(context) = self.interaction.context.clone() else {
            return;
        };
        self.dispatch(onecad_interaction::Action::Cancel);
        self.submit(Command::Sketch(Box::new(Request::Load { id, context })));
    }
}

/// Sketch tools are clickable only on an active, bridge-valid session.
pub fn tools_enabled(app: &App) -> bool {
    app.viewport_ready() && !app.busy() && !app.sketch_gesture.busy() && can_finish(app)
}
fn can_finish(app: &App) -> bool {
    app.sketch
        .session
        .as_ref()
        .is_some_and(|session| session.phase == SketchPhase::Active && app.bridge.valid())
}
/// Finish and Discard stay available on a faulted session so it can be left.
pub fn can_exit(app: &App) -> bool {
    can_finish(app) || session_faulted(app)
}
fn session_faulted(app: &App) -> bool {
    app.sketch
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
    let Some(session) = app.sketch.session.as_ref() else {
        crate::sketch_creation::clear_candidate(ui.ctx());
        return;
    };
    if session.phase == SketchPhase::Closed {
        crate::sketch_creation::clear_candidate(ui.ctx());
        return;
    }
    if session.phase != SketchPhase::Active || (!app.viewport_ready() || !app.bridge.valid()) {
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
    let Some(session) = app.sketch.session.as_ref() else {
        return;
    };
    if session.tool == SketchTool::Select {
        crate::sketch_creation::clear_candidate(ui.ctx());
        if crate::sketch_gestures::viewport(app, ui, rect, response, view) {
            return;
        }
        let Some(session) = app.sketch.session.as_ref() else {
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
                app.status = message.clone();
                app.command_status = true;
                app.diagnostics.record(
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
