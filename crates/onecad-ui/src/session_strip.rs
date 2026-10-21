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
    let Some(session) = app.sketch.session.as_ref() else {
        return;
    };
    let title = session.authoritative.name.clone();
    let feedback = crate::sketch_feedback::summary(&session.status);
    let exit = sketch_ui::can_exit(app);
    let actions = [
        StripAction {
            label: "Discard sketch edits",
            kind: ButtonKind::Secondary,
            enabled: exit,
            tip: Some(Tip::new("Discard sketch edits")),
        },
        StripAction {
            label: "Finish sketch",
            kind: ButtonKind::Primary,
            enabled: exit,
            tip: None,
        },
    ];
    let color = crate::sketch_feedback::color(feedback.assessment, &app.theme);
    let bounds = strip_rect(ctx, chrome, app.ui.explorer_open, app.ui.inspector_open);
    let mut clicked = None;
    frame::floating(egui::Id::unique("session-strip"), bounds.min, Order::Middle)
        .pivot(Align2::LEFT_TOP)
        .show(ctx, |ui| {
            clicked = bounded_strip(ui, bounds.width(), |ui| {
                SessionStrip::new(StripVariant::Sketch)
                    .icon(Some("sketch"))
                    .title(&title)
                    .status(Some((&feedback.label, color, None)))
                    .actions(&actions)
                    .show(ui)
            })
            .inner;
        });
    match clicked {
        Some(0) => app.sketch_dispatch(SketchAction::Discard),
        Some(1) => app.sketch_dispatch(SketchAction::Finish),
        _ => {}
    }
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
    let (top, right) = if upper_right - left >= 320.0 {
        (chrome.viewport.top() + 62.0, upper_right)
    } else {
        (
            (chrome.view_cluster.y + crate::layout::CUBE).max(chrome.view_controls.bottom()) + 10.0,
            right,
        )
    };
    Rect::from_min_size(pos2(left, top), vec2(right - left, 48.0))
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
