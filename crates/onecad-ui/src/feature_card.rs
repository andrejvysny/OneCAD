//! Running-feature card (README 05a): the numeric field, Create / Apply and Cancel for the
//! active model tool. Replaces the old Inspector form; dispatches the same actions.
use crate::{app::App, layout::Chrome, tool_registry::ToolId};
use egui::{Context, Order, Ui};
use onecad_interaction::{Action, DraftStatus, ModelSession, PreviewStatus, SessionPhase};
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
    Some((
        preview_text(&tool.preview),
        matches!(tool.preview, PreviewStatus::Ready),
    ))
}

fn can_apply(app: &App, tool: &ModelSession) -> bool {
    app.viewport_ready()
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
        app.dispatch(Action::Cancel);
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
        out = FeatureCard::new("feature-card", icon, title)
            .width(chrome.feature_card.width())
            .ok_enabled(apply)
            .ok_label(ok_label)
            .cancel_enabled(tool.phase == SessionPhase::Editing)
            .show(ui, |rows| card_rows(app, rows, tool, &mut response));
    });
    (response, out)
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
            .width(170.0)
            .error(error)
            .enabled(editing)
            .keep_focus_on_enter(true)
            .step_keys(!app.composing),
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
        app.dispatch(Action::Cancel);
        response.surrender_focus();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_text_variants() {
        assert_eq!(preview_text(&PreviewStatus::Empty), "");
        assert_eq!(preview_text(&PreviewStatus::Ready), "Preview ready");
        assert_eq!(preview_text(&PreviewStatus::Failed("boom".into())), "boom");
    }
}
