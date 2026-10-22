//! Present worker profile evidence only while its exact sketch visit and document remain current.
use crate::app::App;
use onecad_core::{
    ids::EntityId,
    regen::{Diagnostic, Severity},
    sketch::Sketch,
};
use onecad_interaction::{
    DocumentContext,
    sketch::{
        SelectionMode, SketchAction, SketchInteraction, SketchPhase, SketchSession,
        SketchSessionKey, SketchTag, SketchTarget,
    },
};
use onecad_scene::ViewSnapshot;

const MAX_ROWS: usize = 64;
const MAX_REFERENCES: usize = 16;

#[derive(Clone)]
struct Row {
    severity: Severity,
    code: String,
    message: String,
    stage: Option<String>,
    reason: Option<String>,
    evidence: String,
    references: Vec<String>,
    targets: Option<Vec<SketchTarget>>,
}

pub(crate) struct Report {
    source: SketchTag,
    document: DocumentContext,
    name: String,
    rows: Vec<Row>,
    total: usize,
    regions: Option<usize>,
    failure: Option<String>,
}

impl Report {
    pub fn key(&self) -> &SketchSessionKey {
        &self.source.key
    }

    pub fn observe(interaction: &SketchInteraction, action: &SketchAction) -> Option<Self> {
        let (tag, diagnostics, regions, failure) = match action {
            SketchAction::Finished { tag, result, .. } => (
                tag,
                result.diagnostics.as_slice(),
                Some(result.regions.len()),
                None,
            ),
            SketchAction::FinishRefused {
                tag,
                message,
                diagnostics,
            } => (tag, diagnostics.as_slice(), None, Some(message)),
            SketchAction::Failed { tag, message } => (tag, &[][..], None, Some(message)),
            _ => return None,
        };
        let session = interaction.session.as_ref().filter(|session| {
            session.request == *tag
                && session.phase == SketchPhase::Finishing
                && interaction.context.as_ref() == Some(&session.document)
                && session.document.same_runtime(&tag.document)
        })?;
        Some(Self {
            source: tag.clone(),
            document: session.document.clone(),
            name: session.authoritative.name.clone(),
            rows: diagnostics
                .iter()
                .take(MAX_ROWS)
                .map(|diagnostic| Row::new(diagnostic, &session.authoritative))
                .collect(),
            total: diagnostics.len(),
            regions,
            failure: failure.map(|message| bounded(message, 4096)),
        })
    }

    pub fn from_display(
        interaction: &SketchInteraction,
        display: &crate::bridge::Display,
    ) -> Option<Self> {
        let session = interaction
            .session
            .as_ref()
            .filter(|session| session.phase == SketchPhase::Active)?;
        let document = crate::sketch_services::document_context(display);
        if session.document != document || interaction.context.as_ref() != Some(&document) {
            return None;
        }
        let failure = display
            .profile_failures
            .iter()
            .find(|failure| failure.sketch == session.key.sketch_id)?;
        Some(Self {
            source: session.request.clone(),
            document,
            name: session.authoritative.name.clone(),
            rows: failure
                .diagnostics
                .iter()
                .take(MAX_ROWS)
                .map(|diagnostic| Row::new(diagnostic, &session.authoritative))
                .collect(),
            total: failure.diagnostics.len(),
            regions: None,
            failure: Some(bounded(&failure.message, 4096)),
        })
    }

    pub fn is_current(&self, interaction: &SketchInteraction) -> bool {
        self.current(interaction).is_some()
    }

    #[cfg(test)]
    pub fn has_reason(&self, reason: &str) -> bool {
        self.rows
            .iter()
            .any(|row| row.reason.as_deref() == Some(reason))
    }

    fn current<'a>(&self, interaction: &'a SketchInteraction) -> Option<&'a SketchSession> {
        interaction.session.as_ref().filter(|session| {
            session.key == self.source.key
                && session.request == self.source
                && session.document == self.document
                && interaction.context.as_ref() == Some(&self.document)
        })
    }
}

impl Row {
    fn new(diagnostic: &Diagnostic, sketch: &Sketch) -> Self {
        let (references, complete) = entity_references(diagnostic);
        let targets = references
            .iter()
            .map(|wire_id| {
                let id: EntityId = wire_id.parse().ok()?;
                sketch
                    .contains_entity(id)
                    .then_some(SketchTarget::Entity(id))
            })
            .collect::<Option<Vec<_>>>()
            .filter(|targets| complete && !targets.is_empty());
        Self {
            severity: diagnostic.severity,
            code: bounded(&diagnostic.code, 128),
            message: bounded(&diagnostic.message, 4096),
            stage: diagnostic.stage.as_ref().map(|stage| bounded(stage, 64)),
            reason: diagnostic
                .reason_code
                .as_ref()
                .map(|reason| bounded(reason, 64)),
            evidence: diagnostic
                .evidence
                .as_ref()
                .map_or_else(String::new, |value| bounded(&value.to_string(), 1024)),
            references,
            targets,
        }
    }
}

fn entity_references(diagnostic: &Diagnostic) -> (Vec<String>, bool) {
    let Some(evidence) = diagnostic
        .evidence
        .as_ref()
        .and_then(|value| value.as_object())
    else {
        return (vec![], true);
    };
    let single = evidence.get("entityId");
    let many = evidence.get("entityIds");
    let mut complete = single.is_none_or(|value| value.is_string())
        && many.is_none_or(|value| {
            value.as_array().is_some_and(|values| {
                values.len() <= MAX_REFERENCES && values.iter().all(|value| value.is_string())
            })
        });
    let values = single.and_then(|value| value.as_str()).into_iter().chain(
        many.and_then(|value| value.as_array())
            .into_iter()
            .flatten()
            .filter_map(|value| value.as_str()),
    );
    let mut references = Vec::new();
    for value in values.take(MAX_REFERENCES + 1) {
        complete &= value.len() <= 128;
        let value = bounded(value, 128);
        if !references.contains(&value) {
            references.push(value);
        }
    }
    complete &= references.len() <= MAX_REFERENCES;
    (references, complete)
}

fn bounded(value: &str, maximum: usize) -> String {
    if value.len() <= maximum {
        return value.into();
    }
    let mut end = maximum;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &value[..end])
}

pub(crate) fn record_display_failures(
    diagnostics: &mut crate::diagnostics::Diagnostics,
    display: &crate::bridge::Display,
) {
    if display.profile_failures.is_empty() {
        return;
    }
    let mut messages = String::new();
    for failure in &display.profile_failures {
        let name = display
            .projection
            .sketches
            .values()
            .find(|sketch| sketch.id == failure.sketch.to_string())
            .map_or("Sketch", |sketch| sketch.name.as_str());
        if !messages.is_empty() {
            messages.push('\n');
        }
        messages.push_str(&format!(
            "{}: {}",
            bounded(name, 128),
            bounded(&failure.message, 512)
        ));
        if messages.len() > 4096 {
            messages = bounded(&messages, 4096);
            break;
        }
    }
    diagnostics.record(
        crate::diagnostics::Source::Sketch,
        crate::diagnostics::State::Warning,
        messages,
    );
}

enum Choice {
    Reenter,
    Repair,
    Select(Box<SketchAction>),
}

pub(crate) fn show(app: &mut App, ui: &mut egui::Ui) {
    let Some(report) = app.doc.diagnostics.profile.as_ref() else {
        return;
    };
    let Some(session) = report.current(&app.doc.sketch) else {
        return;
    };
    let available = !app.busy() && app.doc.bridge.valid() && app.viewport_ready();
    let choice = contents(ui, report, session, app.doc.view, available);
    let sketch = report.source.key.sketch_id;
    match choice {
        Some(Choice::Reenter) => app.edit_sketch(sketch),
        Some(Choice::Repair) => {
            if let Some(session) = app.doc.sketch.session.as_ref() {
                app.sketch_dispatch(SketchAction::ReenterForRepair {
                    key: session.key.clone(),
                    document: session.document.clone(),
                });
            }
        }
        Some(Choice::Select(action)) => app.sketch_dispatch(*action),
        None => {}
    }
}

fn contents(
    ui: &mut egui::Ui,
    report: &Report,
    session: &SketchSession,
    view: Option<ViewSnapshot>,
    available: bool,
) -> Option<Choice> {
    ui.separator();
    ui.add(onecad_widgets::components::lists::SectionLabel::new(
        "PROFILE RESULTS",
    ));
    ui.label(&report.name);
    ui.collapsing("Technical details", |ui| {
        ui.small(format!(
            "Source revision {} · snapshot {} · visit {}",
            report.document.revision, report.document.snapshot_id.0, report.source.key.generation
        ));
    });
    if let Some(message) = &report.failure {
        ui.colored_label(ui.visuals().error_fg_color, message);
    }
    if let Some(regions) = report.regions {
        ui.label(format!("{regions} closed region(s) returned"));
    }
    let choice = rows_ui(ui, report, session, view, available);
    repair_actions(ui, session, available).or(choice)
}

fn rows_ui(
    ui: &mut egui::Ui,
    report: &Report,
    session: &SketchSession,
    view: Option<ViewSnapshot>,
    available: bool,
) -> Option<Choice> {
    let mut choice = None;
    egui::ScrollArea::vertical()
        .id_salt("profile-diagnostics")
        .max_height(200.0)
        .show(ui, |ui| {
            if report.rows.is_empty() {
                ui.label("Worker returned no structured profile diagnostics.");
            }
            for (index, row) in report.rows.iter().enumerate() {
                let action = ui
                    .push_id(index, |ui| {
                        row_ui(ui, row, report, session, view, available)
                    })
                    .inner;
                if let Some(action) = action {
                    choice = Some(Choice::Select(Box::new(action)));
                }
            }
            if report.total > report.rows.len() {
                ui.label(format!(
                    "Showing {} of {} diagnostics",
                    report.rows.len(),
                    report.total
                ));
            }
        });
    choice
}

fn repair_actions(ui: &mut egui::Ui, session: &SketchSession, available: bool) -> Option<Choice> {
    let mut choice = None;
    if session.phase == SketchPhase::Closed
        && ui
            .add_enabled(available, egui::Button::new("Re-enter sketch"))
            .clicked()
    {
        choice = Some(Choice::Reenter);
    }
    if session.phase == SketchPhase::Faulted && session.repair_reentry {
        ui.label("Earlier accepted edits are retained. Re-enter to start a fresh repair visit.");
        if ui
            .add_enabled(available, egui::Button::new("Re-enter sketch to repair"))
            .clicked()
        {
            choice = Some(Choice::Repair);
        }
    } else if session.phase == SketchPhase::Faulted {
        ui.label("Finish did not complete. No discard or fresh edit session is confirmed.");
    }
    choice
}

fn row_ui(
    ui: &mut egui::Ui,
    row: &Row,
    report: &Report,
    session: &SketchSession,
    view: Option<ViewSnapshot>,
    available: bool,
) -> Option<SketchAction> {
    let color = match row.severity {
        Severity::Error => ui.visuals().error_fg_color,
        Severity::Warning => ui.visuals().warn_fg_color,
        Severity::Info => ui.visuals().text_color(),
    };
    ui.label(egui::RichText::new(&row.code).strong().color(color));
    ui.add(egui::Label::new(&row.message).wrap().selectable(true));
    ui.collapsing("Technical details", |ui| {
        if let Some(stage) = &row.stage {
            ui.small(format!("Stage: {stage}"));
        }
        if let Some(reason) = &row.reason {
            ui.small(format!("Reason: {reason}"));
        }
        if !row.evidence.is_empty() {
            ui.add(egui::Label::new(&row.evidence).wrap().selectable(true));
        }
    });
    let action = selection(report, session, view, row);
    let clicked = !row.references.is_empty() && ui.add_enabled(available && action.is_some(), egui::Button::new("Select reported entities"))
        .on_disabled_hover_text("Selection requires current native capture and exact persistent entity IDs. Closed visits must be re-entered first.").clicked();
    ui.separator();
    if clicked { action } else { None }
}

fn selection(
    report: &Report,
    session: &SketchSession,
    view: Option<ViewSnapshot>,
    row: &Row,
) -> Option<SketchAction> {
    let view = view.filter(|view| {
        session.phase == SketchPhase::Active
            && session.document == report.document
            && session.key == report.source.key
            && session.accepted_capture == Some(view.identity())
            && view.identity().scene == session.document.scene
    })?;
    let targets = row.targets.as_ref().filter(|targets| {
        targets.len() <= MAX_REFERENCES
            && targets
                .iter()
                .all(|target| target.is_valid(&session.authoritative))
    })?;
    Some(SketchAction::Select {
        key: session.key.clone(),
        document: session.document.clone(),
        capture: view.identity(),
        targets: targets.clone(),
        mode: SelectionMode::Replace,
    })
}

#[cfg(test)]
mod tests;
