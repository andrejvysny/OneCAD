//! Native file intents stay separate from document mutation and dirty-close decisions.
use onecad_app::dto::SaveOutcomeDto;
use onecad_widgets::components::overlays::{Dialog, DialogIntent};
use std::path::PathBuf;

// Mesh exports are offered only by the desktop menu bar so far.
#[cfg_attr(not(feature = "desktop"), allow(dead_code))]
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
    /// `Some` imports that file without a dialog (a system-provided STEP file).
    Import(Option<PathBuf>),
    /// Adds a STEP file to the active document's history; never opens a tab.
    InsertStep,
    Save {
        save_as: bool,
    },
    Export(ExportFormat),
    Extension(crate::native_extensions::Request),
}

impl Request {
    fn diagnostic_action(&self) -> crate::diagnostics::FileAction {
        use crate::diagnostics::FileAction;
        match self {
            Self::New => FileAction::New,
            Self::Open(_) => FileAction::Open,
            Self::Import(_) => FileAction::Import,
            Self::InsertStep => FileAction::InsertStep,
            Self::Save { .. } => FileAction::Save,
            Self::Export(_) => FileAction::Export,
            Self::Extension(_) => FileAction::Export,
            Self::DiscardAndReplace(request) => request.diagnostic_action(),
        }
    }

    pub fn replaces_document(&self) -> bool {
        match self {
            Self::DiscardAndReplace(request) => request.replaces_document(),
            other => matches!(other, Self::New | Self::Open(_) | Self::Import(_)),
        }
    }
}

#[derive(Debug)]
pub enum Outcome {
    Cancelled,
    Replaced {
        warning: Option<String>,
    },
    /// A STEP file joined the active document's history.
    Inserted {
        warning: Option<String>,
    },
    Saved(SaveOutcomeDto),
    Exported(PathBuf),
    Extension(crate::native_extensions::Reply),
    /// The file (or a copy with its identity) is open in another document host.
    AlreadyOpen {
        host: onecad_app::process::HostId,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pending {
    Document(Request),
    /// Close the active document's tab.
    CloseTab,
    Quit,
}

#[derive(Default)]
pub struct Flow {
    pub running: bool,
    extension_request: bool,
    pub(crate) recovery_sketch_save: bool,
    pub(crate) diagnostic_action: Option<crate::diagnostics::FileAction>,
    pub confirm: Option<Pending>,
    pub after_save: Option<Pending>,
}

impl Flow {
    fn report_extension_result(
        &mut self,
        native: &mut crate::native_extensions::State,
        result: &Result<Outcome, String>,
    ) {
        if std::mem::take(&mut self.extension_request)
            && let Err(error) = result
        {
            native.failed(error.clone());
        }
    }
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
                    | SketchAction::CommitStroke { .. }
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
                    | Action::ParameterChanged { .. }
                    | Action::ParameterDrag(
                        onecad_interaction::DragPhase::Begin
                            | onecad_interaction::DragPhase::Update { .. }
                    )
                    | Action::Republish
                    | Action::TransformDraftChanged { .. }
                    | Action::BeginExtrude { .. }
                    | Action::BeginOperation { .. }
                    | Action::BeginEdit { .. }
                    | Action::BeginParameterEdit { .. }
                    | Action::BeginOperationEdit { .. }
                    | Action::ModelingOptionsChanged { .. }
            )
    }

    fn complete(&mut self, result: &Result<Outcome, String>) -> Option<Pending> {
        self.running = false;
        self.extension_request = false;
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
            let message = if request == Request::InsertStep {
                "Finish or cancel the active tool before inserting a STEP file"
            } else {
                "Finish or cancel the active tool before switching documents"
            };
            self.file_status(action, message);
            return;
        }
        let recovery_ready = self
            .doc
            .bridge
            .shared
            .recovery
            .lock()
            .is_ok_and(|recovery| recovery.ready());
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
        if request == Request::InsertStep && !self.document_open() {
            self.file_status(action, "Open a document before inserting a STEP file");
            return;
        }
        if request.replaces_document() && self.document_open() {
            // Opening never replaces an open document: it gets a tab of its own.
            let origin = self.doc.tab;
            if let Err(error) = self.open_tab() {
                self.file_status(action, &error);
                return;
            }
            self.doc.origin = Some(origin);
            self.launch_document(Pending::Document(request));
            return;
        }
        let pending = Pending::Document(request);
        if needs_confirmation(&pending, self.document_dirty()) {
            self.doc.documents.confirm = Some(pending);
        } else {
            self.launch_document(pending);
        }
    }

    fn file_status(&mut self, action: crate::diagnostics::FileAction, message: &str) {
        self.doc.command_status = true;
        self.doc.status = message.into();
        self.doc.diagnostics.record(
            crate::diagnostics::Source::File(action),
            crate::diagnostics::State::Rejected,
            message,
        );
    }

    fn launch_document(&mut self, pending: Pending) {
        let request = match pending {
            Pending::Document(request) => request,
            Pending::CloseTab => {
                self.close_tab_now(None);
                return;
            }
            // Re-evaluated: the next document needing a decision is prompted,
            // or every host stops once none does.
            Pending::Quit => {
                self.close_prompt = true;
                return;
            }
        };
        self.doc.documents.recovery_sketch_save =
            matches!(request, Request::Save { .. }) && crate::bars::recovery_sketch_save(self);
        self.doc.documents.running = true;
        self.doc.documents.extension_request = matches!(&request, Request::Extension(_));
        self.doc.documents.diagnostic_action = Some(request.diagnostic_action());
        self.doc.command_status = true;
        self.doc.status = "File action pending…".into();
        self.doc.diagnostics.record(
            crate::diagnostics::Source::File(request.diagnostic_action()),
            crate::diagnostics::State::Pending,
            &self.doc.status,
        );
        if let Err(error) = self
            .doc
            .bridge
            .submit(crate::bridge::Command::Document(request))
        {
            self.doc.documents.running = false;
            self.doc.documents.extension_request = false;
            self.doc.documents.after_save = None;
            self.doc.status = error;
            if let Some(action) = self.doc.documents.diagnostic_action.take() {
                self.doc.diagnostics.record(
                    crate::diagnostics::Source::File(action),
                    crate::diagnostics::State::Failed,
                    &self.doc.status,
                );
            }
        }
    }

    pub(crate) fn document_finished(&mut self, result: Result<Outcome, String>) {
        self.doc
            .documents
            .report_extension_result(&mut self.ui.extensions.native, &result);
        let recovery_save = std::mem::take(&mut self.doc.documents.recovery_sketch_save);
        tracing::info!(?result, "native file delivery");
        self.doc.command_status = true;
        let action = self.doc.documents.diagnostic_action.take();
        let diagnostic_state = match &result {
            Err(_) => crate::diagnostics::State::Failed,
            Ok(Outcome::Cancelled) => crate::diagnostics::State::Cancelled,
            Ok(Outcome::Replaced { warning: Some(_) } | Outcome::Inserted { warning: Some(_) }) => {
                crate::diagnostics::State::Warning
            }
            Ok(Outcome::Saved(saved)) if !saved.clean => crate::diagnostics::State::Warning,
            Ok(Outcome::Extension(reply)) if reply.warning => crate::diagnostics::State::Warning,
            _ => crate::diagnostics::State::Succeeded,
        };
        let next = self.doc.documents.complete(&result);
        let diagnostic = action.map(|action| (action, diagnostic_state));
        // A close-after-save below may show another tab; Save All follows this one.
        let tab = self.doc.tab;
        let save_all = match &result {
            Ok(Outcome::Cancelled) => Some(crate::documents::SaveAllFinished::Cancelled),
            Ok(Outcome::Saved(_)) => Some(crate::documents::SaveAllFinished::Saved),
            Err(_) => Some(crate::documents::SaveAllFinished::Failed),
            _ => None,
        };
        match result {
            Ok(Outcome::Cancelled) => {
                self.doc.status = "File action cancelled".into();
                self.settle_provisional_tab(None, diagnostic);
            }
            Ok(Outcome::AlreadyOpen { host }) => {
                self.doc.status = "That document is already open".into();
                self.settle_provisional_tab(Some(host), diagnostic);
            }
            Ok(Outcome::Replaced { warning }) => {
                if !self.parked {
                    self.ui.start_open = false;
                }
                self.doc.status = warning.unwrap_or_else(|| "Document opened".into());
                self.doc.origin = None;
                self.draw_dirty = true;
            }
            Ok(Outcome::Inserted { warning }) => {
                self.doc.status = warning.unwrap_or_else(|| "STEP file inserted".into());
                self.draw_dirty = true;
            }
            Ok(Outcome::Saved(saved)) => {
                if saved.clean {
                    self.doc.authoring_dirty = false;
                    self.doc.status = if recovery_save {
                        format!(
                            "Saved accepted sketch recovery for {}. Finish the sketch to regenerate its features.",
                            saved.title
                        )
                    } else {
                        format!("Saved {}", saved.title)
                    };
                } else {
                    self.doc.status =
                        format!("Saved {} — newer changes remain unsaved", saved.title);
                }
            }
            Ok(Outcome::Exported(path)) => {
                self.doc.status = format!("Exported {}", path.display());
                // A sandboxed host publishes the written file through its own picker.
                #[cfg(not(feature = "desktop"))]
                self.exported.push(path);
            }
            Ok(Outcome::Extension(reply)) => {
                self.doc.status = reply.status.clone();
                self.ui.extensions.native.receive(reply);
            }
            Err(error) => {
                self.doc.status = error;
                self.settle_provisional_tab(None, diagnostic);
            }
        }
        if let Some(action) = action {
            self.doc.diagnostics.record(
                crate::diagnostics::Source::File(action),
                diagnostic_state,
                &self.doc.status,
            );
        }
        if let Some(next) = next {
            self.launch_document(next);
        }
        if let Some(finished) = save_all {
            self.save_all_finished(tab, finished);
        }
    }
}

pub(crate) fn needs_confirmation(pending: &Pending, dirty: bool) -> bool {
    dirty
        && match pending {
            Pending::Quit | Pending::CloseTab => true,
            Pending::Document(request) => request.replaces_document(),
        }
}

/// The runtime session the user's discard decision names.
fn discard_session(app: &crate::app::App) -> Option<String> {
    app.doc
        .document_state
        .as_ref()
        .map(|state| state.runtime_session.clone())
        .filter(|session| !session.is_empty())
}

impl crate::app::App {
    /// Closes the active tab, asking first when it holds unsaved work, an open
    /// sketch or an active tool.
    pub fn request_close_tab(&mut self) {
        self.sync_document_state();
        if self.busy() {
            self.doc.status = "Wait for the current command before closing".into();
            return;
        }
        if self.doc.needs_close_decision() {
            self.doc.documents.confirm = Some(Pending::CloseTab);
            self.describe_changes();
        } else {
            self.close_tab_now(None);
        }
    }

    fn close_tab_now(&mut self, discard: Option<&str>) {
        if let Err(error) = self.close_active_tab(discard) {
            self.doc.status = format!("Close document: {error}");
        }
    }
}

pub(crate) fn tool_blocks_request(app: &crate::app::App, request: &Request) -> bool {
    let recovery_save =
        matches!(request, Request::Save { .. }) && crate::bars::recovery_sketch_save(app);
    app.sketch_active() && !recovery_save
        || (request.replaces_document() || *request == Request::InsertStep)
            && app.doc.interaction.tool.is_some()
}

/// Runs without presentation so hidden windows still drain work and protect dirty close.
pub fn prepare_close(app: &mut crate::app::App) -> bool {
    app.sync_document_state();
    if !app.close_prompt || app.busy() {
        return false;
    }
    app.close_prompt = false;
    if !app.doc.needs_close_decision() {
        // Quit decides document by document: bring the next undecided one forward.
        let Some(tab) = app
            .tabs
            .parked
            .iter()
            .find(|doc| doc.needs_close_decision())
            .map(|doc| doc.tab)
        else {
            app.stop();
            return false;
        };
        app.activate_tab(tab);
    }
    app.doc.documents.confirm = Some(Pending::Quit);
    app.describe_changes();
    true
}

pub fn close_prompt(app: &mut crate::app::App, ctx: &egui::Context) {
    if app.doc.documents.confirm.is_none() {
        return;
    }
    if tab_close_prompt(app, ctx) {
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

/// Closing (or quitting with) a document that has a visible tab asks on the tab
/// itself, listing what would be lost. Returns `false` when there is no tab to
/// anchor to, and the window-wide dialog asks instead.
fn tab_close_prompt(app: &mut crate::app::App, ctx: &egui::Context) -> bool {
    if !matches!(
        app.doc.documents.confirm,
        Some(Pending::CloseTab | Pending::Quit)
    ) || app.ui.start_open
    {
        return false;
    }
    let Some(anchor) = app
        .tab_rects
        .iter()
        .find(|(tab, _)| *tab == app.doc.tab)
        .map(|(_, rect)| *rect)
    else {
        return false;
    };
    let title = app.doc.editor_title();
    let body = crate::documents::changes_text(app.doc.close_changes.as_ref());
    let out = onecad_widgets::components::overlays::TabClosePopover::new(&title, anchor)
        .body(Some(&body))
        .saving(app.doc.documents.running || app.sketch_active())
        .show(ctx, egui::Id::unique("document-tab-close"));
    let choice = if out.save {
        Some(0)
    } else if out.discard {
        Some(1)
    } else if out.cancel {
        Some(2)
    } else {
        None
    };
    apply_close_choice(app, choice);
    true
}

pub(crate) fn apply_close_choice(app: &mut crate::app::App, choice: Option<u8>) {
    match choice {
        Some(0) => {
            app.doc.documents.after_save = app.doc.documents.confirm.take();
            app.launch_document(Pending::Document(Request::Save { save_as: false }));
        }
        Some(1) => {
            if let Some(pending) = app.doc.documents.confirm.take() {
                let pending = match pending {
                    Pending::Document(request) => {
                        Pending::Document(Request::DiscardAndReplace(Box::new(request)))
                    }
                    Pending::CloseTab => {
                        app.close_tab_now(discard_session(app).as_deref());
                        return;
                    }
                    Pending::Quit
                        if app.tabs.parked.iter().any(|doc| doc.needs_close_decision()) =>
                    {
                        // Discard this one; the quit continues with the next document.
                        app.close_tab_now(discard_session(app).as_deref());
                        app.close_prompt = true;
                        return;
                    }
                    Pending::Quit => {
                        app.stop_discard();
                        return;
                    }
                };
                app.launch_document(pending);
            }
        }
        Some(2) => app.doc.documents.confirm = None,
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_extensions_ignore_unrelated_document_failure() {
        let command =
            serde_json::from_value(serde_json::json!({"id": "addon.run", "title": "Run"})).unwrap();
        let mut native = crate::native_extensions::State {
            pending: true,
            status: "Extension is running".into(),
            commands: vec![onecad_extension_host::RegisteredCommand {
                extension_id: "addon".into(),
                command,
                enabled: true,
            }],
            ..Default::default()
        };
        let mut ordinary_file = Flow {
            running: true,
            ..Default::default()
        };
        ordinary_file
            .report_extension_result(&mut native, &Err("A parked document could not save".into()));
        assert!(native.pending);
        assert!(native.commands[0].enabled);
        assert_eq!(native.status, "Extension is running");
        let mut extension = Flow {
            running: true,
            extension_request: true,
            ..Default::default()
        };
        extension.report_extension_result(
            &mut native,
            &Err("Extension approval store is unreadable".into()),
        );
        assert!(!native.pending);
        assert!(!native.commands[0].enabled);
        assert_eq!(native.status, "Extension approval store is unreadable");
        assert!(!extension.extension_request);
    }
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
            assert!(flow.blocks_authoring(&Action::Republish));
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
        for request in [Request::New, Request::Open(None), Request::Import(None)] {
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
            Request::InsertStep,
        ] {
            assert!(!request.replaces_document());
            assert!(!needs_confirmation(&Pending::Document(request), true));
        }
        assert!(needs_confirmation(&Pending::Quit, true));
        assert!(!needs_confirmation(&Pending::Quit, false));
    }
}
