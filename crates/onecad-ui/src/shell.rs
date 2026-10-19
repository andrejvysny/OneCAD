//! Editor chrome orchestrator: title/status bars plus the floating panels and overlays.
use crate::{
    app::App, bars, bridge::Command, explorer, feature_card, inspector, layout, overlays, pill,
    session_strip,
};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    crate::ui_state::shortcuts(&mut app.ui, &ctx);
    let chrome = layout::chrome(ctx.content_rect(), &app.ui);
    bars::title(app, ui);
    if app.ui.start_open {
        return;
    }
    bars::status(app, ui);
    explorer::show(app, &ctx, &chrome);
    inspector::show(app, &ctx, &chrome);
    pill::show(app, &ctx, &chrome);
    feature_card::show(app, &ctx, &chrome);
    if app.sketch_active() {
        session_strip::show(app, &ctx, &chrome);
    }
    overlays::nav_pill(app, &ctx, &chrome);
    overlays::view_cluster(app, &ctx, &chrome);
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
