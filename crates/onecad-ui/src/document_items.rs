//! Item deletion confirms an authoritative plan before one fenced history edit.
use crate::{
    app::App,
    bridge::Command,
    diagnostics::{Source, State},
};
use onecad_app::api::{DeleteTarget, DeletionPlan, PreviewContext};
use onecad_widgets::components::overlays::{Dialog, DialogIntent, affected_list, undo_hint};

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

pub(crate) fn selected(app: &App) -> Option<DeleteTarget> {
    use onecad_interaction::selection_set::SelectionItem;
    if let Some(id) = app.doc.selected_sketch() {
        return Some(DeleteTarget::Sketch(id));
    }
    match app.doc.selection.items() {
        [SelectionItem::Body(id)] => Some(DeleteTarget::Body(*id)),
        _ => None,
    }
}

pub(crate) fn request(app: &mut App, target: DeleteTarget) {
    if let Some(reason) = unavailable(app) {
        app.doc.status = reason.into();
        return;
    }
    let Some(display) = &app.doc.display else {
        return;
    };
    let expected = display.context.clone();
    app.doc.items.sequence = app.doc.items.sequence.wrapping_add(1);
    let sequence = app.doc.items.sequence;
    match app.doc.bridge.submit(Command::Items(Request::Plan {
        sequence,
        target,
        expected: expected.clone(),
    })) {
        Ok(()) => {
            app.doc.items.pending = true;
            app.doc.items.expected = Some(expected);
            report(app, State::Pending, "Checking item dependencies…".into());
        }
        Err(error) => report(app, State::Rejected, error),
    }
}

pub(crate) fn receive(app: &mut App, sequence: u64, result: Result<Reply, String>) {
    if sequence != app.doc.items.sequence {
        return;
    }
    if !same_document(app) {
        reset(app);
        return;
    }
    match result {
        Ok(Reply::Planned(plan)) => {
            app.doc.items.pending = false;
            if app
                .doc
                .display
                .as_ref()
                .is_some_and(|display| display.context == plan.expected)
            {
                app.doc.items.confirmation = Some(*plan);
            } else {
                report(
                    app,
                    State::Rejected,
                    "Document changed; select the item again".into(),
                );
            }
        }
        Ok(Reply::Accepted) => {
            app.doc.authoring_dirty = true;
            report(
                app,
                State::Regenerating,
                "Deletion accepted; regenerating…".into(),
            );
        }
        Ok(Reply::Finished(outcome)) => finish(app, *outcome),
        Err(error) => {
            app.doc.items.pending = false;
            app.doc.items.committing = false;
            report(app, State::Failed, format!("Delete failed: {error}"));
        }
    }
}

fn finish(app: &mut App, outcome: crate::outcome::CommandOutcome) {
    app.doc.items.pending = false;
    app.doc.items.committing = false;
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
    app.doc.items.expected.as_ref().is_some_and(|expected| {
        app.doc.document_state.as_ref().is_some_and(|state| {
            state.document_id == expected.document_id
                && state.runtime_session == expected.runtime_session
        })
    })
}

fn report(app: &mut App, state: State, message: String) {
    app.doc.status = message;
    app.doc.command_status = true;
    app.doc
        .diagnostics
        .record(Source::Application, state, &app.doc.status);
}

pub(crate) fn reset(app: &mut App) {
    app.doc.items.sequence = app.doc.items.sequence.wrapping_add(1);
    app.doc.items.pending = false;
    app.doc.items.committing = false;
    app.doc.items.confirmation = None;
    app.doc.items.expected = None;
}

pub(crate) fn show(app: &mut App, ctx: &egui::Context) {
    let Some(plan) = app.doc.items.confirmation.clone() else {
        if app.doc.items.pending && !app.doc.items.committing {
            planning(app, ctx);
        }
        return;
    };
    let current = app
        .doc
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
    let rows: Vec<(&str, &str, &str)> = [
        ("body", "body", &plan.bodies),
        ("sketch", "sketch", &plan.sketches),
        ("extrude", "feature", &plan.records),
    ]
    .into_iter()
    .flat_map(|(icon, kind, items)| {
        items
            .iter()
            .map(move |item| (icon, item.name.as_str(), kind))
    })
    .collect();
    let out = Dialog::new(DialogIntent::Destroy, "Delete selected item?")
        .icon("delete")
        .body("The following items will be removed together.")
        .primary("Delete", true)
        .secondary("Cancel", true)
        .focus_secondary(true)
        .show(ctx, egui::Id::unique("delete-document-item"), |ui| {
            affected_list(ui, &rows);
            undo_hint(ui, "Undo restores this deletion.");
        });
    let (confirm, cancel) = (out.primary, out.secondary);
    if cancel || out.dismissed {
        app.doc.items.confirmation = None;
        report(app, State::Cancelled, "Delete cancelled".into());
    } else if confirm {
        app.doc.items.confirmation = None;
        let request = Request::Delete {
            sequence: app.doc.items.sequence,
            plan: Box::new(plan),
        };
        match app.doc.bridge.submit(Command::Items(request)) {
            Ok(()) => {
                app.doc.items.pending = true;
                app.doc.items.committing = true;
                report(app, State::Pending, "Delete pending…".into());
            }
            Err(error) => report(app, State::Rejected, error),
        }
    }
}

fn planning(app: &mut App, ctx: &egui::Context) {
    let out = Dialog::new(DialogIntent::Input, "Delete selected item?")
        .secondary("Cancel", true)
        .show(ctx, egui::Id::unique("delete-item-planning"), |ui| {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.add_space(8.0);
                ui.label("Checking item dependencies…");
            });
        });
    let cancelled = out.secondary;
    if cancelled || out.dismissed {
        reset(app);
        report(app, State::Cancelled, "Delete cancelled".into());
    }
}
