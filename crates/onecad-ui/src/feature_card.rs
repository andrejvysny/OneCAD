//! Running-feature card (README 05a): the numeric field, Create / Apply and Cancel for the
//! active model tool. Replaces the old Inspector form; dispatches the same actions.
use crate::{app::App, layout::Chrome};
use egui::{Context, Order, Ui};
use onecad_interaction::{
    Action, ModelSession, ParameterKey, PreviewStatus, Readiness, SessionPhase,
};
use onecad_widgets::{
    components::{
        inputs::NumberField,
        overlays::{FeatureCard, FeatureCardOutput, FeatureCardRows},
    },
    frame,
};

fn can_apply(app: &App, tool: &ModelSession) -> bool {
    !app.busy()
        && app.viewport_ready()
        && app.resize_refusal.is_none()
        && app.doc.view.is_some()
        && app.doc.bridge.valid()
        && tool.ready()
}

fn icon(app: &App) -> &'static str {
    app.doc.interaction.tool.as_ref().map_or("body", |tool| {
        crate::tool_registry::operation_icon(tool.record.op.op_type())
    })
}

fn choices(tool: &ModelSession) -> usize {
    if !tool.editing_existing || tool.phase != SessionPhase::Editing {
        return 0;
    }
    tool.params.active().count()
}

pub fn show(app: &mut App, ctx: &Context, chrome: &Chrome) {
    let Some(tool) = app.doc.interaction.tool.clone() else {
        if let Some(id) = app.doc.numeric_id.take() {
            ctx.memory_mut(|memory| memory.surrender_focus(id));
        }
        return;
    };
    if app.sketch_active() || tool.editing_existing {
        return;
    }
    let apply = can_apply(app, &tool);
    let editing = tool.phase == SessionPhase::Editing;
    let (response, out) = card(app, ctx, chrome, &tool, apply);
    name_footer(ctx, out.ok_id, out.cancel_id);
    if let Some(response) = &response {
        focus_keys(app, response, apply);
    }
    if out.ok && apply {
        app.dispatch(Action::Confirm);
    }
    if (out.cancel || out.close) && editing {
        crate::pill::cancel_tool(app);
    }
}

pub(crate) fn history_editor(app: &mut App, ui: &mut Ui, record_id: &str) {
    let Some(tool) = app
        .doc
        .interaction
        .tool
        .clone()
        .filter(|tool| tool.editing_existing && tool.tag.record_id.to_string() == record_id)
    else {
        return;
    };
    ui.push_id(("history-editor", tool.tag.record_id.to_string()), |ui| {
        ui.group(|ui| {
            ui.set_width(ui.available_width());
            ui.strong(format!("Edit {}", tool.record.name));
            FeatureCardRows::inline(ui, |rows| crate::modeling_ui::rows(app, rows, &tool));
            if choices(&tool) > 1 {
                ui.label("Parameter");
                crate::parameter_ui::selector(app, ui, &tool);
            }
            let current = app.doc.interaction.tool.clone().unwrap_or(tool.clone());
            let apply = can_apply(app, &current);
            if let Some(key) = current.params.focus() {
                ui.label(key.label());
                let response = number_field(app, ui, &current, key);
                focus_keys(app, &response, apply);
            }
            if let Some(message) = apply_feedback(app, &current) {
                ui.add(egui::Label::new(message).wrap().selectable(true));
            }
            ui.horizontal(|ui| {
                let confirm = ui.add_enabled(apply, egui::Button::new("Apply ↵"));
                let cancel = ui.add_enabled(
                    current.phase == SessionPhase::Editing,
                    egui::Button::new("Cancel"),
                );
                name_footer(ui.ctx(), Some(confirm.id), Some(cancel.id));
                if confirm.clicked() {
                    app.dispatch(Action::Confirm);
                }
                if cancel.clicked() {
                    crate::pill::cancel_tool(app);
                }
            });
        });
    });
}

/// The running operation's confirm and cancel, wherever they are drawn.
pub(crate) fn name_footer(ctx: &Context, confirm: Option<egui::Id>, cancel: Option<egui::Id>) {
    use crate::automation::{OP_CANCEL, OP_CONFIRM, name};
    for (id, automation) in [(confirm, OP_CONFIRM), (cancel, OP_CANCEL)] {
        if let Some(id) = id {
            name(ctx, id, automation);
        }
    }
}

/// The first declared field is the operation's primary value entry.
pub(crate) fn field_automation(tool: &ModelSession, key: ParameterKey) -> String {
    if tool.params.active().next().map(|field| field.key) == Some(key) {
        crate::automation::DIMENSION_INPUT.to_owned()
    } else {
        crate::automation::param(key.parameter())
    }
}

fn card(
    app: &mut App,
    ctx: &Context,
    chrome: &Chrome,
    tool: &ModelSession,
    apply: bool,
) -> (Option<egui::Response>, FeatureCardOutput) {
    let title = if tool.editing_existing {
        format!(
            "Edit {}",
            crate::tool_registry::running_definition(app)
                .map_or(tool.record.op.op_type(), |definition| definition.label)
        )
    } else {
        tool.record.name.clone()
    };
    let ok_label = if tool.editing_existing {
        "Apply ↵"
    } else {
        "Create ↵"
    };
    let icon = icon(app);
    let mut response = None;
    let mut out = FeatureCardOutput::default();
    frame::floating(
        egui::Id::unique("feature-card"),
        chrome.feature_card.min,
        Order::Middle,
    )
    .show(ctx, |ui| {
        card_scroll(ui, chrome.feature_card.height(), |ui| {
            out = FeatureCard::new("feature-card", icon, &title)
                .width(chrome.feature_card.width())
                .ok_enabled(apply)
                .ok_label(ok_label)
                .cancel_enabled(tool.phase == SessionPhase::Editing)
                .show(ui, |rows| card_rows(app, rows, tool, &mut response));
        });
    });
    (response, out)
}

fn card_scroll<R>(
    ui: &mut Ui,
    height: f32,
    add: impl FnOnce(&mut Ui) -> R,
) -> egui::scroll_area::ScrollAreaOutput<R> {
    // Area remembers the previous tool's height; the current chrome bounds own admission.
    ui.set_max_height(height);
    egui::ScrollArea::vertical()
        .id_salt("feature-card-scroll")
        .max_height(height)
        .auto_shrink([false, true])
        .show(ui, add)
}

fn card_rows(
    app: &mut App,
    rows: &mut FeatureCardRows,
    tool: &ModelSession,
    response: &mut Option<egui::Response>,
) {
    crate::extrude_ui::rows(app, rows, tool);
    crate::modeling_ui::rows(app, rows, tool);
    if choices(tool) > 1 {
        rows.row("Parameter", |ui| {
            crate::parameter_ui::selector(app, ui, tool);
        });
    }
    let current = app
        .doc
        .interaction
        .tool
        .as_ref()
        .is_some_and(|now| now.tag == tool.tag);
    if current {
        // One row per active declared field; the keyboard follows the focused one.
        for key in tool.params.active().map(|field| field.key) {
            rows.row(key.label(), |ui| {
                let field = number_field(app, ui, tool, key);
                if field.has_focus() || response.is_none() && tool.params.focus() == Some(key) {
                    *response = Some(field);
                }
            });
        }
    }
    if let Some(message) = apply_feedback(app, tool) {
        rows.row("Status", |ui| {
            ui.add(egui::Label::new(message).wrap().selectable(true));
        });
    }
}

fn apply_feedback(app: &App, tool: &ModelSession) -> Option<String> {
    if !app.viewport_ready() {
        return Some(format!(
            "Viewport unavailable: {}",
            app.viewport_failure.as_deref().unwrap_or("unknown error")
        ));
    }
    if let Some((_, reason)) = &app.resize_refusal {
        return Some(format!("Viewport unavailable: {reason}"));
    }
    if !app.doc.bridge.valid() {
        return Some("Worker geometry unavailable; Create and Apply are disabled".into());
    }
    if app.doc.view.is_none() {
        return Some("Waiting for an accepted viewport capture".into());
    }
    if app.busy() && tool.phase == SessionPhase::Editing {
        return Some("Wait for the current command before applying".into());
    }
    preview_feedback(&tool.params.readiness(), &tool.preview, tool.phase)
}

fn preview_feedback(
    readiness: &Readiness,
    preview: &PreviewStatus,
    phase: SessionPhase,
) -> Option<String> {
    match phase {
        SessionPhase::Committing => return Some("Submitting feature…".into()),
        SessionPhase::Accepted => return Some("Feature accepted; regenerating geometry…".into()),
        SessionPhase::Completed => return Some("Feature command completed".into()),
        SessionPhase::Editing => {}
    }
    match readiness {
        Readiness::Pending => return Some("Evaluating expression…".into()),
        Readiness::Invalid { message, .. } => return Some(format!("Invalid value: {message}")),
        Readiness::Ready => {}
    }
    match preview {
        PreviewStatus::Empty => Some("Waiting for a preview".into()),
        PreviewStatus::Pending => Some("Building preview…".into()),
        PreviewStatus::Failed(error) => Some(format!("Preview failed: {error}")),
        PreviewStatus::NeedsRepair => Some("Preview needs repair; Create is disabled".into()),
        PreviewStatus::UnavailableForEdit => {
            Some("Edit preview unavailable. Apply regenerates the feature.".into())
        }
        PreviewStatus::Ready => None,
    }
}

fn number_field(
    app: &mut App,
    ui: &mut Ui,
    tool: &ModelSession,
    key: ParameterKey,
) -> egui::Response {
    let unit = if key.site() == "angle" { "deg" } else { "mm" };
    let editing = tool.phase == SessionPhase::Editing;
    let mut draft = tool
        .params
        .field(key)
        .map(|field| field.text.clone())
        .unwrap_or_default();
    let error = tool.params.error(key);
    let salt = (key.salt(), tool.tag.record_id.to_string(), tool.tag.gesture);
    let response = ui.add(
        NumberField::new(salt, &mut draft)
            .unit(unit)
            .error(error)
            .enabled(editing)
            .keep_focus_on_enter(true)
            .step_keys(!app.composing)
            .width(ui.available_width()),
    );
    let focused = tool.params.focus() == Some(key);
    if focused || response.has_focus() {
        app.doc.numeric_id = Some(response.id);
    }
    let label = format!("{} ({unit})", key.label());
    response.widget_info(|| field_info(editing, &draft, &label));
    crate::automation::name(ui.ctx(), response.id, &field_automation(tool, key));
    if response.gained_focus() && !focused {
        app.dispatch(Action::FocusParameter(key));
    }
    if response.changed() {
        app.dispatch(Action::ParameterChanged { key, text: draft });
    }
    response
}

/// The row label is painted beside the field, so assistive tech (and touch
/// automation) gets the name only from the field itself. `text_edit`'s last
/// argument is a placeholder, not a name.
fn field_info(enabled: bool, text: &str, label: &str) -> egui::WidgetInfo {
    let mut info = egui::WidgetInfo::text_edit(enabled, text, text, "");
    info.label = Some(label.to_owned());
    info
}

fn focus_keys(app: &mut App, response: &egui::Response, can_apply: bool) {
    let ctx = response.ctx.clone();
    if crate::input::overlay_owns_keyboard(app, &ctx) {
        return;
    }
    if response.has_focus()
        && !app.composing
        && ctx.input(|input| input.key_pressed(egui::Key::Enter))
        && can_apply
    {
        app.dispatch(Action::Confirm);
    }
    if response.has_focus()
        && !app.composing
        && ctx.input(|input| input.key_pressed(egui::Key::Escape))
    {
        crate::pill::cancel_tool(app);
        response.surrender_focus();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_blocked_preview_state_explains_why_create_is_disabled() {
        let valid = Readiness::Ready;
        for (preview, expected) in [
            (PreviewStatus::Empty, "Waiting for a preview"),
            (PreviewStatus::Pending, "Building preview…"),
            (
                PreviewStatus::NeedsRepair,
                "Preview needs repair; Create is disabled",
            ),
            (
                PreviewStatus::Failed("worker disconnected".into()),
                "Preview failed: worker disconnected",
            ),
        ] {
            assert_eq!(
                preview_feedback(&valid, &preview, SessionPhase::Editing).as_deref(),
                Some(expected)
            );
        }
        assert!(preview_feedback(&valid, &PreviewStatus::Ready, SessionPhase::Editing).is_none());
        assert_eq!(
            preview_feedback(
                &Readiness::Invalid {
                    key: None,
                    message: "unknown variable".into()
                },
                &PreviewStatus::Ready,
                SessionPhase::Editing
            )
            .as_deref(),
            Some("Invalid value: unknown variable")
        );
    }

    #[test]
    fn parameter_field_is_named_for_accessibility() {
        let ctx = Context::default();
        ctx.enable_accesskit();
        let mut text = String::from("10");
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let response = ui.add(egui::TextEdit::singleline(&mut text));
            response.widget_info(|| field_info(true, "10", "Extrude distance (mm)"));
        });
        output.textures_delta.clear();
        let update = output
            .platform_output
            .accesskit_update
            .expect("accesskit enabled");
        let field = update
            .nodes
            .iter()
            .map(|(_, node)| node)
            .find(|node| node.role() == egui::accesskit::Role::TextInput)
            .expect("text input node");
        assert_eq!(field.label(), Some("Extrude distance (mm)"));
        assert_eq!(field.value(), Some("10"));
    }

    #[test]
    fn floating_card_can_grow_after_switching_to_parameter_edit() {
        for scale in [1.0, 2.0] {
            let ctx = Context::default();
            onecad_widgets::theme::install(
                &ctx,
                onecad_widgets::theme::load(false).unwrap(),
                false,
            );
            ctx.set_pixels_per_point(scale);
            for (index, (row_count, height)) in [
                (1, 296.0),
                (1, 296.0),
                (1, 296.0),
                (2, 296.0),
                (2, 296.0),
                (2, 296.0),
                (2, 96.0),
                (2, 96.0),
                (2, 296.0),
                (2, 296.0),
                // One row per declared field: a selector plus three fields keeps the footer.
                (4, 296.0),
                (4, 296.0),
            ]
            .into_iter()
            .enumerate()
            {
                let mut frame = card_frame(&ctx, row_count, height);
                frame.textures_delta.clear();
                if height >= 296.0 {
                    assert_card_footer_visible(&frame, index == 0);
                }
            }
        }
    }

    fn card_frame(ctx: &Context, row_count: usize, height: f32) -> egui::FullOutput {
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 820.0),
                )),
                ..Default::default()
            },
            |_| {
                frame::floating(
                    egui::Id::unique("card-growth"),
                    egui::pos2(120.0, 80.0),
                    Order::Middle,
                )
                .show(ctx, |ui| {
                    let scroll = card_scroll(ui, height, |ui| {
                        FeatureCard::new("growth", "extrude", "Edit feature")
                            .width(280.0)
                            .ok_label("Apply ↵")
                            .show(ui, |rows| {
                                for index in 0..row_count {
                                    rows.row(
                                        if index == 0 { "Parameter" } else { "Distance" },
                                        |ui| {
                                            ui.add_sized(
                                                egui::vec2(130.0, 30.0),
                                                egui::Label::new("8.00 mm"),
                                            );
                                        },
                                    );
                                }
                            });
                    });
                    assert!(scroll.inner_rect.height() <= height + 1.0);
                    if height >= 296.0 {
                        assert!(
                            scroll.content_size.y <= scroll.inner_rect.height() + 1.0,
                            "{} content clipped to {}",
                            scroll.content_size.y,
                            scroll.inner_rect.height()
                        );
                    } else {
                        assert!(scroll.content_size.y > scroll.inner_rect.height());
                    }
                });
            },
        )
    }

    fn assert_card_footer_visible(frame: &egui::FullOutput, sizing_pass: bool) {
        let mut footer_count = 0;
        for shape in &frame.shapes {
            if let egui::epaint::Shape::Text(text) = &shape.shape
                && matches!(text.galley.job.text.as_str(), "Apply ↵" | "Cancel")
            {
                footer_count += 1;
                let bounds = egui::Rect::from_min_size(text.pos, text.galley.size());
                assert!(
                    shape.clip_rect.contains_rect(bounds),
                    "{bounds:?} outside {:?}",
                    shape.clip_rect
                );
            }
        }
        // The initial Area sizing pass may emit no shapes.
        assert!(footer_count == 2 || sizing_pass && footer_count == 0);
    }
}
