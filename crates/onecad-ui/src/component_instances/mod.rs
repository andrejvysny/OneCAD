//! Placed component instances: Explorer rows, the Inspector panel, detach and the
//! upgrade review, all over the application's instance services.
//!
//! The listing is per document (`app.doc.instances`), refreshed whenever the document
//! revision or the library changes. Mutations ride the same ordered service actor as
//! every other edit; the service refreshes the projection and display exactly as a
//! committed edit does, so undo and the viewport follow.
use crate::{app::App, bridge::Command};
use onecad_app::dto::{ComponentInstanceDto, LibraryComponentDto, UpgradeItemDto};
use onecad_core::document::record::ComponentParamValue;
use std::collections::BTreeMap;

pub(crate) mod model;
mod panel;
mod review;
mod review_dialog;
mod section;
mod service;
mod state;

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) use panel::DETACH_WARNING;
pub(crate) use panel::panel;
pub(crate) use section::section;
pub(crate) use service::execute;
pub(crate) use state::State;

use state::Outcome;

/// The document a request belongs to; replies for any other are dropped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Scope {
    pub document_id: String,
    pub runtime_session: String,
}

#[derive(Clone, Debug)]
pub(crate) enum Op {
    List,
    Plan {
        record_ids: Vec<String>,
    },
    SetParams {
        record_id: String,
        params: BTreeMap<String, ComponentParamValue>,
    },
    Reset {
        record_id: String,
        keys: Option<Vec<String>>,
    },
    Detach {
        record_id: String,
    },
    Upgrade {
        approved: Vec<UpgradeItemDto>,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct Request {
    pub sequence: u64,
    pub scope: Scope,
    pub op: Op,
}

/// An instance plus the package record its parameters validate against.
#[derive(Clone, Debug)]
pub(crate) struct InstanceView {
    pub dto: ComponentInstanceDto,
    pub component: Option<LibraryComponentDto>,
}

/// A committed mutation as the host reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Mutated {
    pub what: &'static str,
    /// The request changed nothing, so no history step exists.
    pub noop: bool,
    /// Why regeneration did not publish cleanly, when it did not.
    pub failure: Option<String>,
}

#[derive(Debug)]
pub(crate) enum Reply {
    Listed(Vec<InstanceView>),
    Planned(Vec<UpgradeItemDto>),
    Mutated(Box<Mutated>),
}

pub(crate) fn scope(app: &App) -> Option<Scope> {
    let state = app.doc.document_state.as_ref()?;
    Some(Scope {
        document_id: state.document_id.clone(),
        runtime_session: state.runtime_session.clone(),
    })
}

/// Why instance edits are off right now, or `None`.
pub(crate) fn unavailable(app: &App) -> Option<&'static str> {
    if !app.document_open() {
        Some("Open a model first")
    } else if app.doc.instances.blocks() {
        Some("Wait for the current component update to finish")
    } else if app.busy()
        || app.sketch_active()
        || app.doc.interaction.tool.is_some()
        || app.doc.sketch_entry.open
    {
        Some("Finish the current operation first")
    } else if !app.doc.bridge.valid() {
        Some("Geometry worker unavailable")
    } else {
        None
    }
}

/// Frame hook: keeps the listing current, sends queued requests, draws the dialogs.
pub(crate) fn show(app: &mut App, ctx: &egui::Context) {
    refresh(app);
    flush(app, ctx);
    review_dialog::show(app, ctx);
    panel::detach_dialog(app, ctx);
}

fn refresh(app: &mut App) {
    let Some(scope) = scope(app) else {
        app.doc.instances.forget();
        return;
    };
    let revision = app.doc.document_state.as_ref().map_or(0, |s| s.revision);
    let generation = app.ui.library.catalog.generation();
    app.doc.instances.sync(&scope, revision, generation);
}

fn flush(app: &mut App, ctx: &egui::Context) {
    for request in app.doc.instances.take_outbox() {
        let (sequence, scope) = (request.sequence, request.scope.clone());
        if let Err(error) = app.doc.bridge.submit(Command::Instance(request)) {
            let outcome = app.doc.instances.refuse(&scope, sequence, error);
            report(app, outcome);
            ctx.request_repaint();
        }
    }
}

/// A service reply arrived for this document.
pub(crate) fn receive(app: &mut App, sequence: u64, result: Result<Reply, String>) {
    let Some(scope) = scope(app) else { return };
    let outcome = app.doc.instances.accept(&scope, sequence, result);
    report(app, outcome);
}

fn report(app: &mut App, outcome: Outcome) {
    use crate::diagnostics::State;
    let (state, message) = match outcome {
        Outcome::Mutated(done) if done.noop => {
            (State::Succeeded, format!("{}: no change", done.what))
        }
        Outcome::Mutated(done) => {
            app.doc.authoring_dirty = true;
            match done.failure {
                Some(why) => (
                    State::Failed,
                    format!("{}: regeneration failed: {why}", done.what),
                ),
                None => (State::Succeeded, format!("{} complete", done.what)),
            }
        }
        // The review re-plans and says so itself.
        Outcome::Failed { error, .. } if error.contains(review::STALE) => return,
        Outcome::Failed { what, error } => (State::Failed, format!("{what} failed: {error}")),
        Outcome::ListFailed(error) => (State::Failed, format!("Component list failed: {error}")),
        Outcome::PlanFailed(error) => (State::Failed, format!("Update plan failed: {error}")),
        Outcome::Dropped | Outcome::Listed | Outcome::Planned => return,
    };
    app.doc.status = message;
    app.doc.command_status = true;
    app.doc.diagnostics.record(
        crate::diagnostics::Source::Application,
        state,
        &app.doc.status,
    );
}
