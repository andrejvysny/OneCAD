//! Create Component wizard (ONECAD-301): Source, Details, Parameters, Attachments/Save over
//! the application's `save_as_component`.
//!
//! The form is per document (`app.doc.wizard`). Its two backend calls ride
//! `Command::Author`: resolving a picked attachment and the save itself, both fenced to the
//! document they were asked in and serialised with every other edit on its service actor.
use crate::{app::App, bridge::Command, component_instances::Scope};
use onecad_app::{
    dto::LibraryComponentDto,
    library::{AttachmentSpecInput, NewComponentSpec},
};

mod chrome;
mod model;
pub(crate) mod pick;
mod service;
mod steps;
#[cfg(test)]
mod tests;
pub(crate) mod validate;

pub(crate) use model::{Phase, WizardState};
pub(crate) use service::execute;

use model::Landed;

#[derive(Clone, Debug)]
pub(crate) enum Op {
    /// Classify a picked sub-shape into an attachment, fenced to head `snapshot_id`.
    Pick {
        body_id: String,
        topo_key: String,
        name: String,
        snapshot_id: u64,
    },
    Save {
        body_id: String,
        spec: Box<NewComponentSpec>,
        union_solids: bool,
    },
    /// Test setup: a document variable the form can expose.
    #[cfg(test)]
    Variable { name: String, value: f64 },
}

#[derive(Clone, Debug)]
pub(crate) struct Request {
    pub sequence: u64,
    pub scope: Scope,
    pub op: Op,
}

#[derive(Debug)]
pub(crate) enum Reply {
    Picked(Box<AttachmentSpecInput>),
    Saved(Box<LibraryComponentDto>),
    #[cfg(test)]
    Ready,
}

/// Why a component cannot be made right now, else `None`.
pub(crate) fn availability(app: &App) -> Option<&'static str> {
    if !app.document_open() || app.doc.interaction.context.is_none() {
        Some("Open a model first")
    } else if app.sketch_active() {
        Some("Finish the sketch first")
    } else if app.doc.wizard.is_some() {
        Some("A component is already being made")
    } else if app.busy() || app.doc.interaction.tool.is_some() {
        Some("Finish the current operation first")
    } else if !app.doc.bridge.valid() {
        Some("Geometry is unavailable")
    } else if app.doc.selected_body().is_none() {
        Some("Select the body to make into a component")
    } else {
        None
    }
}

/// Starts the wizard on the selected body and returns to the editor.
pub(crate) fn open(app: &mut App) {
    if availability(app).is_some() {
        return;
    }
    let (Some(body), Some(scope)) = (
        app.doc.selected_body(),
        crate::component_instances::scope(app),
    ) else {
        return;
    };
    let name = crate::pill::body_name(app, body);
    app.doc.wizard = Some(WizardState::new(scope, body, name));
    crate::start_screen::leave(app);
}

/// The wizard owns the editor: it is open (the modal) or waiting for a pick.
pub(crate) fn blocks(app: &App) -> bool {
    app.doc.wizard.is_some()
}

pub(crate) fn picking(app: &App) -> bool {
    app.doc
        .wizard
        .as_ref()
        .is_some_and(|w| w.phase == Phase::Picking)
}

pub(crate) fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut wizard) = app.doc.wizard.take() else {
        return;
    };
    let scope = crate::component_instances::scope(app);
    if scope.as_ref() != Some(&wizard.scope) {
        // Another document is current: the form is gone, but a save already on the wire
        // is kept (hidden) until its reply reports the outcome.
        if wizard.phase == Phase::Saving {
            app.doc.wizard = Some(wizard);
        }
        return;
    }
    let variables = app
        .doc
        .display
        .as_ref()
        .map(|d| d.projection.variables.clone())
        .unwrap_or_default();
    let outcome = if wizard.phase == Phase::Picking {
        chrome::pick_banner(ctx, &mut wizard)
    } else {
        chrome::modal(ctx, &mut wizard, &variables)
    };
    let keep = !matches!(outcome, chrome::Outcome::Close);
    let open_saved = if let chrome::Outcome::OpenInLibrary(saved) = &outcome {
        Some(saved.clone())
    } else {
        None
    };
    for request in wizard.take_outbox() {
        let (sequence, scope) = (request.sequence, request.scope.clone());
        if let Err(error) = app.doc.bridge.submit(Command::Author(request)) {
            wizard.refuse(&scope, sequence, error);
        }
    }
    if keep {
        app.doc.wizard = Some(wizard);
    }
    if let Some(saved) = open_saved {
        app.ui.start_open = true;
        crate::library_pages::open_component(&mut app.ui, &saved.id, &saved.version);
    }
}

/// A service reply for this document's wizard.
pub(crate) fn receive(app: &mut App, sequence: u64, result: Result<Reply, String>) {
    let current = crate::component_instances::scope(app);
    let Some(wizard) = app.doc.wizard.as_mut() else {
        return;
    };
    let moved = current.as_ref() != Some(&wizard.scope);
    let landed = wizard.accept(current.as_ref(), sequence, result);
    let failure = wizard.error.clone();
    match landed {
        Landed::Saved(dto) => saved(app, &dto),
        Landed::Refused if moved => {
            app.doc.status = format!(
                "Saving the component failed: {}",
                failure.unwrap_or_default()
            );
            app.doc.command_status = true;
        }
        Landed::Picked | Landed::Refused | Landed::Dropped => {}
    }
    // A form for a document that is no longer current has nothing left to show.
    if moved && app.doc.wizard.as_ref().is_some_and(|w| !w.busy()) {
        app.doc.wizard = None;
    }
}

/// The component is on disk: the catalog re-reads it and the status says so.
fn saved(app: &mut App, dto: &LibraryComponentDto) {
    app.ui.library.catalog.reindex();
    crate::library_catalog::flush(app);
    app.doc.status = format!("Saved {} {} to the library", dto.name, dto.version);
    app.doc.command_status = true;
    app.doc.diagnostics.record(
        crate::diagnostics::Source::Application,
        crate::diagnostics::State::Succeeded,
        &app.doc.status,
    );
}
