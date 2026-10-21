//! Native file intents stay separate from document mutation and dirty-close decisions.
use onecad_app::dto::SaveOutcomeDto;
use onecad_widgets::components::overlays::{Dialog, DialogIntent};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportFormat {
    Step,
    Stl,
    Obj,
    ThreeMf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    New,
    DiscardAndReplace(Box<Request>),
    Open(Option<PathBuf>),
    Import,
    Save { save_as: bool },
    Export(ExportFormat),
}

impl Request {
    fn diagnostic_action(&self) -> crate::diagnostics::FileAction {
        use crate::diagnostics::FileAction;
        match self {
            Self::New => FileAction::New,
            Self::Open(_) => FileAction::Open,
            Self::Import => FileAction::Import,
            Self::Save { .. } => FileAction::Save,
            Self::Export(_) => FileAction::Export,
            Self::DiscardAndReplace(request) => request.diagnostic_action(),
        }
    }

    pub fn replaces_document(&self) -> bool {
        match self {
            Self::DiscardAndReplace(request) => request.replaces_document(),
            other => matches!(other, Self::New | Self::Open(_) | Self::Import),
        }
    }
}

#[derive(Debug)]
pub enum Outcome {
    Cancelled,
    Replaced { warning: Option<String> },
    Saved(SaveOutcomeDto),
    Exported(PathBuf),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pending {
    Document(Request),
    Quit,
}

#[derive(Default)]
pub struct Flow {
    pub running: bool,
    pub(crate) recovery_sketch_save: bool,
    pub(crate) diagnostic_action: Option<crate::diagnostics::FileAction>,
    pub confirm: Option<Pending>,
    pub after_save: Option<Pending>,
}

impl Flow {
    pub fn blocks_sketch_authoring(
        &self,
        action: &onecad_interaction::sketch::SketchAction,
    ) -> bool {
        use onecad_interaction::sketch::SketchAction;
        (self.running || self.confirm.is_some())
            && matches!(
                action,
                SketchAction::Begin { .. }
                    | SketchAction::SetTool(_)
                    | SketchAction::SetConstructionMode { .. }
                    | SketchAction::Move(_)
                    | SketchAction::Click { .. }
                    | SketchAction::ClickInferred { .. }
                    | SketchAction::CreateNumeric { .. }
                    | SketchAction::CreateNumericAuthored { .. }
                    | SketchAction::PreviewNumeric { .. }
                    | SketchAction::BeginPointNumeric { .. }
                    | SketchAction::Escape
                    | SketchAction::Enter
                    | SketchAction::Finish
                    | SketchAction::Discard
                    | SketchAction::Select { .. }
                    | SketchAction::AddConstraints { .. }
                    | SketchAction::SetDimension { .. }
                    | SketchAction::Repair { .. }
                    | SketchAction::ReenterForRepair { .. }
                    | SketchAction::History(_)
            )
    }

    pub fn blocks_authoring(&self, action: &onecad_interaction::Action) -> bool {
        use onecad_interaction::Action;
        (self.running || self.confirm.is_some())
            && matches!(
                action,
                Action::Confirm
                    | Action::DraftChanged(_)
                    | Action::TransformDraftChanged { .. }
                    | Action::BeginExtrude { .. }
                    | Action::BeginOperation { .. }
                    | Action::BeginEdit { .. }
                    | Action::BeginParameterEdit { .. }
            )
    }

    fn complete(&mut self, result: &Result<Outcome, String>) -> Option<Pending> {
        self.running = false;
        let pending = self.after_save.take();
        if matches!(result, Ok(Outcome::Saved(saved)) if saved.clean) {
            pending
        } else {
            None
        }
    }
}

impl crate::app::App {
    pub fn document_request(&mut self, request: Request) {
        self.sync_document_state();
        let action = request.diagnostic_action();
        if self.busy() || self.stopping {
            self.file_status(
                action,
                "Wait for the current command before using file actions",
            );
            return;
        }
        if tool_blocks_request(self, &request) {
            self.file_status(
                action,
                "Finish or cancel the active tool before switching documents",
            );
            return;
        }
        let recovery_ready = self.bridge.shared.recovery.lock().is_ok_and(|recovery| {
            recovery.opened || (recovery.checked && recovery.offers.is_empty())
        });
        if !recovery_ready {
            self.file_status(action, "Resolve startup recovery before using file actions");
            return;
        }
        if matches!(request, Request::Save { .. } | Request::Export(_)) && !self.document_open() {
            self.file_status(
                action,
                "Create or open a document before saving or exporting",
            );
            return;
        }
        let pending = Pending::Document(request);
        if needs_confirmation(&pending, self.document_dirty()) {
            self.documents.confirm = Some(pending);
        } else {
            self.launch_document(pending);
        }
    }

    fn file_status(&mut self, action: crate::diagnostics::FileAction, message: &str) {
        self.command_status = true;
        self.status = message.into();
        self.diagnostics.record(
            crate::diagnostics::Source::File(action),
            crate::diagnostics::State::Rejected,
            message,
        );
    }

    fn launch_document(&mut self, pending: Pending) {
        let Pending::Document(request) = pending else {
            self.stop();
            return;
        };
        self.documents.recovery_sketch_save =
            matches!(request, Request::Save { .. }) && crate::bars::recovery_sketch_save(self);
        self.documents.running = true;
        self.documents.diagnostic_action = Some(request.diagnostic_action());
        self.command_status = true;
        self.status = "File action pending…".into();
        self.diagnostics.record(
            crate::diagnostics::Source::File(request.diagnostic_action()),
            crate::diagnostics::State::Pending,
            &self.status,
        );
        if let Err(error) = self
            .bridge
            .submit(crate::bridge::Command::Document(request))
        {
            self.documents.running = false;
            self.documents.after_save = None;
            self.status = error;
            if let Some(action) = self.documents.diagnostic_action.take() {
                self.diagnostics.record(
                    crate::diagnostics::Source::File(action),
                    crate::diagnostics::State::Failed,
                    &self.status,
                );
            }
        }
    }

    pub(crate) fn document_finished(&mut self, result: Result<Outcome, String>) {
        let recovery_save = std::mem::take(&mut self.documents.recovery_sketch_save);
        tracing::info!(?result, "native file delivery");
        self.command_status = true;
        let action = self.documents.diagnostic_action.take();
        let diagnostic_state = match &result {
            Err(_) => crate::diagnostics::State::Failed,
            Ok(Outcome::Cancelled) => crate::diagnostics::State::Cancelled,
            Ok(Outcome::Replaced { warning: Some(_) }) => crate::diagnostics::State::Warning,
            Ok(Outcome::Saved(saved)) if !saved.clean => crate::diagnostics::State::Warning,
            _ => crate::diagnostics::State::Succeeded,
        };
        let next = self.documents.complete(&result);
        match result {
            Ok(Outcome::Cancelled) => self.status = "File action cancelled".into(),
            Ok(Outcome::Replaced { warning }) => {
                self.ui.start_open = false;
                self.status = warning.unwrap_or_else(|| "Document opened".into());
                self.draw_dirty = true;
            }
            Ok(Outcome::Saved(saved)) => {
                if saved.clean {
                    self.authoring_dirty = false;
                    self.status = if recovery_save {
                        format!(
                            "Saved accepted sketch recovery for {}. Finish the sketch to regenerate its features.",
                            saved.title
                        )
                    } else {
                        format!("Saved {}", saved.title)
                    };
                } else {
                    self.status = format!("Saved {} — newer changes remain unsaved", saved.title);
                }
            }
            Ok(Outcome::Exported(path)) => self.status = format!("Exported {}", path.display()),
            Err(error) => self.status = error,
        }
        if let Some(action) = action {
            self.diagnostics.record(
                crate::diagnostics::Source::File(action),
                diagnostic_state,
                &self.status,
            );
        }
        if let Some(next) = next {
            self.launch_document(next);
        }
    }
}

pub(crate) fn needs_confirmation(pending: &Pending, dirty: bool) -> bool {
    dirty
        && match pending {
            Pending::Quit => true,
            Pending::Document(request) => request.replaces_document(),
        }
}

pub(crate) fn tool_blocks_request(app: &crate::app::App, request: &Request) -> bool {
    let recovery_save =
        matches!(request, Request::Save { .. }) && crate::bars::recovery_sketch_save(app);
    app.sketch_active() && !recovery_save
        || request.replaces_document() && app.interaction.tool.is_some()
}

/// Runs without presentation so hidden windows still drain work and protect dirty close.
pub fn prepare_close(app: &mut crate::app::App) -> bool {
    app.sync_document_state();
    if !app.close_prompt || app.busy() {
        return false;
    }
    app.close_prompt = false;
    if app.document_dirty() || app.sketch_active() || app.interaction.tool.is_some() {
        app.documents.confirm = Some(Pending::Quit);
        true
    } else {
        app.stop();
        false
    }
}

pub fn close_prompt(app: &mut crate::app::App, ctx: &egui::Context) {
    if app.documents.confirm.is_none() {
        return;
    }
    let out = Dialog::new(DialogIntent::Decide, "Save changes before continuing?")
        .icon("save")
        .body("Save the current document, discard its unsaved changes, or keep editing.")
        .primary("Save", !app.sketch_active())
        .secondary("Cancel", true)
        .tertiary("Discard")
        .show(ctx, egui::Id::unique("document-transition"), |_| {});
    let choice = if out.primary {
        Some(0)
    } else if out.tertiary {
        Some(1)
    } else if out.secondary || out.dismissed {
        Some(2)
    } else {
        None
    };
    apply_close_choice(app, choice);
}

pub(crate) fn apply_close_choice(app: &mut crate::app::App, choice: Option<u8>) {
    match choice {
        Some(0) => {
            app.documents.after_save = app.documents.confirm.take();
            app.launch_document(Pending::Document(Request::Save { save_as: false }));
        }
        Some(1) => {
            if let Some(pending) = app.documents.confirm.take() {
                let pending = match pending {
                    Pending::Document(request) => {
                        Pending::Document(Request::DiscardAndReplace(Box::new(request)))
                    }
                    Pending::Quit => {
                        app.stop_discard();
                        return;
                    }
                };
                app.launch_document(pending);
            }
        }
        Some(2) => app.documents.confirm = None,
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn save_and_save_before_quit_block_authoring_but_allow_completion() {
        use onecad_interaction::Action;
        for after_save in [None, Some(Pending::Quit)] {
            let mut flow = Flow {
                running: true,
                after_save,
                ..Default::default()
            };
            assert!(flow.blocks_authoring(&Action::Confirm));
            assert!(flow.blocks_authoring(&Action::DraftChanged("8".into())));
            assert!(!flow.blocks_authoring(&Action::Cancel));
            assert!(!flow.blocks_authoring(&Action::ContextChanged(None)));
            flow.complete(&saved(true));
            assert!(!flow.blocks_authoring(&Action::Confirm));
        }
        let flow = Flow {
            confirm: Some(Pending::Quit),
            ..Default::default()
        };
        assert!(flow.blocks_authoring(&Action::Confirm));
        assert!(flow.blocks_sketch_authoring(&onecad_interaction::sketch::SketchAction::Enter));
        assert!(flow.blocks_sketch_authoring(&onecad_interaction::sketch::SketchAction::Escape));
        for direction in [
            onecad_core::edit::HistoryDirection::Undo,
            onecad_core::edit::HistoryDirection::Redo,
        ] {
            assert!(flow.blocks_sketch_authoring(
                &onecad_interaction::sketch::SketchAction::History(direction)
            ));
        }
        assert!(!flow.blocks_sketch_authoring(
            &onecad_interaction::sketch::SketchAction::ContextChanged(None)
        ));
    }
    fn saved(clean: bool) -> Result<Outcome, String> {
        Ok(Outcome::Saved(SaveOutcomeDto {
            document_id: "current".into(),
            saved_revision: 4,
            current_revision: if clean { 4 } else { 5 },
            clean,
            path: "/qa/documents/part.onecad".into(),
            title: "part".into(),
        }))
    }

    #[test]
    fn replacement_or_quit_resumes_only_after_a_clean_save() {
        for pending in [Pending::Quit, Pending::Document(Request::New)] {
            let mut flow = Flow {
                running: true,
                after_save: Some(pending.clone()),
                ..Default::default()
            };
            assert_eq!(flow.complete(&saved(true)), Some(pending));
            assert!(!flow.running && flow.after_save.is_none());
        }
        for result in [
            saved(false),
            Ok(Outcome::Cancelled),
            Err("disk full".into()),
        ] {
            let mut flow = Flow {
                running: true,
                after_save: Some(Pending::Quit),
                ..Default::default()
            };
            assert!(flow.complete(&result).is_none());
            assert!(!flow.running && flow.after_save.is_none());
        }
    }
    #[test]
    fn only_dirty_document_replacement_or_exit_needs_confirmation() {
        for request in [Request::New, Request::Open(None), Request::Import] {
            assert!(needs_confirmation(
                &Pending::Document(request.clone()),
                true
            ));
            assert!(!needs_confirmation(&Pending::Document(request), false));
        }
        for request in [
            Request::Save { save_as: false },
            Request::Save { save_as: true },
            Request::Export(ExportFormat::Step),
        ] {
            assert!(!needs_confirmation(&Pending::Document(request), true));
        }
        assert!(needs_confirmation(&Pending::Quit, true));
        assert!(!needs_confirmation(&Pending::Quit, false));
    }
}
