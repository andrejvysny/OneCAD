//! Sketch SessionStrip under the pill: carries Finish / Discard now that the sketch toolbar is gone.
use crate::{app::App, layout::Chrome, sketch_ui};
use egui::{Align2, Context, Order, pos2};
use onecad_interaction::sketch::{SketchAction, SketchStatus};
use onecad_widgets::{
    components::{
        button::ButtonKind,
        overlays::{SessionStrip, StripAction, StripVariant},
        tooltip::Tip,
    },
    frame,
};

fn dof(app: &App) -> Option<u32> {
    match &app.sketch.session.as_ref()?.status {
        SketchStatus::Entered(solve) => Some(solve.dof),
        SketchStatus::Solved(solve) => Some(solve.dof),
        _ => None,
    }
}

pub fn show(app: &mut App, ctx: &Context, chrome: &Chrome) {
    let Some(session) = app.sketch.session.as_ref() else {
        return;
    };
    let title = session.authoritative.name.clone();
    let status = dof(app).map(|n| format!("{n} DOF left"));
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
    let warn = app.theme.created_text;
    let pos = pos2(chrome.pill_centre_x, chrome.viewport.top() + 62.0);
    let mut clicked = None;
    frame::floating(egui::Id::unique("session-strip"), pos, Order::Middle)
        .pivot(Align2::CENTER_TOP)
        .show(ctx, |ui| {
            clicked = SessionStrip::new(StripVariant::Sketch)
                .icon(Some("sketch"))
                .title(&title)
                .status(
                    status
                        .as_deref()
                        .map(|s| (s, warn, Some("underConstrained"))),
                )
                .actions(&actions)
                .show(ui);
        });
    match clicked {
        Some(0) => app.sketch_dispatch(SketchAction::Discard),
        Some(1) => app.sketch_dispatch(SketchAction::Finish),
        _ => {}
    }
}
