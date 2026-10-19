use crate::{
    bridge::{Bridge, Candidate, Command, Display, Message},
    fonts, menu,
    qa::QaPaths,
    theme,
};
use onecad_core::ids::{BodyId, RecordId};
use onecad_interaction::{Action, CommitStatus, DocumentContext, Effect, Interaction, RequestTag};
use onecad_render::Renderer;
use onecad_scene::{Camera, ViewSnapshot};
use std::sync::Arc;

pub struct App {
    pub input_routing: crate::input::RoutingState,
    pub ui: crate::ui_state::UiState,
    pub settings: crate::settings::Store,
    pub documents: crate::document_flow::Flow,
    pub bridge: Bridge,
    pub display: Option<Arc<Display>>,
    pub interaction: Interaction,
    pub sketch: onecad_interaction::sketch::SketchInteraction,
    pub renderer: Renderer,
    pub gpu: egui_wgpu::RenderState,
    pub texture: Option<egui::TextureId>,
    pub theme: theme::Theme,
    pub dark: bool,
    pub camera: Camera,
    pub navigation: crate::navigation::Navigation,
    pub transform_gizmo: crate::transform_ui::TransformGizmo,
    pub topology: crate::viewport_selection::Selection,
    pub pending_display_options: Option<onecad_render::RenderOptions>,
    /// Last drawable capture; `camera` may already contain pending navigation.
    pub view: Option<ViewSnapshot>,
    pub view_number: u64,
    pub viewport_failure: Option<String>,
    pub resize_refusal: Option<([u32; 2], String)>,
    device_failure: Arc<std::sync::Mutex<Option<String>>>,
    pub fit_pending: bool,
    pub selected_body: Option<BodyId>,
    pub selected_feature: Option<RecordId>,
    pub numeric_id: Option<egui::Id>,
    pub composing: bool,
    pub menu: Option<menu::NativeMenu>,
    pub status: String,
    pub command_status: bool,
    pub close_prompt: bool,
    pub may_close: bool,
    pub stopping: bool,
    pub authoring_dirty: bool,
    pub gesture: u64,
    pub pending_preview: Option<(RequestTag, crate::viewport_updates::Preview)>,
    pub sections: crate::viewport_updates::Sections,
    pub clear_preview: bool,
    pub draw_dirty: bool,
    pub cad_needs_fence: bool,
    pub sampled: bool,
    pub sampling_done: Arc<std::sync::atomic::AtomicBool>,
    received_preview: Option<crate::viewport_updates::Preview>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, paths: QaPaths) -> Result<Self, String> {
        let gpu = cc
            .wgpu_render_state
            .clone()
            .ok_or("native host requires wgpu")?;
        let limits = onecad_render::RenderLimits {
            max_cpu_prepared_bytes: crate::budget::MAX_PREPARED_BYTES,
            max_upload_bytes: crate::budget::MAX_PREPARED_BYTES,
            ..Default::default()
        };
        let renderer =
            Renderer::new(&gpu.device, &gpu.queue, [0, 0], limits).map_err(|e| e.to_string())?;
        let device_failure = Arc::new(std::sync::Mutex::new(None));
        let failure = device_failure.clone();
        let context = cc.egui_ctx.clone();
        gpu.device.set_device_lost_callback(move |reason, message| {
            if reason != wgpu::DeviceLostReason::Destroyed {
                if let Ok(mut slot) = failure.lock() {
                    *slot = Some(format!("GPU device lost ({reason:?}): {message}"));
                }
                context.request_repaint();
            }
        });
        let (settings, mut ui, settings_error) =
            crate::settings::Store::load(&paths, cc.egui_ctx.clone());
        ui.start_open = !paths.open_on_start;
        let dark = ui.appearance.resolve_dark(cc.egui_ctx.system_theme());
        let bridge = Bridge::start(paths, cc.egui_ctx.clone(), renderer.preparation_budget())?;
        let resolved = fonts::install(&cc.egui_ctx);
        eprintln!(
            "native fonts: {:?} {:?}; icons: {}",
            resolved.ui,
            resolved.mono,
            onecad_widgets::icon::names().len()
        );
        let theme = theme::load(dark)?;
        theme::install(&cc.egui_ctx, theme.clone(), dark);
        let menu = cc
            .winit_window()
            .map(|_| menu::install(&cc.egui_ctx))
            .transpose()?;
        if let Some(menu) = &menu {
            menu.set_appearance(ui.appearance);
        }
        bridge.submit(Command::Open)?;
        Ok(Self {
            input_routing: Default::default(),
            ui,
            settings,
            documents: Default::default(),
            bridge,
            display: None,
            interaction: Interaction::default(),
            sketch: Default::default(),
            renderer,
            gpu,
            texture: None,
            theme,
            dark,
            camera: Camera::default(),
            navigation: Default::default(),
            transform_gizmo: Default::default(),
            topology: Default::default(),
            pending_display_options: None,
            view: None,
            view_number: 0,
            viewport_failure: None,
            resize_refusal: None,
            device_failure,
            fit_pending: true,
            selected_body: None,
            selected_feature: None,
            numeric_id: None,
            composing: false,
            menu,
            status: settings_error.unwrap_or_else(|| "Opening QA workspace…".into()),
            command_status: false,
            close_prompt: false,
            may_close: false,
            stopping: false,
            authoring_dirty: false,
            gesture: 0,
            pending_preview: None,
            sections: crate::viewport_updates::Sections::new()?,
            clear_preview: true,
            received_preview: None,
            draw_dirty: true,
            cad_needs_fence: false,
            sampled: false,
            sampling_done: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        })
    }

    pub fn dispatch(&mut self, action: Action) {
        if self.documents.blocks_authoring(&action) {
            self.command_status = true;
            self.status = "Finish the file action before editing the document".into();
            return;
        }
        if matches!(
            action,
            Action::Confirm
                | Action::DraftChanged(_)
                | Action::BeginExtrude { .. }
                | Action::BeginOperation { .. }
                | Action::BeginEdit { .. }
                | Action::BeginParameterEdit { .. }
        ) {
            self.command_status = false;
        }
        for effect in self.interaction.dispatch(action) {
            self.effect(effect);
        }
        if self.interaction.tool.is_none()
            && let Some(id) = self.numeric_id.take()
        {
            self.bridge.shared.release_text_focus(id);
        }
    }

    fn effect(&mut self, effect: Effect) {
        match effect {
            Effect::EvaluateExpression {
                tag,
                expression,
                site,
            } => self.bridge.candidate(Candidate::Evaluate {
                tag,
                expression,
                site,
            }),
            Effect::Preview {
                tag,
                operation,
                sketch_id,
                expected_snapshot,
            } => self.bridge.candidate(Candidate::Preview {
                tag,
                operation,
                sketch_id: sketch_id.map(|id| id.to_string()),
                snapshot: expected_snapshot,
            }),
            Effect::ClearPreview { .. } => {
                self.bridge.cancel_candidates();
                self.sections.cancel_preview();
                self.pending_preview = None;
                self.received_preview = None;
                self.clear_preview = true;
            }
            Effect::PublishPreview { tag, .. } => {
                if let Some(preview) = self.received_preview.take() {
                    self.pending_preview = Some((tag, preview));
                }
            }
            Effect::ApplyOperation {
                tag,
                command,
                hide_sketch,
            } => {
                self.bridge.cancel_candidates();
                self.sections.cancel_preview();
                self.pending_preview = None;
                self.clear_preview = true;
                if let Err(message) = self.bridge.submit(Command::Apply {
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
        if self.documents.running || self.documents.confirm.is_some() {
            self.status = "Finish the file action before editing the document".into();
            return;
        }
        self.command_status = true;
        self.status = format!("{} pending…", crate::bridge::command_name(&command));
        if matches!(command, Command::Undo | Command::Redo) && self.sketch_active() {
            self.status =
                "Finish sketch before using model history; sketch undo is not implemented".into();
            return;
        }
        if matches!(command, Command::Undo | Command::Redo) && !self.bridge.valid() {
            self.status = "Geometry unavailable; history controls are disabled".into();
            return;
        }
        if matches!(command, Command::Undo | Command::Redo) {
            self.authoring_dirty = true;
        }
        if let Err(error) = self.bridge.submit(command) {
            self.status = error;
        }
    }

    pub fn poll(&mut self, ctx: &egui::Context) {
        self.poll_device_failure();
        if let Some(display) = self.bridge.take_display()
            && self.bridge.shared.accept_display(&display)
        {
            self.adopt_display(display);
        }
        if let Some(delivery) = self.bridge.take_candidate() {
            self.received_preview = delivery
                .prepared
                .zip(delivery.lease)
                .zip(delivery.scene)
                .map(
                    |((prepared, lease), scene)| crate::viewport_updates::Preview {
                        prepared,
                        source: crate::viewport_updates::Source {
                            scene,
                            lease: Arc::new(lease),
                        },
                    },
                );
            self.dispatch(delivery.action);
            self.received_preview = None;
        }
        // Command permits bound accepted plus terminal messages to at most 16; drain fairly each frame.
        while let Ok(message) = self.bridge.messages.try_recv() {
            self.message(message, ctx);
        }
        let failure = self
            .bridge
            .shared
            .failure
            .lock()
            .ok()
            .and_then(|mut value| value.take());
        if let Some(error) = failure {
            self.status = error;
            self.bridge.cancel_candidates();
            self.interaction.selection = None;
            self.pending_preview = None;
            self.clear_preview = true;
            self.selected_body = None;
            self.topology.invalidate_pending();
            self.transform_gizmo = Default::default();
            self.draw_dirty = true;
        }
    }

    pub fn viewport_ready(&self) -> bool {
        self.viewport_failure.is_none()
    }

    pub fn fail_viewport(&mut self, error: String) {
        self.status = format!("Viewport unavailable: {error}; Save remains available");
        self.viewport_failure = Some(error);
        self.view = None;
        self.bridge.cancel_candidates();
        self.sections.cancel();
        self.dispatch(Action::Cancel);
        self.interaction.selection = None;
        self.selected_body = None;
        self.topology.invalidate_pending();
        self.transform_gizmo = Default::default();
    }

    pub fn poll_device_failure(&mut self) {
        let failure = self
            .device_failure
            .lock()
            .ok()
            .and_then(|mut slot| slot.take());
        if let Some(error) = failure {
            self.fail_viewport(error);
            self.bridge.shared.recovery_flush.notify_one();
        }
    }

    pub fn adopt_display(&mut self, display: Arc<Display>) {
        let projection = &display.projection;
        tracing::info!(document_id = %projection.document_id,
            runtime_session = %projection.runtime_session,
            revision = projection.revision, undo_depth = projection.undo_depth,
            redo_depth = projection.redo_depth, dirty = projection.dirty,
            applied_ops = projection.applied_ops, bodies = projection.bodies.len(),
            "Native document adopted");
        self.authoring_dirty = display.projection.dirty;
        let changed = self
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
        if changed {
            self.selected_body = None;
            self.selected_feature = None;
            self.topology.invalidate_pending();
            self.transform_gizmo = Default::default();
            self.fit_pending = true;
        }
        self.display = Some(display);
    }

    fn message(&mut self, message: Message, ctx: &egui::Context) {
        match message {
            Message::Document { result, .. } => self.document_finished(result),
            Message::Sketch { delivery, .. } => {
                if let Some(display) = delivery.display.clone() {
                    if self.bridge.shared.accept_display(&display) {
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
                if let Some(error) = delivery.error {
                    self.status = error;
                }
            }
            Message::StartupFailed(error) => self.status = error,
            Message::Stopped => {
                self.may_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Message::Accepted {
                tag,
                revision,
                rollback_token,
            } => {
                self.command_status = true;
                self.authoring_dirty = true;
                self.dispatch(Action::CommitAccepted {
                    tag,
                    revision,
                    rollback_token,
                });
                self.status = "Regenerating feature…".into();
            }
            Message::Terminal {
                command,
                tag,
                result,
                ..
            } => {
                self.command_status = true;
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
                        self.status = match completion {
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
                            _ => format!("{command} complete"),
                        };
                        if let Some(rollback) = outcome.rollback {
                            self.status = format!("{}; {}", self.status, rollback.explanation());
                        }
                        if self.interaction.tool.as_ref().is_some_and(|tool| {
                            tool.phase == onecad_interaction::SessionPhase::Completed
                        }) {
                            self.dispatch(Action::Cancel);
                        }
                    }
                    Err(error) => {
                        if let Some(tag) = tag {
                            if matches!(self.interaction.status, CommitStatus::Accepted { .. }) {
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
                        if self.interaction.tool.as_ref().is_some_and(|tool| {
                            tool.phase == onecad_interaction::SessionPhase::Completed
                        }) {
                            self.dispatch(Action::Cancel);
                        }
                        self.status = error;
                    }
                }
            }
        }
    }

    pub fn stop(&mut self) {
        self.stopping = true;
        self.bridge.cancel_candidates();
        self.bridge.request_shutdown();
    }
    pub fn busy(&self) -> bool {
        matches!(
            self.interaction.status,
            CommitStatus::Pending(_) | CommitStatus::Accepted { .. }
        ) || self.stopping
            || self.documents.running
            || self.documents.confirm.is_some()
    }
}

impl eframe::App for App {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        crate::input::route(self, ctx, raw);
    }
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
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
            self.status = format!("Settings: {error}");
        }
    }
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
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
        crate::shell::close_prompt(self, &ctx);
    }
}
