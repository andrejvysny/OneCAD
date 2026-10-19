//! Sketch SessionStrip under the pill: carries Finish / Discard now that the sketch toolbar is gone.
use crate::{app::App, layout::Chrome, sketch_ui};
use egui::{Align2, Context, Order, pos2};
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
            label: "Cancel",
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
    let pos = pos2(chrome.pill_centre_x, chrome.viewport.top() + 62.0);
    let mut clicked = None;
    frame::floating(egui::Id::unique("session-strip"), pos, Order::Middle)
        .pivot(Align2::CENTER_TOP)
        .show(ctx, |ui| {
            clicked = SessionStrip::new(StripVariant::Sketch)
                .icon(Some("sketch"))
                .title(&title)
                .status(Some((&feedback.label, color, None)))
                .actions(&actions)
                .show(ui);
        });
    match clicked {
        Some(0) => app.sketch_dispatch(SketchAction::Discard),
        Some(1) => app.sketch_dispatch(SketchAction::Finish),
        _ => {}
    }
}
