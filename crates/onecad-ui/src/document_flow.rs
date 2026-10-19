//! Native file intents stay separate from document mutation and dirty-close decisions.
use onecad_app::dto::SaveOutcomeDto;
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
                    | SketchAction::Move(_)
                    | SketchAction::Click { .. }
                    | SketchAction::Escape
                    | SketchAction::Enter
                    | SketchAction::Finish
                    | SketchAction::Discard
                    | SketchAction::Select { .. }
                    | SketchAction::AddConstraints { .. }
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
        if self.busy() || self.stopping {
            self.file_status("Wait for the current command before using file actions");
            return;
        }
        if self.sketch_active() || (request.replaces_document() && self.interaction.tool.is_some())
        {
            self.file_status("Finish or cancel the active tool before switching documents");
            return;
        }
        let recovery_ready = self.bridge.shared.recovery.lock().is_ok_and(|recovery| {
            recovery.opened || (recovery.checked && recovery.offers.is_empty())
        });
        if !recovery_ready {
            self.file_status("Resolve startup recovery before using file actions");
            return;
        }
        let pending = Pending::Document(request);
        if needs_confirmation(&pending, self.document_dirty()) {
            self.documents.confirm = Some(pending);
        } else {
            self.launch_document(pending);
        }
    }

    fn file_status(&mut self, message: &str) {
        self.command_status = true;
        self.status = message.into();
    }

    fn launch_document(&mut self, pending: Pending) {
        let Pending::Document(request) = pending else {
            self.stop();
            return;
        };
        self.documents.running = true;
        self.command_status = true;
        self.status = "File action pending…".into();
        if let Err(error) = self
            .bridge
            .submit(crate::bridge::Command::Document(request))
        {
            self.documents.running = false;
            self.documents.after_save = None;
            self.status = error;
        }
    }

    pub(crate) fn document_finished(&mut self, result: Result<Outcome, String>) {
        tracing::info!(?result, "native file delivery");
        self.command_status = true;
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
                    self.status = format!("Saved {}", saved.title);
                    if let Some(next) = next {
                        self.launch_document(next);
                    }
                } else {
                    self.status = format!("Saved {} — newer changes remain unsaved", saved.title);
                }
            }
            Ok(Outcome::Exported(path)) => self.status = format!("Exported {}", path.display()),
            Err(error) => self.status = error,
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
    let mut choice = None;
    let response = egui::Modal::new(egui::Id::unique("document-transition")).show(ctx, |ui| {
        ui.heading("Save changes before continuing?");
        ui.label("Save the current document, discard its unsaved changes, or keep editing.");
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!app.sketch_active(), egui::Button::new("Save"))
                .clicked()
            {
                choice = Some(0);
            }
            if ui.button("Discard").clicked() {
                choice = Some(1);
            }
            if ui.button("Cancel").clicked() {
                choice = Some(2);
            }
        });
    });
    if choice.is_none() && response.should_close() {
        choice = Some(2);
    }
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
