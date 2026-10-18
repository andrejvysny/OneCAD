use crate::{app::App, bridge::Command, fonts, icons};
use onecad_interaction::{Action, DraftStatus, PreviewStatus, SessionPhase};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    title(app, ui);
    status(app, ui);
    tree(app, ui);
    inspector(app, ui);
}

fn title(app: &mut App, ui: &mut egui::Ui) {
    egui::Panel::top("title")
        .exact_size(44.0)
        .frame(
            egui::Frame::new()
                .fill(app.theme.titlebar)
                .inner_margin(egui::Margin::symmetric(12, 0)),
        )
        .resizable(false)
        .show(ui, |ui| {
            let drag = ui.interact(
                ui.max_rect(),
                ui.scope_id().with("window-drag"),
                egui::Sense::drag(),
            );
            if drag.drag_started() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
            ui.horizontal_centered(|ui| {
                ui.add_space(if cfg!(target_os = "macos") { 74.0 } else { 0.0 });
                let title = app.display.as_ref().map_or("OneCAD".to_owned(), |display| {
                    format!(
                        "OneCAD — {}{}",
                        display.projection.title,
                        if display.projection.dirty { " •" } else { "" }
                    )
                });
                ui.label(
                    egui::RichText::new(title)
                        .family(fonts::semibold())
                        .color(app.theme.titlebar_text),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(if app.dark { "Light" } else { "Dark" }).clicked() {
                        app.dark = !app.dark;
                        if let Ok(theme) = crate::theme::load(app.dark) {
                            app.theme = theme;
                            crate::theme::apply(ui.ctx(), &app.theme, app.dark);
                            app.draw_dirty = true;
                        }
                    }
                    if ui.button("Save").clicked() {
                        app.submit(Command::Save);
                    }
                });
            });
        });
    egui::Panel::top("tools")
        .exact_size(40.0)
        .frame(egui::Frame::new().fill(app.theme.panel))
        .resizable(false)
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                let ready =
                    app.bridge.valid() && !app.busy() && !crate::viewport_selection::sectioned(app);
                if app.sketch_active() {
                    crate::sketch_ui::tools(app, ui);
                    return;
                }
                if ui
                    .add_enabled(
                        ready && app.interaction.tool.is_none(),
                        egui::Button::new("New sketch (XY)"),
                    )
                    .clicked()
                {
                    app.new_sketch();
                }
                let selected = app.interaction.selection.is_some();
                if tool(
                    ui,
                    app,
                    "extrude",
                    "Extrude",
                    ready && selected && app.interaction.tool.is_none(),
                ) {
                    app.gesture += 1;
                    let step = app
                        .display
                        .as_ref()
                        .map_or(0, |display| display.projection.applied_ops as u32);
                    app.dispatch(Action::BeginExtrude {
                        record_id: onecad_core::ids::RecordId::new(),
                        gesture: app.gesture,
                        step_index: step,
                    });
                }
                if ui
                    .add_enabled(
                        ready && app.selected_body.is_some() && app.interaction.tool.is_none(),
                        egui::Button::new("Move / Rotate"),
                    )
                    .clicked()
                {
                    app.begin_transform();
                }
                let undo = app
                    .display
                    .as_ref()
                    .is_some_and(|display| display.projection.undo_depth > 0);
                let redo = app
                    .display
                    .as_ref()
                    .is_some_and(|display| display.projection.redo_depth > 0);
                if ui
                    .add_enabled(ready && undo, egui::Button::new("Undo"))
                    .clicked()
                {
                    app.submit(Command::Undo);
                }
                if ui
                    .add_enabled(ready && redo, egui::Button::new("Redo"))
                    .clicked()
                {
                    app.submit(Command::Redo);
                }
                if ui.button("Fit all").clicked() {
                    app.fit_pending = true;
                }
                if ui.button("Home view").clicked() {
                    app.camera = onecad_scene::Camera::default();
                    app.fit_pending = true;
                }
                crate::viewport_options::show(app, ui);
            })
        });
}

fn tool(ui: &mut egui::Ui, app: &App, name: &str, label: &str, enabled: bool) -> bool {
    ui.add_enabled_ui(enabled, |ui| {
        let response = ui.add(egui::Button::new(label).min_size(egui::vec2(90.0, 28.0)));
        let rect = egui::Rect::from_min_size(
            response.rect.min + egui::vec2(3.0, 6.0),
            egui::vec2(16.0, 16.0),
        );
        icons::paint(
            ui.painter(),
            rect,
            name,
            app.theme.ink3,
            app.theme.icon_accent,
            1.7,
        );
        response.on_hover_text(label).clicked()
    })
    .inner
}

fn status(app: &mut App, ui: &mut egui::Ui) {
    egui::Panel::bottom("status")
        .exact_size(34.0)
        .frame(
            egui::Frame::new()
                .fill(app.theme.statusbar)
                .inner_margin(egui::Margin::symmetric(14, 0)),
        )
        .resizable(false)
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.label(&app.status);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label("mm");
                    ui.label(
                        if app.camera.projection == onecad_scene::Projection::Orthographic {
                            "Ortho"
                        } else {
                            "Persp"
                        },
                    );
                });
            });
        });
}

fn tree(app: &mut App, ui: &mut egui::Ui) {
    egui::Panel::left("tree")
        .exact_size(220.0)
        .frame(
            egui::Frame::new()
                .fill(app.theme.panel)
                .inner_margin(egui::Margin::same(12)),
        )
        .resizable(false)
        .show(ui, |ui| {
            let Some(display) = app.display.clone() else {
                ui.label("Opening document…");
                return;
            };
            ui.label(egui::RichText::new("BODIES").small().color(app.theme.ink5));
            for body in display.projection.bodies.values() {
                let id = body.id.parse().ok();
                if ui
                    .selectable_label(id == app.selected_body, &body.name)
                    .clicked()
                {
                    app.selected_body = id;
                    app.topology.select(None);
                    app.draw_dirty = true;
                }
            }
            ui.add_space(16.0);
            ui.label(
                egui::RichText::new("SKETCHES")
                    .small()
                    .color(app.theme.ink5),
            );
            for sketch in display.projection.sketches.values() {
                if ui
                    .add_enabled(
                        app.bridge.valid() && !app.busy() && !app.sketch_active(),
                        egui::Button::new(&sketch.name),
                    )
                    .clicked()
                    && let Ok(id) = sketch.id.parse()
                {
                    app.edit_sketch(id);
                }
                if let Some(dof) = sketch.dof {
                    ui.label(
                        egui::RichText::new(format!("DOF: {dof}"))
                            .small()
                            .color(app.theme.ink5),
                    );
                }
            }
            ui.add_space(16.0);
            ui.label(
                egui::RichText::new("HISTORY")
                    .family(fonts::medium())
                    .small()
                    .color(app.theme.ink5),
            );
            for feature in &display.projection.features {
                let text = format!("{}  {}", feature.label, feature.value_text);
                let editable = display
                    .records
                    .iter()
                    .any(|record| record.record_id.to_string() == feature.id)
                    && app.bridge.valid()
                    && !app.busy()
                    && !app.sketch_active();
                if ui
                    .add_enabled(
                        editable,
                        egui::Button::new(text).selected(
                            Some(feature.id.as_str())
                                == app
                                    .selected_feature
                                    .as_ref()
                                    .map(|id| id.to_string())
                                    .as_deref(),
                        ),
                    )
                    .clicked()
                    && let Some(record) = display
                        .records
                        .iter()
                        .find(|record| record.record_id.to_string() == feature.id)
                    && let Some(context) = app.interaction.context.clone()
                {
                    app.gesture += 1;
                    app.selected_feature = Some(record.record_id);
                    app.dispatch(Action::BeginParameterEdit {
                        parameter: onecad_interaction::numeric_parameters(&record.op)[0].parameter,
                        record: Box::new(record.clone()),
                        gesture: app.gesture,
                        context,
                    });
                }
                if let Some(message) = &feature.status_message {
                    ui.colored_label(app.theme.traffic_close, message);
                }
            }
        });
}

fn inspector(app: &mut App, ui: &mut egui::Ui) {
    egui::Panel::right("inspector")
        .exact_size(259.0)
        .frame(
            egui::Frame::new()
                .fill(app.theme.panel)
                .inner_margin(egui::Margin::same(16)),
        )
        .resizable(false)
        .show(ui, |ui| {
            if app.sketch_active() {
                ui.heading("Edit sketch");
                ui.label("Choose Line, Rectangle or Circle. Click to place points; Enter finishes; Escape cancels a draft.");
                ui.label("Finish keeps sketch edits. Discard restores the entry state when safe.");
                return;
            }
            let Some(tool) = app.interaction.tool.clone() else {
                if let Some(id) = app.numeric_id.take() {
                    ui.memory_mut(|memory| memory.surrender_focus(id));
                }
                ui.heading(if app.interaction.selection.is_some() {
                    "Profile selected"
                } else {
                    "Nothing selected"
                });
                ui.label("Select a closed profile in the viewport, then choose Extrude.");
                return;
            };
            ui.heading(if tool.editing_existing {
                "Edit feature"
            } else {
                tool.record.name.as_str()
            });
            crate::parameter_ui::selector(app, ui, &tool);
            if app.interaction.tool.as_ref().is_none_or(|current| current.tag != tool.tag) { return; }

            ui.add_space(12.0);
            let field = format!(
                "{} ({})",
                tool.parameter.label(),
                if tool.parameter.site() == "angle" {
                    "deg"
                } else {
                    "mm"
                }
            );
            ui.label(&field);
            let mut draft = tool.draft.clone();
            let response = ui.add_enabled(
                tool.phase == SessionPhase::Editing,
                egui::TextEdit::singleline(&mut draft)
                    .id_salt((
                        "extrude-distance",
                        tool.tag.record_id.to_string(),
                        tool.tag.gesture,
                    ))
                    .font(egui::TextStyle::Monospace)
                    .desired_width(200.0)
                    .return_key(None),
            );
            app.numeric_id = Some(response.id);
            response.widget_info(|| {
                egui::WidgetInfo::text_edit(tool.phase == SessionPhase::Editing, "", &draft, &field)
            });
            if response.changed() {
                app.dispatch(Action::DraftChanged(draft));
            }
            if let DraftStatus::Invalid(error) = &tool.draft_status {
                ui.colored_label(app.theme.traffic_close, error);
            }
            ui.label(match &tool.preview {
                PreviewStatus::Empty => "",
                PreviewStatus::Pending => "Computing preview…",
                PreviewStatus::Ready => "Preview ready",
                PreviewStatus::NeedsRepair => "Preview needs repair",
                PreviewStatus::Failed(message) => message,
                PreviewStatus::UnavailableForEdit => "Geometry rebuilds when applied",
            });
            let can_apply = app.bridge.valid()
                && tool.phase == SessionPhase::Editing
                && matches!(tool.draft_status, DraftStatus::Valid(_))
                && matches!(
                    tool.preview,
                    PreviewStatus::Ready | PreviewStatus::UnavailableForEdit
                );
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        can_apply,
                        egui::Button::new(if tool.editing_existing {
                            "Apply"
                        } else {
                            "Create"
                        }),
                    )
                    .clicked()
                {
                    app.dispatch(Action::Confirm);
                }
                if ui
                    .add_enabled(
                        tool.phase == SessionPhase::Editing,
                        egui::Button::new("Cancel"),
                    )
                    .clicked()
                {
                    app.dispatch(Action::Cancel);
                }
            });
            if response.has_focus()
                && !app.composing
                && ui.input(|input| input.key_pressed(egui::Key::Enter))
                && can_apply
            {
                app.dispatch(Action::Confirm);
            }
            if response.has_focus()
                && !app.composing
                && ui.input(|input| input.key_pressed(egui::Key::Escape))
            {
                app.dispatch(Action::Cancel);
                response.surrender_focus();
            }
        });
}

pub fn close_prompt(app: &mut App, ctx: &egui::Context) {
    if !app.close_prompt {
        return;
    }
    if !app.authoring_dirty
        && app
            .display
            .as_ref()
            .is_some_and(|display| !display.projection.dirty)
        && !app.busy()
    {
        app.close_prompt = false;
        app.stop();
        return;
    }
    egui::Window::new("Save changes before closing?")
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            ui.label(
                "The QA document has unsaved changes. Save them before quitting, or discard them.",
            );
            ui.horizontal(|ui| {
                if ui.button("Save and quit").clicked() {
                    app.close_after_save = true;
                    app.close_prompt = false;
                    app.submit(Command::Save);
                }
                if ui.button("Discard and quit").clicked() {
                    app.close_prompt = false;
                    app.stop();
                }
                if ui.button("Cancel").clicked() {
                    app.close_prompt = false;
                }
            });
        });
}
