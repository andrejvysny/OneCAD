//! Sketch SessionStrip under the pill: carries Finish / Discard now that the sketch toolbar is gone.
use crate::{app::App, layout::Chrome, sketch_ui};
use egui::{Align2, Context, Order, Rect, pos2, vec2};
use onecad_interaction::sketch::SketchAction;
use onecad_widgets::{
    components::{
        button::ButtonKind,
        overlays::{SessionStrip, StripAction, StripVariant},
        tooltip::Tip,
    },
    frame,
};

pub fn show(app: &mut App, ctx: &Context, chrome: &Chrome) {
    let Some(session) = app.doc.sketch.session.as_ref() else {
        return;
    };
    let title = session.authoritative.name.clone();
    let feedback = crate::sketch_feedback::summary(&session.status);
    let exit = sketch_ui::can_exit(app);
    let color = crate::sketch_feedback::color(feedback.assessment, &app.theme);
    let lane = strip_bounds(chrome, app.ui.explorer_open, app.ui.inspector_open);
    let mode_key = egui::Id::unique("session-strip-compact");
    let compact = lane.width() < 600.0;
    let bounds = if ctx.data(|data| data.get_temp::<bool>(mode_key)) == Some(compact) {
        strip_rect(ctx, chrome, app.ui.explorer_open, app.ui.inspector_open)
    } else {
        ctx.data_mut(|data| data.insert_temp(mode_key, compact));
        lane
    };
    let mut clicked = None;
    frame::floating(egui::Id::unique("session-strip"), bounds.min, Order::Middle)
        .pivot(Align2::LEFT_TOP)
        .show(ctx, |ui| {
            clicked = bounded_strip(ui, lane.width(), |ui| {
                strip_contents(ui, lane.width(), &title, &feedback.label, color, exit)
            })
            .inner;
        });
    match clicked {
        Some(0) => app.sketch_dispatch(SketchAction::Discard),
        Some(1) => app.sketch_dispatch(SketchAction::Finish),
        _ => {}
    }
}

fn strip_contents(
    ui: &mut egui::Ui,
    width: f32,
    title: &str,
    status: &str,
    color: egui::Color32,
    enabled: bool,
) -> Option<usize> {
    let compact = width < 600.0;
    let actions = [
        StripAction {
            label: if compact {
                "Discard"
            } else {
                "Discard sketch edits"
            },
            kind: ButtonKind::Secondary,
            enabled,
            tip: Some(Tip::new("Discard sketch edits")),
        },
        StripAction {
            label: if compact { "Finish" } else { "Finish sketch" },
            kind: ButtonKind::Primary,
            enabled,
            tip: Some(Tip::new("Finish sketch").reason(Some(status))),
        },
    ];
    let strip = SessionStrip::new(StripVariant::Sketch).actions(&actions);
    let responses = if compact {
        strip.show_actions(ui)
    } else {
        strip
            .icon(Some("sketch"))
            .title(title)
            .status(Some((status, color, None)))
            .show_actions(ui)
    };
    let names = [
        crate::automation::DISCARD_SKETCH,
        crate::automation::FINISH_SKETCH,
    ];
    for (response, name) in responses.iter().zip(names) {
        crate::automation::name(ui.ctx(), response.id, name);
    }
    responses.iter().position(egui::Response::clicked)
}

fn strip_bounds(chrome: &Chrome, explorer: bool, inspector: bool) -> Rect {
    let left = if explorer {
        chrome.explorer.right() + 10.0
    } else {
        chrome.viewport.left() + 10.0
    };
    let right = if inspector {
        chrome.inspector.left() - 10.0
    } else {
        chrome.viewport.right() - 10.0
    };
    let upper_right = right.min(chrome.view_cluster.x - crate::layout::CUBE - 10.0);
    Rect::from_min_size(
        pos2(left, chrome.viewport.top() + 62.0),
        vec2((upper_right - left).max(120.0), 48.0),
    )
}

fn strip_rect(ctx: &Context, chrome: &Chrome, explorer: bool, inspector: bool) -> Rect {
    let bounds = strip_bounds(chrome, explorer, inspector);
    let width = ctx
        .data(|data| data.get_temp::<f32>(egui::Id::unique("session-strip-width")))
        .unwrap_or(bounds.width())
        .min(bounds.width());
    let left = (crate::pill::center_x(ctx, chrome) - width / 2.0)
        .clamp(bounds.left(), bounds.right() - width);
    Rect::from_min_size(pos2(left, bounds.top()), vec2(width, bounds.height()))
}

fn bounded_strip<R>(
    ui: &mut egui::Ui,
    width: f32,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::scroll_area::ScrollAreaOutput<R> {
    ui.set_max_width(width);
    let scroll = egui::ScrollArea::horizontal()
        .id_salt("session-strip-scroll")
        .max_width(width)
        .stick_to_right(true)
        .auto_shrink([true, true]);
    let output = scroll.show(ui, add);
    ui.ctx().data_mut(|data| {
        data.insert_temp(
            egui::Id::unique("session-strip-width"),
            output.content_size.x,
        )
    });
    output
}

#[cfg(test)]
mod tests;
