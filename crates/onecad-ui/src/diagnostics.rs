//! Bounded presentation of application outcomes, independent of simulated extension notices.
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileAction {
    New,
    Open,
    Import,
    InsertStep,
    Save,
    Export,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    File(FileAction),
    Worker,
    Sketch,
    Model,
    Viewport,
    Application,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Pending,
    Regenerating,
    Succeeded,
    Cancelled,
    Rejected,
    Failed,
    Warning,
    Information,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    source: Source,
    state: State,
    message: String,
    runtime_session: Option<String>,
    sketch_visit: Option<onecad_interaction::sketch::SketchSessionKey>,
}

#[derive(Default)]
pub struct Diagnostics {
    pub(crate) profile: Option<crate::profile_diagnostics::Report>,
    entries: VecDeque<Entry>,
    observed_status: String,
    runtime_session: Option<String>,
    sketch_visit: Option<onecad_interaction::sketch::SketchSessionKey>,
}

impl Diagnostics {
    pub fn runtime(&mut self, runtime_session: &str, previous_status: &str) {
        if self.runtime_session.as_deref() == Some(runtime_session) {
            return;
        }
        self.entries
            .retain(|entry| matches!(entry.source, Source::Worker | Source::Viewport));
        self.observed_status = previous_status.into();
        self.runtime_session = Some(runtime_session.into());
        self.profile = None;
        self.sketch_visit = None;
    }

    pub fn sketch_visit(&mut self, key: &onecad_interaction::sketch::SketchSessionKey) {
        if self.sketch_visit.as_ref() != Some(key) {
            self.profile = None;
            self.entries.retain(|entry| entry.source != Source::Sketch);
            self.sketch_visit = Some(key.clone());
        }
    }

    #[cfg(test)]
    pub fn contains(&self, source: Source, state: State, message: &str) -> bool {
        self.entries.iter().any(|entry| {
            entry.source == source && entry.state == state && entry.message.contains(message)
        })
    }

    /// The outcome `source` last recorded (acceptance probes only).
    #[cfg(feature = "semantic-probe")]
    pub(crate) fn latest(&self, source: Source) -> Option<(State, String)> {
        self.entries
            .iter()
            .find(|entry| entry.source == source)
            .map(|entry| (entry.state, entry.message.clone()))
    }

    pub fn model(&mut self, status: &onecad_interaction::CommitStatus) {
        use onecad_interaction::{CommitStatus, FeatureOutcome};
        let (state, message) = match status {
            CommitStatus::Idle => return,
            CommitStatus::Pending(_) => (State::Pending, "Submitting feature…".into()),
            CommitStatus::Rejected(message) => (State::Rejected, message.clone()),
            CommitStatus::Accepted { .. } => (State::Regenerating, "Regenerating feature…".into()),
            CommitStatus::CompletionUnavailable { message, .. } => (State::Failed, message.clone()),
            CommitStatus::Completed { outcome, .. } => match outcome {
                FeatureOutcome::Published => (State::Succeeded, "Feature complete".into()),
                FeatureOutcome::Noop => (State::Succeeded, "Feature unchanged".into()),
                FeatureOutcome::NeedsRepair => (State::Warning, "Feature needs repair".into()),
                FeatureOutcome::Failed(message) => (State::Failed, message.clone()),
                FeatureOutcome::Cancelled => {
                    (State::Cancelled, "Feature regeneration cancelled".into())
                }
            },
        };
        self.record(Source::Model, state, message);
    }

    pub fn record(&mut self, source: Source, state: State, message: impl Into<String>) {
        let entry = Entry {
            source,
            state,
            message: message.into(),
            runtime_session: self.runtime_session.clone(),
            sketch_visit: (source == Source::Sketch)
                .then(|| self.sketch_visit.clone())
                .flatten(),
        };
        if self.entries.iter().any(|previous| previous == &entry) {
            return;
        }
        // Each bounded source retains its outcome until that same source makes another attempt.
        self.entries.retain(|previous| previous.source != source);
        self.entries.push_front(entry);
    }

    /// Presentation only: closing details never clears operation evidence.
    pub(crate) fn show(&self, ctx: &egui::Context) {
        let bounds = ctx.content_rect();
        egui::Area::new(egui::Id::unique("operation-notifications"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-12.0, -12.0))
            .show(ctx, |ui| {
                ui.set_min_width(160.0);
                let label = self.notice().map_or_else(
                    || "Activity".into(),
                    |entry| format!("Activity · {}", entry.state.label()),
                );
                let button = ui
                    .add(egui::Button::new(label).wrap_mode(egui::TextWrapMode::Extend))
                    .on_hover_text("Operation outcomes and details");
                egui::Popup::menu(&button)
                    .width(360.0_f32.min(bounds.width() - 32.0))
                    .show(|ui| self.details(ui, bounds.height() * 0.6));
            });
    }

    fn notice(&self) -> Option<&Entry> {
        if let Some(entry) = self.entries.front()
            && matches!(entry.state, State::Pending | State::Regenerating)
        {
            return Some(entry);
        }
        self.entries
            .iter()
            .find(|entry| {
                matches!(
                    entry.state,
                    State::Failed | State::Rejected | State::Warning
                )
            })
            .or_else(|| self.entries.front())
    }

    fn details(&self, ui: &mut egui::Ui, height: f32) {
        ui.heading("Activity");
        if self.entries.is_empty() {
            ui.label("No operation outcomes yet.");
        }
        egui::ScrollArea::vertical()
            .max_height(height)
            .show(ui, |ui| {
                for entry in &self.entries {
                    ui.separator();
                    ui.label(format!(
                        "{} · {}",
                        entry.source.label(),
                        entry.state.label()
                    ));
                    ui.label(summary(&entry.message));
                    egui::CollapsingHeader::new("Details")
                        .id_salt(format!("{:?}", entry.source))
                        .show(ui, |ui| {
                            if let Some(session) = &entry.runtime_session {
                                ui.label(format!("Document session: {session}"));
                            }
                            if entry.source == Source::Sketch
                                && let Some(visit) = &entry.sketch_visit
                            {
                                ui.label(format!("Sketch visit: {visit:?}"));
                            }
                            ui.add(egui::Label::new(&entry.message).selectable(true).wrap());
                            if ui.small_button("Copy details").clicked() {
                                ui.ctx().copy_text(entry.message.clone());
                            }
                        });
                }
            });
    }

    pub(crate) fn observe(&mut self, status: &str) {
        if self.observed_status == status || status.is_empty() {
            return;
        }
        self.observed_status = status.into();
        if !self.entries.iter().any(|entry| entry.message == status) {
            self.record(Source::Application, State::Information, status);
        }
    }
}

impl State {
    fn label(self) -> &'static str {
        match self {
            Self::Pending => "Pending",
            Self::Regenerating => "Accepted / regenerating",
            Self::Succeeded => "Completed",
            Self::Cancelled => "Cancelled",
            Self::Rejected => "Refused",
            Self::Failed => "Failed",
            Self::Warning => "Needs attention",
            Self::Information => "Information",
        }
    }
}

impl Source {
    fn label(self) -> &'static str {
        match self {
            Self::File(FileAction::New) => "New document",
            Self::File(FileAction::Open) => "Open",
            Self::File(FileAction::Import) => "Import",
            Self::File(FileAction::InsertStep) => "Insert STEP",
            Self::File(FileAction::Save) => "Save",
            Self::File(FileAction::Export) => "Export",
            Self::Worker => "Modeling worker",
            Self::Sketch => "Sketch",
            Self::Model => "Feature",
            Self::Viewport => "Viewport",
            Self::Application => "Application",
        }
    }
}

fn summary(message: &str) -> String {
    if message.trim_start().starts_with(['{', '[']) {
        return "Operation could not complete. Inspect details, then retry.".into();
    }
    let first = message.lines().next().unwrap_or(message);
    let first = first.split(['{', '[']).next().unwrap_or(first).trim_end();
    let mut summary: String = first.chars().take(180).collect();
    if first.chars().count() > 180 {
        summary.push('…');
    }
    summary
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(output: &egui::FullOutput) -> Vec<String> {
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn typed_file_and_model_outcomes_are_rendered_with_readable_details() {
        for dark in [false, true] {
            let ctx = egui::Context::default();
            ctx.set_visuals(if dark {
                egui::Visuals::dark()
            } else {
                egui::Visuals::light()
            });
            for state in [
                State::Pending,
                State::Regenerating,
                State::Succeeded,
                State::Cancelled,
                State::Rejected,
                State::Failed,
                State::Warning,
            ] {
                let mut diagnostics = Diagnostics::default();
                for action in [
                    FileAction::Save,
                    FileAction::Open,
                    FileAction::Import,
                    FileAction::Export,
                ] {
                    diagnostics.record(
                        Source::File(action),
                        state,
                        "Disk access denied; choose a writable location.",
                    );
                }
                let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                    diagnostics.details(ui, 500.0)
                });
                output.textures_delta.clear();
                let text = labels(&output);
                for action in ["Save", "Open", "Import", "Export"] {
                    assert!(
                        text.contains(&format!("{action} · {}", state.label())),
                        "{text:?}"
                    );
                }
                assert!(
                    text.iter()
                        .any(|text| text.contains("choose a writable location"))
                );
            }
        }
    }

    #[test]
    fn compact_affordance_opens_details_without_a_viewport_or_document() {
        let ctx = egui::Context::default();
        let mut diagnostics = Diagnostics::default();
        diagnostics.record(
            Source::Viewport,
            State::Failed,
            "GPU unavailable. Save remains available.",
        );
        let frame = |events| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(900.0, 560.0),
                    )),
                    events,
                    ..Default::default()
                },
                |_| diagnostics.show(&ctx),
            );
            output.textures_delta.clear();
            output
        };
        frame(vec![]);
        let output = frame(vec![]);
        let position = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == "Activity · Failed" => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                _ => None,
            })
            .expect("compact notification button must render");
        for pressed in [true, false] {
            frame(vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
        }
        let output = frame(vec![]);
        assert!(egui::Popup::is_any_open(&ctx));
        assert!(
            labels(&output)
                .iter()
                .any(|text| text.contains("Save remains available"))
        );
        assert!(diagnostics.contains(Source::Viewport, State::Failed, "GPU unavailable"));
    }

    #[test]
    fn same_source_retry_supersedes_failure_without_inventing_completion() {
        let mut diagnostics = Diagnostics::default();
        diagnostics.record(Source::File(FileAction::Save), State::Failed, "disk full");
        diagnostics.record(Source::File(FileAction::Save), State::Pending, "Saving…");
        assert!(!diagnostics.contains(Source::File(FileAction::Save), State::Failed, "disk full"));
        assert!(
            !diagnostics
                .entries
                .iter()
                .any(|entry| entry.state == State::Succeeded)
        );
        assert_eq!(
            summary("{\"kernel\":\"failure\"}"),
            "Operation could not complete. Inspect details, then retry."
        );
    }

    #[test]
    fn runtime_replacement_clears_document_outcomes_without_echoing_old_status() {
        let mut diagnostics = Diagnostics::default();
        diagnostics.runtime("first", "Opening…");
        diagnostics.record(Source::File(FileAction::Save), State::Failed, "disk full");
        diagnostics.record(Source::Sketch, State::Rejected, "conflicting constraint");
        diagnostics.record(Source::Model, State::Failed, "feature failed");
        diagnostics.record(Source::Worker, State::Failed, "worker unavailable");
        diagnostics.runtime("second", "feature failed");
        diagnostics.observe("feature failed");
        assert_eq!(diagnostics.entries.len(), 1);
        assert!(diagnostics.contains(Source::Worker, State::Failed, "worker unavailable"));
    }

    #[test]
    fn unrelated_outcomes_do_not_erase_failed_file_actions() {
        let mut diagnostics = Diagnostics::default();
        for action in [FileAction::Save, FileAction::Open, FileAction::Export] {
            diagnostics.record(Source::File(action), State::Failed, "disk access denied");
        }
        diagnostics.record(Source::Sketch, State::Rejected, "constraint conflicts");
        diagnostics.record(Source::Worker, State::Failed, "worker disconnected");
        diagnostics.observe("Profile selected");
        assert_eq!(diagnostics.entries.len(), 6);
        diagnostics.record(Source::File(FileAction::Save), State::Pending, "Saving…");
        assert_eq!(diagnostics.entries.len(), 6);
        assert_eq!(
            diagnostics
                .entries
                .iter()
                .filter(|entry| entry.state == State::Failed)
                .count(),
            3
        );
        assert_eq!(diagnostics.entries.front().unwrap().state, State::Pending);
    }
}
