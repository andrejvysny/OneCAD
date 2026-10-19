//! Bounded presentation of application outcomes, independent of simulated extension notices.
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileAction {
    New,
    Open,
    Import,
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

impl Source {
    fn label(self) -> &'static str {
        match self {
            Self::File(FileAction::New) => "New document",
            Self::File(FileAction::Open) => "Open",
            Self::File(FileAction::Import) => "Import",
            Self::File(FileAction::Save) => "Save",
            Self::File(FileAction::Export) => "Export",
            Self::Worker => "Worker",
            Self::Sketch => "Sketch",
            Self::Model => "Model",
            Self::Viewport => "Viewport",
            Self::Application => "Application",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Pending,
    Succeeded,
    Cancelled,
    Rejected,
    Failed,
    Warning,
    Information,
}

impl State {
    fn label(self) -> &'static str {
        match self {
            Self::Pending => "Pending",
            Self::Succeeded => "Complete",
            Self::Cancelled => "Cancelled",
            Self::Rejected => "Rejected",
            Self::Failed => "Failed",
            Self::Warning => "Attention",
            Self::Information => "Information",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    source: Source,
    state: State,
    message: String,
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

    pub fn model(&mut self, status: &onecad_interaction::CommitStatus) {
        use onecad_interaction::{CommitStatus, FeatureOutcome};
        let (state, message) = match status {
            CommitStatus::Idle => return,
            CommitStatus::Pending(_) => (State::Pending, "Submitting feature…".into()),
            CommitStatus::Rejected(message) => (State::Rejected, message.clone()),
            CommitStatus::Accepted { .. } => (State::Pending, "Regenerating feature…".into()),
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
        };
        if self.entries.iter().any(|previous| previous == &entry) {
            return;
        }
        // Each bounded source retains its outcome until that same source makes another attempt.
        self.entries.retain(|previous| previous.source != source);
        self.entries.push_front(entry);
    }

    fn observe(&mut self, status: &str) {
        if self.observed_status == status || status.is_empty() {
            return;
        }
        self.observed_status = status.into();
        if !self.entries.iter().any(|entry| entry.message == status) {
            self.record(Source::Application, State::Information, status);
        }
    }
}

pub fn show(app: &mut crate::app::App, ctx: &egui::Context, chrome: &crate::layout::Chrome) {
    app.diagnostics.observe(&app.status);
    let left = if app.ui.explorer_open {
        chrome.explorer.right() + 8.0
    } else {
        chrome.viewport.left() + 10.0
    };
    let right = if app.ui.inspector_open {
        chrome.inspector.left() - 8.0
    } else {
        chrome.viewport.right() - 10.0
    };
    let width = (right - left).clamp(220.0, 340.0);
    activity_window(&app.diagnostics, ctx, chrome.viewport, left, width);
}

fn activity_window(
    diagnostics: &Diagnostics,
    ctx: &egui::Context,
    viewport: egui::Rect,
    left: f32,
    width: f32,
) {
    egui::Window::new("Activity")
        .id(egui::Id::unique("application-diagnostics"))
        .default_pos(egui::pos2(left, viewport.top() + 70.0))
        .default_width(width)
        .min_width(180.0)
        .max_width(width)
        .max_height((viewport.height() - 150.0).clamp(80.0, 180.0))
        .constrain_to(viewport.shrink(8.0))
        .resizable(false)
        .vscroll(true)
        .show(ctx, |ui| contents(ui, diagnostics));
}

fn contents(ui: &mut egui::Ui, diagnostics: &Diagnostics) {
    for (index, entry) in diagnostics.entries.iter().enumerate() {
        if index > 0 {
            ui.separator();
        }
        let color = match entry.state {
            State::Failed | State::Rejected => ui.visuals().error_fg_color,
            State::Warning => ui.visuals().warn_fg_color,
            _ => ui.visuals().text_color(),
        };
        ui.label(
            egui::RichText::new(format!(
                "{} · {}",
                entry.source.label(),
                entry.state.label()
            ))
            .strong()
            .color(color),
        );
        ui.add(egui::Label::new(&entry.message).wrap().selectable(true));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn activity_is_readable_in_both_themes_at_minimum_window() {
        for dark in [false, true] {
            let ctx = egui::Context::default();
            onecad_widgets::theme::install(&ctx, onecad_widgets::theme::load(dark).unwrap(), dark);
            let mut diagnostics = Diagnostics::default();
            diagnostics.record(
                Source::File(FileAction::Save),
                State::Failed,
                "Save failed: disk access denied",
            );
            diagnostics.record(
                Source::Sketch,
                State::Rejected,
                "Sketch mutation rejected: constraint conflicts",
            );
            for frame in 0..3 {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(900.0, 560.0),
                        )),
                        ..Default::default()
                    },
                    |_| {
                        activity_window(
                            &diagnostics,
                            &ctx,
                            egui::Rect::from_min_max(
                                egui::pos2(0.0, 44.0),
                                egui::pos2(900.0, 560.0),
                            ),
                            266.0,
                            328.0,
                        );
                    },
                );
                output.textures_delta.clear();
                let text: Vec<_> = output
                    .shapes
                    .iter()
                    .filter_map(|shape| match &shape.shape {
                        egui::epaint::Shape::Text(text) => Some(text.galley.job.text.as_str()),
                        _ => None,
                    })
                    .collect();
                if frame > 0 {
                    assert!(text.contains(&"Save failed: disk access denied"));
                    assert!(text.contains(&"Sketch mutation rejected: constraint conflicts"));
                    for shape in &output.shapes {
                        if let egui::epaint::Shape::Text(text) = &shape.shape {
                            let bounds = egui::Rect::from_min_size(text.pos, text.galley.size());
                            assert!(ctx.content_rect().contains_rect(bounds));
                        }
                    }
                }
            }
        }
    }
}
