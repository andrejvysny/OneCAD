//! Item deletion confirms an authoritative plan before one fenced history edit.
use crate::{
    app::App,
    bridge::Command,
    diagnostics::{Source, State},
};
use onecad_app::api::{DeleteTarget, DeletionPlan, PreviewContext};

mod service;
pub(crate) use service::execute;

#[derive(Debug)]
pub(crate) enum Request {
    Plan {
        sequence: u64,
        target: DeleteTarget,
        expected: PreviewContext,
    },
    Delete {
        sequence: u64,
        plan: Box<DeletionPlan>,
    },
}

#[derive(Debug)]
pub(crate) enum Reply {
    Planned(Box<DeletionPlan>),
    Accepted,
    Finished(Box<crate::outcome::CommandOutcome>),
}

#[derive(Default)]
pub(crate) struct Flow {
    sequence: u64,
    pending: bool,
    committing: bool,
    expected: Option<PreviewContext>,
    pub(crate) confirmation: Option<DeletionPlan>,
}

impl Flow {
    pub(crate) fn blocks(&self) -> bool {
        self.pending || self.confirmation.is_some()
    }
}

pub(crate) fn unavailable(app: &App) -> Option<&'static str> {
    if !app.document_open() {
        Some("Create or open a document first")
    } else if app.busy()
        || app.sketch_active()
        || app.interaction.tool.is_some()
        || app.ui.sketch_entry.open
    {
        Some("Finish the current operation first")
    } else if !app.bridge.valid() {
        Some("Geometry worker unavailable")
    } else {
        None
    }
}

pub(crate) fn selected(app: &App) -> Option<DeleteTarget> {
    if let Some(id) = app.ui.selected_sketch {
        Some(DeleteTarget::Sketch(id))
    } else if app.topology.selected_kind().is_none() {
        app.selected_body.map(DeleteTarget::Body)
    } else {
        None
    }
}

pub(crate) fn request(app: &mut App, target: DeleteTarget) {
    if let Some(reason) = unavailable(app) {
        app.status = reason.into();
        return;
    }
    let Some(display) = &app.display else { return };
    let expected = display.context.clone();
    app.items.sequence = app.items.sequence.wrapping_add(1);
    let sequence = app.items.sequence;
    match app.bridge.submit(Command::Items(Request::Plan {
        sequence,
        target,
        expected: expected.clone(),
    })) {
        Ok(()) => {
            app.items.pending = true;
            app.items.expected = Some(expected);
            report(app, State::Pending, "Checking item dependencies…".into());
        }
        Err(error) => report(app, State::Rejected, error),
    }
}

pub(crate) fn receive(app: &mut App, sequence: u64, result: Result<Reply, String>) {
    if sequence != app.items.sequence {
        return;
    }
    if !same_document(app) {
        reset(app);
        return;
    }
    match result {
        Ok(Reply::Planned(plan)) => {
            app.items.pending = false;
            if app
                .display
                .as_ref()
                .is_some_and(|display| display.context == plan.expected)
            {
                app.items.confirmation = Some(*plan);
            } else {
                report(
                    app,
                    State::Rejected,
                    "Document changed; select the item again".into(),
                );
            }
        }
        Ok(Reply::Accepted) => {
            app.authoring_dirty = true;
            report(
                app,
                State::Regenerating,
                "Deletion accepted; regenerating…".into(),
            );
        }
        Ok(Reply::Finished(outcome)) => finish(app, *outcome),
        Err(error) => {
            app.items.pending = false;
            app.items.committing = false;
            report(app, State::Failed, format!("Delete failed: {error}"));
        }
    }
}

fn finish(app: &mut App, outcome: crate::outcome::CommandOutcome) {
    app.items.pending = false;
    app.items.committing = false;
    let failure = outcome.completion.as_ref().filter(|result| {
        !matches!(result.outcome.as_str(), "published" | "noop")
            || !result.failed_steps.is_empty()
            || !result.repair_steps.is_empty()
    });
    if let Some(result) = failure {
        report(
            app,
            State::Failed,
            format!(
                "Deletion accepted; regeneration {}: {}",
                result.outcome,
                result
                    .message
                    .as_deref()
                    .unwrap_or("Inspect feature diagnostics")
            ),
        );
    } else {
        report(app, State::Succeeded, "Delete complete".into());
    }
}

fn same_document(app: &App) -> bool {
    app.items.expected.as_ref().is_some_and(|expected| {
        app.document_state.as_ref().is_some_and(|state| {
            state.document_id == expected.document_id
                && state.runtime_session == expected.runtime_session
        })
    })
}

fn report(app: &mut App, state: State, message: String) {
    app.status = message;
    app.command_status = true;
    app.diagnostics
        .record(Source::Application, state, &app.status);
}

pub(crate) fn reset(app: &mut App) {
    app.items.sequence = app.items.sequence.wrapping_add(1);
    app.items.pending = false;
    app.items.committing = false;
    app.items.confirmation = None;
    app.items.expected = None;
}

pub(crate) fn show(app: &mut App, ctx: &egui::Context) {
    let Some(plan) = app.items.confirmation.clone() else {
        if app.items.pending && !app.items.committing {
            planning(app, ctx);
        }
        return;
    };
    let current = app
        .display
        .as_ref()
        .is_some_and(|display| display.context == plan.expected);
    if !current {
        reset(app);
        report(
            app,
            State::Cancelled,
            "Delete cancelled: document changed".into(),
        );
        return;
    }
    let mut confirm = false;
    let mut cancel = false;
    let response = egui::Modal::new(egui::Id::unique("delete-document-item")).show(ctx, |ui| {
        ui.set_max_width(420.0);
        ui.heading("Delete selected item?");
        ui.label("The following items will be removed together. Undo restores this deletion.");
        egui::ScrollArea::vertical()
            .max_height(220.0)
            .show(ui, |ui| {
                for (kind, items) in [
                    ("Body", &plan.bodies),
                    ("Sketch", &plan.sketches),
                    ("Feature", &plan.records),
                ] {
                    for item in items {
                        ui.label(format!("{kind}: {}", item.name));
                    }
                }
            });
        ui.horizontal(|ui| {
            cancel = ui.button("Cancel").clicked();
            confirm = ui.button("Delete").clicked();
        });
    });
    if cancel || response.should_close() {
        app.items.confirmation = None;
        report(app, State::Cancelled, "Delete cancelled".into());
    } else if confirm {
        app.items.confirmation = None;
        let request = Request::Delete {
            sequence: app.items.sequence,
            plan: Box::new(plan),
        };
        match app.bridge.submit(Command::Items(request)) {
            Ok(()) => {
                app.items.pending = true;
                app.items.committing = true;
                report(app, State::Pending, "Delete pending…".into());
            }
            Err(error) => report(app, State::Rejected, error),
        }
    }
}

fn planning(app: &mut App, ctx: &egui::Context) {
    let mut cancelled = false;
    let response = egui::Modal::new(egui::Id::unique("delete-item-planning")).show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label("Checking item dependencies…");
        });
        cancelled = ui.button("Cancel").clicked();
    });
    if cancelled || response.should_close() {
        reset(app);
        report(app, State::Cancelled, "Delete cancelled".into());
    }
}
