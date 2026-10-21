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
}
