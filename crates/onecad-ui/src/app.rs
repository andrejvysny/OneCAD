use crate::{
    bridge::{Candidate, Command, Display, Message},
    fonts, menu,
    qa::QaPaths,
    theme,
};
use onecad_app::host::DialogPort;
use onecad_interaction::{Action, CommitStatus, DocumentContext, Effect, RequestTag};
use onecad_render::Renderer;
use std::sync::Arc;

type MenuInstall<'a> =
    Box<dyn FnOnce(&egui::Context) -> Result<Option<menu::NativeMenu>, String> + 'a>;

pub struct App {
    pub doc: crate::document_editor::DocumentEditor,
    /// True while a background document's messages are applied through this App: suppresses process-level effects (window close, start-screen switching) that belong to the active document only.
    pub parked: bool,
    /// Every other open document, parked behind the active one.
    pub(crate) tabs: crate::documents::Tabs,
    pub(crate) extension_refresh: crate::native_extensions::refresh::Service,
    /// Where each document tab was last painted, to anchor its close prompt.
    pub(crate) tab_rects: Vec<(crate::documents::TabId, egui::Rect)>,
    /// The ⌃Tab switcher while Control is held.
    pub(crate) switcher: Option<crate::tab_switcher::State>,
    pub input_routing: crate::input::RoutingState,
    pub ui: crate::ui_state::UiState,
    pub settings: crate::settings::Store,
    pub renderer: Renderer,
    pub gpu: egui_wgpu::RenderState,
    pub texture: Option<egui::TextureId>,
    pub theme: theme::Theme,
    pub dark: bool,
    pub pending_display_options: Option<onecad_render::RenderOptions>,
    pub view_number: u64,
    pub viewport_failure: Option<String>,
    pub resize_refusal: Option<([u32; 2], String)>,
    device_failure: Arc<std::sync::Mutex<Option<String>>>,
    pub composing: bool,
    pub menu: Option<menu::NativeMenu>,
    pub close_prompt: bool,
    pub may_close: bool,
    pub stopping: bool,
    pub draw_dirty: bool,
    pub cad_needs_fence: bool,
    pub sampled: bool,
    pub sampling_done: Arc<std::sync::atomic::AtomicBool>,
    /// Process-wide display memory: the renderer's share and parked tabs' caches.
    pub display_budget: crate::display_budget::DisplayBudget,
    /// Memory-pressure ceiling on the scene image scale (1 = none); see
    /// `onecad_render::RenderProfile::scene_scale`.
    pub render_scale_limit: f64,
    /// The scene image scale relative to native pixels the viewport last used.
    pub scene_scale: f64,
    /// Files exported since the host last asked (`host::Editor::take_exported`).
    #[cfg(not(feature = "desktop"))]
    pub(crate) exported: Vec<std::path::PathBuf>,
}

impl App {
    #[cfg(feature = "desktop")]
    pub fn new(cc: &eframe::CreationContext<'_>, paths: QaPaths) -> Result<Self, String> {
        let gpu = cc
            .wgpu_render_state
            .clone()
            .ok_or("native host requires wgpu")?;
        let dialogs = Arc::new(crate::dialogs::NativeDialogs::new(paths.documents.clone()));
        let menu: MenuInstall<'_> =
            Box::new(|ctx| cc.winit_window().map(|_| menu::install(ctx)).transpose());
        let backend = onecad_app::worker::WorkerBackend::default();
        let profile = onecad_render::RenderProfile::Desktop;
        Self::build(&cc.egui_ctx, gpu, paths, dialogs, backend, profile, menu)
    }

    /// For hosts that own the egui context, GPU device and file dialogs and drive
    /// [`App::frame`] themselves (the iPadOS bridge). Without a native menu bar the
    /// File actions queue through [`menu::host_menu`].
    pub fn new_with(
        ctx: &egui::Context,
        gpu: egui_wgpu::RenderState,
        paths: QaPaths,
        dialogs: Arc<dyn DialogPort>,
        backend: onecad_app::worker::WorkerBackend,
        profile: onecad_render::RenderProfile,
    ) -> Result<Self, String> {
        Self::build(
            ctx,
            gpu,
            paths,
            dialogs,
            backend,
            profile,
            Box::new(|_| Ok(menu::host_menu())),
        )
    }

    fn build(
        ctx: &egui::Context,
        gpu: egui_wgpu::RenderState,
        paths: QaPaths,
        dialogs: Arc<dyn DialogPort>,
        backend: onecad_app::worker::WorkerBackend,
        profile: onecad_render::RenderProfile,
        install_menu: MenuInstall<'_>,
    ) -> Result<Self, String> {
        let display_budget = crate::display_budget::DisplayBudget::for_profile(profile);
        let renderer = Renderer::with_profile(
            &gpu.device,
            &gpu.queue,
            [0, 0],
            display_budget.render_limits(),
            profile,
        )
        .map_err(|e| e.to_string())?;
        let device_failure = Arc::new(std::sync::Mutex::new(None));
        let failure = device_failure.clone();
        let context = ctx.clone();
        gpu.device.set_device_lost_callback(move |reason, message| {
            if reason != wgpu::DeviceLostReason::Destroyed {
                if let Ok(mut slot) = failure.lock() {
                    *slot = Some(format!("GPU device lost ({reason:?}): {message}"));
                }
                context.request_repaint();
            }
        });
        let (settings, mut ui, settings_error) = crate::settings::Store::load(&paths, ctx.clone());
        ui.start_open = !paths.open_on_start;
        let dark = ui.appearance.resolve_dark(ctx.system_theme());
        let extension_refresh = crate::native_extensions::refresh::Service::new(paths.clone());
        let tabs = crate::documents::Tabs::new(
            crate::service_runtime::ServiceRuntime::start_with(backend)?,
            paths,
            dialogs,
            renderer.preparation_budget(),
        );
        let bridge = tabs.bridge(ctx.clone(), true)?;
        let resolved = fonts::install(ctx);
        eprintln!(
            "native fonts: {:?} {:?}; icons: {}",
            resolved.ui,
            resolved.mono,
            onecad_widgets::icon::names().len()
        );
        let theme = theme::load(dark)?;
        theme::install(ctx, theme.clone(), dark);
        let menu = install_menu(ctx)?;
        if let Some(menu) = &menu {
            menu.set_appearance(ui.appearance);
        }
        bridge.submit(Command::Open)?;
        Ok(Self {
            doc: crate::document_editor::DocumentEditor::new(
                bridge,
                settings_error.unwrap_or_else(|| "Opening QA workspace…".into()),
            )?,
            parked: false,
            tabs,
            extension_refresh,
            switcher: None,
            tab_rects: Vec::new(),
            input_routing: Default::default(),
            ui,
            settings,
            renderer,
            gpu,
            texture: None,
            theme,
            dark,
            pending_display_options: None,
            view_number: 0,
            viewport_failure: None,
            resize_refusal: None,
            device_failure,
            composing: false,
            menu,
            close_prompt: false,
            may_close: false,
            stopping: false,
            draw_dirty: true,
            cad_needs_fence: false,
            sampled: false,
            sampling_done: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            display_budget,
            render_scale_limit: 1.0,
            scene_scale: 1.0,
            #[cfg(not(feature = "desktop"))]
            exported: Vec::new(),
        })
    }

    pub fn dispatch(&mut self, action: Action) {
        if self.doc.bridge.shared.input_trace.enabled() {
            crate::input_trace::app(
                self,
                "model.dispatch",
                crate::input_trace::actions::model(&action),
            );
        }
        if self.doc.documents.blocks_authoring(&action) {
            self.doc.command_status = true;
            self.doc.status = "Finish the file action before editing the document".into();
            crate::input_trace::app(
                self,
                "model.refused",
                serde_json::json!({"reason": self.doc.status}),
            );
            return;
        }
        if matches!(
            action,
            Action::Confirm
                | Action::DraftChanged(_)
                | Action::ParameterChanged { .. }
                | Action::ParameterDrag(onecad_interaction::DragPhase::Update { .. })
                | Action::BeginExtrude { .. }
                | Action::BeginOperation { .. }
                | Action::BeginEdit { .. }
                | Action::BeginParameterEdit { .. }
                | Action::BeginOperationEdit { .. }
                | Action::ModelingOptionsChanged { .. }
        ) {
            self.doc.command_status = false;
        }
        for effect in self.doc.interaction.dispatch(action) {
            self.effect(effect);
        }
        if self.doc.document_state.as_ref().is_none_or(|state| {
            self.doc
                .interaction
                .context
                .as_ref()
                .is_some_and(|context| context.runtime_session == state.runtime_session)
        }) {
            self.doc.diagnostics.model(&self.doc.interaction.status);
        }
        if let CommitStatus::Rejected(reason)
        | CommitStatus::CompletionUnavailable {
            message: reason, ..
        } = &self.doc.interaction.status
        {
            crate::input_trace::app(
                self,
                "model.refused",
                serde_json::json!({"reason": crate::input_trace::bounded_id(reason)}),
            );
        }
        if self.doc.interaction.tool.is_none() {
            // Staged operands never outlive the tool that owned them.
            self.doc.operand_staging = None;
        }
        if self.doc.interaction.tool.is_none()
            && let Some(id) = self.doc.numeric_id.take()
        {
            self.doc.bridge.shared.release_text_focus(id);
        }
    }

    fn effect(&mut self, effect: Effect) {
        match effect {
            Effect::EvaluateExpression {
                tag,
                expression,
                site,
            } => self.doc.bridge.candidate(Candidate::Evaluate {
                tag,
                expression,
                site,
            }),
            Effect::Preview {
                tag,
                operation,
                sketch_id,
                expected_snapshot,
            } => self.doc.bridge.candidate(Candidate::Preview {
                tag,
                operation,
                sketch_id: sketch_id.map(|id| id.to_string()),
                snapshot: expected_snapshot,
            }),
            Effect::ClearPreview { .. } => {
                self.doc.bridge.cancel_candidates();
                self.doc.sections.cancel_preview();
                self.doc.pending_preview = None;
                self.doc.received_preview = None;
                self.doc.clear_preview = true;
            }
            Effect::PreviewExistingEdit { tag, operation } => self
                .doc
                .bridge
                .candidate(Candidate::PreviewExistingEdit { tag, operation }),
            Effect::PublishPreview { tag, result } => {
                if let Some(mut preview) = self.doc.received_preview.take() {
                    if self
                        .doc
                        .interaction
                        .tool
                        .as_ref()
                        .is_some_and(|tool| tool.editing_existing)
                    {
                        preview.suppressed = result
                            .changed_bodies
                            .iter()
                            .filter_map(|id| id.parse().ok())
                            .collect();
                    }
                    self.doc.pending_preview = Some((tag, preview));
                }
            }
            Effect::ApplyOperation {
                tag,
                command,
                hide_sketch,
            } => {
                self.doc.bridge.cancel_candidates();
                self.doc.sections.cancel_preview();
                self.doc.pending_preview = None;
                self.doc.clear_preview = true;
                if let Err(message) = self.doc.bridge.submit(Command::Apply {
                    tag: tag.clone(),
                    command,
                    hide_sketch: hide_sketch.map(|id| id.to_string()),
                }) {
                    self.dispatch(Action::CommitRejected { tag, message });
                }
            }
        }
    }

    pub fn submit(&mut self, command: Command) {
        tracing::info!(
            command = crate::bridge::command_name(&command),
            "Native model command requested"
        );
        if self.doc.documents.running || self.doc.documents.confirm.is_some() {
            self.doc.status = "Finish the file action before editing the document".into();
            return;
        }
        self.doc.command_status = true;
        self.doc.status = format!("{} pending…", crate::bridge::command_name(&command));
        if matches!(command, Command::Undo | Command::Redo) && self.sketch_active() {
            self.doc.status =
                "Finish sketch before using model history; sketch undo is not implemented".into();
            return;
        }
        if matches!(command, Command::Undo | Command::Redo) && !self.doc.bridge.valid() {
            self.doc.status = "Geometry unavailable; history controls are disabled".into();
            return;
        }
        let history = matches!(command, Command::Undo | Command::Redo);
        match self.doc.bridge.submit(command) {
            Ok(()) if history => self.doc.authoring_dirty = true,
            Ok(()) => {}
            Err(error) => self.doc.status = error,
        }
    }

    pub fn poll(&mut self, ctx: &egui::Context) {
        crate::documents::set_current_tab(self.doc.tab);
        self.poll_device_failure();
        self.poll_extension_refresh();
        self.sync_document_state();
        if let Some(display) = self.doc.bridge.take_display()
            && self.doc.bridge.shared.accept_display(&display)
        {
            self.adopt_display(display);
        }
        if let Some(delivery) = self.doc.bridge.take_candidate() {
            self.doc.received_preview = delivery
                .prepared
                .zip(delivery.lease)
                .zip(delivery.scene)
                .map(
                    |((prepared, lease), scene)| crate::viewport_updates::Preview {
                        prepared,
                        suppressed: Vec::new(),
                        source: crate::viewport_updates::Source {
                            scene,
                            lease: Arc::new(lease),
                        },
                    },
                );
            self.dispatch(delivery.action);
            self.doc.received_preview = None;
        }
        // Command permits bound accepted plus terminal messages to at most 16; drain fairly each frame.
        self.drain_messages(ctx);
        crate::operation_start::poll(self);
        self.sync_document_state();
        let failure = self
            .doc
            .bridge
            .shared
            .failure
            .lock()
            .ok()
            .and_then(|mut value| value.take());
        if let Some(error) = failure {
            self.doc.status = error;
            self.doc.diagnostics.record(
                crate::diagnostics::Source::Worker,
                crate::diagnostics::State::Failed,
                &self.doc.status,
            );
            self.doc.bridge.cancel_candidates();
            self.doc.interaction.selection = None;
            self.doc.pending_preview = None;
            self.doc.clear_preview = true;
            self.doc.selection.clear();
            self.doc.topology.invalidate_pending();
            crate::parameter_drag::abandon(self, crate::parameter_drag::Outcome::Lost);
            self.doc.transform_gizmo = Default::default();
            self.draw_dirty = true;
        }
        self.drain_parked(ctx);
    }

    pub fn viewport_ready(&self) -> bool {
        self.viewport_failure.is_none()
    }

    pub fn fail_viewport(&mut self, error: String) {
        self.cancel_sketch_gesture();
        self.doc.status = format!("Viewport unavailable: {error}; Save remains available");
        self.doc.diagnostics.record(
            crate::diagnostics::Source::Viewport,
            crate::diagnostics::State::Failed,
            &self.doc.status,
        );
        self.viewport_failure = Some(error);
        self.doc.view = None;
        self.doc.bridge.cancel_candidates();
        self.doc.sections.cancel();
        crate::parameter_drag::abandon(self, crate::parameter_drag::Outcome::Lost);
        self.dispatch(Action::Cancel);
        self.doc.interaction.selection = None;
        self.doc.selection.clear();
        self.doc.topology.invalidate_pending();
        self.doc.transform_gizmo = Default::default();
    }

    pub fn poll_device_failure(&mut self) {
        let failure = self
            .device_failure
            .lock()
            .ok()
            .and_then(|mut slot| slot.take());
        if let Some(error) = failure {
            self.fail_viewport(error);
            self.doc.bridge.shared.recovery_flush.notify_one();
        }
    }

    pub fn adopt_display(&mut self, display: Arc<Display>) {
        self.doc
            .bridge
            .shared
            .input_trace
            .capture("host.capture_adopt", &display);
        let projection = &display.projection;
        tracing::info!(document_id = %projection.document_id,
            runtime_session = %projection.runtime_session,
            revision = projection.revision, undo_depth = projection.undo_depth,
            redo_depth = projection.redo_depth, dirty = projection.dirty,
            applied_ops = projection.applied_ops, bodies = projection.bodies.len(),
            "Native document adopted");
        let changed = self
            .doc
            .display
            .as_ref()
            .is_none_or(|old| old.context.runtime_session != display.context.runtime_session);
        let context = DocumentContext {
            document_id: display.context.document_id.clone(),
            runtime_session: display.context.runtime_session.clone(),
            revision: display.context.revision.0,
            snapshot_id: display.context.snapshot_id,
            scene: display.scene.identity(),
        };
        self.dispatch(Action::ContextChanged(Some(context.clone())));
        self.sketch_dispatch(onecad_interaction::sketch::SketchAction::ContextChanged(
            Some(context),
        ));
        self.sync_sketch_gesture();
        if changed {
            crate::document_items::reset(self);
            self.doc.selection.clear();
            self.doc.topology.invalidate_pending();
            crate::parameter_drag::abandon(self, crate::parameter_drag::Outcome::Lost);
            self.doc.transform_gizmo = Default::default();
            self.doc.fit_pending = true;
        }
        self.doc
            .diagnostics
            .runtime(&display.context.runtime_session, &self.doc.status);
        crate::profile_diagnostics::record_display_failures(&mut self.doc.diagnostics, &display);
        crate::item_selection::reconcile(self, &display);
        self.doc.display = Some(display);
        crate::item_selection::refresh(self);
    }

    fn model_delivery_current(&self, tag: &RequestTag) -> bool {
        if self
            .doc
            .document_state
            .as_ref()
            .is_some_and(|state| state.runtime_session != tag.context.runtime_session)
        {
            return false;
        }
        if let Some(tool) = &self.doc.interaction.tool {
            return tool.tag == *tag;
        }
        match &self.doc.interaction.status {
            CommitStatus::Pending(current)
            | CommitStatus::Accepted { tag: current, .. }
            | CommitStatus::Completed { tag: current, .. }
            | CommitStatus::CompletionUnavailable { tag: current, .. } => current == tag,
            _ => false,
        }
    }

    pub(crate) fn message(&mut self, message: Message, ctx: &egui::Context) {
        if self.doc.bridge.shared.input_trace.enabled() {
            crate::input_trace::app(
                self,
                "service.delivery_received",
                crate::input_trace::actions::message(&message),
            );
        }
        match message {
            Message::Items {
                sequence, result, ..
            } => crate::document_items::receive(self, sequence, result),
            Message::Document { result, .. } => self.document_finished(result),
            Message::Gesture { delivery, .. } => self.gesture_delivery(*delivery),
            Message::Sketch { delivery, .. } => {
                self.finish_sketch_candidate(delivery.tag.as_ref());
                let current =
                    delivery.tag.as_ref().is_none_or(|tag| {
                        self.doc.document_state.as_ref().is_none_or(|state| {
                            state.runtime_session == tag.document.runtime_session
                        }) && self.doc.sketch.session.as_ref().is_some_and(|session| {
                            session.request == *tag
                                && !matches!(
                                    session.phase,
                                    onecad_interaction::sketch::SketchPhase::Active
                                        | onecad_interaction::sketch::SketchPhase::Closed
                                        | onecad_interaction::sketch::SketchPhase::Faulted
                                )
                        })
                    });
                if let Some(display) = delivery.display.clone() {
                    if self.doc.bridge.shared.accept_display(&display) {
                        self.adopt_display(display);
                    } else {
                        for action in delivery.stale_actions() {
                            self.sketch_dispatch(action);
                        }
                        return;
                    }
                }
                for action in delivery.actions {
                    self.sketch_dispatch(action);
                }
                if let Some(error) = delivery.error.filter(|_| current) {
                    self.doc.status = error;
                }
            }
            Message::StartupFailed(error) => {
                self.doc.status = error;
                self.doc.diagnostics.record(
                    crate::diagnostics::Source::Worker,
                    crate::diagnostics::State::Failed,
                    &self.doc.status,
                );
            }
            Message::Stopped => {
                self.doc.stopped = true;
                // A closed tab stops alone; the window closes once every host stopped.
                if self.stopping && self.all_tabs_stopped() {
                    self.may_close = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            Message::Prepared {
                identity, result, ..
            } => self.adopt_prepared(identity, result),
            Message::Changes {
                runtime_session,
                changes,
                ..
            } => {
                let current = self
                    .doc
                    .document_state
                    .as_ref()
                    .is_some_and(|state| state.runtime_session == runtime_session);
                self.doc.close_changes = current.then_some(changes);
            }
            Message::Accepted {
                tag,
                revision,
                rollback_token,
            } => {
                let current = self.model_delivery_current(&tag);
                if current {
                    self.doc.command_status = true;
                }
                self.dispatch(Action::CommitAccepted {
                    tag,
                    revision,
                    rollback_token,
                });
                if current {
                    self.doc.status = "Regenerating feature…".into();
                }
            }
            Message::Terminal {
                command,
                tag,
                result,
                ..
            } => {
                let current = tag
                    .as_ref()
                    .is_none_or(|tag| self.model_delivery_current(tag));
                if current {
                    self.doc.command_status = true;
                }
                if matches!(command, "Open" | "Recovery") && !self.parked {
                    self.ui.start_open = !self.document_open();
                }
                match result {
                    Ok(outcome) => {
                        let completion = outcome.completion;
                        let target = tag.as_ref().map(|tag| tag.record_id.to_string());
                        if let (Some(tag), Some(completion)) = (tag, completion.as_ref()) {
                            self.dispatch(Action::RegenFinished {
                                tag,
                                completion: completion.clone(),
                            });
                        }
                        let mut status = match completion {
                            Some(ref result) if !result.failed_steps.is_empty() => {
                                format!(
                                    "Feature failed: {}",
                                    result
                                        .failed_steps
                                        .iter()
                                        .find(|step| Some(step.record_id.as_str())
                                            == target.as_deref())
                                        .unwrap_or(&result.failed_steps[0])
                                        .message
                                )
                            }
                            Some(ref result) if !result.repair_steps.is_empty() => {
                                "Feature needs repair".into()
                            }
                            Some(ref result) if result.outcome == "failed" => result
                                .message
                                .clone()
                                .unwrap_or("Regeneration failed".into()),
                            Some(ref result)
                                if !matches!(result.outcome.as_str(), "published" | "noop") =>
                            {
                                format!("Regeneration {}", result.outcome)
                            }
                            _ if command == "Open" && !self.document_open() => {
                                "Create a part or open a document".into()
                            }
                            _ => format!("{command} complete"),
                        };
                        if let Some(rollback) = outcome.rollback {
                            status = format!("{}; {}", status, rollback.explanation());
                        }
                        if current {
                            self.doc.status = status;
                        }
                        if self.doc.interaction.tool.as_ref().is_some_and(|tool| {
                            tool.phase == onecad_interaction::SessionPhase::Completed
                        }) {
                            self.dispatch(Action::Cancel);
                        }
                    }
                    Err(error) => {
                        if let Some(tag) = tag {
                            if matches!(self.doc.interaction.status, CommitStatus::Accepted { .. })
                            {
                                self.dispatch(Action::CompletionUnavailable {
                                    tag,
                                    message: error.clone(),
                                });
                            } else {
                                self.dispatch(Action::CommitRejected {
                                    tag,
                                    message: error.clone(),
                                });
                            }
                        }
                        if self.doc.interaction.tool.as_ref().is_some_and(|tool| {
                            tool.phase == onecad_interaction::SessionPhase::Completed
                        }) {
                            self.dispatch(Action::Cancel);
                        }
                        if current {
                            self.doc.status = error;
                        }
                    }
                }
            }
        }
    }

    pub fn stop_discard(&mut self) {
        self.sync_document_state();
        let session = self
            .doc
            .document_state
            .as_ref()
            .map(|state| state.runtime_session.clone());
        self.stopping = true;
        self.doc.bridge.cancel_candidates();
        if let Some(session) = session.filter(|session| !session.is_empty()) {
            self.doc.bridge.request_discard_shutdown(&session);
        } else {
            self.doc.bridge.request_shutdown();
        }
        self.stop_all_tabs();
    }
    pub fn stop(&mut self) {
        self.stopping = true;
        self.doc.bridge.cancel_candidates();
        self.doc.bridge.request_shutdown();
        self.stop_all_tabs();
    }

    /// A re-prepared scene for the current display (tab activation).
    fn adopt_prepared(
        &mut self,
        identity: onecad_scene::SceneIdentity,
        result: Result<onecad_render::PreparedScene, String>,
    ) {
        // Only the reply the editor is waiting for: a duplicate or superseded one
        // must not park a second prepared scene in an already-drawn display.
        if self.doc.prepare_in_flight != Some(identity) {
            return;
        }
        self.doc.prepare_in_flight = None;
        let Some(display) = self
            .doc
            .display
            .as_ref()
            .filter(|display| display.scene.identity() == identity)
        else {
            return; // A newer display superseded the scene this was prepared for.
        };
        match result {
            Ok(prepared) => {
                if let Ok(mut slot) = display.prepared.lock() {
                    slot.get_or_insert(prepared);
                }
                self.draw_dirty = true;
            }
            Err(error) => self.display_unavailable(error),
        }
    }
    pub fn busy(&self) -> bool {
        self.doc.items.blocks()
            || matches!(
                self.doc.interaction.status,
                CommitStatus::Pending(_) | CommitStatus::Accepted { .. }
            )
            || self.stopping
            || self.doc.documents.running
            || self.doc.documents.confirm.is_some()
            || self
                .doc
                .bridge
                .shared
                .recovery
                .lock()
                .is_ok_and(|recovery| recovery.busy)
    }
}

impl App {
    /// One host frame in eframe's order: route raw input first (this also fences the
    /// previous frame's CAD sampling), then run the pass with logic before ui. The
    /// caller must submit and present this output before the next call.
    pub fn frame(&mut self, ctx: &egui::Context, mut raw: egui::RawInput) -> egui::FullOutput {
        self.raw_input_hook(ctx, &mut raw);
        ctx.run_ui(raw, |ui| {
            self.logic(ui.ctx());
            // The root Ui spans the full viewport; hosts with a status bar or
            // home indicator report them as safe-area insets, so lay out inside
            // the content rect.
            let content = ui.ctx().content_rect();
            ui.scope_builder(egui::UiBuilder::new().max_rect(content), |ui| self.ui(ui));
        })
    }

    pub fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        crate::input::route(self, ctx, raw);
    }

    pub fn logic(&mut self, ctx: &egui::Context) {
        crate::input_trace::app(
            self,
            "host.logic",
            serde_json::json!({"pass": ctx.cumulative_pass_nr()}),
        );
        self.doc.bridge.shared.input_trace.observe_window(ctx);
        self.poll(ctx);
        if ctx.input(|input| input.viewport().close_requested()) && !self.may_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.close_prompt = true;
        }
        if crate::document_flow::prepare_close(self) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        if let Err(error) = self.settings.save_if_changed(&self.ui) {
            self.doc.status = format!("Settings: {error}");
        }
    }
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        crate::input_trace::app(
            self,
            "host.ui",
            serde_json::json!({"pass": ctx.cumulative_pass_nr(), "frame": ctx.cumulative_frame_nr()}),
        );
        if self.ui.appearance == crate::ui_state::Appearance::System {
            crate::appearance::sync(self, &ctx);
        }
        if crate::startup_recovery::show(self, &ctx) {
            crate::shell::close_prompt(self, &ctx);
            return;
        }
        crate::shell::show(self, ui);
        if self.ui.start_open {
            crate::start_screen::show(self, ui);
        } else {
            crate::viewport::show(self, ui);
        }
        // After the viewport, so an in-batch release consumed by a handle wins.
        crate::parameter_drag::settle(self, &ctx);
        crate::component_wizard::show(self, &ctx);
        crate::tab_switcher::show(self, &ctx);
        crate::shell::close_prompt(self, &ctx);
    }
}

#[cfg(feature = "desktop")]
impl eframe::App for App {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        App::raw_input_hook(self, ctx, raw);
    }
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        App::logic(self, ctx);
    }
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        App::ui(self, ui);
    }
}
