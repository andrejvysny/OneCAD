//! Running-feature card (README 05a): the numeric field, Create / Apply and Cancel for the
//! active model tool. Replaces the old Inspector form; dispatches the same actions.
use crate::{app::App, layout::Chrome, tool_registry::ToolId};
use egui::{Context, Order, Ui};
use onecad_interaction::{
    Action, CommitStatus, DraftStatus, ModelSession, PreviewStatus, SessionPhase,
};
use onecad_widgets::{
    components::{
        inputs::NumberField,
        overlays::{FeatureCard, FeatureCardOutput, FeatureCardRows},
    },
    frame,
};

pub fn preview_text(preview: &PreviewStatus) -> &str {
    match preview {
        PreviewStatus::Empty => "",
        PreviewStatus::Pending => "Computing preview…",
        PreviewStatus::Ready => "Preview ready",
        PreviewStatus::NeedsRepair => "Preview needs repair",
        PreviewStatus::Failed(message) => message,
        PreviewStatus::UnavailableForEdit => "Geometry rebuilds when applied",
    }
}

/// Status-bar text for the running tool and whether the preview is ready (green dot).
pub fn status_line(app: &App) -> Option<(&str, bool)> {
    let tool = app.interaction.tool.as_ref()?;
    tool_status(&app.interaction.status, &tool.preview, app.command_status)
}

fn tool_status<'a>(
    status: &'a CommitStatus,
    preview: &'a PreviewStatus,
    command_status: bool,
) -> Option<(&'a str, bool)> {
    if command_status {
        return None;
    }
    match status {
        CommitStatus::Rejected(message) | CommitStatus::CompletionUnavailable { message, .. } => {
            Some((message, false))
        }
        CommitStatus::Pending(_) => Some(("Submitting feature…", false)),
        CommitStatus::Accepted { .. } => Some(("Regenerating feature…", false)),
        _ => Some((
            preview_text(preview),
            matches!(preview, PreviewStatus::Ready),
        )),
    }
}

fn can_apply(app: &App, tool: &ModelSession) -> bool {
    !app.busy()
        && app.viewport_ready()
        && app.resize_refusal.is_none()
        && app.view.is_some()
        && app.bridge.valid()
        && tool.phase == SessionPhase::Editing
        && matches!(tool.draft_status, DraftStatus::Valid(_))
        && matches!(
            tool.preview,
            PreviewStatus::Ready | PreviewStatus::UnavailableForEdit
        )
}

fn icon(app: &App) -> &'static str {
    match crate::tool_registry::running_tool(app) {
        Some(ToolId::MoveRotate) => "move",
        _ => "extrude",
    }
}

fn choices(app: &App, tool: &ModelSession) -> usize {
    if !tool.editing_existing || tool.phase != SessionPhase::Editing {
        return 0;
    }
    app.display
        .as_ref()
        .and_then(|display| {
            display
                .records
                .iter()
                .find(|record| record.record_id == tool.tag.record_id)
        })
        .map_or(0, |record| {
            onecad_interaction::numeric_parameters(&record.op).len()
        })
}

pub fn show(app: &mut App, ctx: &Context, chrome: &Chrome) {
    let Some(tool) = app.interaction.tool.clone() else {
        if let Some(id) = app.numeric_id.take() {
            ctx.memory_mut(|memory| memory.surrender_focus(id));
        }
        return;
    };
    if app.sketch_active() {
        return;
    }
    let apply = can_apply(app, &tool);
    let editing = tool.phase == SessionPhase::Editing;
    let (response, out) = card(app, ctx, chrome, &tool, apply);
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

fn card(
    app: &mut App,
    ctx: &Context,
    chrome: &Chrome,
    tool: &ModelSession,
    apply: bool,
) -> (Option<egui::Response>, FeatureCardOutput) {
    let title = if tool.editing_existing {
        "Edit feature"
    } else {
        tool.record.name.as_str()
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
            out = FeatureCard::new("feature-card", icon, title)
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
    if choices(app, tool) > 1 {
        rows.row("Parameter", |ui| {
            crate::parameter_ui::selector(app, ui, tool);
        });
    }
    let current = app
        .interaction
        .tool
        .as_ref()
        .is_some_and(|now| now.tag == tool.tag);
    if current {
        rows.row(tool.parameter.label(), |ui| {
            *response = Some(number_field(app, ui, tool));
        });
    }
}

fn number_field(app: &mut App, ui: &mut Ui, tool: &ModelSession) -> egui::Response {
    let unit = if tool.parameter.site() == "angle" {
        "deg"
    } else {
        "mm"
    };
    let editing = tool.phase == SessionPhase::Editing;
    let mut draft = tool.draft.clone();
    let error = match &tool.draft_status {
        DraftStatus::Invalid(error) => Some(error.as_str()),
        _ => None,
    };
    let salt = (
        "extrude-distance",
        tool.tag.record_id.to_string(),
        tool.tag.gesture,
    );
    let response = ui.add(
        NumberField::new(salt, &mut draft)
            .unit(unit)
            .error(error)
            .enabled(editing)
            .keep_focus_on_enter(true)
            .step_keys(!app.composing)
            .width(ui.available_width()),
    );
    app.numeric_id = Some(response.id);
    let label = format!("{} ({unit})", tool.parameter.label());
    response.widget_info(|| egui::WidgetInfo::text_edit(editing, "", &draft, &label));
    if response.changed() {
        app.dispatch(Action::DraftChanged(draft));
    }
    response
}

fn focus_keys(app: &mut App, response: &egui::Response, can_apply: bool) {
    let ctx = response.ctx.clone();
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

    #[test]
    fn preview_text_variants() {
        assert_eq!(preview_text(&PreviewStatus::Empty), "");
        assert_eq!(preview_text(&PreviewStatus::Ready), "Preview ready");
        assert_eq!(preview_text(&PreviewStatus::Failed("boom".into())), "boom");
    }

    #[test]
    fn rejected_authoring_and_save_status_override_ready_preview() {
        for message in ["Create rejected", "Apply rejected"] {
            let status = CommitStatus::Rejected(message.into());
            assert_eq!(
                tool_status(&status, &PreviewStatus::Ready, false),
                Some((message, false))
            );
            assert_eq!(tool_status(&status, &PreviewStatus::Ready, true), None);
        }
        // Save lifecycle and errors are displayed from the host command status.
        assert_eq!(
            tool_status(&CommitStatus::Idle, &PreviewStatus::Ready, true),
            None
        );
        assert_eq!(
            tool_status(&CommitStatus::Idle, &PreviewStatus::Ready, false),
            Some(("Preview ready", true))
        );
    }
}
