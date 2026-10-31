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
    Autosave,
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

/// The newest failed outcome, kept for the failure-only toast.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Failure {
    /// Process-unique, so a toast never re-appears for an outcome it already showed.
    pub id: u64,
    pub source: Source,
    pub message: String,
    pub details: String,
}

static NEXT_FAILURE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

#[derive(Default)]
pub struct Diagnostics {
    failure: Option<Failure>,
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
        if state == State::Failed {
            self.failure = Some(failure(&entry));
        }
        self.entries.push_front(entry);
    }

    pub(crate) fn latest_failure(&self) -> Option<&Failure> {
        self.failure.as_ref()
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

impl Source {
    pub(crate) fn label(self) -> &'static str {
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
            Self::Autosave => "Recovery checkpoint",
        }
    }
}

fn failure(entry: &Entry) -> Failure {
    let mut details = format!("{}: {}", entry.source.label(), entry.message);
    if let Some(session) = &entry.runtime_session {
        details.push_str(&format!("\nDocument session: {session}"));
    }
    if let Some(visit) = &entry.sketch_visit {
        details.push_str(&format!("\nSketch visit: {visit:?}"));
    }
    Failure {
        id: NEXT_FAILURE.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        source: entry.source,
        message: entry.message.clone(),
        details,
    }
}

pub(crate) fn summary(message: &str) -> String {
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

    #[test]
    fn only_new_failed_outcomes_become_the_latest_failure() {
        let mut diagnostics = Diagnostics::default();
        diagnostics.record(Source::File(FileAction::Save), State::Succeeded, "saved");
        diagnostics.record(Source::Sketch, State::Rejected, "conflict");
        diagnostics.record(Source::Model, State::Pending, "Submitting feature…");
        assert!(diagnostics.latest_failure().is_none());
        diagnostics.record(Source::File(FileAction::Save), State::Failed, "disk full");
        let first = diagnostics.latest_failure().unwrap().clone();
        assert_eq!(first.source, Source::File(FileAction::Save));
        assert!(first.details.contains("disk full"));
        diagnostics.record(Source::File(FileAction::Save), State::Failed, "disk full");
        assert_eq!(diagnostics.latest_failure(), Some(&first));
        diagnostics.record(Source::Worker, State::Failed, "worker gone");
        assert_ne!(diagnostics.latest_failure().unwrap().id, first.id);
        assert_eq!(diagnostics.latest_failure().unwrap().message, "worker gone");
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
