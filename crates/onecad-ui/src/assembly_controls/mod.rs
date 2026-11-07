//! Native embedded assembly and literal local-datum joint controls.
use crate::{
    app::App,
    bridge::Command,
    diagnostics::{Source, State as DiagnosticState},
};
use onecad_assembly::{
    joint::{Joint, JointId},
    occurrence::{ComponentDefId, OccurrenceId},
};
use onecad_core::edit::AssemblyEdit;

pub(crate) mod definition_edit;
#[cfg(test)]
pub(crate) mod definition_edit_tests;
mod panel;
mod service;
pub(crate) use panel::panel;
pub(crate) use service::execute;
#[cfg(test)]
pub(crate) use service::run;

#[derive(Default)]
pub(crate) struct State {
    pub sequence: u64,
    pub pending: bool,
    pub pending_session: Option<String>,
    pub cancellation: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    pub recovery_session: Option<String>,
    pub definition: Option<ComponentDefId>,
    pub occurrence: Option<OccurrenceId>,
    pub joint: Option<JointId>,
    pub name: String,
    pub occurrence_name: String,
    pub position: [f64; 3],
    pub definition_translation: [f64; 3],
    pub quaternion: [f64; 4],
    pub pose_for: Option<OccurrenceId>,
    pub draft_scene: Option<onecad_scene::SceneIdentity>,
    pub draft: Option<Joint>,
    pub drive: bool,
    pub report: Option<onecad_app::joint_execution::JointSolveReport>,
    pub origin_picks: Vec<OriginPick>,
    pub unbound_sides: Vec<onecad_core::document::component::JointSide>,
}

#[derive(Debug, Clone)]
pub(crate) struct OriginPick {
    pub side: onecad_core::document::component::JointSide,
    pub selection: crate::viewport_selection::OccurrenceSelection,
    pub mesh_generation: u64,
}

#[derive(Debug)]
pub(crate) enum Action {
    Edit(AssemblyEdit),
    #[cfg_attr(not(test), allow(dead_code))]
    Solve {
        joints: Vec<Joint>,
        drives: Vec<(JointId, f64)>,
    },
    SolveBound {
        joints: Vec<Joint>,
        drives: Vec<(JointId, f64)>,
        joint: JointId,
        picks: Vec<OriginPick>,
        unbound: Vec<onecad_core::document::component::JointSide>,
    },
}
#[derive(Debug)]
pub(crate) struct Request {
    pub sequence: u64,
    pub session: String,
    pub generation: u64,
    pub action: Action,
    pub cancellation: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}

#[derive(Debug)]
pub(crate) struct Reply {
    pub report: Option<onecad_app::joint_execution::JointSolveReport>,
    pub session: String,
    pub capture_error: Option<String>,
    pub changed: bool,
}

pub(crate) fn submit(app: &mut App, action: Action) {
    if app.doc.assembly_controls.pending
        || app.busy()
        || app.sketch_active()
        || app.doc.interaction.tool.is_some()
    {
        app.doc.status = "Finish the current operation first".into();
        return;
    }
    let Some(display) = &app.doc.display else {
        return;
    };
    let state = &mut app.doc.assembly_controls;
    let Some(sequence) = state.sequence.checked_add(1) else {
        app.doc.status = "Assembly request sequence exhausted".into();
        return;
    };
    let cancellation = matches!(&action, Action::Solve { .. } | Action::SolveBound { .. })
        .then(|| std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)));
    let request = Request {
        cancellation: cancellation.clone(),
        sequence,
        session: display.context.runtime_session.clone(),
        generation: display.assembly.persisted_generation,
        action,
    };
    match app.doc.bridge.submit(Command::Assembly(request)) {
        Ok(()) => {
            state.sequence = sequence;
            state.pending = true;
            state.pending_session = Some(display.context.runtime_session.clone());
            state.cancellation = cancellation;
            app.doc.status = "Applying assembly change…".into();
            app.doc
                .diagnostics
                .record(Source::Assembly, DiagnosticState::Pending, &app.doc.status);
        }
        Err(error) => {
            app.doc.status = error;
            app.doc
                .diagnostics
                .record(Source::Assembly, DiagnosticState::Failed, &app.doc.status);
        }
    }
}

pub(crate) fn receive(app: &mut App, sequence: u64, session: &str, result: Result<Reply, String>) {
    let current = app
        .doc
        .bridge
        .shared
        .document_state
        .lock()
        .ok()
        .and_then(|slot| slot.latest())
        .map(|(_, state)| state.runtime_session);
    let state = &mut app.doc.assembly_controls;
    if !state.accepts(sequence, session, current.as_deref()) {
        return;
    }
    state.pending = false;
    state.pending_session = None;
    state.cancellation = None;
    let diagnostic_message = result.as_ref().err().cloned();
    let diagnostic_state = match &result {
        Ok(reply) if reply.capture_error.is_some() => DiagnosticState::Failed,
        Ok(_) => DiagnosticState::Succeeded,
        Err(error) if error == "stale preview: joint solve cancelled" => DiagnosticState::Cancelled,
        Err(_) => DiagnosticState::Failed,
    };
    match result {
        Ok(reply) => {
            state.report = reply.report;
            state.origin_picks.clear();
            state.unbound_sides.clear();
            if let Some(error) = reply.capture_error {
                if reply.changed {
                    state.recovery_session = Some(reply.session);
                    app.doc.status = format!(
                        "Assembly change accepted, but scene refresh failed: {error}. Undo is available."
                    );
                } else {
                    state.recovery_session = None;
                    app.doc.status = format!(
                        "Requested edit made no history change; scene refresh failed: {error}."
                    );
                }
            } else {
                state.recovery_session = None;
                app.doc.status = "Assembly updated".into();
            }
        }
        Err(error) => app.doc.status = format!("Assembly: {error}"),
    }
    app.doc.diagnostics.record(
        Source::Assembly,
        diagnostic_state,
        diagnostic_message.as_deref().unwrap_or(&app.doc.status),
    );
    app.draw_dirty = true;
}

pub(crate) fn cancel_pending(
    shared: &crate::bridge::Shared,
    token: &std::sync::atomic::AtomicBool,
) -> Result<(), String> {
    token.store(true, std::sync::atomic::Ordering::Release);
    shared.cancel_joint_solve()
}

pub(crate) fn cancel(app: &mut App) {
    let Some(token) = &app.doc.assembly_controls.cancellation else {
        return;
    };
    match cancel_pending(&app.doc.bridge.shared, token) {
        Ok(()) => app.doc.status = "Cancelling joint solve…".into(),
        Err(error) => app.doc.status = format!("Cancellation failed: {error}"),
    }
}

pub(crate) fn recovery_undo_available(app: &App) -> bool {
    app.doc
        .assembly_controls
        .recovery_session
        .as_ref()
        .is_some_and(|session| {
            app.doc
                .document_state
                .as_ref()
                .is_some_and(|doc| &doc.runtime_session == session)
        })
}

impl State {
    fn accepts(&self, sequence: u64, session: &str, current: Option<&str>) -> bool {
        self.pending
            && self.sequence == sequence
            && self.pending_session.as_deref() == Some(session)
            && current == Some(session)
    }

    fn reset_session(&mut self) {
        if let Some(cancelled) = &self.cancellation {
            cancelled.store(true, std::sync::atomic::Ordering::Release);
        }
        *self = Self {
            sequence: self.sequence,
            ..Self::default()
        };
    }
}

pub(crate) fn reset_session(app: &mut App) {
    app.doc.assembly_controls.reset_session();
}

#[cfg(test)]
mod session_tests {
    use super::*;
    #[test]
    fn old_success_and_error_receipts_cannot_cross_same_bridge_replacement() {
        let token = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut state = State {
            sequence: 7,
            pending: true,
            pending_session: Some("old".into()),
            cancellation: Some(token.clone()),
            ..State::default()
        };
        assert!(state.accepts(7, "old", Some("old")));
        assert!(!state.accepts(7, "old", Some("new")));
        state.reset_session();
        assert!(token.load(std::sync::atomic::Ordering::Acquire));
        assert_eq!(state.sequence, 7);
        state.sequence += 1;
        state.pending = true;
        state.pending_session = Some("new".into());
        assert!(!state.accepts(7, "old", Some("new")));
        assert!(!state.accepts(8, "old", Some("new")));
        assert!(state.accepts(8, "new", Some("new")));
    }
}
